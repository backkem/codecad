//! Bounding volume hierarchies for the path tracer, built on the CPU.
//!
//! One builder serves both levels: a BVH over a mesh's triangles in the
//! mesh's own frame (shared by every item that draws the mesh) and a BVH
//! over the placed items. It is a binned surface-area-heuristic build:
//! at every node the primitives are sorted into [`BINS`] buckets by
//! centroid along each axis, and the split with the lowest estimated cost
//! wins, or the node becomes a leaf when splitting does not pay.
//!
//! The layout is the one `trace.wgsl` reads: 32-byte nodes, the two
//! children of an inner node next to each other, a leaf naming a run of
//! primitives in [`Bvh::order`]. The build is deterministic (ties go to
//! the lower axis and bin, partitions are stable) and its depth is capped
//! at [`MAX_DEPTH`], so the shader's fixed traversal stack never overflows.

/// Buckets per axis in the binned build.
pub const BINS: usize = 16;
/// Deepest node the build makes; the shader's stack holds `MAX_DEPTH + 2`.
pub const MAX_DEPTH: usize = 40;
/// A node with at most this many primitives may become a leaf.
pub const MAX_LEAF: usize = 4;
/// Cost of visiting a node relative to testing one primitive.
const TRAVERSAL_COST: f64 = 1.0;

/// An axis-aligned box in f64.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub lo: [f64; 3],
    pub hi: [f64; 3],
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb {
        lo: [f64::INFINITY; 3],
        hi: [f64::NEG_INFINITY; 3],
    };

    pub fn grow(&mut self, o: &Aabb) {
        for k in 0..3 {
            self.lo[k] = self.lo[k].min(o.lo[k]);
            self.hi[k] = self.hi[k].max(o.hi[k]);
        }
    }

    pub fn grow_point(&mut self, p: [f64; 3]) {
        self.grow(&Aabb { lo: p, hi: p });
    }

    pub fn is_empty(&self) -> bool {
        (0..3).any(|k| {
            self.lo[k]
                .partial_cmp(&self.hi[k])
                .is_none_or(|o| o.is_gt())
        })
    }

    /// Half the surface area (enough for the heuristic's ratios).
    pub fn half_area(&self) -> f64 {
        if self.is_empty() {
            return 0.0;
        }
        let d: [f64; 3] = std::array::from_fn(|k| self.hi[k] - self.lo[k]);
        d[0] * d[1] + d[1] * d[2] + d[2] * d[0]
    }

    pub fn centre(&self) -> [f64; 3] {
        std::array::from_fn(|k| 0.5 * (self.lo[k] + self.hi[k]))
    }
}

/// One node as the shader reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub lo: [f32; 3],
    /// Inner node: index of the left child (the right one follows it).
    /// Leaf: index of the first primitive in [`Bvh::order`].
    pub first: u32,
    pub hi: [f32; 3],
    /// Primitives in a leaf; 0 for an inner node.
    pub count: u32,
}

/// A built hierarchy.
#[derive(Debug, Clone, Default)]
pub struct Bvh {
    /// Root first.
    pub nodes: Vec<Node>,
    /// Primitive indices in leaf order.
    pub order: Vec<u32>,
    /// Deepest leaf (root = 0).
    pub depth: usize,
}

/// The largest f32 not above `v`.
fn down(v: f64) -> f32 {
    let f = v as f32;
    if (f as f64) > v {
        f.next_down()
    } else {
        f
    }
}

/// The smallest f32 not below `v`.
fn up(v: f64) -> f32 {
    let f = v as f32;
    if (f as f64) < v {
        f.next_up()
    } else {
        f
    }
}

/// `b` as f32, rounded outward, so it still encloses what it enclosed.
pub fn node_box(b: &Aabb) -> ([f32; 3], [f32; 3]) {
    (b.lo.map(down), b.hi.map(up))
}

struct Builder<'a> {
    boxes: &'a [Aabb],
    max_leaf: usize,
    centres: Vec<[f64; 3]>,
    nodes: Vec<Node>,
    order: Vec<u32>,
    depth: usize,
}

impl Bvh {
    /// Build over primitives with these boxes. An empty input gives one
    /// empty leaf.
    pub fn build(boxes: &[Aabb]) -> Bvh {
        Bvh::build_with(boxes, MAX_LEAF)
    }

