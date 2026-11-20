//! A wall cut open where purges come in (WipeTower2.cpp
//! `TowerInterface::cut_wall_gaps`, with `move_point_along_polygon`,
//! `insert_points` and `ray_intersetion_line`), in single precision as
//! there.

use edge_geom::{Point2, Polygon};

use super::writer::V2;

/// `unscale(p).cast<float>()`: the integer times the scaling factor in
/// double precision, then a single.
fn unscaled(p: Point2) -> V2 {
    ((p.x as f64 * 1e-6) as f32, (p.y as f64 * 1e-6) as f32)
}

/// `scaled(Vec2f)`: a single over the scaling factor as a single,
/// truncated (Point.hpp, `(v / SCALING_FACTOR).cast<coord_t>()`).
fn scaled(p: V2) -> Point2 {
    Point2 {
        x: (p.0 / 1e-6f32) as i64,
        y: (p.1 / 1e-6f32) as i64,
    }
}

/// libslic3r's `EPSILON`.
const EPSILON: f32 = 1e-4;

/// Where a gap is, and on which side of the tower it opens
/// (`TowerInterface::GapPoint`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct GapPoint {
    pub pos: V2,
    pub is_left: bool,
}

#[derive(Clone, Copy, Debug)]
struct Intersection {
    pos: V2,
    idx: usize,
    pair: i32,
    from_idx: f32,
    forward: bool,
}

#[derive(Clone, Copy, Debug)]
struct Flagged {
    pos: V2,
    pair: i32,
    forward: bool,
}

fn cross(a: V2, b: V2) -> f32 {
    a.0 * b.1 - a.1 * b.0
}

fn norm(v: V2) -> f32 {
    (v.0 * v.0 + v.1 * v.1).sqrt()
}

fn sub(a: V2, b: V2) -> V2 {
    (a.0 - b.0, a.1 - b.1)
}

/// `ray_intersetion_line`: where the ray from `a` along `v1` meets the
/// segment from `b` to `c`.
fn ray_meets(a: V2, v1: V2, b: V2, c: V2) -> Option<V2> {
    let v2 = sub(c, b);
    let denom = f64::from(cross(v1, v2));
    if denom.abs() < 1e-4 {
        return None;
    }
    let v12 = sub(a, b);
    let t1 = f64::from(cross(v2, v12)) / denom;
    let t2 = f64::from(cross(v1, v12)) / denom;
    if t1 >= 0.0 && (0.0..=1.0).contains(&t2) {
        let t = t1 as f32;
        Some((a.0 + t * v1.0, a.1 + t * v1.1))
    } else {
        None
    }
}

/// `move_point_along_polygon`.
fn move_along(
    points: &[V2],
    start: V2,
    start_idx: usize,
    offset: f32,
    forward: bool,
    pair: i32,
) -> Intersection {
    let n = points.len();
    let mut remaining = offset;
    let normalized = |v: V2| {
        let l = norm(v);
        if l > 0.0 { (v.0 / l, v.1 / l) } else { v }
    };
    if forward {
        let next = (start_idx + 1) % n;
        remaining -= norm(sub(points[next], start));
        if remaining <= 0.0 {
            let dir = normalized(sub(points[next], start));
            let pos = (start.0 + dir.0 * offset, start.1 + dir.1 * offset);
            return Intersection {
                pos,
                idx: start_idx,
                pair,
                from_idx: norm(sub(points[start_idx], pos)),
                forward,
            };
        }
        let mut i = (start_idx + 1) % n;
        while i != start_idx {
            let length = norm(sub(points[(i + 1) % n], points[i]));
            if remaining <= length {
                let ratio = remaining / length;
                let d = sub(points[(i + 1) % n], points[i]);
                return Intersection {
                    pos: (points[i].0 + ratio * d.0, points[i].1 + ratio * d.1),
                    idx: i,
                    pair,
                    from_idx: remaining,
                    forward,
                };
            }
            remaining -= length;
            i = (i + 1) % n;
        }
        let idx = (start_idx + n - 1) % n;
        let pos = points[start_idx];
        Intersection {
            pos,
            idx,
            pair,
            from_idx: norm(sub(pos, points[idx])),
            forward,
        }
    } else {
        let next = (start_idx + 1) % n;
        remaining -= norm(sub(points[start_idx], start));
        if remaining <= 0.0 {
            let dir = normalized(sub(points[next], points[start_idx]));
            let pos = (start.0 - dir.0 * offset, start.1 - dir.1 * offset);
            return Intersection {
                pos,
                idx: start_idx,
                pair,
                from_idx: norm(sub(pos, points[start_idx])),
                forward,
            };
        }
        let mut i = (start_idx + n - 1) % n;
        while i != start_idx {
            let length = norm(sub(points[(i + 1) % n], points[i]));
            if remaining <= length {
                let ratio = remaining / length;
                let d = sub(points[(i + 1) % n], points[i]);
                let b = points[(i + 1) % n];
                return Intersection {
                    pos: (b.0 - ratio * d.0, b.1 - ratio * d.1),
                    idx: i,
                    pair,
                    from_idx: length - remaining,
                    forward,
                };
            }
            remaining -= length;
            i = (i + n - 1) % n;
        }
        Intersection {
            pos: points[start_idx],
            idx: start_idx,
            pair,
            from_idx: 0.0,
            forward,
        }
    }
}

