//! Whether the toolhead clears the print when the prime tower leaves out
//! the layers with no tool change (`wipe_tower_no_sparse_layers`).
//!
//! The tower then stands lower than the print, and the nozzle goes down to
//! it at every tool change. Anything printed already that rises too far
//! above the tower, near enough to it, is in the way: of the nozzle's cone
//! right beside the tower, of the head's body within its radius, and
//! further off of the rod the head runs on (in the tower's band across the
//! bed) or the lid. The reference refuses such a plate twice over: before
//! slicing, from the tower's size as estimated and each object's whole
//! height, and once the tower is planned, from what it lays and how far
//! each object rises above it on each layer it is laid on (Print.cpp
//! `compacted_wipe_tower_clearance_valid`,
//! `validate_compacted_wipe_tower_clearance`, `compacted_wipe_tower_zone`,
//! `compacted_wipe_tower_clearance`). Both are here.
//!
//! Not as the reference in one thing: the tower's ground is grown by the
//! reach of the spiral the nozzle lifts in, worked out with `travel_slope`
//! in degrees as the reference's G-code writer takes it
//! (GCodeWriter.cpp, Extruder.cpp `travel_slope`). The reference's check
//! takes the degrees for radians, and so grows it by about a twenty-fourth
//! of the spiral's reach at a slope of 3 degrees.

use edge_clip::{ClipError, Join};
use edge_config::{ConfigError, Settings, reference_defaults};
use edge_geom::{ExPolygon, Point2, Polygon, convex_hull, scaled};

use crate::Tower;

/// What the reference says, word for word, when the toolhead would meet
/// the print (Print.cpp `compacted_wipe_tower_clearance_error`).
pub const CLEARANCE_ERROR: &str = "The relative position of the model and the prime tower does not \
     meet the requirements of the \"No sparse layers\" feature. Please adjust their relative \
     positions, lower the model height, or turn off \"No sparse layers\".";

/// How wide the nozzle's cone is, in mm (Print.hpp `MAX_OUTER_NOZZLE_DIAMETER`).
const NOZZLE_CONE: f64 = 4.0;
/// libslic3r's `EPSILON`.
const EPSILON: f64 = 1e-4;

/// What the toolhead clears, from the printer's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Clearance {
    /// How far past the tower the spiral the nozzle lifts in reaches.
    spiral_reach: f64,
    /// `extruder_clearance_dist_to_rod`: how far the rod is from the
    /// nozzle across the bed, half on each side of the band it sweeps.
    rod_distance: f64,
    /// `extruder_clearance_height_to_rod` and `_to_lid`: how far below
    /// each the nozzle is.
    to_rod: f64,
    to_lid: f64,
    /// `nozzle_height`: how far up the cone the body starts.
    nozzle_height: f64,
    /// The head's radius (PrintConfig.cpp `sequential_clearance_radius`).
    body_radius: f64,
    /// What the estimated ground is grown by beyond its brim: a line
    /// width each side (`compacted_tower_footprint_padding`).
    padding: f64,
    /// Whether the plate is printed layer by layer, all objects together;
    /// object by object is not checked here.
    by_layer: bool,
}

