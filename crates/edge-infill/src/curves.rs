//! Sparse patterns that are not straight rows: the lines of each, laid
//! over a rectangle, to be cut to the area they fill.
//!
//! Each works in the frame the reference works in: scaled units, with the
//! area already turned so the pattern lies square to the axes, and
//! measured from the middle of what is printed, which is where the
//! reference's object coordinates begin.

use std::f64::consts::PI;

use edge_geom::{Coord, Point2, UNITS_PER_MM};

/// The least and greatest corners of a rectangle.
pub(crate) type Bounds = (Point2, Point2);

/// `at` moved down to a multiple of `step` (Point.hpp `align_to_grid`).
fn align(at: Coord, step: Coord) -> Coord {
    let step = step.max(1);
    if at < 0 {
        ((at - step + 1) / step) * step
    } else {
        (at / step) * step
    }
}

fn point(x: f64, y: f64) -> Point2 {
    Point2 {
        x: x.round() as Coord,
        y: y.round() as Coord,
    }
}

/// Cross hatch: straight lines one way for a stretch of layers, then the
/// other, turning from one into the other through zigzags over the layers
/// between (FillCrossHatch.cpp). `spacing` is the line's spacing in mm,
/// `density` the share of the area filled, and `z` the layer's top.
pub(crate) fn crosshatch(bounds: Bounds, spacing: f64, density: f64, z: f64) -> Vec<Vec<Point2>> {
    let (low, high) = bounds;
    let mut pitch = (spacing * UNITS_PER_MM / density) as Coord;
    if density < 0.999 {
        pitch = (pitch as f64 * 1.08) as Coord;
    }
    if pitch <= 0 {
        return Vec::new();
    }
    // The pattern starts on a grid four lines wide, the same for every
    // area and layer.
    let low = Point2 {
        x: align(low.x, pitch * 4).min(low.x),
        y: align(low.y, pitch * 4).min(low.y),
    };
    let (width, height) = ((high.x - low.x) as f64, (high.y - low.y) as f64);
    // At low densities the straight stretches are kept shorter.
    let repeat_ratio = if density < 0.3 {
        (1.0 - (-5.0 * density).exp()).clamp(0.2, 1.0)
    } else {
        1.0
    };
    let grid = pitch as f64;

    let turning = grid * 0.4;
    let straight = grid * repeat_ratio;
    // Raised so the first few layers are straight ones.
    let z = z * UNITS_PER_MM + straight / 2.0 + turning;
    let period = turning + straight;
    let remains = z - (z / period).floor() * period;
    let into_turn = remains - straight;
    let phase = ((z % (period * 2.0)) - (period - 1.0)) as i64;
    let direction = if phase <= 0 { -1 } else { 1 };
    let mut lines = if into_turn < 0.0 {
        straight_rows(direction, grid, width, height)
    } else {
        let progress = (into_turn % turning) / turning;
        if progress < 0.5 {
            zigzag_rows((progress + 0.1) * 2.0, direction, grid, width, height)
        } else {
            zigzag_rows((1.1 - progress) * 2.0, -direction, grid, width, height)
        }
    };
    for point in lines.iter_mut().flatten() {
        point.x += low.x;
        point.y += low.y;
    }
    lines
}

/// Straight rows a pitch apart; along y for a direction below zero.
fn straight_rows(direction: i32, grid: f64, width: f64, height: f64) -> Vec<Vec<Point2>> {
    let (width, height) = if direction < 0 {
        (height, width)
    } else {
        (width, height)
    };
    let count = (height / grid) as i64 + 1;
    (0..count)
        .map(|row| {
            let across = grid * row as f64;
            let line = [point(0.0, across), point(width, across)];
            line.map(|p| {
                if direction < 0 {
                    Point2 { x: p.y, y: p.x }
                } else {
                    p
                }
            })
            .to_vec()
        })
        .collect()
}

