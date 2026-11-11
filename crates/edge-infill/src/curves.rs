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

/// Honeycomb: columns of half hexagons, each column and the next making
/// a row of whole ones, turned by `turn` radians about the middle of the
/// first hexagon (FillHoneycomb.cpp). `bounds` are of the area as it
/// lies, not turned.
pub(crate) fn honeycomb(bounds: Bounds, spacing: f64, density: f64, turn: f64) -> Vec<Vec<Point2>> {
    let min_spacing = (spacing * UNITS_PER_MM) as Coord;
    if density <= 0.0 || min_spacing <= 0 {
        return Vec::new();
    }
    let distance = (min_spacing as f64 / density) as Coord;
    let root3 = 3f64.sqrt();
    let hex_side = (distance as f64 / (root3 / 2.0)) as Coord;
    let hex_width = distance * 2;
    let pattern_height = hex_side * 3;
    let y_short = (distance as f64 * root3 / 3.0) as Coord;
    let x_offset = min_spacing / 2;
    let y_offset = (x_offset as f64 * root3 / 3.0) as Coord;
    let centre = Point2 {
        x: hex_width / 2,
        y: hex_side,
    };
    let about = |p: Point2, angle: f64| {
        let (sin, cos) = angle.sin_cos();
        let (x, y) = ((p.x - centre.x) as f64, (p.y - centre.y) as f64);
        Point2 {
            x: (centre.x as f64 + cos * x - sin * y).round() as Coord,
            y: (centre.y as f64 + cos * y + sin * x).round() as Coord,
        }
    };
    // The area's bounds turned with the pattern, squared off again, and
    // begun on the pattern's own grid.
    let (low, high) = bounds;
    let corners = [
        low,
        Point2 {
            x: high.x,
            y: low.y,
        },
        high,
        Point2 {
            x: low.x,
            y: high.y,
        },
    ]
    .map(|corner| about(corner, turn));
    let (mut low, mut high) = (corners[0], corners[0]);
    for corner in corners {
        (low.x, low.y) = (low.x.min(corner.x), low.y.min(corner.y));
        (high.x, high.y) = (high.x.max(corner.x), high.y.max(corner.y));
    }
    low = Point2 {
        x: align(low.x, hex_width).min(low.x),
        y: align(low.y, pattern_height).min(low.y),
    };

    let mut lines = Vec::new();
    let mut x = low.x;
    while x <= high.x {
        let mut line: Vec<Point2> = Vec::new();
        let mut across = [x + x_offset, x + distance - x_offset];
        for _ in 0..2 {
            // The first half is turned upside down for the second to
            // carry on from.
            line.reverse();
            let mut y = low.y;
            while y <= high.y {
                line.extend([
                    Point2 {
                        x: across[1],
                        y: y + y_offset,
                    },
                    Point2 {
                        x: across[0],
                        y: y + y_short - y_offset,
                    },
                    Point2 {
                        x: across[0],
                        y: y + y_short + hex_side + y_offset,
                    },
                    Point2 {
                        x: across[1],
                        y: y + 2 * y_short + hex_side - y_offset,
                    },
                    Point2 {
                        x: across[1],
                        y: y + 2 * y_short + 2 * hex_side + y_offset,
                    },
                ]);
                y += 2 * (y_short + hex_side);
            }
            across = [across[1] + distance, across[0] + distance];
            x += distance;
        }
        lines.push(line.into_iter().map(|p| about(p, -turn)).collect());
    }
    lines
}

