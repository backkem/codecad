//! Geometry helper functions. Pure computations, no Document mutation.
//! All operate on kurbo::Point (f64 coordinates).

use kurbo::Point;
use std::f64::consts::PI;

/// Euclidean distance between two points.
pub fn distance(a: Point, b: Point) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
}

/// Midpoint of two points.
pub fn midpoint(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
}

/// Linear interpolation between two points. t=0 returns a, t=1 returns b.
pub fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + t * (b.x - a.x), a.y + t * (b.y - a.y))
}

/// Point at a given distance along the segment from a to b.
pub fn along(a: Point, b: Point, dist: f64) -> Point {
    let len = distance(a, b);
    if len < 1e-12 {
        return a;
    }
    lerp(a, b, dist / len)
}

/// Direction angle from a to b in degrees (0 = east, 90 = north, Y-up).
pub fn direction(a: Point, b: Point) -> f64 {
    let rad = (b.y - a.y).atan2(b.x - a.x);
    rad * 180.0 / PI
}

/// Unit normal vector of the segment a->b (perpendicular, left-hand side).
/// For a Y-up coordinate system, this points "left" when walking from a to b.
pub fn normal(a: Point, b: Point) -> (f64, f64) {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-12 {
        return (0.0, 0.0);
    }
    (-dy / len, dx / len)
}

/// Perpendicular foot: project point p onto the infinite line through a and b.
pub fn project_onto(p: Point, a: Point, b: Point) -> Point {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-24 {
        return a;
    }
    let t = ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2;
    Point::new(a.x + t * dx, a.y + t * dy)
}

/// Shortest distance from point p to the line segment a-b.
pub fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-24 {
        return distance(p, a);
    }
    let t = ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2;
    let t = t.clamp(0.0, 1.0);
    let proj = Point::new(a.x + t * dx, a.y + t * dy);
    distance(p, proj)
}

