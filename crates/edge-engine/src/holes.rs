//! The hole options: round holes and round outlines resized by what the
//! filament is known to print them off by (`enable_circle_compensation`),
//! and round holes walled as polygons of few sides (`hole_to_polyhole`).
//!
//! The geometry is `edge-slice`'s; here are the settings, the order the
//! two are done in, and which walls go round a compensated circle.
//!
//! As the reference orders it (PrintObjectSlice.cpp): each layer has its
//! XY size compensation and elephant foot compensation, then its circle
//! compensation (`PrintObject::slice_volumes`, the "make_slices in
//! parallel" part), and once every layer is done the holes that run
//! through them are made polyholes (`PrintObject::slice`,
//! `_transform_hole_to_polyholes`). So a polyhole is drawn round a hole
//! as the compensations left it.
//!
//! The reference works region by region, a region being what one set of
//! settings prints; here a layer is shared out by filament, and each
//! filament's share is taken as a region. A layer's outline as a whole,
//! where it is held apart from the shares, follows them ring by ring: a
//! ring of it that is a ring replaced in a share is replaced too, as the
//! reference replaces the holes of its `lslices` by those of the regions.
//! The first layers' outlines as modelled, kept from before elephant foot
//! compensation, are left as they are, as in the reference.

use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_geom::{ExPolygon, Point2, Polygon, scaled};
use edge_perimeter::{Params, Perimeters};
use edge_slice::{CircleCompensation, LayerRegions, Margin, Marked, Polyholes, Replaced, Share};
use rayon::prelude::*;

use crate::Failure;

/// What the settings ask of holes and round outlines.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HoleSettings {
    /// `hole_to_polyhole` and what goes with it; none when it is off.
    pub polyholes: Option<Polyholes>,
    /// `enable_circle_compensation` and what goes with it; none when it
    /// is off.
    pub circles: Option<Circles>,
    /// `nozzle_diameter`, by extruder.
    nozzles: Vec<f64>,
}

/// The auto circle compensation's settings.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Circles {
    /// By filament, counting from 0.
    filaments: Vec<CircleCompensation>,
    /// `circle_compensation_manual_offset`, in mm.
    pub manual_offset: f64,
}

impl Circles {
    /// A filament's own figures; one the lists do not reach takes the
    /// first's (the reference's `get_at`).
    pub(crate) fn of(&self, filament: usize) -> CircleCompensation {
        self.filaments
            .get(filament.saturating_sub(1))
            .or(self.filaments.first())
            .copied()
            .unwrap_or_default()
    }
}