impl Clearance {
    pub fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = reference_defaults();
        let number = |key: &str| -> Result<f64, ConfigError> {
            Ok(match settings.float(key)? {
                Some(value) => value,
                None => defaults.float(key)?.unwrap_or(0.0),
            })
        };
        let list = |key: &str| -> Result<Vec<f64>, ConfigError> {
            let values = match settings.floats(key)? {
                Some(values) => values,
                None => defaults.floats(key)?.unwrap_or_default(),
            };
            Ok(values
                .into_iter()
                .map(|value| value.unwrap_or(0.0))
                .collect())
        };
        // The widest a filament's spiral can reach: two of its radius past
        // where it starts (`compacted_wipe_tower_zone`).
        let (hops, slopes) = (list("z_hop")?, list("travel_slope")?);
        let mut spiral_reach = 0.0f64;
        for (index, hop) in hops.iter().enumerate() {
            let lift = hop.min(5.0);
            let slope = slopes.get(index).copied().unwrap_or(0.0);
            if lift < EPSILON || slope < EPSILON {
                continue;
            }
            let radius = lift / (std::f64::consts::TAU * slope.to_radians().atan());
            spiral_reach = spiral_reach.max(2.0 * radius);
        }
        let bambu = settings
            .text("printer_model")
            .is_some_and(|model| model.starts_with("Bambu Lab"));
        let most = number("extruder_clearance_max_radius")?;
        let body_radius = if bambu && most > 0.0 {
            most
        } else {
            number("extruder_clearance_radius")?
        };
        let nozzle = list("nozzle_diameter")?.first().copied().unwrap_or(0.4);
        Ok(Self {
            spiral_reach,
            rod_distance: number("extruder_clearance_dist_to_rod")?,
            to_rod: number("extruder_clearance_height_to_rod")?,
            to_lid: number("extruder_clearance_height_to_lid")?,
            nozzle_height: number("nozzle_height")?,
            body_radius,
            padding: 2.0 * nozzle,
            by_layer: settings.text("print_sequence") != Some("by object"),
        })
    }
}

/// An object on the plate: its outline on the bed, as the convex hull of
/// its parts, and how tall it stands.
#[derive(Clone, Debug)]
pub struct Object {
    pub hull: Polygon,
    pub top: f64,
}

/// Half of a clearance, the share each of the two outlines carries, less
/// the 0.1 mm a side the reference allows for rounding
/// (Print.hpp `compacted_tower_half_clearance`).
fn half(clearance: f64) -> f64 {
    0.5 * (clearance - 0.2)
}

fn grown(polygon: &Polygon, by: f64) -> Result<Vec<ExPolygon>, ClipError> {
    edge_clip::offset(
        &[ExPolygon {
            contour: polygon.clone(),
            holes: Vec::new(),
        }],
        scaled(by),
        Join::Round {
            tolerance: scaled(0.1),
        },
    )
}

/// The band across the bed a box sweeps, front to back: its y extent,
/// grown by `by` each way.
fn band(polygon: &Polygon, by: f64) -> (i64, i64) {
    let (low, high) = polygon
        .points
        .iter()
        .fold((i64::MAX, i64::MIN), |(low, high), p| {
            (low.min(p.y), high.max(p.y))
        });
    (low - scaled(by), high + scaled(by))
}

/// What the toolhead keeps clear of round the tower
/// (`compacted_wipe_tower_zone`).
struct Zone {
    /// The band across the bed the rod sweeps.
    rod: (i64, i64),
    /// The tower's ground, grown by half the head's radius and by half
    /// the nozzle's cone.
    body: Vec<ExPolygon>,
    nozzle: Vec<ExPolygon>,
}

impl Zone {
    fn new(clearance: &Clearance, ground: &Polygon) -> Result<Self, ClipError> {
        // Where the nozzle lifts off the tower in a spiral, it reaches out
        // past it while still low.
        let mut hull = ground.clone();
        if clearance.spiral_reach > EPSILON {
            let spread = grown(ground, clearance.spiral_reach)?;
            let spread = convex_hull(
                spread
                    .iter()
                    .flat_map(|region| region.contour.points.clone()),
            );
            if spread.points.len() >= 3 {
                hull = spread;
            }
        }
        Ok(Self {
            rod: band(&hull, clearance.rod_distance * 0.5),
            body: grown(&hull, half(clearance.body_radius))?,
            nozzle: grown(&hull, half(NOZZLE_CONE))?,
        })
    }

