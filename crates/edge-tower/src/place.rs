//! Where the tower goes when its saved place no longer fits it, as the
//! reference's command line puts it before slicing (Snapmaker_Orca.cpp,
//! "the stored (or default) tower position may not fit the tower these
//! plates need"; PartPlate.cpp `estimate_wipe_tower_polygon`, Orca #16728):
//! the tower's size is estimated from the settings before anything is
//! sliced (WipeTowerEstimate.cpp `estimate_wipe_tower_footprint`), its
//! place held a millimetre and its brim inside the bed, and a place more
//! than 5 mm out moved 15 mm and its brim in, onto free ground nearest the
//! saved place where the objects leave some.
//!
//! Worked in single precision where the reference works in it.

use edge_clip::ClipError;
use edge_config::{ConfigError, Settings};
use edge_geom::{ExPolygon, Point2, Polygon, convex_hull, scaled, unscaled};

use crate::{TowerSettings, least_depth};

/// How far inside the bed a tower placed by hand has to stand, and one
/// placed by the slicer (libslic3r.h `WIPE_TOWER_MARGIN`,
/// `WIPE_TOWER_AUTO_MARGIN`).
const MARGIN: f32 = 1.0;
const AUTO_MARGIN: f32 = 15.0;
/// A place this far out of bounds or less is taken as dropped just past
/// them, and only pulled back onto them (`estimate_wipe_tower_polygon`).
const TOLERANCE: f32 = 5.0;

/// What the tower's place is judged against.
#[derive(Clone, Debug)]
pub struct Ground {
    /// The bed, as min x, min y, max x, max y in mm.
    pub bed: [f64; 4],
    /// The filaments the plate prints in, counting from 0.
    pub filaments: Vec<usize>,
    /// The thinnest layer the plate's objects are sliced at, and the
    /// first layer's height.
    pub layer_height: f64,
    pub first_layer_height: f64,
    /// The tallest object, in mm.
    pub height: f64,
}

/// A tower moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moved {
    pub x: f64,
    pub y: f64,
    /// Whether it had to go onto an object, for want of free ground.
    pub onto_objects: bool,
}

