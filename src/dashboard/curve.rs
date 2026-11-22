use crate::trend::{Bin, Point};
use std::fmt::Write;

pub(super) fn anchors(run: &[Bin]) -> Vec<Point> {
    let mut result: Vec<Point> = Vec::new();
    for bin in run {
        let mut points = [bin.first, bin.range.low, bin.range.high, bin.last];
        points.sort_unstable_by_key(|p| p.at);
        for point in points {
            match result.as_mut_slice() {
                [.., last] if last.at == point.at => (),
                [.., previous, last] if previous.rh == point.rh && last.rh == point.rh => {
                    *last = point;
                }
                _ => result.push(point),
            }
        }
    }
    result
}

type Vertex = (f64, f64);

fn slope(a: Vertex, b: Vertex) -> f64 {
    (b.1 - a.1) / (b.0 - a.0)
}

fn tangent(a: Vertex, b: Vertex, c: Vertex) -> f64 {
    let left = slope(a, b);
    let right = slope(b, c);
    if left.is_sign_positive() != right.is_sign_positive() {
        return 0.0;
    }
    let average = (left * (c.0 - b.0) + right * (b.0 - a.0)) / (c.0 - a.0);
    left.signum() * (2.0 * left.abs()).min(2.0 * right.abs()).min(average.abs())
}

pub(super) fn path(points: &[Vertex]) -> String {
    let Some((&first, rest)) = points.split_first() else {
        return String::new();
    };
    let mut path = format!("M{:.4},{:.3}", first.0, first.1);
    match rest {
        [] => (),
        [last] => write!(path, "L{:.4},{:.3}", last.0, last.1).unwrap(),
        [second, third, ..] => {
            let mut left = (3.0 * slope(first, *second) - tangent(first, *second, *third)) / 2.0;
            for (i, pair) in points.windows(2).enumerate() {
                let (a, b) = (pair[0], pair[1]);
                let right = match points.get(i + 2) {
                    Some(&c) => tangent(a, b, c),
                    None => (3.0 * slope(a, b) - left) / 2.0,
                };
                let dx = (b.0 - a.0) / 3.0;
                write!(
                    path,
                    "C{:.4},{:.3} {:.4},{:.3} {:.4},{:.3}",
                    a.0 + dx,
                    a.1 + dx * left,
                    b.0 - dx,
                    b.1 - dx * right,
                    b.0,
                    b.1,
                )
                .unwrap();
                left = right;
            }
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trend::Extrema;

    #[test]
    fn keeps_timed_extremes_and_the_edges_of_a_plateau() {
        let point = |at, rh| Point { at, rh };
        let bin = |first, high, low, last| Bin {
            first,
            last,
            range: Extrema { low, high },
        };
        let p = [point(0, 44), point(60, 87), point(180, 21), point(240, 44)];
        let flat = |at| {
            let p = point(at, 44);
            bin(p, p, p, p)
        };
        assert_eq!(
            anchors(&[bin(p[0], p[1], p[2], p[3]), flat(300), flat(360)]),
            [p[0], p[1], p[2], p[3], point(360, 44)]
        );
    }

    #[test]
    fn preserves_straight_lines_and_extrema_at_irregular_times() {
        assert_eq!(path(&[]), "");
        assert_eq!(path(&[(1.0, 2.0)]), "M1.0000,2.000");
        assert_eq!(
            path(&[(1.0, 2.0), (4.0, 8.0)]),
            "M1.0000,2.000L4.0000,8.000"
        );
        assert_eq!(tangent((0., 0.), (3., 6.), (9., 24.)), 7. / 3.);
        assert_eq!(tangent((0., 24.), (6., 6.), (9., 0.)), -7. / 3.);
        assert_eq!(tangent((0., 0.), (3., 3.), (6., 33.)), 2.);
        assert_eq!(tangent((0., 0.), (3., 30.), (6., 33.)), 2.);
        assert_eq!(tangent((0., 1.), (1., 2.), (2., 2.)), 0.);
        assert_eq!(tangent((0., 2.), (1., 2.), (2., 1.)), 0.);
        assert_eq!(
            path(&[(0., 3.), (3., 9.), (9., 3.)]),
            "M0.0000,3.000C1.0000,6.000 2.0000,9.000 3.0000,9.000C5.0000,9.000 7.0000,6.000 9.0000,3.000"
        );
    }

    #[test]
    fn rendered_curves_stay_inside_every_observed_interval() {
        for a in [0., 10., 100.] {
            for b in [0., 25., 80., 100.] {
                for c in [0., 60., 100.] {
                    let values = [a, b, c, 55.];
                    let vertices = [(0., a), (3., b), (15., c), (21., 55.)];
                    let curve = path(&vertices);
                    let segments: Vec<_> = curve.split('C').skip(1).collect();
                    assert_eq!(segments.len(), values.len() - 1);
                    for (i, segment) in segments.iter().enumerate() {
                        let controls: Vec<(f64, f64)> = segment
                            .split_whitespace()
                            .map(|pair| {
                                let (x, y) = pair.split_once(',').unwrap();
                                (x.parse().unwrap(), y.parse().unwrap())
                            })
                            .collect();
                        let [left, right, end] = controls[..] else {
                            panic!("cubic controls")
                        };
                        let (start, target) = (vertices[i], vertices[i + 1]);
                        assert!(start.0 < left.0 && left.0 < right.0 && right.0 < end.0);
                        assert_eq!(end, target);
                        let range = start.1.min(end.1)..=start.1.max(end.1);
                        assert!(range.contains(&left.1) && range.contains(&right.1));
                        let mut previous = start.1;
                        for step in 1..=20 {
                            let t = f64::from(step) / 20.;
                            let u = 1. - t;
                            let y = u * u * u * start.1
                                + 3. * u * u * t * left.1
                                + 3. * u * t * t * right.1
                                + t * t * t * end.1;
                            assert!((y - previous) * (end.1 - start.1) >= -1e-9);
                            previous = y;
                        }
                    }
                }
            }
        }
    }
}
