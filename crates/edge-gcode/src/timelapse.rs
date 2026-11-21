//! The timelapse photo of a Bambu Lab printer of two nozzles (the H2D):
//! which nozzle takes it, and the safe spot the head is parked at for it
//! (GCode.cpp `timelapse_most_used_extruder`, `compute_farthest_point`,
//! `generate_timelapse_gcode`; GCode/TimelapsePosPicker.cpp, the
//! reference's port of Bambu Studio's picker, v02.08.04.57).
//!
//! The picker keeps the spot outside the objects (their boxes grown by
//! the printer's `extruder_clearance_max_radius`), outside the cone from
//! the camera at the bed's origin to each object, inside the active
//! nozzle's `extruder_printable_area` and off the prime tower; with a
//! farthest point, the safe point nearest to it. Its results reach the
//! printer's `time_lapse_gcode` as `timelapse_pos_x`, `timelapse_pos_y`
//! and `has_timelapse_safe_pos` (and `clear_to_x0`).
//!
//! Coordinates are the plate's, as the writer's are; numbers are cut to
//! whole units where the reference converts with a cast (`coord_t(scale_())`,
//! `int()`), so that the spot comes out in the same whole millimetres.

use std::collections::HashSet;

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon, Point2, Polygon, UNITS_PER_MM};

use crate::{Layer, NozzleMap};

/// The reference's "no safe spot" (`DefaultTimelapsePos`), and where it
/// puts the camera (`DefaultCameraPos`).
const NONE: Point2 = Point2 { x: 0, y: 0 };
/// Areas narrower than twice this are no place to park (`FILTER_THRESHOLD`, mm).
const FILTER_THRESHOLD: f64 = 5.0;
/// How many candidate spots are kept, best first (`MAX_CANDIDATE_SIZE`).
const MAX_CANDIDATES: usize = 5;

/// `coord_t(scale_(mm))`: scaled, the fraction cut off.
fn cut(mm: f64) -> Coord {
    (mm * UNITS_PER_MM) as Coord
}

/// `unscale_(units)`.
fn mm(units: Coord) -> f64 {
    units as f64 / UNITS_PER_MM
}

fn rectangle(min: Point2, max: Point2) -> Polygon {
    Polygon {
        points: vec![
            min,
            Point2 { x: max.x, y: min.y },
            max,
            Point2 { x: min.x, y: max.y },
        ],
    }
}

fn region(contour: Polygon) -> ExPolygon {
    ExPolygon {
        contour,
        holes: Vec::new(),
    }
}

/// A copy's box on the plate, in mm (`get_real_instance_bbox`: the
/// instance's box; no raft is printed here, so nothing is added for one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Box3 {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Box3 {
    fn centre(&self) -> [f64; 2] {
        [
            (self.min[0] + self.max[0]) / 2.0,
            (self.min[1] + self.max[1]) / 2.0,
        ]
    }

    /// Its footprint as the reference scales it.
    fn footprint(&self) -> Polygon {
        rectangle(
            Point2 {
                x: cut(self.min[0]),
                y: cut(self.min[1]),
            },
            Point2 {
                x: cut(self.max[0]),
                y: cut(self.max[1]),
            },
        )
    }
}

/// What one photo's spot is picked for (`PosPickCtx`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Ask {
    /// Where the head is.
    pub at: Point2,
    /// The height of the layer being printed (`print_z`).
    pub z: f64,
    /// The extruder (from 0) that takes the photo, and the active one.
    pub photo: usize,
    pub active: usize,
    /// Printing object by object, several objects: the objects printed so
    /// far, the current one last, by number.
    pub printed: Option<Vec<usize>>,
    /// The layer's farthest point, with a farthest-point timelapse: the
    /// safe spot nearest to it is taken.
    pub farthest: Option<Point2>,
}

/// The printer's and the plate's side of the picker (`TimelapsePosPicker`).
#[derive(Clone, Debug)]
pub(crate) struct Picker {
    bed: Polygon,
    plate_width: Coord,
    plate_height: Coord,
    /// Each extruder's area to park in: the bed less what is excluded and
    /// the tower, within the extruder's own printable area.
    areas: Vec<Vec<ExPolygon>>,
    by_object: bool,
    /// The smooth timelapse: one spot for the whole print.
    all_layers: bool,
    height_to_rod: i64,
    /// `extruder_clearance_max_radius` (68 mm where 0), and that as the
    /// whole millimetres the camera's cone is grown by.
    clearance: f64,
    clearance_whole: i64,
    /// The extruder that can be lifted (the lower printable height), and
    /// the height between the two, where both extruders are used.
    liftable: Option<usize>,
    height_gap: Option<i64>,
    /// The copies of each object, in the plate's order of objects.
    objects: Vec<Vec<Box3>>,
    all_layer_spot: Option<Point2>,
    /// The safe areas and the collision areas worked out so far, by what
    /// they depend on ([`Self::pick_for_layer`]).
    safe_areas: std::collections::HashMap<Vec<i64>, (Vec<ExPolygon>, Vec<ExPolygon>)>,
}