/// 3D honeycomb: a slice through truncated octahedra packed together,
/// which is rows of zigzags along x for half the height of a cell and
/// along y for the other half, with a stub at each end of a row
/// (Fill3DHoneycomb.cpp). `height` is the layer's thickness in mm. Laid
/// one line wide and without the reference's closing of cell tops, which
/// is off unless asked for.
pub(crate) fn honeycomb_3d(
    bounds: Bounds,
    spacing: f64,
    density: f64,
    z: f64,
    height: f64,
) -> Vec<Vec<Point2>> {
    let (mut low, mut high) = bounds;
    let grow = (5.0 * spacing * UNITS_PER_MM) as Coord;
    (low.x, low.y, high.x, high.y) = (low.x - grow, low.y - grow, high.x + grow, high.y + grow);
    let line = spacing * UNITS_PER_MM;
    let layer = height * UNITS_PER_MM;
    let mut z_scale = 2f64.sqrt();
    let mut grid = line * ((z_scale + 1.0) / 2.0) / density;
    let per_module = (grid * 2.0 / (z_scale * layer) + 0.05).floor();
    if density > 0.42 || per_module < 2.0 {
        grid = line * 1.1 / density;
        z_scale = grid * 2.0 / (2.0 * layer);
    }
    if grid <= 0.0 || !grid.is_finite() {
        return Vec::new();
    }
    let cell = (grid * 2.0).round() as Coord;
    low = Point2 {
        x: align(low.x, cell).min(low.x),
        y: align(low.y, cell).min(low.y),
    };
    let (length_x, length_y) = ((high.x - low.x) as f64, (high.y - low.y) as f64);
    let z = z * UNITS_PER_MM * z_scale + grid / 2.0 + layer / 2.0;

    const TINY: f64 = 1e-4;
    let sign = |value: f64| f64::from(i8::from(value > 0.0) - i8::from(value < 0.0));
    // A triangle wave over two cells, between nothing and half a cell.
    let tri = |at: f64| {
        let turn = (at / (grid * 2.0)) as f32 + 0.25;
        let turn = f64::from(turn - (turn as i32) as f32);
        (1.0 - (turn * 8.0 - 4.0).abs()) * (grid / 4.0) + grid / 4.0
    };
    // The same, cut off at half what the height's own wave says.
    let troct = |at: f64| {
        let limit = tri(z) / 2.0;
        let wave = tri(at);
        if wave.abs() > limit.abs() {
            sign(wave) * limit
        } else {
            wave * sign(limit)
        }
    };
    let share = (tri(z) / 2.0).abs() / grid;
    if share <= 0.0 {
        return Vec::new();
    }
    // Where along a cell the zigzag turns.
    let turns = [
        grid * share,
        grid * (1.0 - share),
        grid * (1.0 + share),
        grid * (2.0 - share),
    ];
    let cycle = ((z + grid / 2.0) % (grid * 2.0)) / (grid * 2.0);
    let upright = cycle < 0.5;
    let reach = |length: f64| ((length - TINY) / grid).ceil() * grid;
    // Along x when `upright`, with the points' coordinates swapped.
    let place = |along: f64, across: f64| {
        if upright {
            point(across, along)
        } else {
            point(along, across)
        }
    };

    let mut lines: Vec<Vec<Point2>> = Vec::new();
    // The zigzags: one a cell, leaning alternate ways, drawn to and fro.
    let run = reach(if upright { length_y } else { length_x });
    let mut lean = -1.0;
    let mut backwards = false;
    let mut across = 0.0;
    while across < reach(if upright { length_x } else { length_y }) {
        let mut zigzag = vec![place(0.0, 0.0)];
        let (mut along, mut first) = (0.0, 0usize);
        while along < run {
            for turn in turns.iter().skip(first).take(2) {
                zigzag.push(place(
                    along - (first as f64 * grid / 2.0) + turn,
                    troct(*turn) * lean,
                ));
            }
            along += grid;
            first = (first + 2) % 4;
        }
        zigzag.push(place(run, 0.0));
        if backwards {
            zigzag.reverse();
        }
        // Moved across to its own cell.
        let moved = if upright {
            point(across, 0.0)
        } else {
            point(0.0, across)
        };
        for p in &mut zigzag {
            p.x += moved.x;
            p.y += moved.y;
        }
        lines.push(zigzag);
        backwards = !backwards;
        lean = -lean;
        across += grid;
    }

    // The stubs at either end of every second row.
    let flat = !upright;
    let offset = if (sign((cycle % 0.5) - 0.25) > 0.0) == upright {
        0.0
    } else {
        1.0
    };
    let stub = |side: f64| -> Vec<Point2> {
        let at = |a: f64, b: f64| if flat { point(a, b) } else { point(b, a) };
        let mut stub = vec![at(0.0, 0.0)];
        for turn in turns.iter().take(2) {
            stub.push(at(side * troct(*turn).abs(), *turn));
        }
        stub.push(at(0.0, grid));
        stub
    };
    let (start, end) = (stub(-1.0), stub(1.0));
    let (max_x, max_y) = (reach(length_x), reach(length_y));
    let mut at = offset * grid;
    while at < (if flat { max_y } else { max_x }) - TINY {
        let (near, far) = if upright {
            (point(at, 0.0), point(at, max_y))
        } else {
            (point(0.0, at), point(max_x, at))
        };
        for (stub, by) in [(&start, near), (&end, far)] {
            lines.push(
                stub.iter()
                    .map(|p| Point2 {
                        x: p.x + by.x,
                        y: p.y + by.y,
                    })
                    .collect(),
            );
        }
        at += grid * 2.0;
    }
    for p in lines.iter_mut().flatten() {
        p.x += low.x;
        p.y += low.y;
    }
    lines
}

