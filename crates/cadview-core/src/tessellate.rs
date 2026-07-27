use crate::geo;
use crate::types::{FillEdge, LwVertex};
use kurbo::{Affine, BezPath, Point};
use std::f64::consts::PI;

/// `tolerance` is the max deviation in world units for arc tessellation.
pub fn flatten_fill_edges(edges: &[FillEdge], tolerance: f64) -> Vec<Point> {
    let mut pts = Vec::new();
    for edge in edges {
        match edge {
            FillEdge::LineTo(p) => {
                pts.push(*p);
            }
            FillEdge::ArcTo {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                // Sagitta-based step count: max chord error < tolerance.
                let mut sweep = end_angle - start_angle;
                if sweep < 0.0 {
                    sweep += 2.0 * PI;
                }
                let steps = if *radius < tolerance {
                    4usize
                } else {
                    let theta = 2.0 * (1.0 - tolerance / radius).acos();
                    (sweep / theta).ceil().clamp(4.0, 4096.0) as usize
                };
                // Skip first point (it connects to the previous edge's endpoint)
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let angle = start_angle + t * sweep;
                    pts.push(Point::new(
                        center.x + radius * angle.cos(),
                        center.y + radius * angle.sin(),
                    ));
                }
            }
            FillEdge::EllipseArcTo {
                center,
                major_axis,
                minor_ratio,
                start_param,
                end_param,
            } => {
                let epts = tessellate_ellipse(
                    *center,
                    *major_axis,
                    *minor_ratio,
                    *start_param,
                    *end_param,
                    tolerance,
                );
                // Skip first point (connects to previous edge)
                if epts.len() > 1 {
                    pts.extend_from_slice(&epts[1..]);
                }
            }
            FillEdge::SplineTo {
                degree,
                knots,
                control_points,
            } => {
                let spts = tessellate_spline(*degree, knots, control_points, tolerance);
                // Skip first point (connects to previous edge)
                if spts.len() > 1 {
                    pts.extend_from_slice(&spts[1..]);
                }
            }
            FillEdge::PolylineTo(points) => {
                pts.extend_from_slice(points);
            }
        }
    }
    // Remove trailing duplicate of first point (closed polygon)
    if pts.len() > 2 {
        let first = pts[0];
        let last = *pts.last().unwrap();
        if (first.x - last.x).abs() < 1e-6 && (first.y - last.y).abs() < 1e-6 {
            pts.pop();
        }
    }
    pts
}

/// Flatten a BezPath into polyline points at the given tolerance.
/// Returns a Vec of contours (each contour is a Vec<Point>).
pub fn flatten_bezpath_adaptive(path: &BezPath, tolerance: f64) -> Vec<Vec<Point>> {
    let mut contours = Vec::new();
    let mut current = Vec::new();
    let mut flattened = Vec::new();
    kurbo::flatten(path.iter(), tolerance, |el| flattened.push(el));
    for el in flattened {
        match el {
            kurbo::PathEl::MoveTo(p) => {
                if current.len() >= 2 {
                    contours.push(std::mem::take(&mut current));
                }
                current = vec![p];
            }
            kurbo::PathEl::LineTo(p) => current.push(p),
            kurbo::PathEl::ClosePath => {
                if let Some(&first) = current.first() {
                    current.push(first);
                }
            }
            _ => {}
        }
    }
    if current.len() >= 2 {
        contours.push(current);
    }
    contours
}

/// Triangulate a fill boundary (FillEdge sequence with holes) at a given
/// tolerance. Convenience wrapper: flattens edges then triangulates.
pub fn triangulate_fill(
    boundary: &[FillEdge],
    holes: &[Vec<FillEdge>],
    tolerance: f64,
) -> (Vec<[f32; 2]>, Vec<u32>) {
    let flat_boundary = flatten_fill_edges(boundary, tolerance);
    let flat_holes: Vec<Vec<Point>> = holes
        .iter()
        .map(|h| flatten_fill_edges(h, tolerance))
        .collect();
    triangulate_polygon(&flat_boundary, &flat_holes)
}