/// The settings the picker reads, as the writer's variables give them.
pub(crate) struct PickerInput<'a> {
    /// `printable_area`'s corners, mm.
    pub bed: &'a [[f64; 2]],
    /// Boxes kept off (`bed_exclude_area`, as the writer reads it: the box
    /// round its corners), mm.
    pub excluded: &'a [[f64; 4]],
    /// Each extruder's `extruder_printable_area`, mm.
    pub extruder_areas: &'a [Vec<[f64; 2]>],
    /// `extruder_printable_height`, one per extruder.
    pub printable_heights: &'a [f64],
    pub extruders: usize,
    /// The prime tower's ground, brim included, mm (min x, min y, max x, max y).
    pub tower: Option<[f64; 4]>,
    pub height_to_rod: f64,
    pub clearance: f64,
    pub by_object: bool,
    pub smooth: bool,
    /// Whether the print uses both extruders.
    pub both_used: bool,
    pub objects: Vec<Vec<Box3>>,
}

impl Picker {
    /// `init`, `construct_printable_area_by_printer`.
    pub(crate) fn new(input: PickerInput<'_>) -> Result<Self, ClipError> {
        let clearance = if input.clearance > 0.0 {
            input.clearance
        } else {
            68.0
        };
        let bed = Polygon {
            points: (input.bed.iter())
                .map(|[x, y]| Point2 {
                    x: cut(*x),
                    y: cut(*y),
                })
                .collect(),
        };
        let max = bed
            .points
            .iter()
            .fold(Point2 { x: 0, y: 0 }, |m, p| Point2 {
                x: m.x.max(p.x),
                y: m.y.max(p.y),
            });
        // The gap between the two printable heights counts only with both
        // extruders used. Printing object by object too, as in Bambu
        // Studio, whose grouping is made before its picker is set up: the
        // reference sets the picker up (GCode.cpp 3965) before its
        // by-object grouping (4026), so it never has the gap there and
        // places photos below it where the idle photo nozzle hangs lower
        // than the printed parts; that is not followed.
        let (liftable, height_gap) = match input.printable_heights {
            [left, right, ..] if input.extruders > 1 => (
                Some(if left < right { 0 } else { 1 }),
                input.both_used.then(|| (left - right).abs() as i64),
            ),
            _ => (None, None),
        };
        let mut picker = Self {
            plate_width: cut(mm(max.x).trunc()),
            plate_height: cut(mm(max.y).trunc()),
            bed,
            areas: Vec::new(),
            by_object: input.by_object,
            all_layers: input.smooth,
            height_to_rod: input.height_to_rod as i64,
            clearance,
            clearance_whole: clearance as i64,
            liftable,
            height_gap,
            objects: input.objects,
            all_layer_spot: None,
            safe_areas: std::collections::HashMap::new(),
        };
        // The bed less what is excluded, less the tower's box grown by the
        // clearance.
        let excluded: Vec<ExPolygon> = (input.excluded.iter())
            .map(|[a, b, c, d]| {
                region(rectangle(
                    Point2 {
                        x: cut(*a),
                        y: cut(*b),
                    },
                    Point2 {
                        x: cut(*c),
                        y: cut(*d),
                    },
                ))
            })
            .collect();
        let tower: Vec<ExPolygon> = match input.tower {
            Some([a, b, c, d]) => picker
                .grown(
                    &rectangle(
                        Point2 {
                            x: cut(a),
                            y: cut(b),
                        },
                        Point2 {
                            x: cut(c),
                            y: cut(d),
                        },
                    ),
                    true,
                )?
                .into_iter()
                .map(region)
                .collect(),
            None => Vec::new(),
        };
        let open = edge_clip::difference(
            &edge_clip::difference(&[region(picker.bed.clone())], &excluded)?,
            &tower,
        )?;
        for extruder in 0..input.extruders.max(1) {
            let own = (input.extruder_areas.get(extruder))
                .filter(|corners| !corners.is_empty())
                .map(|corners| Polygon {
                    points: (corners.iter())
                        .map(|[x, y]| Point2 {
                            x: cut(*x),
                            y: cut(*y),
                        })
                        .collect(),
                });
            picker.areas.push(match own {
                Some(own) => edge_clip::intersection(&open, &[region(own)])?,
                None => open.clone(),
            });
        }
        Ok(picker)
    }

    /// `expand_object_projection`: a box grown by the clearance, or by
    /// half of it.
    fn grown(&self, footprint: &Polygon, higher: bool) -> Result<Option<Polygon>, ClipError> {
        let radius = if self.by_object && higher {
            (self.clearance * UNITS_PER_MM) as f32
        } else {
            (self.clearance / 2.0 * UNITS_PER_MM) as f32
        };
        let grown = edge_clip::offset(&[region(footprint.clone())], radius as Coord, Join::MITER)?;
        Ok(grown.into_iter().next().map(|region| region.contour))
    }

    fn object_list(&self, printed: Option<&[usize]>) -> Vec<usize> {
        match printed {
            Some(printed) => printed.to_vec(),
            None => (0..self.objects.len()).collect(),
        }
    }

