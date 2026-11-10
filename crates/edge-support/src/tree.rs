//! Tree supports: thin branches from many points under an overhang that
//! lean together into trunks on their way down.
//!
//! A first working version after the reference's "organic" trees
//! (TreeSupport3D.cpp), which is what `tree(auto)` gives by default. The
//! numbers are the reference's: how a branch thickens, how far it may
//! lean, how far apart tips are set, where a trunk flares out at its
//! foot. The method is much simpler:
//!
//! - the reference follows, for every branch, the whole area it could be
//!   in on each layer, and picks positions afterwards from the bed up;
//!   here each branch is one point that is moved layer by layer on the way
//!   down, towards its nearest neighbour if the two can meet before the
//!   bed, and out of the way of the part;
//! - the reference builds each branch as a tube and slices it, so a
//!   leaning branch is an oval on a layer; here it is a circle;
//! - the reference smooths branches afterwards; here they are left as moved.
//!
//! What keeps a branch from being trapped is the same: a branch's centre
//! stays out of every place from which it could not get down to the bed.

use std::f64::consts::{PI, TAU};

use edge_clip::Join;
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::{ExPolygon, Point2, Polygon, scaled, unscaled};
use rayon::prelude::*;

use crate::{Job, Overhang, SupportError, fill};

/// Diameter a trunk flares out to where it stands on the bed, in mm
/// (TreeSupportCommon.hpp `support_tree_bp_diameter`).
const FOOT_DIAMETER: f64 = 7.5;
/// The smallest area worth a roof, in mm² (`minimum_roof_area`).
const MIN_ROOF_AREA: f64 = 1.0;
/// A branch that comes to rest on the part must be at least this tall, in
/// mm, to be worth printing (`support_tree_min_height_to_model`).
const MIN_HEIGHT_ON_PART: f64 = 1.0;
/// An island of branches larger than this, in mm², gets a second wall
/// (SupportParameters.hpp: a quarter of pi times 5 mm squared).
const DOUBLE_WALL_AREA: f64 = 0.25 * PI * 25.0;
/// A gap between crowded branches smaller than this, in mm², gets no wall
/// of its own.
const SMALL_GAP: f64 = 12.5;
/// How far a branch is pushed clear of what it must avoid, in mm.
const CLEAR: f64 = 0.02;

/// The shape of the trees, worked out once from the settings as
/// TreeSupportSettings does.
struct Shape {
    tip_radius: f64,
    branch_radius: f64,
    /// Layers over which a tip thickens into a branch.
    tip_layers: usize,
    /// What a branch gains in radius on each layer below that.
    growth: f64,
    /// How far a branch's centre may move on one layer: as a rule, and
    /// when it has to get out of the way.
    slow: f64,
    fast: f64,
    /// The flare at a trunk's foot: how many layers it takes and what it
    /// gains on each.
    foot_layers: usize,
    foot_growth: f64,
    /// How far apart tips are set.
    pitch: f64,
    /// Radii the part's surroundings are worked out for; a branch uses the
    /// first that is no smaller than its own.
    classes: Vec<f64>,
    min_layers_on_part: usize,
}

impl Shape {
    fn new(job: &Job<'_>, flow: &Flow) -> Self {
        let s = job.settings;
        let height = job.layer_height;
        let branch_radius = (s.tree_branch_diameter / 2.0).max(0.1);
        let tip_radius = (s.tree_tip_diameter / 2.0).clamp(0.05, branch_radius);
        let tip_layers = (((branch_radius - tip_radius) / (flow.width / 3.0)).floor())
            .max((branch_radius / height).floor())
            .max(1.0) as usize;
        let fast_angle = s.tree_branch_angle.clamp(0.0, 89.0);
        let slow_angle = s.tree_angle_slow.clamp(0.0, fast_angle);
        let foot_growth = (0.7_f64.tan() * height).min(0.5 * flow.width);
        let foot_radius = FOOT_DIAMETER / 2.0;
        // With a roof over them tips can stand further apart than when
        // each holds the part up by itself.
        let connect = flow.width / s.tree_top_rate.clamp(0.05, 1.0)
            + (2.0 * tip_radius - flow.width).max(0.0);
        let pitch = if s.interface_top_layers > 0 {
            1.6 * connect
        } else {
            connect
        };
        // TreeModelVolumes::ceilRadius: the tip's radius, 1.5 mm, then half
        // as much again each time. Before them, no clearance at all, for a
        // tip squeezed against the part.
        let mut classes = vec![0.0, tip_radius];
        let mut next: f64 = 1.5;
        while classes.len() < 7 {
            if next > tip_radius {
                classes.push(next);
            }
            next *= 1.5;
        }
        Self {
            tip_radius,
            branch_radius,
            tip_layers,
            growth: s
                .tree_branch_diameter_angle
                .clamp(0.0, 45.0)
                .to_radians()
                .tan()
                * height,
            slow: slow_angle.to_radians().tan() * height,
            fast: fast_angle.to_radians().tan() * height,
            foot_layers: ((foot_radius - branch_radius).max(0.0) / foot_growth) as usize,
            foot_growth,
            pitch,
            classes,
            min_layers_on_part: (MIN_HEIGHT_ON_PART / height).ceil() as usize,
        }
    }