/// Intersection of two line segments. Returns None if parallel or no intersection.
pub fn intersection(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<Point> {
    let d1x = a1.x - a0.x;
    let d1y = a1.y - a0.y;
    let d2x = b1.x - b0.x;
    let d2y = b1.y - b0.y;
    let denom = d1x * d2y - d1y * d2x;
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = ((b0.x - a0.x) * d2y - (b0.y - a0.y) * d2x) / denom;
    Some(Point::new(a0.x + t * d1x, a0.y + t * d1y))
}

/// Intersection of an infinite line (through a, b) with a circle (center, radius).
/// Returns 0, 1, or 2 intersection points.
pub fn line_circle_intersection(a: Point, b: Point, center: Point, radius: f64) -> Vec<Point> {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let fx = a.x - center.x;
    let fy = a.y - center.y;

    let a_coeff = dx * dx + dy * dy;
    let b_coeff = 2.0 * (fx * dx + fy * dy);
    let c_coeff = fx * fx + fy * fy - radius * radius;

    let disc = b_coeff * b_coeff - 4.0 * a_coeff * c_coeff;
    if disc < -1e-12 {
        return Vec::new();
    }
    let disc = disc.max(0.0);

    if disc.abs() < 1e-12 {
        let t = -b_coeff / (2.0 * a_coeff);
        vec![Point::new(a.x + t * dx, a.y + t * dy)]
    } else {
        let sq = disc.sqrt();
        let t1 = (-b_coeff - sq) / (2.0 * a_coeff);
        let t2 = (-b_coeff + sq) / (2.0 * a_coeff);
        vec![
            Point::new(a.x + t1 * dx, a.y + t1 * dy),
            Point::new(a.x + t2 * dx, a.y + t2 * dy),
        ]
    }
}

/// Intersection of two circles. Returns 0, 1, or 2 intersection points.
pub fn circle_circle_intersection(c1: Point, r1: f64, c2: Point, r2: f64) -> Vec<Point> {
    let d = distance(c1, c2);
    if d > r1 + r2 + 1e-12 {
        return Vec::new();
    } // too far apart
    if d < (r1 - r2).abs() - 1e-12 {
        return Vec::new();
    } // one inside the other
    if d < 1e-12 {
        return Vec::new();
    } // concentric

    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h2 = r1 * r1 - a * a;
    let h = if h2 < 0.0 { 0.0 } else { h2.sqrt() };

    // Point along the line between centers, at distance a from c1
    let px = c1.x + a * (c2.x - c1.x) / d;
    let py = c1.y + a * (c2.y - c1.y) / d;

    if h < 1e-12 {
        // Tangent: single point
        vec![Point::new(px, py)]
    } else {
        // Two intersection points, offset perpendicular to the center-center line
        let ox = h * (c2.y - c1.y) / d;
        let oy = h * (c2.x - c1.x) / d;
        vec![Point::new(px + ox, py - oy), Point::new(px - ox, py + oy)]
    }
}

/// Project from a center point through a target point onto a circle.
/// Returns the point on the circle (center, radius) along the ray from center through p.
pub fn project_onto_circle(p: Point, center: Point, radius: f64) -> Point {
    let dx = p.x - center.x;
    let dy = p.y - center.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-12 {
        return Point::new(center.x + radius, center.y);
    }
    Point::new(center.x + radius * dx / len, center.y + radius * dy / len)
}

/// Compute the angle (in degrees) of a point relative to a circle center.
/// 0 = east, 90 = north (Y-up).
pub fn angle_of(p: Point, center: Point) -> f64 {
    (p.y - center.y).atan2(p.x - center.x) * 180.0 / PI
}

/// Test if point p is inside a polygon (array of vertices, assumed closed).
/// Uses the ray casting algorithm.
pub fn point_in_polygon(p: Point, polygon: &[Point]) -> bool {
    let n = polygon.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let pi = polygon[i];
        let pj = polygon[j];
        if ((pi.y > p.y) != (pj.y > p.y))
            && (p.x < (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Centroid (arithmetic mean) of polygon vertices.
pub fn centroid(polygon: &[Point]) -> Point {
    if polygon.is_empty() {
        return Point::ZERO;
    }
    let n = polygon.len() as f64;
    let sx: f64 = polygon.iter().map(|p| p.x).sum();
    let sy: f64 = polygon.iter().map(|p| p.y).sum();
    Point::new(sx / n, sy / n)
}

/// Bounding box of a set of points: (min_x, min_y, max_x, max_y).
pub fn bounds_of(points: &[Point]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    for p in points {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    (min_x, min_y, max_x, max_y)
}

/// Rotate a point around a center by the given angle in degrees.
pub fn rotate_point(p: Point, degrees: f64, center: Point) -> Point {
    let r = degrees * PI / 180.0;
    let dx = p.x - center.x;
    let dy = p.y - center.y;
    Point::new(
        center.x + dx * r.cos() - dy * r.sin(),
        center.y + dx * r.sin() + dy * r.cos(),
    )
}

/// Signed area of a polygon (positive if CCW, negative if CW).
pub fn signed_area(polygon: &[Point]) -> f64 {
    let n = polygon.len();
    if n < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        sum += polygon[i].x * polygon[j].y;
        sum -= polygon[j].x * polygon[i].y;
    }
    sum / 2.0
}

/// Absolute area of a polygon.
pub fn area(polygon: &[Point]) -> f64 {
    signed_area(polygon).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn test_distance() {
        assert!((distance(pt(0.0, 0.0), pt(3.0, 4.0)) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_midpoint() {
        let m = midpoint(pt(0.0, 0.0), pt(10.0, 6.0));
        assert!((m.x - 5.0).abs() < 1e-10);
        assert!((m.y - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_lerp() {
        let p = lerp(pt(0.0, 0.0), pt(10.0, 0.0), 0.3);
        assert!((p.x - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_along() {
        let p = along(pt(0.0, 0.0), pt(10.0, 0.0), 7.5);
        assert!((p.x - 7.5).abs() < 1e-10);
    }

    #[test]
    fn test_direction() {
        assert!((direction(pt(0.0, 0.0), pt(1.0, 0.0)) - 0.0).abs() < 1e-10);
        assert!((direction(pt(0.0, 0.0), pt(0.0, 1.0)) - 90.0).abs() < 1e-10);
        assert!((direction(pt(0.0, 0.0), pt(-1.0, 0.0)) - 180.0).abs() < 1e-10);
    }

    #[test]
    fn test_distance_to_segment() {
        // Point above middle of horizontal segment
        let d = distance_to_segment(pt(5.0, 3.0), pt(0.0, 0.0), pt(10.0, 0.0));
        assert!((d - 3.0).abs() < 1e-10);

        // Point beyond segment end
        let d = distance_to_segment(pt(15.0, 0.0), pt(0.0, 0.0), pt(10.0, 0.0));
        assert!((d - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_intersection() {
        let p = intersection(pt(0.0, 0.0), pt(10.0, 10.0), pt(10.0, 0.0), pt(0.0, 10.0));
        let p = p.unwrap();
        assert!((p.x - 5.0).abs() < 1e-10);
        assert!((p.y - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_parallel_lines_no_intersection() {
        assert!(intersection(pt(0.0, 0.0), pt(10.0, 0.0), pt(0.0, 5.0), pt(10.0, 5.0)).is_none());
    }

    #[test]
    fn test_point_in_polygon() {
        let square = vec![pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 10.0), pt(0.0, 10.0)];
        assert!(point_in_polygon(pt(5.0, 5.0), &square));
        assert!(!point_in_polygon(pt(15.0, 5.0), &square));
        assert!(!point_in_polygon(pt(-1.0, -1.0), &square));
    }

    #[test]
    fn test_area() {
        // 10x10 square
        let square = vec![pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 10.0), pt(0.0, 10.0)];
        assert!((area(&square) - 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_rotate_point() {
        let p = rotate_point(pt(1.0, 0.0), 90.0, pt(0.0, 0.0));
        assert!((p.x - 0.0).abs() < 1e-10);
        assert!((p.y - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_centroid() {
        let square = vec![pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 10.0), pt(0.0, 10.0)];
        let c = centroid(&square);
        assert!((c.x - 5.0).abs() < 1e-10);
        assert!((c.y - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_line_circle_intersection() {
        // Horizontal line y=3 through circle at origin r=5
        let pts = line_circle_intersection(pt(-10.0, 3.0), pt(10.0, 3.0), pt(0.0, 0.0), 5.0);
        assert_eq!(pts.len(), 2);
        // x^2 + 9 = 25 -> x = +/- 4
        let mut xs: Vec<f64> = pts.iter().map(|p| p.x).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((xs[0] - (-4.0)).abs() < 1e-10);
        assert!((xs[1] - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_line_circle_miss() {
        // Line y=10, circle r=5 -> no intersection
        let pts = line_circle_intersection(pt(-10.0, 10.0), pt(10.0, 10.0), pt(0.0, 0.0), 5.0);
        assert_eq!(pts.len(), 0);
    }

    #[test]
    fn test_circle_circle_intersection() {
        // Two circles: (0,0) r=5 and (6,0) r=5 -> intersect at two points
        let pts = circle_circle_intersection(pt(0.0, 0.0), 5.0, pt(6.0, 0.0), 5.0);
        assert_eq!(pts.len(), 2);
        // Intersection x = 3, y = +/- 4
        for p in &pts {
            assert!((p.x - 3.0).abs() < 1e-10);
            assert!((p.y.abs() - 4.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_project_onto_circle() {
        let p = project_onto_circle(pt(3.0, 4.0), pt(0.0, 0.0), 10.0);
        assert!((p.x - 6.0).abs() < 1e-10);
        assert!((p.y - 8.0).abs() < 1e-10);
    }

    #[test]
    fn test_angle_of() {
        assert!((angle_of(pt(1.0, 0.0), pt(0.0, 0.0)) - 0.0).abs() < 1e-10);
        assert!((angle_of(pt(0.0, 1.0), pt(0.0, 0.0)) - 90.0).abs() < 1e-10);
    }
}