impl HoleSettings {
    pub(crate) fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        let bad = |key: &str, found: String, expected: &'static str| ConfigError::BadValue {
            key: key.into(),
            found,
            expected,
        };
        let nozzles: Vec<f64> = settings
            .floats("nozzle_diameter")?
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect();
        if let Some(nozzle) = nozzles.iter().find(|nozzle| **nozzle <= 0.0) {
            return Err(bad(
                "nozzle_diameter",
                nozzle.to_string(),
                "a diameter above 0",
            ));
        }

        // PrintConfig.cpp: 0.01 mm, twisted, 50 edges at most and never
        // under 3 (`def->min = 3`).
        let margin = match settings
            .float_or_percent("hole_to_polyhole_threshold")?
            .unwrap_or(FloatOrPercent::Value(0.01))
        {
            FloatOrPercent::Value(mm) => Margin::Mm(mm),
            FloatOrPercent::Percent(percent) => Margin::Percent(percent),
        };
        let max_edges = settings
            .integer("hole_to_polyhole_max_edges")?
            .unwrap_or(50);
        let max_edges = i32::try_from(max_edges)
            .ok()
            .filter(|edges| *edges >= 3)
            .ok_or_else(|| {
                bad(
                    "hole_to_polyhole_max_edges",
                    max_edges.to_string(),
                    "a whole number, 3 or more",
                )
            })?;
        let twisted = settings.flag("hole_to_polyhole_twisted")?.unwrap_or(true);
        let polyholes = settings
            .flag("hole_to_polyhole")?
            .unwrap_or(false)
            .then_some(Polyholes {
                margin,
                twisted,
                max_edges,
            });

        // Each of the filament's figures is a list, one entry a filament.
        let defaults = CircleCompensation::default();
        let list = |key: &str| -> Result<Vec<f64>, ConfigError> {
            settings
                .floats(key)?
                .unwrap_or_default()
                .into_iter()
                .map(|entry| entry.ok_or_else(|| bad(key, "nil".into(), "a number")))
                .collect()
        };
        let speed = list("circle_compensation_speed")?;
        let (c1, c2, c3) = (
            list("counter_coef_1")?,
            list("counter_coef_2")?,
            list("counter_coef_3")?,
        );
        let (h1, h2, h3) = (
            list("hole_coef_1")?,
            list("hole_coef_2")?,
            list("hole_coef_3")?,
        );
        let (c_min, c_max) = (list("counter_limit_min")?, list("counter_limit_max")?);
        let (h_min, h_max) = (list("hole_limit_min")?, list("hole_limit_max")?);
        let limit = list("diameter_limit")?;
        // PrintConfig.cpp: `def->min = 0` on the speed.
        if let Some(negative) = speed.iter().find(|speed| **speed < 0.0) {
            return Err(bad(
                "circle_compensation_speed",
                negative.to_string(),
                "a speed, 0 or more",
            ));
        }
        let count = [
            &speed, &c1, &c2, &c3, &h1, &h2, &h3, &c_min, &c_max, &h_min, &h_max, &limit,
        ]
        .iter()
        .map(|values| values.len())
        .max()
        .unwrap_or(0)
        .max(1);
        // An entry a list does not reach is its first; a list left out
        // altogether is the reference's default.
        let at = |values: &[f64], filament: usize, default: f64| {
            values
                .get(filament)
                .or(values.first())
                .copied()
                .unwrap_or(default)
        };
        let filaments = (0..count)
            .map(|f| CircleCompensation {
                speed: at(&speed, f, defaults.speed),
                contour_coefs: [
                    at(&c1, f, defaults.contour_coefs[0]),
                    at(&c2, f, defaults.contour_coefs[1]),
                    at(&c3, f, defaults.contour_coefs[2]),
                ],
                hole_coefs: [
                    at(&h1, f, defaults.hole_coefs[0]),
                    at(&h2, f, defaults.hole_coefs[1]),
                    at(&h3, f, defaults.hole_coefs[2]),
                ],
                contour_limits: (
                    at(&c_min, f, defaults.contour_limits.0),
                    at(&c_max, f, defaults.contour_limits.1),
                ),
                hole_limits: (
                    at(&h_min, f, defaults.hole_limits.0),
                    at(&h_max, f, defaults.hole_limits.1),
                ),
                diameter_limit: at(&limit, f, defaults.diameter_limit),
            })
            .collect();
        let manual_offset = settings
            .float("circle_compensation_manual_offset")?
            .unwrap_or(0.0);
        let circles = settings
            .flag("enable_circle_compensation")?
            .unwrap_or(false)
            .then_some(Circles {
                filaments,
                manual_offset,
            });
        Ok(Self {
            polyholes,
            circles,
            nozzles,
        })
    }

    /// The nozzle a filament is printed through, in mm: its own
    /// extruder's, or the first where there are fewer extruders
    /// (ToolSlots.cpp `nozzle_diameter_for_filament`, for a printer
    /// with a nozzle to each filament or one for all).
    fn nozzle(&self, filament: usize) -> f64 {
        self.nozzles
            .get(filament.saturating_sub(1))
            .or(self.nozzles.first())
            .copied()
            .unwrap_or(0.4)
    }
}

/// Which rings of each region were resized as small circles, by
/// filament in use and then by layer; empty where nothing was.
#[derive(Debug, Default)]
pub(crate) struct CircleMarks(Vec<Vec<Vec<Marked>>>);