    /// A branch's radius `below_top` layers under its tip
    /// (TreeSupportSettings::getRadius).
    fn radius(&self, below_top: usize) -> f64 {
        if below_top <= self.tip_layers {
            self.tip_radius
                + (self.branch_radius - self.tip_radius) * below_top as f64 / self.tip_layers as f64
        } else {
            self.branch_radius + (below_top - self.tip_layers) as f64 * self.growth
        }
    }

    /// The radius a trunk that reaches the bed is drawn with on a layer:
    /// its own, or the flare's if that is wider.
    fn foot_radius(&self, radius: f64, layer: usize) -> f64 {
        if radius >= self.branch_radius && layer < self.foot_layers {
            radius.max(self.branch_radius + (self.foot_layers - layer) as f64 * self.foot_growth)
        } else {
            radius
        }
    }

    fn class_of(&self, radius: f64) -> usize {
        self.classes
            .iter()
            .position(|class| *class >= radius - 1e-9)
            .unwrap_or(self.classes.len() - 1)
    }
}

/// Where a branch of one radius may not have its centre, layer by layer.
struct Class {
    /// Too near the part.
    collision: Vec<Vec<ExPolygon>>,
    /// Too near the part, or somewhere it could not get down to the bed
    /// from at its usual lean.
    avoidance: Vec<Vec<ExPolygon>>,
}

/// The part's surroundings for every radius a branch can have, worked out
/// when first asked for.
struct Volumes<'a> {
    /// Where no support may be, for each layer up to the highest tip.
    keep_off: &'a [Vec<ExPolygon>],
    radii: &'a [f64],
    lean: f64,
    classes: Vec<Option<Class>>,
}

impl Volumes<'_> {
    fn class(&mut self, index: usize) -> Result<&Class, SupportError> {
        if self.classes[index].is_none() {
            let radius = scaled(self.radii[index]);
            let collision: Vec<Vec<ExPolygon>> = self
                .keep_off
                .par_iter()
                .map(|keep_off| edge_clip::offset(keep_off, radius, Join::MITER))
                .collect::<Result<_, _>>()?;
            // From the bed up: what is unreachable on a layer is what is
            // unreachable on the layer below, less what leaning could make
            // up, and what the part itself takes.
            let mut avoidance: Vec<Vec<ExPolygon>> = Vec::with_capacity(collision.len());
            for (layer, own) in collision.iter().enumerate() {
                if layer == 0 {
                    avoidance.push(own.clone());
                    continue;
                }
                let mut all =
                    edge_clip::offset(&avoidance[layer - 1], -scaled(self.lean), Join::MITER)?;
                all.extend(own.iter().cloned());
                avoidance.push(edge_clip::union_ex(&all)?);
            }
            self.classes[index] = Some(Class {
                collision,
                avoidance,
            });
        }
        Ok(self.classes[index].as_ref().expect("set above"))
    }
}

fn mm(a: Point2, b: Point2) -> f64 {
    unscaled(b.x - a.x).hypot(unscaled(b.y - a.y))
}

/// Whether a point is inside any of some regions.
fn inside(regions: &[ExPolygon], point: Point2) -> bool {
    regions.iter().any(|region| {
        let mut within = false;
        for ring in region.rings() {
            let points = &ring.points;
            for index in 0..points.len() {
                let (a, b) = (points[index], points[(index + 1) % points.len()]);
                if (a.y > point.y) != (b.y > point.y) {
                    let side = i128::from(b.x - a.x) * i128::from(point.y - a.y)
                        - i128::from(point.x - a.x) * i128::from(b.y - a.y);
                    if (side > 0) == (b.y > a.y) {
                        within = !within;
                    }
                }
            }
        }
        within
    })
}