/// TPMS-D's share of full density for a given line spacing
/// (`FillTpmsD::DensityAdjust`).
const TPMS_D_DENSITY: f64 = 2.1;
/// How far its waves may be drawn from the true curve, in mm.
const TPMS_D_TOLERANCE: f64 = 0.1;

/// TPMS-D: slices of the Schwarz D surface, which are waves along the
/// two diagonals (FillTpmsD.cpp `make_waves`). The area must already be
/// turned by the reference's `CorrectionAngle`.
pub(crate) fn tpms_d(bounds: Bounds, spacing: f64, density: f64, z: f64) -> Vec<Vec<Point2>> {
    let density = (density * TPMS_D_DENSITY).max(0.0);
    if density <= 0.0 {
        return Vec::new();
    }
    let (low, high) = bounds;
    let distance = (spacing * UNITS_PER_MM / density) as Coord;
    if distance <= 0 {
        return Vec::new();
    }
    let cell = (2.0 * PI * distance as f64).round() as Coord;
    let low = Point2 {
        x: align(low.x, cell).min(low.x),
        y: align(low.y, cell).min(low.y),
    };
    let width = ((high.x - low.x) as f64 / distance as f64).ceil() + 1.0;
    let height = ((high.y - low.y) as f64 / distance as f64).ceil() + 1.0;

    let scale = spacing * UNITS_PER_MM / density;
    let tolerance = (spacing / 2.0).min(TPMS_D_TOLERANCE) / (scale / UNITS_PER_MM);
    let turn = z * UNITS_PER_MM / scale;
    let (mut a, mut b) = (turn.sin() - turn.cos(), turn.sin() + turn.cos());
    let (mut min_u, mut max_u) = (-height, width);
    let (mut min_v, mut max_v) = (0.0, width + height);
    let swapped = a.abs() > b.abs();
    if swapped {
        (a, b) = (b, a);
        (min_u, min_v) = (min_v, min_u);
        (max_u, max_v) = (max_v, max_u);
    }
    // One turn of the wave, finer where it bends, then as many more as
    // reach across.
    let height_at = |u: f64| (a / b * u.cos()).acos();
    let mut wave: Vec<(f64, f64)> = (0..=16)
        .map(|step| {
            let u = min_u + 2.0 * PI * f64::from(step) / 16.0;
            (u, height_at(u))
        })
        .collect();
    let mut at = 0;
    while at + 1 < wave.len() {
        let (left, right) = (wave[at], wave[at + 1]);
        let u = (left.0 + right.0) / 2.0;
        let v = height_at(u);
        if (v - (left.1 + right.1) / 2.0).abs() > tolerance {
            wave.insert(at + 1, (u, v));
        } else {
            at += 1;
        }
    }
    let mut next = 1;
    while next < wave.len() && wave[wave.len() - 1].0 < max_u {
        wave.push((wave[next].0 + 2.0 * PI, wave[next].1));
        next += 1;
    }
    let mut lines = Vec::new();
    let mut shift = (min_v / (2.0 * PI)).floor() * 2.0 * PI;
    while shift < max_v + 2.0 * PI {
        for forward in [false, true] {
            lines.push(
                wave.iter()
                    .map(|&(u, v)| {
                        let v = if forward { v } else { -v } + shift;
                        let x = (u + v) / 2.0;
                        let y = (v - u) / 2.0 * if swapped { -1.0 } else { 1.0 };
                        Point2 {
                            x: (x * scale).round() as Coord + low.x,
                            y: (y * scale).round() as Coord + low.y,
                        }
                    })
                    .collect(),
            );
        }
        shift += 2.0 * PI;
    }
    lines
}