    /// How far `object` may rise above the tower's top when it stands so
    /// high above it (`compacted_wipe_tower_clearance`): not at all within
    /// the nozzle's cone of the tower; within the head's radius, while it
    /// is taller than the cone, no higher than the cone; and otherwise up
    /// to the rod where it shares the tower's band across the bed, and to
    /// the lid where it does not.
    fn allowed(
        &self,
        clearance: &Clearance,
        object: &Polygon,
        rise: f64,
    ) -> Result<f64, ClipError> {
        let (low, high) = band(object, clearance.rod_distance * 0.5);
        let shares_band = high.min(self.rod.1) - low.max(self.rod.0) > 0;
        let far = if shares_band {
            clearance.to_rod
        } else {
            clearance.to_lid
        };
        let near_nozzle =
            !edge_clip::intersection(&self.nozzle, &grown(object, half(NOZZLE_CONE))?)?.is_empty();
        if near_nozzle {
            return Ok(0.0);
        }
        let short = rise <= clearance.nozzle_height + EPSILON;
        if !short
            && !edge_clip::intersection(&self.body, &grown(object, half(clearance.body_radius))?)?
                .is_empty()
        {
            return Ok(far.min(clearance.nozzle_height));
        }
        Ok(far)
    }
}

/// Before slicing: whether every object, standing on the bed, clears a
/// tower of the size estimated (`compacted_wipe_tower_clearance_valid`).
/// `place` is where the tower's corner is set, `size` its estimated width,
/// depth and brim. As high as an object is counted, since how low the
/// tower stands is not known yet; so this errs on the strict side.
pub fn estimate_clears(
    clearance: &Clearance,
    place: (f64, f64),
    size: (f64, f64, f64),
    objects: &[Object],
) -> Result<bool, ClipError> {
    let (width, depth, brim) = size;
    if !clearance.by_layer || depth < EPSILON {
        return Ok(true);
    }
    let padding = brim.max(0.0) + clearance.padding;
    let (x, y) = (place.0 - padding, place.1 - padding);
    let (width, depth) = (width + 2.0 * padding, depth + 2.0 * padding);
    let ground = Polygon {
        points: [
            (x, y),
            (x + width, y),
            (x + width, y + depth),
            (x, y + depth),
        ]
        .map(|(x, y)| Point2::from_mm(x, y))
        .to_vec(),
    };
    let zone = Zone::new(clearance, &ground)?;
    for object in objects
        .iter()
        .filter(|object| object.hull.points.len() >= 3)
    {
        if object.top > zone.allowed(clearance, &object.hull, object.top)? + EPSILON {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Once the tower is planned: whether every object clears it on each layer
/// it is laid on, by what the tower lays there and how far the object
/// rises above it then (`validate_compacted_wipe_tower_clearance`). `tops`
/// is the top of each of the print's layers. A tower that leaves no layer
/// out stands as tall as the print and clears it.
pub fn tower_clears(
    clearance: &Clearance,
    tower: &Tower,
    tops: &[f64],
    objects: &[Object],
) -> Result<bool, ClipError> {
    if !clearance.by_layer {
        return Ok(true);
    }
    // The ground the tower takes: the hull of every line it lays.
    let laid: Vec<(usize, f64)> = (tower.layers.iter().enumerate())
        .filter_map(|(index, layer)| layer.z.map(|z| (index, z)))
        .collect();
    if laid.is_empty() {
        return Ok(true);
    }
    let points = laid.iter().flat_map(|(index, _)| {
        let layer = &tower.layers[*index];
        let rams =
            (layer.changes.iter()).flat_map(|block| block.ram.iter().flat_map(|ram| &ram.lines));
        (layer.finish.iter())
            .chain(layer.changes.iter().flat_map(|block| &block.lines))
            .chain(rams)
            .flat_map(|line| line.points.iter().copied())
    });
    let ground = convex_hull(points);
    if ground.points.len() < 3 {
        return Ok(true);
    }
    let zone = Zone::new(clearance, &ground)?;
    for object in objects
        .iter()
        .filter(|object| object.hull.points.len() >= 3)
    {
        // The worst layer: nothing above the layer being printed is there
        // yet, so a tall object counts only up to it.
        let rise = laid
            .iter()
            .map(|(index, z)| {
                let top = tops.get(*index).copied().unwrap_or(*z);
                object.top.min(top) - z
            })
            .fold(0.0, f64::max);
        if rise > zone.allowed(clearance, &object.hull, rise)? + EPSILON {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, side: f64) -> Polygon {
        Polygon {
            points: [(x, y), (x + side, y), (x + side, y + side), (x, y + side)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    /// A U1's: rod 27.5 mm up, lid 140, head 72.5 mm round, cone 2.5 mm
    /// tall, spiral of 0.4 mm at 3 degrees.
    fn u1() -> Clearance {
        let settings = Settings::from_json(
            r#"{"printer_model": "Snapmaker U1", "z_hop": ["0.4", "0.4"],
                "travel_slope": ["3", "3"], "extruder_clearance_height_to_rod": "27.5",
                "extruder_clearance_height_to_lid": "140", "extruder_clearance_radius": "72.5",
                "nozzle_diameter": ["0.4", "0.4"]}"#,
        )
        .unwrap();
        Clearance::from_settings(&settings).unwrap()
    }

    #[test]
    fn the_spiral_reach_takes_the_slope_in_degrees() {
        let clearance = u1();
        // Twice 0.4 / (2 pi atan(3 degrees)).
        assert!((clearance.spiral_reach - 2.4335).abs() < 1e-3);
        assert_eq!(clearance.body_radius, 72.5);
        assert_eq!(clearance.nozzle_height, 2.5);
        assert_eq!(clearance.rod_distance, 40.0);
    }

    #[test]
    fn a_bambu_lab_printer_takes_its_largest_radius() {
        let settings = Settings::from_json(
            r#"{"printer_model": "Bambu Lab P1S", "extruder_clearance_radius": "57",
                "extruder_clearance_max_radius": "68"}"#,
        )
        .unwrap();
        assert_eq!(
            Clearance::from_settings(&settings).unwrap().body_radius,
            68.0
        );
    }

    #[test]
    fn an_object_far_off_and_lower_than_the_rod_clears_the_estimate() {
        let clearance = u1();
        let far = Object {
            hull: square(125.5, 126.0, 20.0),
            top: 14.0,
        };
        assert!(estimate_clears(&clearance, (15.0, 220.0), (30.0, 20.0, 3.0), &[far]).unwrap());
    }

    #[test]
    fn an_object_beside_the_tower_does_not() {
        let clearance = u1();
        // Within the head's radius and taller than the cone.
        let near = Object {
            hull: square(60.0, 220.0, 20.0),
            top: 14.0,
        };
        assert!(!estimate_clears(&clearance, (15.0, 220.0), (30.0, 20.0, 3.0), &[near]).unwrap());
        // As near, and no taller than the cone: it may stand there.
        let short = Object {
            hull: square(60.0, 220.0, 20.0),
            top: 2.0,
        };
        assert!(estimate_clears(&clearance, (15.0, 220.0), (30.0, 20.0, 3.0), &[short]).unwrap());
        // Touching the cone's reach, not even that.
        let touching = Object {
            hull: square(48.0, 220.0, 20.0),
            top: 1.0,
        };
        assert!(
            !estimate_clears(&clearance, (15.0, 220.0), (30.0, 20.0, 3.0), &[touching]).unwrap()
        );
    }

    #[test]
    fn far_off_the_rod_and_the_lid_bound_the_height() {
        let clearance = u1();
        // In the tower's band across the bed: the rod, 27.5 mm.
        let in_band = |top| Object {
            hull: square(200.0, 225.0, 20.0),
            top,
        };
        assert!(
            estimate_clears(
                &clearance,
                (15.0, 220.0),
                (30.0, 20.0, 3.0),
                &[in_band(27.0)]
            )
            .unwrap()
        );
        assert!(
            !estimate_clears(
                &clearance,
                (15.0, 220.0),
                (30.0, 20.0, 3.0),
                &[in_band(28.0)]
            )
            .unwrap()
        );
        // Out of it: the lid, 140 mm.
        let out_of_band = Object {
            hull: square(200.0, 20.0, 20.0),
            top: 100.0,
        };
        assert!(
            estimate_clears(&clearance, (15.0, 220.0), (30.0, 20.0, 3.0), &[out_of_band]).unwrap()
        );
    }
}