/// Puts into a layer's outline as a whole the rings replaced in its
/// shares.
fn follow(layers: &mut [LayerRegions], replaced: &[Replaced]) {
    for change in replaced {
        let Some(layer) = layers.get_mut(change.layer) else {
            continue;
        };
        for ring in layer
            .regions
            .iter_mut()
            .flat_map(|region| std::iter::once(&mut region.contour).chain(&mut region.holes))
        {
            if edge_slice::same_ring(&ring.points, &change.from.points) {
                *ring = change.to.clone();
            }
        }
    }
}

/// Resizes round outlines and holes and makes polyholes, in the
/// reference's order, on layers that have had their XY size and elephant
/// foot compensation. `shares` is each filament's share of every layer,
/// by filament in `used` and then by layer; it is empty with one
/// filament, whose share is the layer itself.
pub(crate) fn apply(
    settings: &HoleSettings,
    layers: &mut [LayerRegions],
    shares: &mut [Vec<Vec<ExPolygon>>],
    used: &[usize],
) -> Result<CircleMarks, Failure> {
    let mut marks = CircleMarks::default();
    if let Some(circles) = &settings.circles {
        for (index, filament) in used.iter().enumerate() {
            let of = circles.of(*filament);
            let compensate = |(layer, regions): (usize, &mut Vec<ExPolygon>)| {
                edge_slice::auto_circle_compensation(layer, regions, &of, circles.manual_offset)
                    .map_err(|e| e.to_string())
            };
            let done: Vec<(Vec<Marked>, Vec<Replaced>)> = match shares.get_mut(index) {
                Some(own) => own
                    .par_iter_mut()
                    .enumerate()
                    .map(compensate)
                    .collect::<Result<_, String>>(),
                None => layers
                    .par_iter_mut()
                    .map(|layer| &mut layer.regions)
                    .enumerate()
                    .map(compensate)
                    .collect::<Result<_, String>>(),
            }?;
            let (marked, replaced): (Vec<_>, Vec<_>) = done.into_iter().unzip();
            if !shares.is_empty() {
                for replaced in &replaced {
                    follow(layers, replaced);
                }
            }
            marks.0.push(marked);
        }
    }
    if let Some(polyholes) = &settings.polyholes {
        if shares.is_empty() {
            let filament = used.first().copied().unwrap_or(1);
            let mut by_layer: Vec<Vec<Share<'_>>> = layers
                .iter_mut()
                .map(|layer| {
                    vec![Share {
                        filament,
                        nozzle_diameter: settings.nozzle(filament),
                        regions: &mut layer.regions,
                    }]
                })
                .collect();
            edge_slice::hole_to_polyholes(&mut by_layer, polyholes);
        } else {
            let mut by_layer: Vec<Vec<Share<'_>>> = (0..layers.len()).map(|_| Vec::new()).collect();
            for (own, filament) in shares.iter_mut().zip(used) {
                for (layer, regions) in by_layer.iter_mut().zip(own.iter_mut()) {
                    layer.push(Share {
                        filament: *filament,
                        nozzle_diameter: settings.nozzle(*filament),
                        regions,
                    });
                }
            }
            let replaced = edge_slice::hole_to_polyholes(&mut by_layer, polyholes);
            follow(layers, &replaced);
        }
    }
    Ok(marks)
}

/// Whether `point` is inside `region`: within its outline and in none
/// of its holes.
fn holds(region: &ExPolygon, point: Point2) -> bool {
    let within = |ring: &Polygon| {
        let points = &ring.points;
        let mut inside = false;
        for (index, a) in points.iter().enumerate() {
            let b = points[(index + 1) % points.len()];
            if (a.y > point.y) != (b.y > point.y) {
                // Which side of the edge the point is on, exactly.
                let side = i128::from(b.x - a.x) * i128::from(point.y - a.y)
                    - i128::from(point.x - a.x) * i128::from(b.y - a.y);
                if (side > 0) == (b.y > a.y) {
                    inside = !inside;
                }
            }
        }
        inside
    };
    within(&region.contour) && !region.holes.iter().any(within)
}