    /// Which copies `collect_object_slices_data` takes for the layer at
    /// `z`: those whose height meets it (from `range` below or above it),
    /// each with whether it stands taller (not the last object of
    /// `list`); none for the whole bed, where that range reaches the bed.
    fn meeting(&self, z: f64, range: f32, list: &[usize]) -> Option<Vec<(usize, usize, bool)>> {
        let meet = |a: i64, b: i64, c: i64, d: i64| (a <= c && c <= b) || (c <= a && a <= d);
        let target = z as f32;
        let (low, high) = if range < 0.0 {
            (target + range, target)
        } else {
            (target, target + range)
        };
        if low <= 0.0 {
            return None;
        }
        let mut out = Vec::new();
        for (at, object) in list.iter().enumerate() {
            let last = at + 1 == list.len();
            for (index, copy) in self.objects.get(*object).into_iter().flatten().enumerate() {
                let higher = !last && copy.max[2] > f64::from(target);
                if meet(
                    copy.min[2] as i64,
                    copy.max[2] as i64,
                    low as i64,
                    high as i64,
                ) {
                    out.push((*object, index, higher));
                }
            }
        }
        Some(out)
    }

    /// `collect_object_slices_data`: the boxes, grown, of the copies
    /// [`Self::meeting`] gives, or the whole bed.
    fn slices(
        &self,
        meeting: Option<&[(usize, usize, bool)]>,
    ) -> Result<Vec<ExPolygon>, ClipError> {
        let Some(meeting) = meeting else {
            return Ok(vec![region(self.bed.clone())]);
        };
        let mut out = Vec::new();
        for (object, index, higher) in meeting {
            if let Some(grown) = self.grown(&self.objects[*object][*index].footprint(), *higher)? {
                out.push(region(grown));
            }
        }
        edge_clip::union_ex(&out)
    }

    /// `get_limit_area_for_camera`: what lies between the camera and an
    /// object (its first copy's box, grown).
    fn camera_cone(&self, object: usize) -> Option<Polygon> {
        let copy = self.objects.get(object)?.first()?;
        let radius = (self.clearance_whole as f32 / 2.0) as f64;
        let d = 2f64.sqrt() * radius;
        let (min_x, min_y) = (copy.min[0] - d, copy.min[1] - d);
        let (max_x, max_y) = (copy.max[0] + d, copy.max[1] + d);
        let at = |mm: f64| ((mm * UNITS_PER_MM).max(0.0)) as Coord;
        Some(Polygon {
            points: vec![
                NONE,
                Point2 {
                    x: at(max_x),
                    y: at(min_y),
                },
                Point2 {
                    x: at(max_x),
                    y: at(max_y),
                },
                Point2 {
                    x: at(min_x),
                    y: at(max_y),
                },
            ],
        })
    }

    fn camera_cones(&self, list: &[usize]) -> Result<Vec<ExPolygon>, ClipError> {
        let cones: Vec<Polygon> = list
            .iter()
            .filter_map(|object| self.camera_cone(*object))
            .collect();
        edge_clip::union(&cones)
    }