/// Rows of zigzags two pitches apart, with a second set half a period
/// along and a pitch across: `progress` is how far each has opened from a
/// straight line towards meeting its neighbours.
fn zigzag_rows(
    progress: f64,
    direction: i32,
    pitch: f64,
    width: f64,
    height: f64,
) -> Vec<Vec<Point2>> {
    let grid = pitch * 2.0;
    let (width, height) = if direction < 0 {
        (height, width)
    } else {
        (width, height)
    };
    //    o---o
    //   /     \
    //  /       \       /
    //           \     /
    //            o---o
    let offset = progress / 8.0 * grid;
    let cycle = [
        (0.25 * grid - offset, offset),
        (0.25 * grid + offset, offset),
        (0.75 * grid - offset, -offset),
        (0.75 * grid + offset, -offset),
    ]
    .map(|(x, y)| point(x, y));
    let cycles = (width / grid) as i64 + 2;
    let row: Vec<Point2> = (0..cycles)
        .flat_map(|at| {
            let along = point(at as f64 * grid, 0.0).x;
            cycle.map(move |p| Point2 {
                x: p.x + along,
                y: p.y,
            })
        })
        .collect();
    let rows = (height / grid) as i64 + 2;
    let moved = |by: Point2| -> Vec<Point2> {
        row.iter()
            .map(|p| Point2 {
                x: p.x + by.x,
                y: p.y + by.y,
            })
            .collect()
    };
    let mut lines: Vec<Vec<Point2>> = (0..rows)
        .map(|at| moved(point(0.0, grid * at as f64)))
        .collect();
    lines.extend((0..rows).map(|at| moved(point(-0.5 * grid, (at as f64 + 0.5) * grid))));
    if direction < 0 {
        for point in lines.iter_mut().flatten() {
            (point.x, point.y) = (point.y, point.x);
        }
    }
    lines
}

/// The gyroid's share of full density for a given line spacing
/// (`FillGyroid::DensityAdjust`).
const GYROID_DENSITY: f64 = 2.44;
/// How far a wave may be drawn from the true curve, in mm
/// (`FillGyroid::PatternTolerance`).
const GYROID_TOLERANCE: f64 = 0.2;

/// Gyroid: waves that swing from running along x to running along y and
/// back as the layers go up (FillGyroid.cpp `make_gyroid_waves`).
/// `spacing` is the line's spacing in mm, `density` the share of the area
/// filled, and `z` the layer's top. The area must already be turned by
/// the reference's `CorrectionAngle`.
pub(crate) fn gyroid(bounds: Bounds, spacing: f64, density: f64, z: f64) -> Vec<Vec<Point2>> {
    let density = (density * GYROID_DENSITY).max(0.0);
    if density <= 0.0 {
        return Vec::new();
    }
    let (mut low, mut high) = bounds;
    let distance = (spacing * UNITS_PER_MM / density) as Coord;
    let shift = (10.0 * spacing * UNITS_PER_MM) as Coord;
    let cell = (2.0 * PI * distance as f64).round() as Coord;
    let origin = Point2 {
        x: align(low.x, cell) - shift,
        y: align(low.y, cell) - shift,
    };
    // A line's spacing beyond the area all round.
    let margin = (spacing * UNITS_PER_MM) as Coord;
    (low.x, low.y, high.x, high.y) = (
        low.x - margin,
        low.y - margin,
        high.x + margin,
        high.y + margin,
    );

    let scale = spacing * UNITS_PER_MM / density;
    let tolerance = (spacing / 2.0).min(GYROID_TOLERANCE) / (scale / UNITS_PER_MM);
    let turn = z * UNITS_PER_MM / scale;
    let (z_sin, z_cos) = turn.sin_cos();
    let vertical = z_sin.abs() <= z_cos.abs();
    let mut lo = (
        (low.x - origin.x) as f64 / scale,
        (low.y - origin.y) as f64 / scale,
    );
    let mut hi = (
        (high.x - origin.x) as f64 / scale,
        (high.y - origin.y) as f64 / scale,
    );
    let (lower_bound, flip) = if vertical { (-PI, false) } else { (0.0, true) };
    if vertical {
        lo = (lo.1, lo.0);
        hi = (hi.1, hi.0);
    }
    // Alternate waves are half a turn out of step.
    let odd = one_period(z_cos, z_sin, vertical, flip, tolerance);
    let even = one_period(z_cos, z_sin, vertical, !flip, tolerance);
    let heights = || odd.iter().chain(&even).map(|point| point.1);
    let (least, most) = (
        heights().fold(f64::MAX, f64::min),
        heights().fold(f64::MIN, f64::max),
    );
    let mut waves = Vec::new();
    let mut number = ((lo.1 - most - lower_bound) / PI).ceil() as i64;
    while lower_bound + number as f64 * PI + least <= hi.1 {
        let period = if number % 2 == 0 { &odd } else { &even };
        let mut wave = wave(
            period,
            lo.0,
            hi.0,
            lower_bound + number as f64 * PI,
            scale,
            vertical,
        );
        for point in &mut wave {
            point.x += origin.x;
            point.y += origin.y;
        }
        waves.push(wave);
        number += 1;
    }
    waves
}