/// The nearest place just outside `regions` for a point inside them, if
/// there is one within `reach` mm.
fn escape(regions: &[ExPolygon], point: Point2, reach: f64) -> Option<Point2> {
    let mut nearest: Option<(f64, f64, f64)> = None;
    let (px, py) = (unscaled(point.x), unscaled(point.y));
    for ring in regions.iter().flat_map(ExPolygon::rings) {
        let points = &ring.points;
        for index in 0..points.len() {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            let (ax, ay) = (unscaled(a.x), unscaled(a.y));
            let (dx, dy) = (unscaled(b.x) - ax, unscaled(b.y) - ay);
            let along = if dx == 0.0 && dy == 0.0 {
                0.0
            } else {
                (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0)
            };
            let (x, y) = (ax + along * dx, ay + along * dy);
            let distance = (x - px).hypot(y - py);
            if nearest.is_none_or(|(least, ..)| distance < least) {
                nearest = Some((distance, x, y));
            }
        }
    }
    let (distance, x, y) = nearest?;
    if distance + CLEAR > reach {
        return None;
    }
    // A little past the edge, the way the point was already heading.
    let (ux, uy) = if distance > 1e-9 {
        ((x - px) / distance, (y - py) / distance)
    } else {
        (1.0, 0.0)
    };
    let out = Point2::from_mm(x + ux * CLEAR, y + uy * CLEAR);
    (!inside(regions, out)).then_some(out)
}

/// The point `share` of the way from `from` to `to`.
fn toward(from: Point2, to: Point2, share: f64) -> Point2 {
    Point2 {
        x: from.x + ((to.x - from.x) as f64 * share).round() as i64,
        y: from.y + ((to.y - from.y) as f64 * share).round() as i64,
    }
}

/// A circle as a polygon, with enough corners to stay within 0.015 mm of
/// round (TreeSupport3D.cpp `extrude_branch`).
fn circle(centre: Point2, radius: f64) -> ExPolygon {
    let corners = if radius <= 0.015 {
        8
    } else {
        ((TAU / (2.0 * (1.0 - 0.015 / radius).acos())).ceil() as usize).clamp(8, 96)
    };
    let (cx, cy) = (unscaled(centre.x), unscaled(centre.y));
    ExPolygon {
        contour: Polygon {
            points: (0..corners)
                .map(|corner| {
                    let turn = TAU * corner as f64 / corners as f64;
                    Point2::from_mm(cx + radius * turn.cos(), cy + radius * turn.sin())
                })
                .collect(),
        },
        holes: Vec::new(),
    }
}

/// Where tips go under an area: along its edge, so the edge is held, and
/// on a grid across it, `pitch` mm apart.
fn sample(area: &[ExPolygon], pitch: f64, inset: f64) -> Result<Vec<Point2>, SupportError> {
    let mut points: Vec<Point2> = Vec::new();
    let add = |point: Point2, points: &mut Vec<Point2>| {
        if points.iter().all(|other| mm(*other, point) >= 0.5 * pitch) {
            points.push(point);
        }
    };
    for region in area {
        // Tips stand inside the edge by their own radius where there is
        // room, and on it where there is not.
        let mut within = Vec::new();
        for inset in [inset, CLEAR] {
            within = edge_clip::offset(std::slice::from_ref(region), -scaled(inset), Join::MITER)?;
            if !within.is_empty() {
                break;
            }
        }
        if within.is_empty() {
            within = vec![region.clone()];
        }
        for inner in &within {
            for ring in inner.rings() {
                let corners = &ring.points;
                if corners.is_empty() {
                    continue;
                }
                add(corners[0], &mut points);
                let mut since = 0.0;
                for index in 0..corners.len() {
                    let (a, b) = (corners[index], corners[(index + 1) % corners.len()]);
                    let length = mm(a, b);
                    let mut at = pitch - since;
                    while at < length {
                        let share = at / length;
                        add(
                            Point2 {
                                x: a.x + ((b.x - a.x) as f64 * share).round() as i64,
                                y: a.y + ((b.y - a.y) as f64 * share).round() as i64,
                            },
                            &mut points,
                        );
                        at += pitch;
                    }
                    since = (since + length) % pitch;
                }
            }
            let [min_x, min_y, max_x, max_y] = crate::bounds(inner);
            let step = scaled(pitch).max(1);
            let mut y = min_y.div_euclid(step) * step;
            while y <= max_y {
                let mut x = min_x.div_euclid(step) * step;
                while x <= max_x {
                    let point = Point2 { x, y };
                    if inside(std::slice::from_ref(inner), point) {
                        add(point, &mut points);
                    }
                    x += step;
                }
                y += step;
            }
        }
    }
    Ok(points)
}