/// Tessellate an ellipse/elliptic arc to polyline points at given tolerance.
pub fn tessellate_ellipse(
    center: Point,
    major_axis: (f64, f64),
    minor_ratio: f64,
    start_param: f64,
    end_param: f64,
    tolerance: f64,
) -> Vec<Point> {
    let ma_len = (major_axis.0 * major_axis.0 + major_axis.1 * major_axis.1).sqrt();
    if ma_len < 1e-12 {
        return Vec::new();
    }
    let angle = major_axis.1.atan2(major_axis.0);
    let a = ma_len;
    let b = ma_len * minor_ratio;
    let r_max = a.max(b);

    let mut sweep = end_param - start_param;
    if sweep.abs() < 1e-10 {
        sweep = 2.0 * PI;
    }
    if sweep < 0.0 {
        sweep += 2.0 * PI;
    }

    // Sagitta-based step count using max radius
    let steps = if r_max < tolerance {
        4usize
    } else {
        let theta = 2.0 * (1.0 - tolerance / r_max).acos();
        (sweep / theta).ceil().clamp(4.0, 4096.0) as usize
    };

    let cos_a = angle.cos();
    let sin_a = angle.sin();
    (0..=steps)
        .map(|i| {
            let t = start_param + sweep * (i as f64 / steps as f64);
            let ex = a * t.cos();
            let ey = b * t.sin();
            Point::new(
                center.x + ex * cos_a - ey * sin_a,
                center.y + ex * sin_a + ey * cos_a,
            )
        })
        .collect()
}

/// Tessellate a B-spline to polyline points using De Boor evaluation.
/// Step count is adaptive based on control hull length and tolerance.
pub fn tessellate_spline(
    degree: i32,
    knots: &[f64],
    control_points: &[Point],
    tolerance: f64,
) -> Vec<Point> {
    let d = degree as usize;
    let n = control_points.len();
    if n < 2 || knots.len() < n + d + 1 {
        return control_points.to_vec();
    }

    // Estimate total arc length from control hull
    let mut hull_len = 0.0f64;
    for i in 1..n {
        let dx = control_points[i].x - control_points[i - 1].x;
        let dy = control_points[i].y - control_points[i - 1].y;
        hull_len += (dx * dx + dy * dy).sqrt();
    }

    // Adaptive step count: max of hull_len/tolerance and n*8 (baseline quality)
    let tol_steps = (hull_len / tolerance).ceil();
    let base_steps = (n * 8) as f64;
    let steps = tol_steps.max(base_steps).clamp(16.0, 4096.0) as usize;

    let t_start = knots[d];
    let t_end = knots[n];
    if (t_end - t_start).abs() < 1e-12 {
        return control_points.to_vec();
    }

    (0..=steps)
        .map(|i| {
            let t = t_start + (t_end - t_start) * (i as f64 / steps as f64);
            de_boor_eval(d, knots, control_points, t)
        })
        .collect()
}

/// De Boor's algorithm for B-spline evaluation at parameter t.
pub(crate) fn de_boor_eval(degree: usize, knots: &[f64], cps: &[Point], t: f64) -> Point {
    let n = cps.len();
    // Find knot span k such that knots[k] <= t < knots[k+1]
    let mut k = degree;
    for i in degree..n {
        if i + 1 >= knots.len() {
            break;
        }
        if t >= knots[i] && t < knots[i + 1] {
            k = i;
            break;
        }
    }
    // Handle t at/beyond the last knot
    if t >= knots[n] {
        k = n - 1;
    }

    // Copy the affected control points
    let mut d: Vec<[f64; 2]> = (0..=degree)
        .map(|j| {
            let idx = ((k as isize - degree as isize + j as isize) as usize).min(n - 1);
            [cps[idx].x, cps[idx].y]
        })
        .collect();

    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i_knot = (k as isize - degree as isize + j as isize) as usize;
            let ki = if i_knot < knots.len() {
                knots[i_knot]
            } else {
                *knots.last().unwrap()
            };
            let ki_end_idx = i_knot + degree + 1 - r;
            let ki_end = if ki_end_idx < knots.len() {
                knots[ki_end_idx]
            } else {
                *knots.last().unwrap()
            };
            let denom = ki_end - ki;
            let alpha = if denom.abs() < 1e-12 {
                0.0
            } else {
                (t - ki) / denom
            };
            d[j][0] = (1.0 - alpha) * d[j - 1][0] + alpha * d[j][0];
            d[j][1] = (1.0 - alpha) * d[j - 1][1] + alpha * d[j][1];
        }
    }
    Point::new(d[degree][0], d[degree][1])
}