/// The height of a gyroid wave at `x`, for a layer whose height has the
/// given sine and cosine.
fn wave_height(x: f64, z_sin: f64, z_cos: f64, vertical: bool, flip: bool) -> f64 {
    if vertical {
        let phase = if z_cos < 0.0 { PI } else { 0.0 } + PI;
        let a = (x + phase).sin();
        let b = -z_cos;
        let res = z_sin * (x + phase + if flip { PI } else { 0.0 }).cos();
        let r = a.hypot(b);
        (a / r).asin() + (res / r).asin() + PI
    } else {
        let phase = if z_sin < 0.0 { PI } else { 0.0 };
        let a = (x + phase).cos();
        let b = -z_sin;
        let res = z_cos * (x + phase + if flip { 0.0 } else { PI }).sin();
        let r = a.hypot(b);
        (a / r).asin() + (res / r).asin() + 0.5 * PI
    }
}

/// One turn of a wave as points, more of them where it bends.
fn one_period(
    z_cos: f64,
    z_sin: f64,
    vertical: bool,
    flip: bool,
    tolerance: f64,
) -> Vec<(f64, f64)> {
    let height = |x: f64| wave_height(x, z_sin, z_cos, vertical, flip);
    let limit = 2.0 * PI;
    let mut points = Vec::new();
    let mut x = 0.0;
    while x < limit - 1e-4 {
        points.push((x, height(x)));
        x += PI / 2.0;
    }
    points.push((limit, height(limit)));
    loop {
        let size = points.len();
        for at in 1..size {
            let (left, right) = (points[at - 1], points[at]);
            let x = left.0 + (right.0 - left.0) / 2.0;
            let middle = (x, height(x));
            let cross = (middle.0 - left.0) * (middle.1 - right.1)
                - (middle.1 - left.1) * (middle.0 - right.0);
            if cross.abs() > tolerance * tolerance {
                points.push(middle);
            }
        }
        if points.len() == size {
            return points;
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
}

/// A wave from `x_min` to `x_max`, `offset` across, in scaled units.
fn wave(
    period: &[(f64, f64)],
    x_min: f64,
    x_max: f64,
    offset: f64,
    scale: f64,
    vertical: bool,
) -> Vec<Point2> {
    let length = period[period.len() - 1].0;
    let count = period.len() - 1;
    let mut start = (x_min / length).floor() * length;
    let mut at = 0;
    while at + 1 < count && start + period[at + 1].0 <= x_min {
        at += 1;
    }
    let mut points = Vec::new();
    loop {
        let (mut x, mut y) = (start + period[at].0, period[at].1 + offset);
        let last = x >= x_max;
        if vertical {
            (x, y) = (y, x);
        }
        // Cut off, not rounded, as the reference's cast does.
        points.push(Point2 {
            x: (x * scale) as Coord,
            y: (y * scale) as Coord,
        });
        if last {
            return points;
        }
        at += 1;
        if at == count {
            at = 0;
            start += length;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(side: f64) -> Bounds {
        (Point2::from_mm(0.0, 0.0), Point2::from_mm(side, side))
    }

    fn length(lines: &[Vec<Point2>]) -> f64 {
        lines
            .iter()
            .flat_map(|line| line.windows(2))
            .map(|pair| ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64))
            .sum::<f64>()
            / UNITS_PER_MM
    }

    #[test]
    fn cross_hatch_runs_one_way_then_the_other_with_zigzags_between() {
        // Lines 0.45 mm apart at 20%: 2.43 mm between rows, straight for
        // most of a row's worth of height, then turning over two fifths.
        let straight = |z: f64| {
            let lines = crosshatch(bounds(40.0), 0.45, 0.2, z);
            assert!(!lines.is_empty(), "nothing at {z}");
            let along_x = lines
                .iter()
                .all(|line| line.len() == 2 && line[0].y == line[1].y);
            let along_y = lines
                .iter()
                .all(|line| line.len() == 2 && line[0].x == line[1].x);
            (along_x, along_y)
        };
        let kinds: Vec<(bool, bool)> = (1..=40)
            .map(|layer| straight(0.2 * f64::from(layer)))
            .collect();
        // Both directions turn up, and layers of zigzags between them.
        assert!(kinds.contains(&(true, false)));
        assert!(kinds.contains(&(false, true)));
        assert!(kinds.contains(&(false, false)));
        // A straight layer's rows are a pitch apart, on the shared grid.
        let layer = kinds.iter().position(|kind| kind.0 || kind.1).unwrap();
        let rows = crosshatch(bounds(40.0), 0.45, 0.2, 0.2 * (layer + 1) as f64);
        let pitch = ((0.45e6 / 0.2) as i64 as f64 * 1.08) as i64;
        let across: Vec<i64> = rows
            .iter()
            .map(|line| {
                if line[0].y == line[1].y {
                    line[0].y
                } else {
                    line[0].x
                }
            })
            .collect();
        assert!(
            across
                .windows(2)
                .all(|pair| (pair[1] - pair[0] - pitch).abs() <= 1)
        );
        assert_eq!(across[0], 0);
        // About the same plastic whichever kind of layer it is.
        let (first, turning) = (
            length(&crosshatch(bounds(40.0), 0.45, 0.2, 0.2)),
            length(&crosshatch(bounds(40.0), 0.45, 0.2, 1.2)),
        );
        assert!(first > 500.0 && turning > 500.0);
    }

    #[test]
    fn gyroid_waves_cover_the_area_and_swing_as_the_layers_go_up() {
        let area = (Point2::from_mm(-20.0, -20.0), Point2::from_mm(20.0, 20.0));
        let mut along_x = 0;
        let mut along_y = 0;
        for layer in 1..=60 {
            let waves = gyroid(area, 0.45, 0.15, 0.2 * f64::from(layer));
            assert!(waves.len() > 3);
            for wave in &waves {
                // Each wave goes right across, in steps no longer than a
                // few millimetres.
                let (first, last) = (wave[0], wave[wave.len() - 1]);
                let (dx, dy) = ((last.x - first.x).abs(), (last.y - first.y).abs());
                assert!(dx.max(dy) >= 40_000_000, "a wave stops short");
                if dx > dy {
                    along_x += 1;
                } else {
                    along_y += 1;
                }
            }
        }
        assert!(along_x > 0 && along_y > 0);
        // Denser fill, more waves.
        let few = gyroid(area, 0.45, 0.1, 1.0).len();
        let many = gyroid(area, 0.45, 0.3, 1.0).len();
        assert!(many > 2 * few);
    }

    #[test]
    fn aligning_goes_down_for_negative_coordinates_too() {
        assert_eq!(align(7, 5), 5);
        assert_eq!(align(-7, 5), -10);
        assert_eq!(align(-10, 5), -10);
        assert_eq!(align(0, 5), 0);
    }
}