impl CircleMarks {
    /// Marks the walls of filament `filament` (an index into the
    /// filaments in use) on layer `layer` that go round a compensated
    /// circle. `regions` is what the walls were made for and `params`
    /// what they were made with.
    ///
    /// As the reference tells them (PerimeterGenerator.cpp). With walls
    /// of one width, `process_classic`: every outline wall of a region
    /// whose outline is marked, so long as the region simplified is
    /// still one piece, and every wall round a hole whose middle is
    /// within a thousandth of a millimetre of a marked hole's, "as the
    /// onion shells keep a circle's centre". With walls of changing
    /// width, `flag_arachne_circle_compensation`: closed walls whose
    /// middle is within 0.05 mm of the marked outline's or of a marked
    /// hole's. The reference marks loops as it makes them, region by
    /// region; here they are marked once made, and a wall belongs to
    /// the region it lies in.
    pub(crate) fn flag(
        &self,
        filament: usize,
        layer: usize,
        regions: &[ExPolygon],
        params: &Params,
        walls: &mut Perimeters,
    ) -> Result<(), edge_clip::ClipError> {
        let Some(marks) = self.0.get(filament).and_then(|own| own.get(layer)) else {
            return Ok(());
        };
        let arachne = params.arachne.is_some();
        let reach = if arachne { scaled(0.05) as f64 } else { 1000.0 };
        for (region, marked) in regions.iter().zip(marks) {
            if !marked.contour && marked.holes.is_empty() {
                continue;
            }
            // `last.size() == 1`.
            let outline = marked.contour && {
                let mut last =
                    edge_clip::simplify(std::slice::from_ref(region), params.resolution)?;
                if arachne {
                    let (width, spacing) =
                        (params.outer.scaled_width(), params.outer.scaled_spacing());
                    let inset = if params.precise_outer_wall {
                        width - spacing
                    } else {
                        width / 2 - spacing / 2
                    };
                    last = edge_clip::offset(&last, -inset, edge_clip::Join::MITER)?;
                }
                last.len() == 1
            };
            let holes: Vec<Point2> = marked
                .holes
                .iter()
                .filter_map(|&index| region.holes.get(index))
                .map(|hole| edge_slice::centroid(&hole.points))
                .collect();
            if !outline && holes.is_empty() {
                continue;
            }
            let middle = edge_slice::centroid(&region.contour.points);
            let near = |a: Point2, b: Point2| {
                let (dx, dy) = ((a.x - b.x) as f64, (a.y - b.y) as f64);
                (dx * dx + dy * dy).sqrt() < reach
            };
            for wall in &mut walls.loops {
                let Some(&first) = wall.polygon.points.first() else {
                    continue;
                };
                if !holds(region, first) {
                    continue;
                }
                let centre = edge_slice::centroid(&wall.polygon.points);
                wall.circle_compensation = if wall.is_hole {
                    holes.iter().any(|hole| near(*hole, centre))
                } else {
                    outline && (!arachne || near(middle, centre))
                };
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(json: &str) -> Result<HoleSettings, ConfigError> {
        HoleSettings::from_settings(&Settings::from_json(json).unwrap())
    }

    #[test]
    fn both_options_are_off_unless_asked_for() {
        let plain = read("{}").unwrap();
        assert_eq!((plain.polyholes, plain.circles), (None, None));
        // What goes with an option turns nothing on by itself.
        let off = read(
            r#"{"hole_to_polyhole": "0", "hole_to_polyhole_threshold": "5%",
                "enable_circle_compensation": "0", "circle_compensation_manual_offset": "0.2"}"#,
        )
        .unwrap();
        assert_eq!((off.polyholes, off.circles), (None, None));
        // The reference's own defaults leave both off.
        let defaults = HoleSettings::from_settings(edge_config::reference_defaults()).unwrap();
        assert_eq!((defaults.polyholes, defaults.circles), (None, None));
    }

    #[test]
    fn polyholes_take_the_references_defaults_and_what_is_set() {
        let on = read(r#"{"hole_to_polyhole": "1"}"#).unwrap();
        assert_eq!(
            on.polyholes,
            Some(Polyholes {
                margin: Margin::Mm(0.01),
                twisted: true,
                max_edges: 50,
            })
        );
        assert!(on.circles.is_none());
        let set = read(
            r#"{"hole_to_polyhole": "1", "hole_to_polyhole_threshold": "5%",
                "hole_to_polyhole_twisted": "0", "hole_to_polyhole_max_edges": "12"}"#,
        )
        .unwrap();
        assert_eq!(
            set.polyholes,
            Some(Polyholes {
                margin: Margin::Percent(5.0),
                twisted: false,
                max_edges: 12,
            })
        );
        // As the reference's defaults have them.
        let mut defaults = edge_config::reference_defaults().clone();
        defaults.insert("hole_to_polyhole", edge_config::Value::One("1".into()));
        assert_eq!(
            HoleSettings::from_settings(&defaults).unwrap().polyholes,
            on.polyholes
        );
    }

    #[test]
    fn a_polyhole_is_sized_by_the_nozzle_of_its_filament() {
        let several = read(r#"{"nozzle_diameter": ["0.4", "0.6", "0.8"]}"#).unwrap();
        assert_eq!(
            [1, 2, 3, 4, 0].map(|filament| several.nozzle(filament)),
            [0.4, 0.6, 0.8, 0.4, 0.4]
        );
        // One nozzle for every filament; and 0.4 mm where none is said.
        let one = read(r#"{"nozzle_diameter": ["0.6"]}"#).unwrap();
        assert_eq!([1, 2].map(|filament| one.nozzle(filament)), [0.6, 0.6]);
        assert_eq!(read("{}").unwrap().nozzle(1), 0.4);
    }

    #[test]
    fn each_filament_has_its_own_circle_compensation() {
        let plain = read(r#"{"enable_circle_compensation": "1"}"#).unwrap();
        let circles = plain.circles.unwrap();
        assert_eq!(circles.manual_offset, 0.0);
        assert_eq!(circles.of(1), CircleCompensation::default());
        // As the reference's defaults have them.
        let mut defaults = edge_config::reference_defaults().clone();
        defaults.insert(
            "enable_circle_compensation",
            edge_config::Value::One("1".into()),
        );
        let from_defaults = HoleSettings::from_settings(&defaults).unwrap();
        assert_eq!(
            from_defaults.circles.unwrap().of(1),
            CircleCompensation::default()
        );

        let set = read(
            r#"{"enable_circle_compensation": "1", "circle_compensation_manual_offset": "-0.1",
                "circle_compensation_speed": ["200", "150"],
                "counter_coef_1": ["0", "0.0001"], "counter_coef_2": ["0.008", "0.003"],
                "counter_coef_3": ["-0.041", "0.014"],
                "hole_coef_1": ["0"], "hole_coef_2": ["-0.008", "-0.0028"],
                "hole_coef_3": ["0.23415", "0.12"],
                "counter_limit_min": ["-0.035", "0.014"], "counter_limit_max": ["0.033", "0.076"],
                "hole_limit_min": ["0.088", "0.05"], "hole_limit_max": ["0.22", "0.12"],
                "diameter_limit": ["50", "30"]}"#,
        )
        .unwrap();
        let circles = set.circles.unwrap();
        assert_eq!(circles.manual_offset, -0.1);
        assert_eq!(
            circles.of(2),
            CircleCompensation {
                speed: 150.0,
                contour_coefs: [0.0001, 0.003, 0.014],
                hole_coefs: [0.0, -0.0028, 0.12],
                contour_limits: (0.014, 0.076),
                hole_limits: (0.05, 0.12),
                diameter_limit: 30.0,
            }
        );
        assert_eq!(circles.of(1).hole_coefs, [0.0, -0.008, 0.23415]);
        // A filament the lists do not reach has the first's.
        assert_eq!(circles.of(3), circles.of(1));
    }

    #[test]
    fn bad_values_are_errors() {
        for json in [
            r#"{"hole_to_polyhole": "yes"}"#,
            r#"{"hole_to_polyhole_threshold": "wide"}"#,
            r#"{"hole_to_polyhole_twisted": "2"}"#,
            // The reference allows no fewer than three sides.
            r#"{"hole_to_polyhole_max_edges": "2"}"#,
            r#"{"hole_to_polyhole_max_edges": "-4"}"#,
            r#"{"hole_to_polyhole_max_edges": "1.5"}"#,
            r#"{"enable_circle_compensation": "on"}"#,
            r#"{"circle_compensation_manual_offset": "x"}"#,
            r#"{"circle_compensation_manual_offset": ["0.1", "0.2"]}"#,
            r#"{"circle_compensation_speed": ["-1"]}"#,
            r#"{"counter_coef_2": ["nil"]}"#,
            r#"{"hole_coef_3": ["0.2", "much"]}"#,
            r#"{"diameter_limit": ["wide"]}"#,
            r#"{"nozzle_diameter": ["0.4", "0"]}"#,
        ] {
            assert!(read(json).is_err(), "{json}");
        }
        // Whether or not the option they go with is on.
        assert!(read(r#"{"hole_to_polyhole": "0", "hole_to_polyhole_max_edges": "2"}"#).is_err());
        // Three sides are allowed.
        assert!(read(r#"{"hole_to_polyhole_max_edges": "3"}"#).is_ok());
    }

    fn square(x0: i64, y0: i64, side: i64, clockwise: bool) -> Polygon {
        let mut points = vec![
            Point2 { x: x0, y: y0 },
            Point2 {
                x: x0 + side,
                y: y0,
            },
            Point2 {
                x: x0 + side,
                y: y0 + side,
            },
            Point2 {
                x: x0,
                y: y0 + side,
            },
        ];
        if clockwise {
            points.reverse();
        }
        Polygon { points }
    }

    #[test]
    fn a_wall_belongs_to_the_region_it_lies_in() {
        let region = ExPolygon {
            contour: square(0, 0, 100, false),
            holes: vec![square(40, 40, 20, true)],
        };
        assert!(holds(&region, Point2 { x: 10, y: 10 }));
        assert!(holds(&region, Point2 { x: 39, y: 50 }));
        // In the hole, or outside altogether.
        assert!(!holds(&region, Point2 { x: 50, y: 50 }));
        assert!(!holds(&region, Point2 { x: 150, y: 50 }));
        assert!(!holds(&region, Point2 { x: -1, y: 50 }));
    }

    /// A plate 20 mm square about (`x`, 0) with a round hole of
    /// `radius` mm through its middle.
    fn plate(x: f64, radius: f64) -> ExPolygon {
        let mut hole: Vec<Point2> = (0..64)
            .map(|i| {
                let angle = std::f64::consts::TAU * f64::from(i) / 64.0;
                Point2::from_mm(x + radius * angle.cos(), radius * angle.sin())
            })
            .collect();
        hole.reverse();
        ExPolygon {
            contour: Polygon {
                points: [(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)]
                    .map(|(dx, dy)| Point2::from_mm(x + dx, dy))
                    .to_vec(),
            },
            holes: vec![Polygon { points: hole }],
        }
    }

    #[test]
    fn each_filaments_share_is_done_by_itself_and_the_whole_follows() {
        // Two plates side by side on four layers, one in each of two
        // filaments, the second through a nozzle twice as wide.
        let mut layers: Vec<LayerRegions> = (0..4)
            .map(|index| LayerRegions {
                z: 0.1 + 0.2 * f64::from(index),
                regions: vec![plate(0.0, 2.5), plate(30.0, 2.5)],
                by_filament: Vec::new(),
                cut_by_filament: Vec::new(),
                open_chains: 0,
            })
            .collect();
        let mut shares = vec![
            vec![vec![plate(0.0, 2.5)]; 4],
            vec![vec![plate(30.0, 2.5)]; 4],
        ];
        let settings = read(
            r#"{"hole_to_polyhole": "1", "enable_circle_compensation": "1",
                "nozzle_diameter": ["0.4", "0.8"],
                "hole_coef_2": ["0", "0"], "hole_coef_3": ["0.1", "0.2"],
                "hole_limit_min": ["0", "0"], "hole_limit_max": ["1", "1"]}"#,
        )
        .unwrap();
        let marks = apply(&settings, &mut layers, &mut shares, &[1, 2]).unwrap();
        // Filament 1: 0.1 mm larger, then 4 * 2.6 = 10 sides (10.4).
        // Filament 2: 0.2 mm larger, then 4 * 2.7 / 2 = 5 sides (5.4).
        for layer in 0..4 {
            assert_eq!(shares[0][layer][0].holes[0].points.len(), 10);
            assert_eq!(shares[1][layer][0].holes[0].points.len(), 5);
            // The layer as a whole has the same two holes.
            assert_eq!(
                layers[layer].regions[0].holes[0],
                shares[0][layer][0].holes[0]
            );
            assert_eq!(
                layers[layer].regions[1].holes[0],
                shares[1][layer][0].holes[0]
            );
            // And its outlines, which are no circles, as they were.
            assert_eq!(layers[layer].regions[0].contour, plate(0.0, 2.5).contour);
        }
        // Both holes are marked, each in its own filament's share.
        for filament in 0..2 {
            for layer in 0..4 {
                assert_eq!(
                    marks.0[filament][layer],
                    vec![Marked {
                        contour: false,
                        holes: vec![0]
                    }]
                );
            }
        }
        // One filament, whose share is the layer: the same, in place.
        let mut alone: Vec<LayerRegions> = (0..4)
            .map(|index| LayerRegions {
                z: 0.1 + 0.2 * f64::from(index),
                regions: vec![plate(0.0, 2.5)],
                by_filament: Vec::new(),
                cut_by_filament: Vec::new(),
                open_chains: 0,
            })
            .collect();
        let marks = apply(&settings, &mut alone, &mut [], &[2]).unwrap();
        assert!(
            alone
                .iter()
                .all(|layer| layer.regions[0].holes[0].points.len() == 5)
        );
        assert_eq!(marks.0.len(), 1);
        // With neither option on, nothing is touched.
        let mut untouched = vec![alone[0].clone()];
        untouched[0].regions = vec![plate(0.0, 2.5)];
        let off = read("{}").unwrap();
        let marks = apply(&off, &mut untouched, &mut [], &[1]).unwrap();
        assert_eq!(untouched[0].regions, vec![plate(0.0, 2.5)]);
        assert!(marks.0.is_empty());
    }

    #[test]
    fn the_layer_as_a_whole_follows_its_shares_ring_by_ring() {
        // The whole holds the ring from another of its points.
        let mut turned = square(40, 40, 20, true);
        turned.points.rotate_left(2);
        let mut layers = vec![LayerRegions {
            z: 0.1,
            regions: vec![ExPolygon {
                contour: square(0, 0, 100, false),
                holes: vec![turned],
            }],
            by_filament: Vec::new(),
            cut_by_filament: Vec::new(),
            open_chains: 0,
        }];
        let change = Replaced {
            layer: 0,
            from: square(40, 40, 20, true),
            to: square(38, 38, 24, true),
        };
        follow(&mut layers, std::slice::from_ref(&change));
        assert_eq!(layers[0].regions[0].holes[0], change.to);
        assert_eq!(layers[0].regions[0].contour, square(0, 0, 100, false));
        // A ring that is no ring of the whole changes nothing.
        let before = layers[0].regions.clone();
        follow(
            &mut layers,
            &[Replaced {
                layer: 0,
                from: square(1, 1, 5, true),
                to: square(0, 0, 7, true),
            }],
        );
        assert_eq!(layers[0].regions, before);
    }
}