/// Tessellate an LwPolyline (vertices with bulge arcs) to polyline points.
/// Bulge arcs use sagitta-based step count for zoom-adaptive quality.
pub fn tessellate_lwpolyline(vertices: &[LwVertex], closed: bool, tolerance: f64) -> Vec<Point> {
    let n = vertices.len();
    if n == 0 {
        return Vec::new();
    }

    let mut pts = Vec::new();
    let segments = if closed { n } else { n.saturating_sub(1) };
    pts.push(vertices[0].point);

    for i in 0..segments {
        let j = (i + 1) % n;
        let p0 = vertices[i].point;
        let p1 = vertices[j].point;
        let bulge = vertices[i].bulge;

        if bulge.abs() < 1e-10 {
            pts.push(p1);
        } else {
            let included = 4.0 * bulge.atan();
            let dx = p1.x - p0.x;
            let dy = p1.y - p0.y;
            let chord = (dx * dx + dy * dy).sqrt();
            if chord < 1e-12 {
                pts.push(p1);
                continue;
            }
            let radius = chord / (2.0 * included.sin().abs());
            let sagitta = bulge * chord / 2.0;
            let mx = (p0.x + p1.x) / 2.0;
            let my = (p0.y + p1.y) / 2.0;
            let nx = -dy / chord;
            let ny = dx / chord;
            let d = radius - sagitta.abs();
            let sign = if bulge > 0.0 { 1.0 } else { -1.0 };
            let cx = mx + sign * d * nx;
            let cy = my + sign * d * ny;

            let start_ang = (p0.y - cy).atan2(p0.x - cx);
            // Sagitta-based step count
            let steps = if radius < tolerance {
                4usize
            } else {
                let theta = 2.0 * (1.0 - tolerance / radius).acos();
                (included.abs() / theta).ceil().clamp(4.0, 4096.0) as usize
            };
            let step_angle = included / steps as f64;
            for s in 1..steps {
                let a = start_ang + step_angle * s as f64;
                pts.push(Point::new(cx + radius * a.cos(), cy + radius * a.sin()));
            }
            pts.push(p1);
        }
    }
    pts
}

/// Convert fit points to a smooth cubic BezPath using Catmull-Rom interpolation.
/// Produces a C1-continuous curve passing through all points.
pub fn catmull_rom_to_bezpath(points: &[Point], closed: bool) -> BezPath {
    let n = points.len();
    let mut path = BezPath::new();
    if n < 2 {
        return path;
    }

    path.move_to(points[0]);

    if n == 2 {
        path.line_to(points[1]);
        return path;
    }

    // Catmull-Rom to cubic Bezier conversion:
    // For segment from P1 to P2 with neighbors P0 and P3:
    //   CP1 = P1 + (P2 - P0) / 6
    //   CP2 = P2 - (P3 - P1) / 6
    let segments = if closed { n } else { n - 1 };
    for i in 0..segments {
        let p0 = if i == 0 && !closed {
            points[0]
        } else {
            points[(i + n - 1) % n]
        };
        let p1 = points[i % n];
        let p2 = points[(i + 1) % n];
        let p3 = if i == segments - 1 && !closed {
            points[n - 1]
        } else {
            points[(i + 2) % n]
        };

        let cp1 = Point::new(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0);
        let cp2 = Point::new(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0);
        path.curve_to(cp1, cp2, p2);
    }

    if closed {
        path.close_path();
    }
    path
}

/// Triangulate a polygon with holes using earcutr. Returns vertex positions
/// (f32 for GPU) and triangle indices. This is a render-time utility; the
/// model stores only the abstract boundary and holes.
///
/// Cleans up the polygon before triangulation:
/// 1. Remove near-duplicate consecutive points (vertex welding)
/// 2. Remove collinear points (reduces sliver triangles)
/// 3. Ensure correct winding (outer CCW, holes CW)
pub fn triangulate_polygon(boundary: &[Point], holes: &[Vec<Point>]) -> (Vec<[f32; 2]>, Vec<u32>) {
    // Adaptive tolerance: 0.1% of polygon diagonal, min 0.5, max 10.
    let (bx0, by0, bx1, by1) = geo::bounds_of(boundary);
    let diag = ((bx1 - bx0).powi(2) + (by1 - by0).powi(2)).sqrt();
    let tol = (diag * 0.001).clamp(0.5, 10.0);

    let mut clean_boundary = simplify_polygon(boundary, tol);
    if clean_boundary.len() < 3 {
        clean_boundary = boundary.to_vec();
    }

    // Ensure outer boundary is CCW
    if signed_polygon_area(&clean_boundary) < 0.0 {
        clean_boundary.reverse();
    }

    let mut clean_holes: Vec<Vec<Point>> = Vec::new();
    for hole in holes {
        let mut ch = simplify_polygon(hole, tol);
        if ch.len() < 3 {
            continue;
        }
        // Ensure holes are CW (opposite winding from outer)
        if signed_polygon_area(&ch) > 0.0 {
            ch.reverse();
        }
        clean_holes.push(ch);
    }

    // Build coordinate array for earcutr
    let mut coords: Vec<f64> = Vec::new();
    let mut hole_indices: Vec<usize> = Vec::new();

    for p in &clean_boundary {
        coords.push(p.x);
        coords.push(p.y);
    }
    for hole in &clean_holes {
        hole_indices.push(coords.len() / 2);
        for p in hole {
            coords.push(p.x);
            coords.push(p.y);
        }
    }

    let indices = earcutr::earcut(&coords, &hole_indices, 2).unwrap_or_default();

    let triangles: Vec<[f32; 2]> = coords
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[x, y]| [x as f32, y as f32])
        .collect();
    let tri_indices: Vec<u32> = indices.iter().map(|&i| i as u32).collect();

    (triangles, tri_indices)
}