/// One branch on one layer.
#[derive(Clone, Debug)]
struct Node {
    at: Point2,
    /// Layers under the tip, which sets the radius.
    below_top: usize,
    /// Which of the radii the part's surroundings are worked out for this
    /// branch keeps clear by; it lags the radius where room is short.
    class: usize,
    /// Whether the branch can still get down to the bed from here.
    to_bed: bool,
    /// Whether it holds up a roof, which must not be left on nothing.
    roofed: bool,
    /// Which node of the layer below this one continues in.
    below: Option<usize>,
    /// Whether the branch stops here, standing on the part.
    rests: bool,
}

/// Where a branch starts.
#[derive(Clone, Copy, Debug)]
struct Tip {
    at: Point2,
    /// Layers of roof over it, which a tip counts as its own height.
    below_top: usize,
    /// Which overhang it holds up.
    source: usize,
}

struct Grown {
    /// Branches on every layer.
    layers: Vec<Vec<Node>>,
    /// For each tip that found room: its layer, its place there and its overhang.
    planted: Vec<(usize, usize, usize)>,
}

/// Branches on every layer, from tips down to wherever each one ends.
fn grow(
    tips: &[Vec<Tip>],
    shape: &Shape,
    volumes: &mut Volumes<'_>,
    plate_only: bool,
) -> Result<Grown, SupportError> {
    let mut layers: Vec<Vec<Node>> = vec![Vec::new(); tips.len()];
    let mut planted = Vec::new();
    for layer in (0..tips.len()).rev() {
        // New tips, where there is room for one with its own radius to
        // spare. An overhang none of whose tips find that much room has
        // them squeezed against the part instead, sooner than go without.
        let mut roomy: Vec<usize> = Vec::new();
        for squeezed in [false, true] {
            for tip in &tips[layer] {
                if squeezed && roomy.contains(&tip.source) {
                    continue;
                }
                let class = if squeezed {
                    0
                } else {
                    shape.class_of(shape.radius(tip.below_top))
                };
                let volume = volumes.class(class)?;
                if inside(&volume.collision[layer], tip.at) {
                    continue;
                }
                let to_bed = !inside(&volume.avoidance[layer], tip.at);
                if to_bed || !plate_only {
                    planted.push((layer, layers[layer].len(), tip.source));
                    layers[layer].push(Node {
                        at: tip.at,
                        below_top: tip.below_top,
                        class,
                        to_bed,
                        roofed: tip.below_top > 0,
                        below: None,
                        rests: false,
                    });
                    if !squeezed && !roomy.contains(&tip.source) {
                        roomy.push(tip.source);
                    }
                }
            }
        }
        if layer == 0 {
            break;
        }

        // Each branch's place on the layer below.
        let here: Vec<(Point2, bool)> = layers[layer]
            .iter()
            .map(|node| (node.at, node.to_bed))
            .collect();
        let mut next: Vec<Node> = Vec::new();
        for index in 0..here.len() {
            let (at, to_bed) = here[index];
            let held = layers[layer][index].class;
            let roofed = layers[layer][index].roofed;
            let above = layers[layer][index].below_top;
            let below_top = above + 1;
            // How far the centre may go on this layer: the usual lean, and
            // as much again as the branch thickens by, which takes its far
            // side no further out (TreeSupport3D.cpp `increase_areas_one_layer`).
            let step = shape.slow + (shape.radius(below_top) - shape.radius(above)).max(0.0);
            // Towards the nearest branch of its kind, if the two can meet
            // before they run out of layers to lean over.
            let mate = here
                .iter()
                .enumerate()
                .filter(|(other, (_, kind))| *other != index && *kind == to_bed)
                .map(|(_, (other, _))| (mm(at, *other), *other))
                .filter(|(distance, _)| *distance <= 2.0 * shape.slow * layer as f64)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let drawn = mate.map_or(at, |(distance, other)| {
                let share = if distance <= 2.0 * step {
                    0.5
                } else {
                    step / distance
                };
                toward(at, other, share)
            });

            // A branch keeps clear of the part by its own radius. Where
            // there is no room for the radius it has grown to, it keeps
            // the clearance it had, and its outline is cut back to fit.
            // The bed is tried first; a branch that cannot get there goes
            // on down beside the part until it comes to rest on it.
            let wanted = shape.class_of(shape.radius(below_top)).max(held);
            let mut place: Option<(Point2, bool, usize)> = None;
            'place: for reaches in [true, false] {
                if !reaches && plate_only {
                    break;
                }
                for class in [wanted, held] {
                    let volume = volumes.class(class)?;
                    let blocked = if reaches {
                        &volume.avoidance[layer - 1]
                    } else {
                        &volume.collision[layer - 1]
                    };
                    let stuck = inside(blocked, at);
                    // Only a branch already bound for the bed may push its
                    // way out of what blocks the way there.
                    if !(reaches && stuck && !to_bed) {
                        let to = if !inside(blocked, drawn) {
                            Some(drawn)
                        } else if !stuck {
                            Some(at)
                        } else {
                            escape(blocked, at, 1.5 * step.max(shape.fast))
                        };
                        if let Some(to) = to {
                            place = Some((to, reaches, class));
                            break 'place;
                        }
                    }
                    if class == held {
                        break;
                    }
                }
            }
            let Some((mut to, reaches, class)) = place else {
                layers[layer][index].rests = true;
                continue;
            };
            // Short of room, it edges away from the part so that it can
            // thicken further down.
            if class < wanted {
                let volume = volumes.class(wanted)?;
                let blocked = if reaches {
                    &volume.avoidance[layer - 1]
                } else {
                    &volume.collision[layer - 1]
                };
                if let Some(out) = escape(blocked, at, f64::INFINITY) {
                    let edged = toward(at, out, (step / mm(at, out).max(1e-9)).min(1.0));
                    let volume = volumes.class(class)?;
                    let blocked = if reaches {
                        &volume.avoidance[layer - 1]
                    } else {
                        &volume.collision[layer - 1]
                    };
                    if !inside(blocked, edged) {
                        to = edged;
                    }
                }
            }
            // Branches that have come together go on as one.
            let joined = next
                .iter()
                .position(|node| node.to_bed == reaches && mm(node.at, to) <= shape.slow);
            let slot = match joined {
                Some(slot) => {
                    next[slot].below_top = next[slot].below_top.max(below_top);
                    next[slot].class = next[slot].class.min(class);
                    next[slot].roofed |= roofed;
                    slot
                }
                None => {
                    next.push(Node {
                        at: to,
                        below_top,
                        class,
                        to_bed: reaches,
                        roofed,
                        below: None,
                        rests: false,
                    });
                    next.len() - 1
                }
            };
            layers[layer][index].below = Some(slot);
        }
        layers[layer - 1] = next;
    }
    Ok(Grown { layers, planted })
}