    /// Build with leaves of at most `max_leaf` primitives (at least 1), as
    /// far as the depth cap allows: 1 for the level over placed items,
    /// whose leaves the shader reads as one item each.
    pub fn build_with(boxes: &[Aabb], max_leaf: usize) -> Bvh {
        let mut b = Builder {
            boxes,
            max_leaf: max_leaf.max(1),
            centres: boxes.iter().map(Aabb::centre).collect(),
            nodes: Vec::with_capacity(2 * boxes.len().max(1)),
            order: (0..boxes.len() as u32).collect(),
            depth: 0,
        };
        b.nodes.push(Node {
            lo: [0.0; 3],
            first: 0,
            hi: [0.0; 3],
            count: 0,
        });
        b.split(0, 0, boxes.len(), 0);
        Bvh {
            nodes: b.nodes,
            order: b.order,
            depth: b.depth,
        }
    }
}

impl Builder<'_> {
    fn bounds(&self, start: usize, end: usize) -> (Aabb, Aabb) {
        let mut all = Aabb::EMPTY;
        let mut cen = Aabb::EMPTY;
        for &i in &self.order[start..end] {
            all.grow(&self.boxes[i as usize]);
            cen.grow_point(self.centres[i as usize]);
        }
        (all, cen)
    }

    fn leaf(&mut self, node: usize, all: &Aabb, start: usize, end: usize, depth: usize) {
        let (lo, hi) = if all.is_empty() {
            ([0.0; 3], [-1.0; 3])
        } else {
            node_box(all)
        };
        self.nodes[node] = Node {
            lo,
            first: start as u32,
            hi,
            count: (end - start) as u32,
        };
        self.depth = self.depth.max(depth);
    }

    fn split(&mut self, node: usize, start: usize, end: usize, depth: usize) {
        let n = end - start;
        let (all, cen) = self.bounds(start, end);
        if n <= 1 || depth >= MAX_DEPTH {
            return self.leaf(node, &all, start, end, depth);
        }
        // Best binned split over the three axes.
        let mut best: Option<(f64, usize, usize)> = None; // (cost, axis, bin)
        for axis in 0..3 {
            let (lo, hi) = (cen.lo[axis], cen.hi[axis]);
            // Also skips NaN.
            if hi.partial_cmp(&lo) != Some(std::cmp::Ordering::Greater) {
                continue;
            }
            let scale = BINS as f64 / (hi - lo);
            let bin_of = |c: f64| (((c - lo) * scale) as usize).min(BINS - 1);
            let mut bins = [(Aabb::EMPTY, 0usize); BINS];
            for &i in &self.order[start..end] {
                let b = &mut bins[bin_of(self.centres[i as usize][axis])];
                b.0.grow(&self.boxes[i as usize]);
                b.1 += 1;
            }
            // Sweep from the right, then from the left.
            let mut right = [(0.0f64, 0usize); BINS];
            let mut acc = (Aabb::EMPTY, 0usize);
            for k in (1..BINS).rev() {
                acc.0.grow(&bins[k].0);
                acc.1 += bins[k].1;
                right[k] = (acc.0.half_area(), acc.1);
            }
            let mut left = (Aabb::EMPTY, 0usize);
            for k in 0..BINS - 1 {
                left.0.grow(&bins[k].0);
                left.1 += bins[k].1;
                let (ra, rn) = right[k + 1];
                if left.1 == 0 || rn == 0 {
                    continue;
                }
                let cost = left.0.half_area() * left.1 as f64 + ra * rn as f64;
                if best.is_none_or(|(c, _, _)| cost < c) {
                    best = Some((cost, axis, k));
                }
            }
        }
        let area = all.half_area().max(1e-300);
        let split = match best {
            Some((cost, axis, bin)) => {
                let split_cost = TRAVERSAL_COST + cost / area;
                if n <= self.max_leaf && split_cost >= n as f64 {
                    None
                } else {
                    Some((axis, bin))
                }
            }
            None if n <= self.max_leaf => None,
            // Every centroid in one place: halve the run.
            None => Some((usize::MAX, 0)),
        };
        let Some((axis, bin)) = split else {
            return self.leaf(node, &all, start, end, depth);
        };
        let mid = if axis == usize::MAX {
            start + n / 2
        } else {
            let (lo, hi) = (cen.lo[axis], cen.hi[axis]);
            let scale = BINS as f64 / (hi - lo);
            let centres = &self.centres;
            let goes_left = |i: &u32| {
                ((((centres[*i as usize][axis] - lo) * scale) as usize).min(BINS - 1)) <= bin
            };
            // Stable partition.
            let (l, r): (Vec<u32>, Vec<u32>) =
                self.order[start..end].iter().partition(|i| goes_left(i));
            let m = start + l.len();
            self.order[start..m].copy_from_slice(&l);
            self.order[m..end].copy_from_slice(&r);
            m
        };
        let left = self.nodes.len();
        let blank = Node {
            lo: [0.0; 3],
            first: 0,
            hi: [0.0; 3],
            count: 0,
        };
        self.nodes.push(blank);
        self.nodes.push(blank);
        let (lo, hi) = node_box(&all);
        self.nodes[node] = Node {
            lo,
            first: left as u32,
            hi,
            count: 0,
        };
        self.split(left, start, mid, depth + 1);
        self.split(left + 1, mid, end, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri_box(p: [[f64; 3]; 3]) -> Aabb {
        let mut b = Aabb::EMPTY;
        for q in p {
            b.grow_point(q);
        }
        b
    }

    /// Every primitive appears in exactly one leaf, every node encloses its
    /// children, and the depth stays within the cap.
    #[test]
    fn hierarchy_is_complete_and_nested() {
        let mut boxes = Vec::new();
        // A grid of small triangles plus a few long ones.
        for i in 0..40 {
            for j in 0..25 {
                let (x, y) = (i as f64, j as f64 * 1.3);
                boxes.push(tri_box([[x, y, 0.0], [x + 0.9, y, 0.1], [x, y + 0.9, 0.2]]));
            }
        }
        for k in 0..10 {
            let y = k as f64 * 3.0;
            boxes.push(tri_box([
                [-5.0, y, 1.0],
                [45.0, y, 1.0],
                [-5.0, y + 0.1, 1.0],
            ]));
        }
        let bvh = Bvh::build(&boxes);
        let mut seen = vec![0u32; boxes.len()];
        let mut stack = vec![(0usize, 0usize)];
        while let Some((i, d)) = stack.pop() {
            let n = bvh.nodes[i];
            assert!(d <= MAX_DEPTH);
            if n.count > 0 {
                for k in n.first..n.first + n.count {
                    let p = bvh.order[k as usize] as usize;
                    seen[p] += 1;
                    let b = boxes[p];
                    for a in 0..3 {
                        assert!(n.lo[a] as f64 <= b.lo[a] && n.hi[a] as f64 >= b.hi[a]);
                    }
                }
            } else {
                for c in [n.first as usize, n.first as usize + 1] {
                    let m = bvh.nodes[c];
                    for a in 0..3 {
                        assert!(n.lo[a] <= m.lo[a] && n.hi[a] >= m.hi[a]);
                    }
                    stack.push((c, d + 1));
                }
            }
        }
        assert!(seen.iter().all(|&s| s == 1));
        assert!(bvh.depth < 30, "depth {}", bvh.depth);
        // Deterministic.
        let again = Bvh::build(&boxes);
        assert_eq!(again.nodes, bvh.nodes);
        assert_eq!(again.order, bvh.order);
    }

    #[test]
    fn degenerate_inputs() {
        let empty = Bvh::build(&[]);
        assert_eq!(empty.nodes.len(), 1);
        assert_eq!(empty.nodes[0].count, 0);
        // A thousand identical boxes: split by halves, depth stays bounded.
        let same = vec![tri_box([[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]); 1000];
        let bvh = Bvh::build(&same);
        assert!(bvh.depth <= MAX_DEPTH);
        let total: u32 = bvh.nodes.iter().map(|n| n.count).sum();
        assert_eq!(total, 1000);
        // Leaves of one: every leaf holds exactly one primitive, also for
        // boxes that share a centre.
        for boxes in [
            &same[..12],
            &same[..],
            &[tri_box([[0.0; 3], [9.0, 0.0, 0.0], [0.0, 1.0, 0.0]]); 3][..],
        ] {
            let one = Bvh::build_with(boxes, 1);
            let leaves: Vec<u32> = one
                .nodes
                .iter()
                .map(|n| n.count)
                .filter(|&c| c > 0)
                .collect();
            assert_eq!(leaves.len(), boxes.len());
            assert!(leaves.iter().all(|&c| c == 1));
        }
        // f32 boxes round outward.
        let (lo, hi) = node_box(&Aabb {
            lo: [0.1; 3],
            hi: [0.3; 3],
        });
        assert!(lo[0] as f64 <= 0.1 && hi[0] as f64 >= 0.3);
    }
}