/// Build an Affine that reflects across the line from p1 to p2.
pub(crate) fn mirror_affine(p1: Point, p2: Point) -> Affine {
    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    let len2 = dx * dx + dy * dy;
    // Reflection matrix for line through origin with direction (dx, dy):
    //   [(dx²-dy²)/l², 2·dx·dy/l²]
    //   [2·dx·dy/l²,   (dy²-dx²)/l²]
    // Composed with translate to/from p1.
    let a = (dx * dx - dy * dy) / len2;
    let b = 2.0 * dx * dy / len2;
    // M = T(p1) · R · T(-p1), where R is the reflection about origin-line
    Affine::new([
        a,
        b,
        b,
        -a,
        p1.x - a * p1.x - b * p1.y,
        p1.y - b * p1.x + a * p1.y,
    ])
}

/// Simplify a polygon by removing near-duplicate points and collinear vertices.
/// `tolerance` is the minimum distance between consecutive points and the
/// maximum perpendicular deviation for collinear removal.
pub(crate) fn simplify_polygon(pts: &[Point], tolerance: f64) -> Vec<Point> {
    if pts.len() < 3 {
        return pts.to_vec();
    }

    // Step 1: remove near-duplicate consecutive points
    let mut deduped = Vec::with_capacity(pts.len());
    deduped.push(pts[0]);
    for p in &pts[1..] {
        let prev = deduped.last().unwrap();
        if (prev.x - p.x).abs() > tolerance || (prev.y - p.y).abs() > tolerance {
            deduped.push(*p);
        }
    }
    // Also check last vs first
    if deduped.len() > 2 {
        let first = deduped[0];
        let last = *deduped.last().unwrap();
        if (first.x - last.x).abs() < tolerance && (first.y - last.y).abs() < tolerance {
            deduped.pop();
        }
    }

    // Step 2: remove collinear vertices (point on the line between neighbors)
    let mut changed = true;
    while changed {
        changed = false;
        let n = deduped.len();
        if n < 4 {
            break;
        }
        let mut keep = vec![true; n];
        for i in 0..n {
            if !keep[i] {
                continue;
            }
            let prev = (0..n).rev().cycle().skip(n - i).find(|&j| keep[j]).unwrap();
            let next = (0..n).cycle().skip(i + 1).find(|&j| keep[j]).unwrap();
            if prev == next {
                continue;
            }
            let a = deduped[prev];
            let b = deduped[i];
            let c = deduped[next];
            // Perpendicular distance from b to line a-c
            let dx = c.x - a.x;
            let dy = c.y - a.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-10 {
                continue;
            }
            let dist = ((b.x - a.x) * dy - (b.y - a.y) * dx).abs() / len;
            if dist < tolerance {
                keep[i] = false;
                changed = true;
            }
        }
        deduped = deduped
            .into_iter()
            .enumerate()
            .filter(|(i, _)| keep[*i])
            .map(|(_, p)| p)
            .collect();
    }

    deduped
}

/// Signed area of a polygon. Positive = CCW, Negative = CW.
pub(crate) fn signed_polygon_area(pts: &[Point]) -> f64 {
    let n = pts.len();
    let mut area = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        area += pts[i].x * pts[j].y;
        area -= pts[j].x * pts[i].y;
    }
    area / 2.0
}