/// The tower's size as estimated before slicing, in mm: width, depth and
/// the brim's real width (WipeTowerEstimate.cpp
/// `estimate_wipe_tower_footprint`).
fn footprint(
    settings: &Settings,
    tower: &TowerSettings,
    ground: &Ground,
) -> Result<Option<(f32, f32, f32)>, ConfigError> {
    let count = ground.filaments.len();
    if count < 2 || ground.layer_height <= 0.0 {
        return Ok(None);
    }
    let number = |key: &str, default: f64| -> Result<f64, ConfigError> {
        Ok(settings
            .floats(key)?
            .and_then(|values| values.first().copied().flatten())
            .unwrap_or(default))
    };
    let first_layer = ground.first_layer_height;
    let layer = if first_layer > 0.0 {
        ground.layer_height.min(first_layer)
    } else {
        ground.layer_height
    } as f32;
    let type1 = tower.bambu_tower;
    let width = tower.width as f32;
    let spacing = (tower.extra_spacing * 1.0) as f32;
    let nozzle = tower.nozzle_diameter as f32;
    let height = ground.height as f32;
    let rib = match tower.wall {
        crate::Wall::Ribs { width, extra, .. } => Some((width as f32, extra as f32)),
        crate::Wall::Box => None,
    };
    // Bambu Studio's prime_tower_line_width (Type1 only; 0 is from the nozzle).
    let line_width = if type1 {
        number("prime_tower_line_width", 0.0)?.max(0.0) as f32
    } else {
        0.0
    };
    let pw = if line_width > 0.0 {
        line_width
    } else {
        nozzle * 1.25
    };
    let mut volume = tower.prime_volume as f32 * (count - 1) as f32;
    let semm_flush = tower.purge_in_tower;
    if semm_flush {
        volume = semm_flush_volume(tower, count);
    }
    let min_depth = least_depth(f64::from(height)) as f32;
    let (w, depth) = if type1 {
        let purges: Vec<f32> = (ground.filaments.iter())
            .map(|filament| {
                let own = settings
                    .floats("filament_prime_volume")
                    .ok()
                    .flatten()
                    .and_then(|volumes| {
                        volumes.get(*filament).or(volumes.last()).copied().flatten()
                    })
                    .unwrap_or(tower.prime_volume);
                if settings.text("prime_volume_mode") == Some("Saving") {
                    15.0
                } else {
                    own as f32
                }
            })
            .collect();
        let categories: Vec<i64> = (ground.filaments.iter())
            .map(|filament| {
                settings
                    .texts("filament_adhesiveness_category")
                    .and_then(|all| all.get(*filament).or(all.last()).cloned())
                    .and_then(|text| text.trim().parse().ok())
                    .unwrap_or(0)
            })
            .collect();
        let blocks = |width: f32| blocks_depth(&purges, &categories, width, layer, pw, spacing);
        match rib {
            Some((rib_width, extra)) => {
                let square = align_ceil((blocks(width) * width).sqrt(), pw);
                let side = rib_side(square, blocks(square), rib_width, extra, height);
                (side, side)
            }
            None => (width, blocks(width).max(min_depth + pw)),
        }
    } else {
        match rib {
            Some((rib_width, extra)) => {
                let square = if volume > 1e-4 {
                    (volume / layer * spacing).sqrt()
                } else {
                    min_depth
                };
                let side = rib_side(square, square, rib_width, extra, height);
                (side, side)
            }
            None => {
                let mut depth = volume / (layer * width);
                // The flushing volumes already hold the spacing.
                if !semm_flush {
                    depth *= spacing;
                }
                (width, depth.max(min_depth))
            }
        }
    };
    // The brim as it is laid, in whole loops (WipeTower.cpp
    // `estimate_brim_real_width`); "auto" (-1) by the tower's height.
    let mut brim = number("prime_tower_brim_width", 3.0)? as f32;
    if brim < 0.0 {
        brim = if height < 100.0 {
            height / 100.0 * 8.0
        } else {
            8.0
        };
    }
    if brim > 0.0 {
        let laid = if first_layer > 0.0 {
            first_layer
        } else {
            ground.layer_height
        } as f32;
        let spacing = pw - laid * (1.0 - std::f32::consts::FRAC_PI_4);
        if spacing > 1e-4 {
            let loops = ((brim + spacing / 2.0) / spacing) as i32;
            brim = loops as f32 * spacing + if type1 { spacing / 2.0 } else { 0.0 };
        }
    }
    Ok(Some((w, depth, brim)))
}

fn align_ceil(value: f32, base: f32) -> f32 {
    (value / base).ceil() * base
}

/// How wide a tower with ribs stands, ribs and all, as a square
/// (WipeTower.cpp `rib_footprint_side`).
fn rib_side(width: f32, depth: f32, rib_width: f32, extra: f32, height: f32) -> f32 {
    if width < 1e-4 || depth < 1e-4 {
        return 0.0;
    }
    let diagonal = (width * width + depth * depth).sqrt();
    let least = least_depth(f64::from(height)) as f32;
    let mut length = diagonal;
    if depth + 1e-4 < least {
        length = length.max(least * 2f32.sqrt());
    }
    length = diagonal.max(length + extra);
    let rib = rib_width.min(width.min(depth) / 2.0);
    let per_side = ((length - diagonal) / 2.0 + rib / 2.0) / 2f32.sqrt();
    width.max(depth) + 2.0 * per_side
}