/// `insert_points`.
fn insert(pl: &mut Vec<Flagged>, idx: usize, pos: V2, pair: i32, forward: bool) {
    let next = (idx + 1) % pl.len();
    let (a, b) = (pl[idx].pos, pl[next].pos);
    let sq = |v: V2| v.0 * v.0 + v.1 * v.1;
    if sq(sub(pos, a)) < EPSILON {
        pl[idx].pair = pair;
        pl[idx].forward = forward;
    } else if sq(sub(pos, b)) < EPSILON {
        pl[next].pair = pair;
        pl[next].forward = forward;
    } else {
        pl.insert(idx + 1, Flagged { pos, pair, forward });
    }
}

/// The nearest crossing of a gap's ray with the closed wall.
fn nearest_crossing(points: &[V2], gap: &GapPoint) -> Option<(usize, V2)> {
    let ray = if gap.is_left { (-1.0, 0.0) } else { (1.0, 0.0) };
    let mut best: Option<(f32, usize, V2)> = None;
    for j in 0..points.len() {
        if let Some(pos) = ray_meets(gap.pos, ray, points[j], points[(j + 1) % points.len()]) {
            let t = norm(sub(pos, gap.pos));
            if best.is_none_or(|(least, ..)| t < least) {
                best = Some((t, j, pos));
            }
        }
    }
    best.map(|(_, j, pos)| (j, pos))
}

/// The wall cut open `gap_length` along it either side of each gap's
/// crossing, as open pieces in wall order.
pub(crate) fn cut_wall_gaps(
    polygon: &Polygon,
    gaps: &[GapPoint],
    gap_length: f32,
) -> Vec<Vec<Point2>> {
    if gaps.is_empty() || polygon.points.len() < 3 {
        let mut whole = polygon.points.clone();
        if let Some(first) = whole.first().copied() {
            whole.push(first);
        }
        return vec![whole];
    }
    let raw: Vec<V2> = polygon.points.iter().map(|p| unscaled(*p)).collect();
    let crossings: Vec<V2> = gaps
        .iter()
        .filter_map(|gap| nearest_crossing(&raw, gap).map(|(_, pos)| pos))
        .collect();
    // From the vertex farthest from every gap, so no gap lies across the
    // start.
    let mut anchor = 0;
    let mut best = -1.0f32;
    for (k, p) in raw.iter().enumerate() {
        let d = crossings
            .iter()
            .map(|c| norm(sub(*p, *c)))
            .fold(f32::MAX, f32::min);
        if d > best {
            best = d;
            anchor = k;
        }
    }
    let n = raw.len();
    let points: Vec<V2> = (0..n).map(|k| raw[(anchor + k) % n]).collect();

    let mut inter = Vec::new();
    for (i, gap) in gaps.iter().enumerate() {
        let Some((j, pos)) = nearest_crossing(&points, gap) else {
            continue;
        };
        let forward = move_along(&points, pos, j, gap_length, true, i as i32);
        let mut backward = move_along(&points, pos, j, gap_length, false, i as i32);
        backward.forward = false;
        inter.push(backward);
        inter.push(Intersection {
            forward: true,
            ..forward
        });
    }
    let mut pl: Vec<Flagged> = points
        .iter()
        .map(|p| Flagged {
            pos: *p,
            pair: -1,
            forward: false,
        })
        .collect();
    inter.sort_by(|a, b| {
        if a.idx == b.idx {
            a.from_idx.total_cmp(&b.from_idx)
        } else {
            a.idx.cmp(&b.idx)
        }
    });
    for at in inter.iter().rev() {
        insert(&mut pl, at.idx, at.pos, at.pair, at.forward);
    }

    let mut result = Vec::new();
    let begin = 0;
    let mut skip = true;
    let mut i = begin;
    let mut piece: Vec<Point2> = Vec::new();
    loop {
        if skip || pl[i].pair == -1 {
            piece.push(scaled(pl[i].pos));
            i = (i + 1) % pl.len();
            skip = false;
        } else {
            if !piece.is_empty() {
                piece.push(scaled(pl[i].pos));
                result.push(std::mem::take(&mut piece));
            }
            let mut left = pl[i].pair;
            let mut j = (i + 1) % pl.len();
            while j != begin && pl[j].pair != left {
                if pl[j].pair != -1 && !pl[j].forward {
                    left = pl[j].pair;
                }
                j = (j + 1) % pl.len();
            }
            i = j;
            skip = true;
        }
        if i == begin {
            break;
        }
    }
    if !piece.is_empty() {
        if pl[i].pair == -1 {
            piece.push(scaled(pl[i].pos));
        }
        result.push(piece);
    }
    result
}