/// What is known of a branch from what happens to it further down.
#[derive(Clone, Copy, Default)]
struct Fate {
    /// Whether it ends somewhere it can stand: the bed, or the part.
    stands: bool,
    reaches_bed: bool,
    /// Layers above where it comes to rest on the part.
    above_rest: Option<usize>,
}

pub(crate) fn build(
    job: &Job<'_>,
    overhangs: Vec<Overhang>,
) -> Result<Vec<Vec<ExtrusionPath>>, SupportError> {
    let settings = job.settings;
    let count = job.slices.len();
    let flow = settings.flow(job.layer_height, false)?;
    let shape = Shape::new(job, &flow);

    // An overhang on layer L is met by a contact on layer L - 1 - gap.
    let contact_of = |layer: usize| layer.checked_sub(1 + job.gap_above);
    let Some(top) = overhangs
        .iter()
        .enumerate()
        .filter(|(_, overhang)| !overhang.whole.is_empty())
        .filter_map(|(layer, _)| contact_of(layer))
        .max()
    else {
        return Ok(vec![Vec::new(); count]);
    };
    let keep_off: Vec<Vec<ExPolygon>> = (0..=top)
        .into_par_iter()
        .map(|layer| job.keep_off(layer))
        .collect::<Result<_, _>>()?;

    // Under each overhang, a roof of dense layers where there is room for
    // one, and tips under the roof.
    let mut roofs: Vec<Vec<(usize, ExPolygon)>> = vec![Vec::new(); top + 1];
    let mut tips: Vec<Vec<Tip>> = vec![Vec::new(); top + 1];
    // Overhangs whose roof has something under it; a roof that comes down
    // to the bed stands by itself.
    let mut held_up = vec![false; overhangs.len()];
    for (layer, overhang) in overhangs.iter().enumerate() {
        let Some(contact) = contact_of(layer).filter(|_| !overhang.whole.is_empty()) else {
            continue;
        };
        let mut roofed = 0;
        while roofed < settings.interface_top_layers && roofed <= contact {
            let at = contact - roofed;
            let area = edge_clip::difference(&overhang.whole, &keep_off[at])?;
            if area.iter().map(ExPolygon::area_mm2).sum::<f64>() < MIN_ROOF_AREA {
                break;
            }
            roofs[at].extend(area.into_iter().map(|region| (layer, region)));
            roofed += 1;
        }
        // A roof that goes all the way down to the bed needs no branches.
        let Some(at) = contact.checked_sub(roofed) else {
            held_up[layer] = true;
            continue;
        };
        let area = edge_clip::difference(&overhang.whole, &keep_off[at])?;
        // Without a roof every tip holds the part up by itself, so an
        // overhang too small for a roof gets its tips close together.
        let pitch = if roofed > 0 {
            shape.pitch
        } else {
            shape.pitch.min(shape.tip_radius.max(0.2) * 2.0)
        };
        let points = sample(&area, pitch, shape.radius(roofed))?;
        // No room for a branch because the part is right there: the roof
        // lies on the part, with the gap between them.
        if points.is_empty() && roofed > 0 && !settings.on_build_plate_only {
            held_up[layer] = true;
        }
        for point in points {
            tips[at].push(Tip {
                at: point,
                below_top: roofed,
                source: layer,
            });
        }
    }

    let mut volumes = Volumes {
        keep_off: &keep_off,
        radii: &shape.classes,
        // A branch commits to the bed only where it can get there at its
        // usual lean, as the reference prefers; otherwise it stands on
        // the part.
        lean: shape.slow,
        classes: shape.classes.iter().map(|_| None).collect(),
    };
    let Grown { layers, planted } =
        grow(&tips, &shape, &mut volumes, settings.on_build_plate_only)?;

    // From the bed up: which branches end somewhere they can stand.
    let mut fates: Vec<Vec<Fate>> = Vec::with_capacity(layers.len());
    for (layer, nodes) in layers.iter().enumerate() {
        let fate = nodes
            .iter()
            .map(|node| match node.below {
                _ if layer == 0 => Fate {
                    stands: true,
                    reaches_bed: true,
                    above_rest: None,
                },
                Some(below) => {
                    let lower: Fate = fates[layer - 1][below];
                    Fate {
                        above_rest: lower.above_rest.map(|layers| layers + 1),
                        ..lower
                    }
                }
                // Standing on the part, if it is tall enough to be worth it.
                None => Fate {
                    stands: node.rests && !settings.on_build_plate_only,
                    reaches_bed: false,
                    above_rest: Some(0),
                },
            })
            .collect();
        fates.push(fate);
    }
    // A stub on the part shorter than the least height is dropped whole,
    // unless it holds up a roof: height is counted from the tip, so the
    // test is made where it rests.
    for layer in 0..layers.len() {
        for index in 0..layers[layer].len() {
            let node = &layers[layer][index];
            if node.rests
                && node.below.is_none()
                && !node.roofed
                && node.below_top < shape.min_layers_on_part
            {
                fates[layer][index].stands = false;
            }
        }
    }
    for layer in 1..layers.len() {
        for index in 0..layers[layer].len() {
            if let Some(below) = layers[layer][index].below {
                fates[layer][index].stands = fates[layer - 1][below].stands;
            }
        }
    }

    // A roof with no branch under it that stands would be printed on
    // nothing, so it is not printed.
    for (layer, index, source) in planted {
        held_up[source] |= fates[layer][index].stands;
    }
    let roofs: Vec<Vec<ExPolygon>> = roofs
        .into_iter()
        .map(|layer| {
            layer
                .into_iter()
                .filter(|(source, _)| held_up[*source])
                .map(|(_, region)| region)
                .collect()
        })
        .collect();

    let mut supports: Vec<Vec<ExtrusionPath>> = (0..=top)
        .into_par_iter()
        .map(|layer| {
            let first = layer == 0;
            let flow = settings.flow(job.height_of(layer), first)?;
            let mut paths = Vec::new();
            // Branches as circles; those in the first layers above where
            // they rest on the part are dense instead of hollow.
            let (mut hollow, mut feet) = (Vec::new(), Vec::new());
            for (node, fate) in layers[layer].iter().zip(&fates[layer]) {
                if !fate.stands {
                    continue;
                }
                let radius = shape.radius(node.below_top);
                let radius = if fate.reaches_bed {
                    shape.foot_radius(radius, layer)
                } else {
                    radius
                };
                let dense = fate
                    .above_rest
                    .is_some_and(|layers| layers < settings.interface_bottom_layers);
                if dense { &mut feet } else { &mut hollow }.push(circle(node.at, radius));
            }
            let mut blocked = keep_off[layer].clone();
            blocked.extend(roofs[layer].iter().cloned());
            let blocked = edge_clip::union_ex(&blocked)?;
            let clear = |circles: &[ExPolygon]| -> Result<Vec<ExPolygon>, SupportError> {
                if circles.is_empty() {
                    return Ok(Vec::new());
                }
                Ok(edge_clip::difference(
                    &edge_clip::union_ex(circles)?,
                    &blocked,
                )?)
            };
            let feet = clear(&feet)?;
            let mut hollow = edge_clip::difference(&clear(&hollow)?, &feet)?;
            // Small gaps left where branches crowd together are closed up:
            // a wall round each would hold nothing.
            for island in &mut hollow {
                island
                    .holes
                    .retain(|hole| -hole.signed_area_mm2() > SMALL_GAP);
            }

            if first {
                // On the bed a trunk is filled in and outlined.
                walls(&hollow, &flow, false, &mut paths)?;
                let inner = edge_clip::offset(&hollow, -flow.scaled_width(), Join::MITER)?;
                fill(
                    &inner,
                    flow.spacing / settings.first_layer_density.clamp(0.1, 1.0),
                    settings.angle + 90.0,
                    Role::Support,
                    &flow,
                    false,
                    &mut paths,
                )?;
            } else {
                walls(&hollow, &flow, settings.tree_wall_count >= 2, &mut paths)?;
                if settings.tree_wall_count == 0 {
                    let wide: Vec<ExPolygon> = hollow
                        .iter()
                        .filter(|island| island.area_mm2() > DOUBLE_WALL_AREA)
                        .cloned()
                        .collect();
                    second_wall(&wide, &flow, &mut paths)?;
                }
            }
            fill(
                &feet,
                flow.spacing + settings.bottom_interface_spacing,
                settings.angle + 90.0,
                Role::SupportInterface,
                &flow,
                true,
                &mut paths,
            )?;
            fill(
                &edge_clip::union_ex(&roofs[layer])?,
                flow.spacing + settings.interface_spacing,
                settings.angle + 90.0,
                Role::SupportInterface,
                &flow,
                true,
                &mut paths,
            )?;
            Ok(paths)
        })
        .collect::<Result<_, SupportError>>()?;
    supports.resize(count, Vec::new());
    Ok(supports)
}