    /// `collect_limit_areas_for_rod`: printing object by object, the
    /// bands of the bed across which the rod would meet an object taller
    /// than it clears.
    fn rod_limits(&self, list: &[usize], ask: &Ask) -> Vec<ExPolygon> {
        let limit = self.height_to_rod as f64 + ask.z;
        let current = ask.printed.as_ref().and_then(|printed| printed.last());
        let half = if self.by_object {
            self.clearance
        } else {
            self.clearance / 2.0
        };
        let mut boxes: Vec<[f64; 2]> = list
            .iter()
            .filter(|object| Some(*object) != current)
            .filter_map(|object| self.objects.get(*object)?.first())
            .filter(|copy| copy.max[2] >= limit)
            .map(|copy| [copy.min[1] - half, copy.max[1] + half])
            .collect();
        if boxes.is_empty() {
            return Vec::new();
        }
        boxes.sort_by(|a, b| {
            a[0].partial_cmp(&b[0])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal))
        });
        let height = (mm(self.plate_height)) as i64;
        let width = (mm(self.plate_width)) as i64;
        let mut ranges: Vec<[i64; 2]> = vec![[0, 0]];
        for [low, high] in boxes {
            let last = ranges.last_mut().expect("one range at least");
            if last[1] as f64 >= low {
                last[1] = high as i64;
            } else {
                ranges.push([low as i64, high as i64]);
            }
        }
        if ranges.last().is_some_and(|last| last[1] < height) {
            ranges.push([height, height]);
        }
        let at_y = (mm(ask.at.y)) as i64;
        let band = ranges
            .windows(2)
            .find(|pair| at_y >= pair[0][1] && at_y <= pair[1][0]);
        let Some(pair) = band else {
            return vec![region(self.bed.clone())];
        };
        let (lower, upper) = (pair[0][1], pair[1][0]);
        let units = |mm: i64| cut(mm as f64);
        vec![
            region(rectangle(
                Point2 { x: 0, y: 0 },
                Point2 {
                    x: units(width),
                    y: units(lower),
                },
            )),
            region(rectangle(
                Point2 {
                    x: 0,
                    y: units(upper),
                },
                Point2 {
                    x: units(width),
                    y: units(height),
                },
            )),
        ]
    }

    fn objects_centre(&self, list: &[usize]) -> Point2 {
        let (mut x, mut y, mut count) = (0.0, 0.0, 0usize);
        for object in list {
            for copy in self.objects.get(*object).into_iter().flatten() {
                let [cx, cy] = copy.centre();
                x += cx;
                y += cy;
                count += 1;
            }
        }
        if count == 0 {
            return NONE;
        }
        Point2 {
            x: cut(x / count as f64),
            y: cut(y / count as f64),
        }
    }

    /// `get_object_center`: the middle of an object's first copy.
    fn object_centre(&self, object: usize) -> Point2 {
        match self.objects.get(object).and_then(|copies| copies.first()) {
            Some(copy) => {
                let [x, y] = copy.centre();
                Point2 {
                    x: cut(x),
                    y: cut(y),
                }
            }
            None => NONE,
        }
    }

    /// The spot for one photo (`pick_pos`), in whole millimetres; (0, 0)
    /// for none.
    pub(crate) fn pick(&mut self, ask: &Ask) -> Result<[i64; 2], ClipError> {
        let spot = if self.all_layers {
            self.pick_for_all_layers(ask)?
        } else {
            self.pick_for_layer(ask)?
        };
        Ok([mm(spot.x) as i64, mm(spot.y) as i64])
    }

    /// `pick_pos_for_curr_layer`.
    fn pick_for_layer(&mut self, ask: &Ask) -> Result<Point2, ClipError> {
        let mut gap = 0f32;
        if ask.active != ask.photo
            && let (Some(liftable), Some(height)) = (self.liftable, self.height_gap)
            && ask.photo != liftable
        {
            gap = -(height as f32);
        }
        let printed = ask.printed.as_deref().filter(|list| !list.is_empty());
        let by_object = self.by_object;
        let list = self.object_list(ask.printed.as_deref());
        let active = ask.active.min(self.areas.len().saturating_sub(1));
        let meeting = self.meeting(ask.z, gap, &list);
        let rods = match printed {
            Some(_) if by_object => self.rod_limits(&list, ask),
            _ => Vec::new(),
        };
        let before = match printed {
            Some(printed) if by_object => {
                Some(self.meeting(ask.z, gap, &printed[..printed.len() - 1]))
            }
            _ => None,
        };
        // The safe areas and what a way to them must not cross depend on
        // these alone, which most photos of a print share: worked out once.
        let mut key: Vec<i64> = vec![active as i64];
        let push_meeting =
            |key: &mut Vec<i64>, meeting: &Option<Vec<(usize, usize, bool)>>| match meeting {
                None => key.push(-1),
                Some(copies) => {
                    key.push(copies.len() as i64);
                    for (object, index, higher) in copies {
                        key.extend([*object as i64, *index as i64, i64::from(*higher)]);
                    }
                }
            };
        push_meeting(&mut key, &meeting);
        key.push(-2);
        key.extend(list.iter().map(|object| *object as i64));
        key.push(-3);
        for rod in &rods {
            key.extend(rod.contour.points.iter().flat_map(|p| [p.x, p.y]));
        }
        key.push(-4);
        if let Some(before) = &before {
            push_meeting(&mut key, before);
        }
        if !self.safe_areas.contains_key(&key) {
            let slices = self.slices(meeting.as_deref())?;
            let cones = self.camera_cones(&list)?;
            let unplaceable = edge_clip::union_ex(
                &[
                    edge_clip::union_ex(&[slices, cones].concat())?,
                    rods.clone(),
                ]
                .concat(),
            )?;
            let area = self.areas.get(active).cloned().unwrap_or_default();
            let safe = edge_clip::difference(&area, &unplaceable)?;
            let safe = edge_clip::opening(&safe, cut(FILTER_THRESHOLD), Join::MITER)?;
            let collision = match &before {
                Some(before) => {
                    let slices = self.slices(before.as_deref())?;
                    edge_clip::union_ex(&[slices, rods].concat())?
                }
                None => Vec::new(),
            };
            self.safe_areas.insert(key.clone(), (safe, collision));
        }
        let centre = match printed {
            Some(printed) if by_object => self.object_centre(printed[printed.len() - 1]),
            _ => self.objects_centre(&list),
        };
        let (safe, collision) = &self.safe_areas[&key];
        Ok(best_spot(centre, safe, collision, by_object, ask.farthest))
    }

    /// `pick_pos_for_all_layer`: the smooth timelapse's one spot.
    fn pick_for_all_layers(&mut self, ask: &Ask) -> Result<Point2, ClipError> {
        if self.by_object {
            return Ok(NONE);
        }
        if let Some(gap) = self.height_gap
            && ask.z <= gap as f64
        {
            return Ok(NONE);
        }
        if let Some(spot) = self.all_layer_spot {
            return Ok(spot);
        }
        let list = self.object_list(None);
        let mut boxes = Vec::new();
        for object in &list {
            for copy in &self.objects[*object] {
                if let Some(grown) = self.grown(&copy.footprint(), true)? {
                    boxes.push(region(grown));
                }
            }
        }
        let boxes = edge_clip::union_ex(&boxes)?;
        let cones = self.camera_cones(&list)?;
        let unplaceable = edge_clip::union_ex(&[boxes, cones].concat())?;
        let area = match self.areas.as_slice() {
            [first, second, ..] => edge_clip::intersection(first, second)?,
            [only] => only.clone(),
            [] => Vec::new(),
        };
        let safe = edge_clip::difference(&area, &unplaceable)?;
        let safe = edge_clip::opening(&safe, cut(FILTER_THRESHOLD), Join::MITER)?;
        let spot = best_spot(self.objects_centre(&list), &safe, &[], false, None);
        self.all_layer_spot = Some(spot);
        Ok(spot)
    }

    /// `get_is_clear_to_x0`: whether the head can go straight to X 0
    /// from where it is without passing over another object's box.
    pub(crate) fn clear_to_x0(&self, ask: &Ask) -> Result<bool, ClipError> {
        let list = self.object_list(ask.printed.as_deref());
        let meet = |a: i64, b: i64, c: i64, d: i64| (a <= c && c <= b) || (c <= a && a <= d);
        let target = ask.z as f32;
        let (low, high) = ((ask.z - 0.5) as f32, (ask.z + 0.5) as f32);
        for (at, object) in list.iter().enumerate() {
            let current = at + 1 == list.len() || !self.by_object;
            for copy in self.objects.get(*object).into_iter().flatten() {
                let higher = copy.max[2] > f64::from(target);
                if current
                    || !meet(
                        copy.min[2] as i64,
                        copy.max[2] as i64,
                        low as i64,
                        high as i64,
                    )
                {
                    continue;
                }
                let Some(grown) = self.grown(&copy.footprint(), higher)? else {
                    continue;
                };
                let (mut lo, mut hi) = (
                    Point2 {
                        x: Coord::MAX,
                        y: Coord::MAX,
                    },
                    Point2 {
                        x: Coord::MIN,
                        y: Coord::MIN,
                    },
                );
                for p in &grown.points {
                    lo = Point2 {
                        x: lo.x.min(p.x),
                        y: lo.y.min(p.y),
                    };
                    hi = Point2 {
                        x: hi.x.max(p.x),
                        y: hi.y.max(p.y),
                    };
                }
                if ask.at.y < lo.y || ask.at.y > hi.y {
                    continue;
                }
                if lo.x <= ask.at.x {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

/// Slic3r's `Polygon::contains`: crossings of a ray to the right.
fn inside(polygon: &Polygon, point: Point2) -> bool {
    let points = &polygon.points;
    let mut result = false;
    let Some(mut j) = points.len().checked_sub(1) else {
        return false;
    };
    for i in 0..points.len() {
        let (a, b) = (points[i], points[j]);
        if (a.y > point.y) != (b.y > point.y)
            && (point.x as f64)
                < (b.x - a.x) as f64 * (point.y - a.y) as f64 / (b.y - a.y) as f64 + a.x as f64
        {
            result = !result;
        }
        j = i;
    }
    result
}

fn contains(region: &ExPolygon, point: Point2) -> bool {
    inside(&region.contour, point) && !region.holes.iter().any(|hole| inside(hole, point))
}

/// `pick_pos_internal`: where the head already is when that is safe, else
/// along each edge of the safe areas in 5 mm steps the best point of the
/// edge (nearest to the farthest point, or nearest to `from` and away
/// from the camera), the five best of them in turn, the first whose way
/// from `from` meets no collision area where that is checked; (0, 0) for
/// none.
///
/// The reference keeps the five in a `std::priority_queue`, which leaves
/// the order of equal penalties to the standard library; here the edge
/// met first wins a tie.
pub(crate) fn best_spot(
    from: Point2,
    safe: &[ExPolygon],
    collision: &[ExPolygon],
    check: bool,
    farthest: Option<Point2>,
) -> Point2 {
    if safe.iter().any(|region| contains(region, from)) {
        return from;
    }
    if safe.is_empty() {
        return NONE;
    }
    let l1 = |a: Point2, b: Point2| ((a.x - b.x).abs() + (a.y - b.y).abs()) as f64;
    let penalty = |candidate: Point2| match farthest {
        Some(farthest) => l1(farthest, candidate),
        None => l1(from, candidate) - l1(NONE, candidate) / 3.0,
    };
    let step = 5.0 * UNITS_PER_MM;
    // (penalty, the order it came in, the point): the five lowest.
    let mut best: Vec<(f64, usize, Point2)> = Vec::new();
    let mut order = 0;
    for region in safe {
        for polygon in std::iter::once(&region.contour).chain(&region.holes) {
            let points = &polygon.points;
            for (i, start) in points.iter().enumerate() {
                let end = points[(i + 1) % points.len()];
                let (mut best_penalty, mut candidate) = (f64::MAX, NONE);
                if l1(*start, end) < step {
                    candidate = *start;
                    best_penalty = penalty(candidate);
                } else {
                    let length = l1(*start, end);
                    let steps = (length / step) as i64;
                    let dx = ((end.x - start.x) as f64 * step / length) as Coord;
                    let dy = ((end.y - start.y) as f64 * step / length) as Coord;
                    for k in 0..=steps {
                        let point = Point2 {
                            x: start.x + dx * k,
                            y: start.y + dy * k,
                        };
                        let value = penalty(point);
                        if value < best_penalty {
                            best_penalty = value;
                            candidate = point;
                        }
                    }
                }
                best.push((best_penalty, order, candidate));
                order += 1;
                if best.len() > MAX_CANDIDATES {
                    // Drop the worst; of equal ones the last come.
                    let worst = (0..best.len())
                        .max_by(|a, b| {
                            best[*a]
                                .0
                                .partial_cmp(&best[*b].0)
                                .unwrap_or(std::cmp::Ordering::Equal)
                                .then(best[*a].1.cmp(&best[*b].1))
                        })
                        .expect("candidates");
                    best.remove(worst);
                }
            }
        }
    }
    best.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    for (_, _, point) in best {
        if !check {
            return point;
        }
        match edge_clip::clip_lines(&[vec![from, point]], collision) {
            Ok(hits) if hits.is_empty() => return point,
            _ => {}
        }
    }
    NONE
}

/// The extruder that prints on the most layers takes the photo, the later
/// of two equally used (GCode.cpp `timelapse_most_used_extruder`, Bambu
/// Studio's `calc_most_used_extruder`). `layers` are the print's layers,
/// each its filaments.
pub(crate) fn photo_extruder<'a, 'b: 'a>(
    layers: impl IntoIterator<Item = &'a [Layer<'b>]>,
    map: &NozzleMap,
    extruders: usize,
) -> usize {
    let extruders = extruders.max(1);
    let mut used = vec![0usize; extruders];
    for parts in layers {
        let on: HashSet<usize> = parts
            .iter()
            .map(|part| map.extruder(part.filament))
            .filter(|extruder| *extruder < extruders)
            .collect();
        for extruder in on {
            used[extruder] += 1;
        }
    }
    let mut most = 0;
    for extruder in 1..extruders {
        if used[extruder] >= used[most] {
            most = extruder;
        }
    }
    most
}

/// The writer's side of the photos on two nozzles: the picker, the photo
/// extruder of each copy printed in turn (or of the print), and what the
/// layer being written found.
#[derive(Clone, Debug)]
pub(crate) struct Photos {
    pub picker: Picker,
    /// The photo extruder (from 0): the print's, or each run's printing
    /// object by object (`process_layers` works it out per tool ordering).
    pub photo: Vec<usize>,
    /// Logical extruder to physical (`physical_extruder_map`).
    pub physical: Vec<usize>,
    pub map: NozzleMap,
    /// A farthest-point timelapse (`farthest_point_timelapse`, traditional).
    pub farthest: bool,
    /// Photos are placed by the writer (`need_insert_timelapse_gcode_for_traditional`):
    /// all but a smooth timelapse with its tower, which takes one at
    /// each layer's start.
    pub placed: bool,
    /// Printing object by object: each run's object, by number.
    pub run_objects: Vec<usize>,
    /// The layer's farthest point, and whether the photo extruder prints it.
    pub far: Option<Point2>,
    pub far_on_photo_head: bool,
    /// A photo was taken in passing or inside a change this layer
    /// (`inserted_this_layer`), which a photo inside a change waits on.
    pub inserted: bool,
}

impl Photos {
    pub(crate) fn photo_extruder(&self, run: Option<usize>) -> usize {
        let run = run.unwrap_or(0);
        self.photo
            .get(run)
            .or(self.photo.first())
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn physical(&self, extruder: usize) -> usize {
        self.physical.get(extruder).copied().unwrap_or(extruder)
    }

    /// The photo is taken inline at the farthest point this layer.
    pub(crate) fn inline(&self) -> bool {
        self.farthest && self.far_on_photo_head
    }

    /// Whether a photo may be taken before a change away from a nozzle
    /// using `filament` (`timelapse_head_ok`): from the photo nozzle, or,
    /// where the farthest point is the other nozzle's, from that one.
    pub(crate) fn head_ok(&self, filament: usize, run: Option<usize>) -> bool {
        let on_photo = self.map.extruder(filament) == self.photo_extruder(run);
        if self.farthest && !self.far_on_photo_head {
            !on_photo
        } else {
            on_photo
        }
    }
}

/// The corners of a "XxY,XxY" list, each entry of a list of them an area
/// of its own, and `#` too parting areas (`extruder_printable_area`).
pub(crate) fn areas_of(values: &[edge_template::Value]) -> Vec<Vec<[f64; 2]>> {
    values
        .iter()
        .flat_map(|value| {
            value
                .to_string()
                .split('#')
                .map(|area| {
                    area.split(',')
                        .filter_map(|corner| {
                            let (x, y) = corner.split_once('x')?;
                            Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .filter(|area| !area.is_empty())
        .collect()
}

/// What the writer needs for the photos of a Bambu Lab printer of two
/// nozzles, from its variables and the plate. `groups` are the print's
/// layers (entries of `layers`), `own` each one's run.
pub(crate) fn photos(
    variables: &edge_template::Variables,
    settings: &crate::PrintSettings,
    layers: &[Layer<'_>],
    groups: &[std::ops::Range<usize>],
    own: &[(usize, usize)],
) -> Result<Option<Photos>, ClipError> {
    let Some(map) = settings.plate.nozzles.clone() else {
        return Ok(None);
    };
    let number = |name: &str| {
        variables
            .value(name)
            .and_then(|value| value.to_string().trim().parse::<f64>().ok())
    };
    let numbers = |name: &str| -> Vec<f64> {
        variables
            .list(name)
            .unwrap_or(&[])
            .iter()
            .filter_map(|value| value.to_string().trim().parse().ok())
            .collect()
    };
    let text = |name: &str| variables.value(name).map(|value| value.to_string());
    let extruders = variables
        .list("nozzle_diameter")
        .map_or(1, <[edge_template::Value]>::len)
        .max(1);
    let smooth = text("timelapse_type").is_some_and(|kind| matches!(kind.trim(), "1" | "smooth"));
    let by_object = text("print_sequence").is_some_and(|kind| kind == "by object");
    let farthest = !smooth
        && text("farthest_point_timelapse").is_some_and(|on| matches!(on.trim(), "1" | "true"));
    let tower = settings.tower_mesh_bottom.or(settings.tower_footprint);
    let mut used: Vec<usize> = layers
        .iter()
        .map(|layer| map.extruder(layer.filament))
        .collect();
    used.sort_unstable();
    used.dedup();
    // Each object's copies, in the plate's order of objects.
    let labels = &settings.plate.labels;
    let count = labels
        .iter()
        .map(|label| label.object + 1)
        .max()
        .unwrap_or(0);
    let mut objects = vec![Vec::new(); count];
    for label in labels {
        let [a, b, c, d, e, f] = label.bounds;
        objects[label.object].push(Box3 {
            min: [a, b, c],
            max: [d, e, f],
        });
    }
    let bed = settings.bed_outline.clone();
    let extruder_areas = areas_of(variables.list("extruder_printable_area").unwrap_or(&[]));
    let picker = Picker::new(PickerInput {
        bed: &bed,
        excluded: &settings.bed_exclusions,
        extruder_areas: &extruder_areas,
        printable_heights: &numbers("extruder_printable_height"),
        extruders,
        tower,
        height_to_rod: number("extruder_clearance_height_to_rod").unwrap_or(0.0),
        clearance: number("extruder_clearance_max_radius").unwrap_or(0.0),
        by_object,
        smooth,
        both_used: used.len() > 1,
        objects,
    })?;
    // The photo extruder of the print, or of each run.
    let runs = settings.plate.by_object.len().max(1);
    let photo = (0..runs)
        .map(|run| {
            photo_extruder(
                groups
                    .iter()
                    .zip(own)
                    .filter(|(_, (of, _))| settings.plate.by_object.is_empty() || *of == run)
                    .map(|(group, _)| &layers[group.clone()]),
                &map,
                extruders,
            )
        })
        .collect();
    let physical = (0..extruders)
        .map(|extruder| {
            variables
                .list("physical_extruder_map")
                .and_then(|list| list.get(extruder))
                .and_then(|value| value.to_string().trim().parse::<usize>().ok())
                .unwrap_or(extruder)
        })
        .collect();
    let run_objects = (settings.plate.by_object.iter())
        .map(|run| labels.get(run.copy).map_or(0, |label| label.object))
        .collect();
    Ok(Some(Photos {
        picker,
        photo,
        physical,
        map,
        farthest,
        placed: !(smooth && tower.is_some()),
        run_objects,
        far: None,
        far_on_photo_head: false,
        inserted: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(min: [f64; 2], max: [f64; 2]) -> ExPolygon {
        region(rectangle(
            Point2::from_mm(min[0], min[1]),
            Point2::from_mm(max[0], max[1]),
        ))
    }

    /// The H2D's bed and nozzles, one object in the middle (155..195 x
    /// 150..170, 8 mm tall) and a prime tower near the back left.
    fn h2d(by_object: bool, smooth: bool, both_used: bool) -> Picker {
        Picker::new(PickerInput {
            bed: &[[0.0, 0.0], [350.0, 0.0], [350.0, 320.0], [0.0, 320.0]],
            excluded: &[],
            extruder_areas: &[
                vec![[0.0, 0.0], [325.0, 0.0], [325.0, 320.0], [0.0, 320.0]],
                vec![[25.0, 0.0], [350.0, 0.0], [350.0, 320.0], [25.0, 320.0]],
            ],
            printable_heights: &[320.0, 325.0],
            extruders: 2,
            tower: Some([40.2, 219.5, 101.2, 243.7]),
            height_to_rod: 47.4,
            clearance: 96.0,
            by_object,
            smooth,
            both_used,
            objects: vec![vec![Box3 {
                min: [155.0, 150.0, 0.0],
                max: [195.0, 170.0, 8.0],
            }]],
        })
        .unwrap()
    }

    fn ask(z: f64, photo: usize, active: usize) -> Ask {
        Ask {
            at: Point2::from_mm(175.0, 160.0),
            z,
            photo,
            active,
            printed: None,
            farthest: Some(Point2::from_mm(194.6, 169.6)),
        }
    }

    /// Where the head already is when that is safe; nothing when nothing
    /// is; else along the edges, in 5 mm steps, the point nearest the
    /// farthest point (L1), or nearest the start and away from the camera.
    #[test]
    fn the_best_spot_is_the_nearest_safe_point() {
        let safe = [square([100.0, 100.0], [200.0, 200.0])];
        let inside = Point2::from_mm(150.0, 150.0);
        assert_eq!(best_spot(inside, &safe, &[], false, None), inside);
        assert_eq!(
            best_spot(Point2::from_mm(10.0, 10.0), &[], &[], false, None),
            NONE
        );
        // Towards a farthest point right of the square: its right edge,
        // at the step nearest to it.
        let far = Point2::from_mm(260.0, 151.0);
        let spot = best_spot(Point2::from_mm(10.0, 10.0), &safe, &[], false, Some(far));
        assert_eq!(spot, Point2::from_mm(200.0, 150.0));
        // Without one: near the start, the camera at the origin weighing a
        // third against it.
        let spot = best_spot(Point2::from_mm(300.0, 150.0), &safe, &[], false, None);
        assert_eq!(spot.x, Point2::from_mm(200.0, 0.0).x);
        // A way to it that meets a collision area is not taken.
        let wall = [square([210.0, 0.0], [220.0, 320.0])];
        let spot = best_spot(Point2::from_mm(300.0, 150.0), &safe, &wall, true, None);
        assert_eq!(spot, NONE);
    }

    /// The spot keeps clear of the object's box grown by half the
    /// clearance (48 mm), of the tower's, and stays on the active
    /// nozzle's part of the bed, in whole millimetres.
    #[test]
    fn the_spot_keeps_clear_of_objects_and_the_tower() {
        let mut picker = h2d(false, false, true);
        let [x, y] = picker.pick(&ask(0.2, 0, 1)).unwrap();
        assert_ne!([x, y], [0, 0]);
        let (x, y) = (x as f64, y as f64);
        let clear_of = |min: [f64; 2], max: [f64; 2], by: f64| {
            x <= min[0] - by || x >= max[0] + by || y <= min[1] - by || y >= max[1] + by
        };
        assert!(
            clear_of([155.0, 150.0], [195.0, 170.0], 48.0 - 1.0),
            "{x} {y}"
        );
        assert!(
            clear_of([40.2, 219.5], [101.2, 243.7], 48.0 - 1.0),
            "{x} {y}"
        );
        assert!((25.0..=350.0).contains(&x) && (0.0..=320.0).contains(&y));
    }

    /// With both nozzles used, the photo nozzle not the liftable one and
    /// another nozzle active, everything below the printable heights' gap
    /// (5 mm on the H2D) is in the way: no spot until the layer is above
    /// it. With one nozzle used, no gap.
    #[test]
    fn below_the_nozzles_height_gap_there_is_no_spot() {
        let mut picker = h2d(false, false, true);
        assert_eq!(picker.pick(&ask(0.2, 1, 0)).unwrap(), [0, 0]);
        assert_ne!(picker.pick(&ask(6.0, 1, 0)).unwrap(), [0, 0]);
        // The photo nozzle is the liftable one, or it is active: no gap.
        assert_ne!(picker.pick(&ask(0.2, 0, 1)).unwrap(), [0, 0]);
        assert_ne!(picker.pick(&ask(0.2, 1, 1)).unwrap(), [0, 0]);
        let mut one = h2d(false, false, false);
        assert_ne!(one.pick(&ask(0.2, 1, 0)).unwrap(), [0, 0]);
    }

    /// A smooth timelapse parks at one spot for the whole print, none
    /// below the gap and none printing object by object.
    #[test]
    fn a_smooth_timelapse_has_one_spot() {
        let mut picker = h2d(false, true, true);
        assert_eq!(picker.pick(&ask(2.0, 0, 1)).unwrap(), [0, 0]);
        let first = picker.pick(&ask(6.0, 0, 1)).unwrap();
        assert_ne!(first, [0, 0]);
        assert_eq!(picker.pick(&ask(7.0, 1, 0)).unwrap(), first);
        let mut by_object = h2d(true, true, false);
        assert_eq!(by_object.pick(&ask(6.0, 0, 1)).unwrap(), [0, 0]);
    }

    /// Printing object by object: the way to X 0 is not clear past an
    /// earlier object's box at the head's height.
    #[test]
    fn the_way_to_x0_passes_earlier_objects() {
        let mut picker = h2d(true, false, false);
        picker.objects.push(vec![Box3 {
            min: [250.0, 150.0, 0.0],
            max: [290.0, 170.0, 8.0],
        }]);
        let mut at = ask(4.0, 0, 0);
        at.printed = Some(vec![0, 1]);
        at.at = Point2::from_mm(300.0, 160.0);
        assert!(!picker.clear_to_x0(&at).unwrap());
        at.at = Point2::from_mm(300.0, 300.0);
        assert!(picker.clear_to_x0(&at).unwrap());
        // Layer by layer every object is the current one.
        let by_layer = h2d(false, false, false);
        at.printed = None;
        at.at = Point2::from_mm(300.0, 160.0);
        assert!(by_layer.clear_to_x0(&at).unwrap());
    }

    /// The photo is taken by the extruder printing on the most layers, the
    /// later of two equally used.
    #[test]
    fn the_most_used_extruder_takes_the_photo() {
        let map = NozzleMap {
            extruders: vec![1, 2, 1],
            nozzles: vec![0, 1, 0],
            volumes: vec![0, 0, 0],
        };
        let part = |filament: usize| Layer {
            region: None,
            z: 0.2,
            height: 0.2,
            filament,
            ram: None,
            enter: &[],
            wipe: None,
            tower: &[],
            tower_z: None,
            tower_first: false,
            visits: &[],
            around: &[],
            support: &[],
            loops: &[],
            islands: &[],
            paths: &[],
            shelter: &[],
            below: &[],
        };
        let a = [part(0), part(2)];
        let b = [part(0), part(1)];
        let c = [part(1)];
        assert_eq!(photo_extruder([&a[..], &b[..]], &map, 2), 0);
        assert_eq!(photo_extruder([&a[..], &c[..]], &map, 2), 1);
        assert_eq!(photo_extruder([&b[..], &c[..], &c[..]], &map, 2), 1);
        assert_eq!(photo_extruder([&a[..]], &map, 1), 0);
    }
}