/// How deep a Bambu Lab tower's blocks are for one purge of each filament
/// (WipeTower.cpp `estimate_tower_blocks_depth`, without the second
/// nozzle of a two-nozzle printer).
fn blocks_depth(
    purges: &[f32],
    categories: &[i64],
    width: f32,
    layer: f32,
    pw: f32,
    spacing: f32,
) -> f32 {
    let line = width - 2.0 * pw;
    if purges.is_empty() || line <= 1e-4 {
        return 0.0;
    }
    let area = layer * (pw - layer * (1.0 - std::f32::consts::FRAC_PI_4));
    let gap = pw + (spacing - 1.0) * pw;
    // By category: its depth, its smallest purge and how many it holds.
    let mut blocks: Vec<(i64, f32, f32, usize)> = Vec::new();
    for (purge, category) in purges.iter().zip(categories) {
        let depth = (purge / area / line).ceil() * gap;
        match blocks.iter_mut().find(|block| block.0 == *category) {
            Some(block) => {
                block.1 += depth;
                block.2 = block.2.min(depth);
                block.3 += 1;
            }
            None => blocks.push((*category, depth, depth, 1)),
        }
    }
    blocks.sort_by_key(|block| block.0);
    let mut depth = pw;
    for (_, sum, least, count) in blocks {
        depth += if count == purges.len() {
            sum - least
        } else {
            sum
        };
    }
    depth
}

/// What one nozzle's tower purges at most in a layer, as estimated
/// before slicing (WipeTower2.cpp `estimate_semm_flush_volume`): each
/// filament's largest flush to another, times the multiplier and at
/// least its least purge, summed, scaled to the filaments in use, and
/// cut to six tenths ("it's overshooting a bit").
fn semm_flush_volume(tower: &TowerSettings, count: usize) -> f32 {
    let matrix = &tower.flush_matrix;
    let n = ((matrix.len() as f64).sqrt() + 1e-9) as usize;
    if n == 0 {
        return 0.0;
    }
    let mut most = 0.0f32;
    for from in 0..n {
        let mut row = f32::MIN;
        for to in 0..n {
            let least = (tower.minimal_purge.get(to))
                .or(tower.minimal_purge.last())
                .copied()
                .unwrap_or(15.0) as f32;
            let volume = (matrix[from * n + to] as f32 * tower.flush_multiplier as f32).max(least);
            row = row.max(volume);
        }
        most += row;
    }
    most * count as f32 / n as f32 * 0.6
}

/// The place for the tower, when the one saved does not fit; none where
/// it does (`PartPlate::estimate_wipe_tower_polygon`, on a rectangular
/// bed and a tower square to it). `obstacles` gives each object's outline
/// on the bed, with its brim and a millimetre round it
/// (`PartPlate::wipe_tower_obstacles`), asked for only when the tower
/// is to be moved.
pub fn relocate(
    settings: &Settings,
    tower: &TowerSettings,
    ground: &Ground,
    obstacles: impl FnOnce() -> Result<Vec<Polygon>, ClipError>,
) -> Result<Option<Moved>, PlaceError> {
    if !tower.enabled {
        return Ok(None);
    }
    let Some((w, depth, brim)) = footprint(settings, tower, ground)? else {
        return Ok(None);
    };
    let [bx0, by0, bx1, by1] = ground.bed.map(|value| value as f32);
    let margin = MARGIN + brim;
    let (x_lo, y_lo) = (bx0 + margin, by0 + margin);
    let (x_hi, y_hi) = (
        (bx1 - w - margin).max(x_lo),
        (by1 - depth - margin).max(y_lo),
    );
    let comfort = AUTO_MARGIN + brim;
    let (mut x_lo_c, mut x_hi_c) = (bx0 + comfort, bx1 - w - comfort);
    let (mut y_lo_c, mut y_hi_c) = (by0 + comfort, by1 - depth - comfort);
    if x_lo_c > x_hi_c {
        (x_lo_c, x_hi_c) = (x_lo, x_hi);
    }
    if y_lo_c > y_hi_c {
        (y_lo_c, y_hi_c) = (y_lo, y_hi);
    }
    let (saved_x, saved_y) = (tower.x as f32, tower.y as f32);
    let replace_x = saved_x < x_lo - TOLERANCE || saved_x > x_hi + TOLERANCE;
    let replace_y = saved_y < y_lo - TOLERANCE || saved_y > y_hi + TOLERANCE;
    let mut x = if replace_x {
        saved_x.clamp(x_lo_c, x_hi_c)
    } else {
        saved_x.clamp(x_lo, x_hi)
    };
    let mut y = if replace_y {
        saved_y.clamp(y_lo_c, y_hi_c)
    } else {
        saved_y.clamp(y_lo, y_hi)
    };
    let mut onto_objects = false;
    if replace_x || replace_y {
        let obstacles = obstacles()?;
        // Clear of every object where there is room: the place found, or
        // the clear one nearest the saved place.
        let shape = [
            (-brim, -brim),
            (w + brim, -brim),
            (w + brim, depth + brim),
            (-brim, depth + brim),
        ]
        .map(|(x, y)| Point2::from_mm(f64::from(x), f64::from(y)));
        let at = |x: f32, y: f32| Point2::from_mm(f64::from(x), f64::from(y));
        let placed = at(x, y);
        if nearest_clear([placed, placed], &shape, &obstacles, placed)?.is_none() {
            let saved = at(saved_x, saved_y);
            let clear = match nearest_clear(
                [at(x_lo_c, y_lo_c), at(x_hi_c, y_hi_c)],
                &shape,
                &obstacles,
                saved,
            )? {
                Some(clear) => Some(clear),
                None => nearest_clear([at(x_lo, y_lo), at(x_hi, y_hi)], &shape, &obstacles, saved)?,
            };
            match clear {
                Some(clear) => {
                    x = unscaled(clear.x) as f32;
                    y = unscaled(clear.y) as f32;
                }
                None => onto_objects = true,
            }
        }
    }
    if x == saved_x && y == saved_y {
        return Ok(None);
    }
    Ok(Some(Moved {
        x: f64::from(x),
        y: f64::from(y),
        onto_objects,
    }))
}