/// One wall round each island, and a second inside it when `double`.
fn walls(
    islands: &[ExPolygon],
    flow: &Flow,
    double: bool,
    paths: &mut Vec<ExtrusionPath>,
) -> Result<(), SupportError> {
    if double {
        second_wall(islands, flow, paths)?;
    }
    loops(islands, flow.scaled_width() / 2, flow, paths)
}

fn second_wall(
    islands: &[ExPolygon],
    flow: &Flow,
    paths: &mut Vec<ExtrusionPath>,
) -> Result<(), SupportError> {
    loops(islands, flow.scaled_width() * 3 / 2, flow, paths)
}

/// The outlines of some islands, `inset` in from their edges, as closed paths.
fn loops(
    islands: &[ExPolygon],
    inset: i64,
    flow: &Flow,
    paths: &mut Vec<ExtrusionPath>,
) -> Result<(), SupportError> {
    for island in edge_clip::offset(islands, -inset, Join::MITER)? {
        for ring in island.rings() {
            if ring.points.len() < 3 {
                continue;
            }
            let mut points = ring.points.clone();
            points.push(points[0]);
            paths.push(ExtrusionPath {
                points,
                role: Role::Support,
                flow: *flow,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SupportSettings;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    fn shape() -> Shape {
        let settings = SupportSettings {
            line_width: edge_config::FloatOrPercent::Value(0.42),
            ..SupportSettings::default()
        };
        let job = Job {
            slices: &[],
            first_layer_height: 0.2,
            layer_height: 0.2,
            settings: &settings,
            gap_above: 1,
            gap_below: 1,
        };
        Shape::new(&job, &settings.flow(0.2, false).unwrap())
    }

    #[test]
    fn a_branch_thickens_as_the_reference_has_it() {
        let shape = shape();
        // The reference's worked example at 0.2 mm layers and 0.42 mm lines.
        assert_eq!(shape.tip_layers, 5);
        assert!((shape.slow - 25.0_f64.to_radians().tan() * 0.2).abs() < 1e-12);
        assert!((shape.fast - 40.0_f64.to_radians().tan() * 0.2).abs() < 1e-12);
        assert_eq!(shape.foot_layers, 16);
        assert_eq!(shape.min_layers_on_part, 5);
        // From the tip's 0.4 mm to the branch's 1 mm over the tip layers,
        // then by the diameter angle.
        assert!((shape.radius(0) - 0.4).abs() < 1e-12);
        assert!((shape.radius(5) - 1.0).abs() < 1e-12);
        let grown = 1.0 + 100.0 * 5.0_f64.to_radians().tan() * 0.2;
        assert!((shape.radius(105) - grown).abs() < 1e-9);
        // A trunk flares towards 7.5 mm across at the bed; a tip does not.
        assert!(shape.foot_radius(1.0, 0) > 3.5 && shape.foot_radius(1.0, 0) <= 3.75);
        assert_eq!(shape.foot_radius(1.0, 16), 1.0);
        assert_eq!(shape.foot_radius(0.6, 0), 0.6);
        // Clearances are worked out for the tip, 1.5 mm, then half as much again.
        assert_eq!(shape.classes[..5], [0.0, 0.4, 1.5, 2.25, 3.375]);
        assert_eq!(
            (
                shape.class_of(0.4),
                shape.class_of(0.41),
                shape.class_of(99.0)
            ),
            (1, 2, shape.classes.len() - 1)
        );
    }

    #[test]
    fn tips_line_the_edge_and_cover_the_middle() {
        let area = [square(0.0, 0.0, 20.0, 10.0)];
        let tips = sample(&area, 2.5, 0.5).unwrap();
        assert!(tips.len() > 20, "{} tips", tips.len());
        // All inside, none closer together than half the pitch.
        for (index, tip) in tips.iter().enumerate() {
            assert!(inside(&area, *tip) || tips[..index].is_empty());
            assert!(tips[..index].iter().all(|other| mm(*other, *tip) >= 1.25));
        }
        // No part of the area is far from a tip.
        for (x, y) in [(1.0, 1.0), (19.0, 9.0), (10.0, 5.0), (19.0, 1.0)] {
            let nearest = tips
                .iter()
                .map(|tip| mm(*tip, Point2::from_mm(x, y)))
                .fold(f64::INFINITY, f64::min);
            assert!(nearest < 2.5, "{x} x {y} is {nearest} mm from a tip");
        }
        // An area too small for the grid still gets a tip.
        assert!(
            !sample(&[square(0.3, 0.3, 0.9, 0.9)], 2.5, 0.5)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_point_is_pushed_out_by_the_shortest_way_or_not_at_all() {
        let block = [square(0.0, 0.0, 10.0, 10.0)];
        let at = Point2::from_mm(9.0, 5.0);
        assert!(inside(&block, at) && !inside(&block, Point2::from_mm(11.0, 5.0)));
        let out = escape(&block, at, 2.0).unwrap();
        assert!(!inside(&block, out));
        assert!((unscaled(out.x) - 10.02).abs() < 1e-6 && out.y == at.y);
        // Too far to go in one step.
        assert_eq!(escape(&block, at, 0.5), None);
    }

    #[test]
    fn branches_lean_no_further_than_they_may_and_always_end_somewhere() {
        // A block beside the tips, which they have to keep clear of.
        let part = vec![vec![square(0.0, 0.0, 10.0, 10.0)]; 60];
        let slices: Vec<&[ExPolygon]> = part.iter().map(Vec::as_slice).collect();
        let settings = SupportSettings {
            enabled: true,
            ..SupportSettings::default()
        };
        let job = Job {
            slices: &slices,
            first_layer_height: 0.2,
            layer_height: 0.2,
            settings: &settings,
            gap_above: 1,
            gap_below: 1,
        };
        let shape = Shape::new(&job, &settings.flow(0.2, false).unwrap());
        let keep_off: Vec<Vec<ExPolygon>> =
            (0..50).map(|layer| job.keep_off(layer).unwrap()).collect();
        let mut volumes = Volumes {
            keep_off: &keep_off,
            radii: &shape.classes,
            lean: shape.slow,
            classes: shape.classes.iter().map(|_| None).collect(),
        };
        let mut tips = vec![Vec::new(); 50];
        for step in 0..8 {
            let along = f64::from(step) * 2.5;
            // A row close beside the block and a row further out.
            for x in [11.0, 14.0] {
                tips[49].push(Tip {
                    at: Point2::from_mm(x, along - 5.0),
                    below_top: 0,
                    source: 0,
                });
            }
        }
        let Grown { layers, planted } = grow(&tips, &shape, &mut volumes, true).unwrap();
        assert_eq!(planted.len(), 16);
        assert_eq!(layers[49].len(), 16);
        for layer in 1..50 {
            for node in &layers[layer] {
                // Every branch goes on to the layer below: none is lost.
                let below = &layers[layer - 1][node.below.expect("branch ends in mid-air")];
                let moved = mm(node.at, below.at);
                let most = 1.5 * shape.fast.max(shape.slow + 0.2);
                assert!(moved <= most + 1e-6, "moved {moved} mm on layer {layer}");
            }
        }
        // They join up on the way down, and none ends up inside the block's clearance.
        assert!(layers[0].len() < 8, "{} trunks", layers[0].len());
        for node in &layers[0] {
            let (x, y) = (unscaled(node.at.x), unscaled(node.at.y));
            assert!(!(-0.3..10.3).contains(&x) || !(-0.3..10.3).contains(&y));
        }
    }
}