/// A single line that fills the plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Curve {
    Hilbert,
    Archimedean,
    Octagram,
}

/// How finely a spiral is cut into chords, in mm (the reference's
/// `resolution` setting at its default).
const RESOLUTION: f64 = 0.01;

/// One line over the whole of `bounds`, a pitch between its turns: a
/// Hilbert curve from the least corner, or a spiral from the middle
/// (FillPlanePath.cpp). `bounds` are of the object, turned as the area is.
pub(crate) fn plane_path(
    curve: Curve,
    bounds: Bounds,
    spacing: f64,
    density: f64,
) -> Vec<Vec<Point2>> {
    let pitch = spacing * UNITS_PER_MM / density;
    if pitch <= 0.0 || !pitch.is_finite() {
        return Vec::new();
    }
    let (low, high) = bounds;
    // The spirals turn about the middle; the Hilbert curve starts in the
    // corner.
    let shift = match curve {
        Curve::Hilbert => low,
        _ => Point2 {
            x: (low.x + high.x) / 2,
            y: (low.y + high.y) / 2,
        },
    };
    let cells = |at: Coord| (at as f64 / pitch).ceil() as i64;
    let (min_x, min_y) = (cells(low.x - shift.x), cells(low.y - shift.y));
    let (max_x, max_y) = (cells(high.x - shift.x), cells(high.y - shift.y));
    let mut points: Vec<(f64, f64)> = Vec::new();
    match curve {
        Curve::Archimedean => {
            let resolution = RESOLUTION * UNITS_PER_MM / pitch;
            let reach = (max_x as f64).hypot(max_y as f64) * 2f64.sqrt() + 1.5;
            let (mut theta, mut r) = (0.0f64, 1.0f64);
            points.extend([(0.0, 0.0), (1.0, 0.0)]);
            while r < reach {
                theta += 2.0 * (1.0 - resolution / r).acos();
                r = 1.0 + theta / (2.0 * PI);
                points.push((r * theta.cos(), r * theta.sin()));
            }
        }
        Curve::Octagram => {
            let reach = (max_x as f64).hypot(max_y as f64) * 2f64.sqrt() + 1.5;
            let step = 2f64.sqrt();
            let mut r = 0.0;
            points.push((0.0, 0.0));
            while r < reach {
                r += step;
                let rx = r / step;
                let r2 = r + rx;
                points.extend([
                    (r, 0.0),
                    (r2, rx),
                    (rx, rx),
                    (rx, r2),
                    (0.0, r),
                    (-rx, r2),
                    (-rx, rx),
                    (-r2, rx),
                    (-r, 0.0),
                    (-r2, -rx),
                    (-rx, -rx),
                    (-rx, -r2),
                    (0.0, -r),
                    (rx, -r2),
                    (rx, -rx),
                    (r2 + step, -rx),
                ]);
            }
        }
        Curve::Hilbert => {
            let across = (max_x + 1 - min_x).max(max_y + 1 - min_y).max(0) as u64;
            let mut side = 2u64;
            while side < across {
                side <<= 1;
            }
            for number in 0..side * side {
                let (x, y) = hilbert(number);
                points.push(((x + min_x) as f64, (y + min_y) as f64));
            }
        }
    }
    let line: Vec<Point2> = points
        .into_iter()
        .map(|(x, y)| Point2 {
            x: (x * pitch + 0.5).floor() as Coord + shift.x,
            y: (y * pitch + 0.5).floor() as Coord + shift.y,
        })
        .collect();
    vec![line]
}