/// What can go wrong placing the tower.
#[derive(Debug, thiserror::Error)]
pub enum PlaceError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Clip(#[from] ClipError),
}

/// Of the places for the tower's corner inside `bounds` (two corners),
/// the one nearest `target` where `shape`, set there, meets none of
/// `obstacles`, each taken as its convex hull; none where every one does
/// (WipeTowerEstimate.cpp `nearest_clear_wipe_tower_anchor`).
fn nearest_clear(
    bounds: [Point2; 2],
    shape: &[Point2],
    obstacles: &[Polygon],
    target: Point2,
) -> Result<Option<Point2>, ClipError> {
    let tower = convex_hull(shape.iter().copied());
    // The corners that put the tower on a convex obstacle: the hull of
    // the differences of their corners.
    let blocked: Vec<ExPolygon> = obstacles
        .iter()
        .map(|obstacle| {
            let points = obstacle.points.iter().flat_map(|o| {
                tower.points.iter().map(move |f| Point2 {
                    x: o.x - f.x,
                    y: o.y - f.y,
                })
            });
            ExPolygon {
                contour: convex_hull(points),
                holes: Vec::new(),
            }
        })
        .filter(|blocked| blocked.contour.points.len() >= 3)
        .collect();
    // Grown by a hair, so bounds pinned to one coordinate still have an area.
    let hair = scaled(1e-4).max(1);
    let (low, high) = (bounds[0], bounds[1]);
    let area = ExPolygon {
        contour: Polygon {
            points: vec![
                Point2 {
                    x: low.x - hair,
                    y: low.y - hair,
                },
                Point2 {
                    x: high.x + hair,
                    y: low.y - hair,
                },
                Point2 {
                    x: high.x + hair,
                    y: high.y + hair,
                },
                Point2 {
                    x: low.x - hair,
                    y: high.y + hair,
                },
            ],
        },
        holes: Vec::new(),
    };
    let clear = edge_clip::difference(&[area], &blocked)?;
    let mut nearest: Option<(f64, Point2)> = None;
    for region in &clear {
        if inside(region, target) {
            return Ok(Some(target));
        }
        for ring in region.rings() {
            let count = ring.points.len();
            for index in 0..count {
                let (a, b) = (ring.points[index], ring.points[(index + 1) % count]);
                let closest = closest_on(a, b, target);
                let distance = ((closest.x - target.x) as f64).hypot((closest.y - target.y) as f64);
                if nearest.is_none_or(|(least, _)| distance < least) {
                    nearest = Some((distance, closest));
                }
            }
        }
    }
    Ok(nearest.map(|(_, point)| point))
}