/// Where the Hilbert curve is after `number` steps.
fn hilbert(number: u64) -> (i64, i64) {
    const NEXT: [usize; 16] = [4, 0, 0, 12, 0, 4, 4, 8, 12, 8, 8, 4, 8, 12, 12, 0];
    const X: [i64; 16] = [0, 1, 1, 0, 0, 0, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0];
    const Y: [i64; 16] = [0, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1];
    let mut digits = 0;
    let mut rest = number;
    while rest > 0 {
        rest >>= 2;
        digits += 1;
    }
    let mut state = if digits % 2 == 1 { 4 } else { 0 };
    let (mut x, mut y) = (0, 0);
    for digit in (0..digits).rev() {
        state += ((number >> (digit * 2)) & 3) as usize;
        x |= X[state] << digit;
        y |= Y[state] << digit;
        state = NEXT[state];
    }
    (x, y)
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
    fn honeycomb_is_sides_of_hexagons_whichever_way_it_is_turned() {
        // Every stretch is upright or thirty degrees off level, and a
        // turn of sixty degrees leaves that so.
        for turn in [0.0, PI / 3.0, 2.0 * PI / 3.0] {
            let lines = honeycomb(bounds(40.0), 0.45, 0.2, turn);
            assert!(lines.len() > 5);
            for pair in lines.iter().flat_map(|line| line.windows(2)) {
                let (dx, dy) = (
                    (pair[1].x - pair[0].x) as f64,
                    (pair[1].y - pair[0].y) as f64,
                );
                // But for the step, a line wide, where a column turns
                // back at the foot.
                if dx.hypot(dy) < 0.5 * UNITS_PER_MM {
                    continue;
                }
                let lean = dy.atan2(dx).rem_euclid(PI / 3.0);
                assert!(
                    (lean - PI / 6.0).abs() < 1e-3,
                    "a side leans {} degrees",
                    dy.atan2(dx).to_degrees()
                );
            }
        }
        // Twice the plastic, half the distance between columns.
        let (few, many) = (
            honeycomb(bounds(40.0), 0.45, 0.1, 0.0).len(),
            honeycomb(bounds(40.0), 0.45, 0.2, 0.0).len(),
        );
        assert!(many.abs_diff(2 * few) <= 2, "{few} columns, then {many}");
        assert!(honeycomb(bounds(40.0), 0.45, 0.0, 0.0).is_empty());
    }

    #[test]
    fn three_d_honeycomb_runs_along_one_axis_then_the_other() {
        let mut along = [0, 0];
        for layer in 1..=60 {
            let lines = honeycomb_3d(bounds(40.0), 0.45, 0.2, 0.2 * f64::from(layer), 0.2);
            assert!(lines.len() > 6, "layer {layer}");
            // The zigzags are the long lines; the stubs at their ends
            // are a cell long.
            let longest = lines
                .iter()
                .max_by_key(|line| line.len())
                .expect("there are lines");
            let (first, last) = (longest[0], longest[longest.len() - 1]);
            let (dx, dy) = ((last.x - first.x).abs(), (last.y - first.y).abs());
            assert!(dx.max(dy) >= 40_000_000 && dx.min(dy) == 0);
            along[usize::from(dy > dx)] += 1;
        }
        assert!(along[0] > 10 && along[1] > 10, "{along:?}");
        assert!(honeycomb_3d(bounds(40.0), 0.45, 0.0, 1.0, 0.2).is_empty());
    }

    #[test]
    fn tpms_d_waves_cross_the_area_and_change_with_height() {
        let area = (Point2::from_mm(-20.0, -20.0), Point2::from_mm(20.0, 20.0));
        let low = tpms_d(area, 0.45, 0.15, 0.2);
        assert!(low.len() > 6);
        assert_ne!(low, tpms_d(area, 0.45, 0.15, 1.0));
        // Every corner of the area has a wave past it.
        let reach = |pick: fn(&Point2) -> Coord| {
            let all = low.iter().flatten().map(pick);
            (all.clone().min().unwrap(), all.max().unwrap())
        };
        let ((x0, x1), (y0, y1)) = (reach(|p| p.x), reach(|p| p.y));
        assert!(x0 <= -20_000_000 && x1 >= 20_000_000 && y0 <= -20_000_000 && y1 >= 20_000_000);
        assert!(tpms_d(area, 0.45, 0.3, 0.2).len() > low.len());
        assert!(tpms_d(area, 0.45, 0.0, 0.2).is_empty());
    }

    #[test]
    fn curves_over_the_plane_are_one_line_a_pitch_apart() {
        let pitch = 0.45e6 / 0.2;
        // Hilbert: every step one pitch along x or y, no point twice,
        // from the least corner to past the greatest.
        let hilbert = plane_path(Curve::Hilbert, bounds(40.0), 0.45, 0.2);
        assert_eq!(hilbert.len(), 1);
        let line = &hilbert[0];
        assert_eq!(line[0], Point2::from_mm(0.0, 0.0));
        for pair in line.windows(2) {
            let (dx, dy) = ((pair[1].x - pair[0].x).abs(), (pair[1].y - pair[0].y).abs());
            assert!(dx.min(dy) == 0 && (dx.max(dy) as f64 - pitch).abs() <= 1.0);
        }
        let mut seen: Vec<Point2> = line.clone();
        seen.sort_by_key(|p| (p.x, p.y));
        seen.dedup();
        assert_eq!(seen.len(), line.len());
        assert!(line.iter().any(|p| p.x >= 40_000_000 && p.y >= 40_000_000));

        // The spirals start in the middle and wind out past the corners,
        // the Archimedean one a pitch further every turn.
        let middle = Point2::from_mm(20.0, 20.0);
        let out = |p: &Point2| ((p.x - middle.x) as f64).hypot((p.y - middle.y) as f64);
        for curve in [Curve::Archimedean, Curve::Octagram] {
            let spiral = plane_path(curve, bounds(40.0), 0.45, 0.2);
            assert_eq!(spiral.len(), 1);
            assert_eq!(spiral[0][0], middle);
            let furthest = spiral[0].iter().map(out).fold(0.0, f64::max);
            assert!(furthest > 20e6 * 2f64.sqrt(), "{curve:?} stops short");
        }
        let chords = &plane_path(Curve::Archimedean, bounds(40.0), 0.45, 0.2)[0];
        assert!(
            chords[1..]
                .windows(2)
                .all(|pair| out(&pair[1]) >= out(&pair[0]) - 1.0)
        );
        // Where it crosses the line to the right of the middle, turn after turn.
        let crossings: Vec<f64> = chords
            .windows(2)
            .filter(|pair| pair[0].y < middle.y && pair[1].y >= middle.y && pair[1].x > middle.x)
            .map(|pair| out(&pair[1]))
            .collect();
        assert!(crossings.len() > 5);
        assert!(
            crossings
                .windows(2)
                .all(|pair| ((pair[1] - pair[0]) / pitch - 1.0).abs() < 0.05)
        );
        assert!(plane_path(Curve::Hilbert, bounds(40.0), 0.45, 0.0).is_empty());
    }

    #[test]
    fn hilbert_steps_to_a_neighbour_every_time() {
        assert_eq!(hilbert(0), (0, 0));
        let points: Vec<(i64, i64)> = (0..256).map(hilbert).collect();
        assert!(
            points
                .windows(2)
                .all(|pair| (pair[1].0 - pair[0].0).abs() + (pair[1].1 - pair[0].1).abs() == 1)
        );
        // 256 steps fill a square sixteen across.
        assert!(
            points
                .iter()
                .all(|&(x, y)| (0..16).contains(&x) && (0..16).contains(&y))
        );
    }

    #[test]
    fn aligning_goes_down_for_negative_coordinates_too() {
        assert_eq!(align(7, 5), 5);
        assert_eq!(align(-7, 5), -10);
        assert_eq!(align(-10, 5), -10);
        assert_eq!(align(0, 5), 0);
    }
}