/// The point of the segment from `a` to `b` nearest `p`.
fn closest_on(a: Point2, b: Point2, p: Point2) -> Point2 {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let length = dx * dx + dy * dy;
    if length == 0.0 {
        return a;
    }
    let t = ((((p.x - a.x) as f64) * dx + ((p.y - a.y) as f64) * dy) / length).clamp(0.0, 1.0);
    Point2 {
        x: a.x + (t * dx).round() as i64,
        y: a.y + (t * dy).round() as i64,
    }
}

/// Whether `p` is inside the region, on its edge included.
fn inside(region: &ExPolygon, p: Point2) -> bool {
    let crossings = |ring: &Polygon| {
        let count = ring.points.len();
        let mut odd = false;
        for index in 0..count {
            let (a, b) = (ring.points[index], ring.points[(index + 1) % count]);
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x as f64 + (p.y - a.y) as f64 * (b.x - a.x) as f64 / (b.y - a.y) as f64;
                if (p.x as f64) < x {
                    odd = !odd;
                }
            }
            if closest_on(a, b, p) == p {
                return true;
            }
        }
        odd
    };
    crossings(&region.contour)
        && !region.holes.iter().any(|hole| {
            let count = hole.points.len();
            (0..count).all(|index| {
                closest_on(hole.points[index], hole.points[(index + 1) % count], p) != p
            }) && crossings(hole)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground() -> Ground {
        Ground {
            bed: [0.0, 0.0, 256.0, 256.0],
            filaments: vec![0, 1],
            layer_height: 0.2,
            first_layer_height: 0.2,
            height: 6.0,
        }
    }

    fn square(x: f64, y: f64, side: f64) -> Polygon {
        Polygon {
            points: [(x, y), (x + side, y), (x + side, y + side), (x, y + side)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    fn tower(json: &str) -> (Settings, TowerSettings) {
        let settings = Settings::from_json(json).unwrap();
        let tower = TowerSettings::from_settings(&settings, 0).unwrap();
        (settings, tower)
    }

    #[test]
    fn a_tower_that_fits_stays_where_it_was_put() {
        let (settings, tower) = tower(
            r#"{"enable_prime_tower": "1", "prime_tower_width": "35", "wipe_tower_x": ["15"],
                "wipe_tower_y": ["220"], "single_extruder_multi_material": "1",
                "purge_in_prime_tower": "0", "wipe_tower_wall_type": "rib"}"#,
        );
        assert_eq!(
            relocate(&settings, &tower, &ground(), || Ok(Vec::new())).unwrap(),
            None
        );
    }

    #[test]
    fn a_tower_made_deeper_by_its_flush_is_moved_onto_the_bed() {
        // The Centauri Carbon's twotone cube purging in the tower with the
        // flush from filament 1 to 2 doubled and the multiplier at 0.5:
        // the reference moves it from 15, 220 to 15, 192.194.
        let json = r#"{"enable_prime_tower": "1", "prime_tower_width": "35", "wipe_tower_x": ["15"],
            "wipe_tower_y": ["220"], "single_extruder_multi_material": "1",
            "purge_in_prime_tower": "1", "wipe_tower_wall_type": "rib", "wipe_tower_rib_width": "8",
            "wipe_tower_extra_rib_length": "0", "flush_volumes_matrix": ["0", "836", "228", "0"],
            "flush_multiplier": "0.5", "filament_minimal_purge_on_wipe_tower": ["15", "15"],
            "prime_tower_brim_width": "3", "nozzle_diameter": ["0.4"], "prime_volume": "45"}"#;
        let (settings, tower) = tower(json);
        let moved = relocate(&settings, &tower, &ground(), || Ok(Vec::new()))
            .unwrap()
            .unwrap();
        assert_eq!(moved.x, 15.0);
        assert!((moved.y - 192.194).abs() < 1e-3, "{moved:?}");
        // An object where it would go: the nearest clear place.
        let moved = relocate(&settings, &tower, &ground(), || {
            Ok(vec![square(0.0, 150.0, 100.0)])
        })
        .unwrap()
        .unwrap();
        assert!(moved.x > 100.0 && !moved.onto_objects, "{moved:?}");
    }
}
