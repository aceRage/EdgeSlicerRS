//! Where branches start (TreeSupport3D.cpp `generate_initial_areas` and
//! `sample_overhang_area`): under each overhang a roof of dense layers
//! while the overhang stays large enough for one, and tips laid out along
//! the lines a support fill would have there, a few millimetres apart, or
//! along the overhang's outline where it is too small for lines.

use std::f64::consts::{FRAC_PI_4, PI, TAU};

use edge_geom::{Coord, ExPolygon, Point2, Polygon, scaled};
use rayon::prelude::*;

use super::config::TreeConfig;
use super::element::{Element, State};
use super::geometry::{
    MITER, Polys, area, closed_line, diff, diff_safe, distance, intersection, offset,
    polyline_length, safe_offset_inc, union, union2,
};
use super::volumes::{Avoidance, Volumes};
use crate::SupportError;

/// Where a tip can go from where it is (`LineStatus`).
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    ToModel,
    ToModelGracious,
    ToModelGraciousSafe,
    ToBp,
    ToBpSafe,
}

type Line = Vec<(Point2, Status)>;

/// Roofs, by layer: the contact under the overhang and the dense layers
/// under that.
#[derive(Default)]
pub(crate) struct Roofs {
    pub contacts: Vec<Polys>,
    pub interfaces: Vec<Polys>,
}

impl Roofs {
    fn add(&mut self, polys: Polys, layer: usize, dtt_roof: usize) {
        let target = if dtt_roof == 0 {
            &mut self.contacts
        } else {
            &mut self.interfaces
        };
        if target.len() <= layer {
            target.resize(layer + 1, Vec::new());
        }
        target[layer].extend(polys);
    }
}

pub(crate) struct Tips {
    /// The tips on each layer.
    pub layers: Vec<Vec<Element>>,
    pub roofs: Roofs,
}

/// What one overhang island gives.
#[derive(Default)]
struct Placed {
    tips: Vec<Element>,
    roofs: Vec<(Polys, usize, usize)>,
}

struct Placer<'a> {
    volumes: &'a Volumes,
    config: &'a TreeConfig,
    support_angle: f64,
    min_xy_dist: bool,
    force_tip_to_roof: bool,
}

/// A small circle of `SUPPORT_TREE_CIRCLE_RESOLUTION` corners: the
/// influence area a tip starts with.
fn circle(centre: Point2, radius: Coord) -> ExPolygon {
    let corners = 25;
    ExPolygon {
        contour: Polygon {
            points: (0..corners)
                .map(|corner| {
                    let angle = TAU * f64::from(corner) / f64::from(corners);
                    Point2 {
                        x: centre.x + (radius as f64 * angle.cos()).round() as Coord,
                        y: centre.y + (radius as f64 * angle.sin()).round() as Coord,
                    }
                })
                .collect(),
        },
        holes: Vec::new(),
    }
}

impl Placer<'_> {
    /// `get_avoidance_status` for a tip's radius on a layer.
    fn status(&self, point: Point2, layer: usize) -> Option<Status> {
        let (v, c) = (self.volumes, self.config);
        let r = c.radius(0, 0.0);
        let min = self.min_xy_dist;
        let outside = |polys: &[ExPolygon]| !super::geometry::contains(polys, point);
        if outside(v.avoidance(r, layer, Avoidance::FastSafe, false, min)) {
            Some(Status::ToBpSafe)
        } else if outside(v.avoidance(r, layer, Avoidance::Fast, false, min)) {
            Some(Status::ToBp)
        } else if c.rests_on_model && outside(v.avoidance(r, layer, Avoidance::FastSafe, true, min))
        {
            Some(Status::ToModelGraciousSafe)
        } else if c.rests_on_model && outside(v.avoidance(r, layer, Avoidance::Fast, true, min)) {
            Some(Status::ToModelGracious)
        } else if c.rests_on_model && outside(v.collision(r, layer, min)) {
            Some(Status::ToModel)
        } else {
            None
        }
    }

    /// `convert_lines_to_internal`: each point with where it can go, a line
    /// broken where a point can go nowhere.
    fn classify(&self, polylines: &[Vec<Point2>], layer: usize) -> Vec<Line> {
        let mut out = Vec::new();
        for polyline in polylines {
            let mut line = Vec::new();
            for &point in polyline {
                match self.status(point, layer) {
                    Some(status) => line.push((point, status)),
                    None => {
                        if !line.is_empty() {
                            out.push(std::mem::take(&mut line));
                        }
                    }
                }
            }
            if !line.is_empty() {
                out.push(line);
            }
        }
        out
    }

    /// Whether a point still has somewhere to go a layer down
    /// (`evaluate_point_for_next_layer_function`).
    fn valid_below(&self, layer: usize, (point, status): (Point2, Status)) -> bool {
        let (v, c) = (self.volumes, self.config);
        let r = c.radius(0, 0.0);
        let min = self.min_xy_dist;
        let below = layer - 1;
        let kind = if status == Status::ToBpSafe {
            Avoidance::FastSafe
        } else {
            Avoidance::Fast
        };
        if !super::geometry::contains(v.avoidance(r, below, kind, false, min), point) {
            return true;
        }
        if c.rests_on_model && !matches!(status, Status::ToBp | Status::ToBpSafe) {
            let polys = match status {
                Status::ToModelGracious | Status::ToModelGraciousSafe => v.avoidance(
                    r,
                    below,
                    if status == Status::ToModelGraciousSafe {
                        Avoidance::FastSafe
                    } else {
                        Avoidance::Fast
                    },
                    true,
                    min,
                ),
                _ => v.collision(r, below, min),
            };
            return !super::geometry::contains(polys, point);
        }
        false
    }

    /// Lines a support fill would lay in `area` (`generate_support_infill_lines`):
    /// zig-zags across it, the roof's at its own spacing and turned 45
    /// degrees either way from layer to layer, the body's at
    /// `distance` apart.
    fn fill_lines(
        &self,
        area: &[ExPolygon],
        roof: bool,
        layer: usize,
        distance: Coord,
    ) -> Vec<Vec<Point2>> {
        let c = self.config;
        let spacing = if roof {
            c.roof_line_spacing
        } else {
            c.line_spacing
        };
        let (gap, angle) = if roof {
            let interface = self.support_angle + PI / 2.0;
            (
                c.roof_line_distance - c.roof_line_width,
                interface
                    + if layer & 1 == 1 {
                        -FRAC_PI_4
                    } else {
                        FRAC_PI_4
                    },
            )
        } else {
            (distance, self.support_angle)
        };
        let pitch = spacing + gap.max(0);
        let Ok(regions) = union(area) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for region in &regions {
            out.extend(edge_infill::lines::paths(
                region,
                pitch,
                &[angle],
                0,
                3 * pitch,
            ));
        }
        out
    }

    /// Takes tips off a line `distance` apart (`ensure_maximum_distance_polyline`).
    fn spaced(&self, input: &[Vec<Point2>], wanted: f64, min_points: usize) -> Vec<Vec<Point2>> {
        let mut result = Vec::new();
        for part in input {
            if part.is_empty() {
                continue;
            }
            let mut part = part.clone();
            let length = polyline_length(&part);
            let mut line: Vec<Point2> = Vec::new();
            let mut current_distance = wanted.max(scaled(0.1) as f64);
            if length < 2.0 * wanted && min_points <= 1 {
                // The point half way along.
                line.push(point_at(&part, length / 2.0));
            } else {
                let mut optimal_end = part.len() - 1;
                if part.first() == part.last() && part.len() > 2 {
                    // A closed outline: start and end at its two points
                    // furthest apart, which an overhang's thin ends are.
                    let (mut start, mut most) = (0, 0.0);
                    for i in 0..part.len() - 1 {
                        for j in 0..part.len() - 1 {
                            let d = distance_sq(part[i], part[j]);
                            if d > most {
                                (start, optimal_end, most) = (i, j, d);
                            }
                        }
                    }
                    let n = part.len() - 1;
                    part[..n].rotate_left(start);
                    part[n] = part[0];
                    optimal_end = (part.len() + optimal_end - start - 1) % (part.len() - 1);
                }
                while line.len() < min_points && current_distance >= scaled(0.1) as f64 {
                    line.clear();
                    let mut current_point = part[0];
                    line.push(part[0]);
                    if min_points > 1 || distance(part[0], part[optimal_end]) > current_distance {
                        line.push(part[optimal_end]);
                    }
                    let mut current_index = 0;
                    let mut next_distance = current_distance;
                    while let Some((next, index)) =
                        next_point_at_distance(&part, current_point, current_index, next_distance)
                    {
                        let nearest = line
                            .iter()
                            .map(|p| distance(*p, next))
                            .fold(f64::INFINITY, f64::min);
                        if nearest >= current_distance {
                            line.push(next);
                            current_point = next;
                            current_index = index;
                            next_distance = current_distance;
                        } else {
                            if current_point == next {
                                if next_distance > 2.0 * current_distance {
                                    break;
                                }
                                next_distance += current_distance;
                                continue;
                            }
                            next_distance = (current_distance - nearest).max(scaled(0.1) as f64);
                            current_point = next;
                            current_index = index;
                        }
                    }
                    current_distance *= 0.9;
                }
            }
            result.push(line);
        }
        result
    }

    fn tip(
        &self,
        (point, status): (Point2, Status),
        layer: usize,
        dont_move_until: usize,
        roof: bool,
        skip_ovalisation: bool,
        out: &mut Placed,
    ) {
        let to_bp = matches!(status, Status::ToBp | Status::ToBpSafe);
        if !self.config.rests_on_model && !to_bp {
            return;
        }
        let state = State {
            target_height: layer,
            target_position: point,
            next_position: point,
            layer,
            effective_radius_height: 0,
            distance_to_top: 0,
            result: Some(point),
            to_buildplate: to_bp,
            to_model_gracious: to_bp
                || matches!(
                    status,
                    Status::ToModelGracious | Status::ToModelGraciousSafe
                ),
            use_min_xy_dist: self.min_xy_dist,
            supports_roof: roof,
            dont_move_until,
            can_use_safe_radius: matches!(status, Status::ToBpSafe | Status::ToModelGraciousSafe),
            skip_ovalisation,
            ..State::default()
        };
        out.tips.push(Element {
            state,
            parents: Vec::new(),
            area: vec![circle(point, scaled(0.01))],
        });
    }

    /// `RichInterfacePlacer::add_points_along_lines`.
    fn add_points_along_lines(
        &self,
        mut lines: Vec<Line>,
        insert_layer: usize,
        roof_tip_layers: usize,
        supports_roof_layers: usize,
        dont_move_until: usize,
        out: &mut Placed,
    ) {
        let mut dtt_roof_tip = 0;
        // With tips wider than the smallest roof, the layers under a roof
        // are roofs too where the tips stay put.
        while dtt_roof_tip < roof_tip_layers && insert_layer > dtt_roof_tip {
            let this_layer = insert_layer - dtt_roof_tip;
            let (keep, free) = split(&lines, |p| self.valid_below(this_layer, p));
            lines = keep;
            for line in &free {
                for &point in line {
                    self.tip(
                        point,
                        this_layer,
                        roof_tip_layers - dtt_roof_tip,
                        dtt_roof_tip + supports_roof_layers > 0,
                        false,
                        out,
                    );
                }
            }
            let circles: Polys = lines
                .iter()
                .flatten()
                .map(|(p, _)| circle(*p, self.config.min_radius))
                .collect();
            if !circles.is_empty() {
                out.roofs
                    .push((circles, this_layer, dtt_roof_tip + supports_roof_layers));
            }
            dtt_roof_tip += 1;
        }
        let roof = dtt_roof_tip + supports_roof_layers > 0;
        for line in &lines {
            // A long line of tips a line apart stands for a fill of
            // support lines; it is not made oval.
            let disable = self.config.min_radius < 3 * self.config.line_width
                && roof_tip_layers == 0
                && dtt_roof_tip == 0
                && line.len() > 5;
            for &point in line {
                self.tip(
                    point,
                    insert_layer - dtt_roof_tip,
                    dont_move_until.saturating_sub(dtt_roof_tip),
                    roof,
                    disable,
                    out,
                );
            }
        }
    }

    /// `sample_overhang_area`.
    fn sample(
        &self,
        mut overhang: Polys,
        large_horizontal_roof: bool,
        layer: usize,
        roof_layers: usize,
        connect_length: Coord,
    ) -> Result<Placed, SupportError> {
        let (v, c) = (self.volumes, self.config);
        let mut out = Placed::default();
        let mut lines: Vec<Line> = Vec::new();
        let mut dtt_roof = 0;
        let mut generation_dtt = 0;
        if large_horizontal_roof {
            let mut added: Vec<Polys> = Vec::new();
            let mut last = overhang.clone();
            while dtt_roof < roof_layers && layer > dtt_roof {
                let below = layer - (dtt_roof + 1);
                let forbidden = if c.rests_on_model {
                    v.collision(c.radius(0, 0.0), below, self.min_xy_dist)
                } else {
                    v.avoidance(
                        c.radius(0, 0.0),
                        below,
                        Avoidance::Fast,
                        false,
                        self.min_xy_dist,
                    )
                };
                let forbidden = offset(&union(forbidden)?, scaled(0.005), MITER)?;
                let next = diff(&overhang, &forbidden)?;
                if area(&next) < c.minimum_roof_area {
                    // Too small for a roof a layer down: the tips go under
                    // the last roof, along its lines.
                    if dtt_roof > 0 {
                        let before = dtt_roof - 1;
                        let polylines = self.spaced(
                            &self.fill_lines(&last, true, layer - before, c.roof_line_distance),
                            connect_length as f64,
                            1,
                        );
                        let classified = self.classify(&polylines, layer - before);
                        lines = split(&classified, |p| self.valid_below(layer - before, p)).0;
                    }
                    break;
                }
                added.push(overhang.clone());
                last = std::mem::replace(&mut overhang, next);
                dtt_roof += 1;
            }
            generation_dtt = dtt_roof.max(1) - 1;
            // A roof too narrow for lines is no roof.
            if lines.is_empty()
                && dtt_roof != 0
                && self
                    .fill_lines(
                        &overhang,
                        true,
                        layer - generation_dtt,
                        c.roof_line_distance,
                    )
                    .is_empty()
            {
                for (index, roof) in added.iter().enumerate() {
                    if self
                        .fill_lines(roof, true, layer - index, c.roof_line_distance)
                        .is_empty()
                    {
                        dtt_roof = index;
                        generation_dtt = dtt_roof.max(1) - 1;
                        break;
                    }
                }
            }
            added.truncate(dtt_roof);
            for (index, roof) in added.into_iter().enumerate() {
                if !roof.is_empty() {
                    out.roofs.push((roof, layer - index, index));
                }
            }
        }

        if lines.is_empty() {
            let supports_roof = dtt_roof > 0;
            let continuous = !supports_roof && large_horizontal_roof;
            let mut polylines = self.spaced(
                &self.fill_lines(
                    &overhang,
                    supports_roof,
                    layer - generation_dtt,
                    if supports_roof {
                        c.roof_line_distance
                    } else {
                        c.branch_distance
                    },
                ),
                if continuous {
                    (c.min_radius / 2) as f64
                } else {
                    connect_length as f64
                },
                1,
            );
            let points: usize = polylines.iter().map(Vec::len).sum();
            let outline_length: f64 = overhang
                .iter()
                .flat_map(ExPolygon::rings)
                .map(|ring| ring.perimeter_mm() * 1e6)
                .sum();
            let min_points = ((outline_length / connect_length as f64) as usize).clamp(1, 3);
            if points <= min_points {
                // Too few: lay them along the overhang's outline instead,
                // pulled in where that still leaves it all held up.
                let reduced = offset(
                    &union(&overhang)?,
                    -(c.line_width as f64 / 2.2) as Coord,
                    MITER,
                )?;
                let use_reduced = !reduced.is_empty()
                    && area(&offset(
                        &diff(&overhang, &reduced)?,
                        c.line_width.max(connect_length),
                        MITER,
                    )?) < super::geometry::tiny_area();
                let source = if use_reduced { &reduced } else { &overhang };
                let outlines: Vec<Vec<Point2>> = source
                    .iter()
                    .flat_map(ExPolygon::rings)
                    .map(closed_line)
                    .collect();
                polylines = self.spaced(&outlines, connect_length as f64, min_points);
            }
            lines = self.classify(&polylines, layer - dtt_roof);
        }

        if dtt_roof >= layer && large_horizontal_roof {
            // The roof reaches down to the bed.
            out.roofs.push((overhang, 0, dtt_roof.min(c.roof_layers)));
        } else {
            let roof_enabled = roof_layers > 0;
            self.add_points_along_lines(
                lines,
                layer - dtt_roof,
                if self.force_tip_to_roof {
                    roof_layers - dtt_roof
                } else {
                    0
                },
                dtt_roof,
                if roof_enabled {
                    roof_layers - dtt_roof
                } else {
                    0
                },
                &mut out,
            );
        }
        Ok(out)
    }
}

fn distance_sq(a: Point2, b: Point2) -> f64 {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    dx * dx + dy * dy
}

fn point_at(polyline: &[Point2], mut along: f64) -> Point2 {
    for pair in polyline.windows(2) {
        let length = distance(pair[0], pair[1]);
        if length >= along && length > 0.0 {
            let t = along / length;
            return Point2 {
                x: pair[0].x + ((pair[1].x - pair[0].x) as f64 * t) as Coord,
                y: pair[0].y + ((pair[1].y - pair[0].y) as f64 * t) as Coord,
            };
        }
        along -= length;
    }
    *polyline.last().expect("not empty")
}

/// `polyline_sample_next_point_at_distance`: the next point of a polyline,
/// from segment `start_index` on, `dist` from `start`.
fn next_point_at_distance(
    polyline: &[Point2],
    start: Point2,
    start_index: usize,
    dist: f64,
) -> Option<(Point2, usize)> {
    let dist2 = dist * dist;
    let eps = scaled(0.01) as f64;
    for i in start_index + 1..polyline.len() {
        let p1 = polyline[i];
        if distance_sq(p1, start) >= dist2.floor() {
            let p0 = polyline[i - 1];
            let (vx, vy) = ((p1.x - p0.x) as f64, (p1.y - p0.y) as f64);
            let l2v = vx * vx + vy * vy;
            if l2v < eps * eps {
                let c = Point2 {
                    x: (p0.x + p1.x) / 2,
                    y: (p0.y + p1.y) / 2,
                };
                if (distance(start, c) - dist).abs() < eps {
                    return Some((c, i - 1));
                }
                continue;
            }
            let (px, py) = ((start.x - p0.x) as f64, (start.y - p0.y) as f64);
            let along = (px * vx + py * vy) / l2v;
            let (fx, fy) = (vx * along, vy * along);
            let (xf, yf) = (px - fx, py - fy);
            let l2_from_line = xf * xf + yf * yf;
            let mut l2_intersection = dist2 - l2_from_line;
            if l2_intersection > -(super::config::SCALED_EPSILON as f64) {
                l2_intersection = l2_intersection.max(0.0);
                if (vx - fx).powi(2) + (vy - fy).powi(2) >= l2_intersection {
                    let s = (l2_intersection / l2v).sqrt();
                    let p = Point2 {
                        x: p0.x + (fx + vx * s) as Coord,
                        y: p0.y + (fy + vy * s) as Coord,
                    };
                    return Some((p, i - 1));
                }
            }
        }
    }
    None
}

/// `split_lines`: the stretches of lines whose points pass, and those whose
/// points do not.
fn split(lines: &[Line], mut keep: impl FnMut((Point2, Status)) -> bool) -> (Vec<Line>, Vec<Line>) {
    let (mut kept, mut freed) = (Vec::new(), Vec::new());
    for line in lines {
        let mut current_keep = true;
        let mut run: Line = Vec::new();
        for &point in line {
            if keep(point) != current_keep {
                if !run.is_empty() {
                    if current_keep { &mut kept } else { &mut freed }
                        .push(std::mem::take(&mut run));
                }
                current_keep = !current_keep;
            }
            run.push(point);
        }
        if !run.is_empty() {
            if current_keep { &mut kept } else { &mut freed }.push(run);
        }
    }
    (kept, freed)
}

/// Roofs and tips under every overhang (`generate_initial_areas`).
pub(crate) fn place(
    volumes: &Volumes,
    config: &TreeConfig,
    overhangs: &[Polys],
    support_angle: f64,
) -> Result<Tips, SupportError> {
    let z_distance_delta = config.z_distance_top_layers + 1;
    let layer_count = overhangs.len();
    let num_support_layers = layer_count.saturating_sub(z_distance_delta);
    let min_xy_dist = config.xy_distance > config.xy_min_distance;
    let roof_layers = config.roof_layers;
    let roof_enabled = roof_layers > 0;
    let force_tip_to_roof = roof_enabled
        && (config.z_distance_top_layers == 0
            || (config.min_radius as f64).powi(2) * PI > config.minimum_roof_area);
    let connect_length = config.connect_length();
    let circle_length_to_half_linewidth_change = if config.min_radius < config.line_width {
        config.min_radius / 2
    } else {
        let r = config.min_radius as f64;
        let inner = (config.min_radius - config.line_width / 2) as f64;
        (r * r - inner * inner).sqrt() as Coord
    };
    let extra_outset = (config.min_radius - config.line_width / 2).max(0)
        + if min_xy_dist {
            config.line_width / 2
        } else {
            0
        };
    let placer = Placer {
        volumes,
        config,
        support_angle,
        min_xy_dist,
        force_tip_to_roof,
    };
    let raw: Vec<usize> = (1..num_support_layers)
        .filter(|layer| !overhangs[layer + z_distance_delta].is_empty())
        .collect();

    let placed: Vec<Vec<Placed>> = raw
        .par_iter()
        .map(|&layer| -> Result<Vec<Placed>, SupportError> {
            let overhang_raw = &overhangs[layer + z_distance_delta];
            let r0 = config.radius(0, 0.0);
            let forbidden = if config.rests_on_model {
                volumes.collision(r0, layer, min_xy_dist)
            } else {
                volumes.avoidance(r0, layer, Avoidance::Fast, false, min_xy_dist)
            };
            let forbidden = offset(&union(forbidden)?, scaled(0.005), MITER)?;
            let mut regular = diff(&union(overhang_raw)?, &forbidden)?;
            // Where the overhang is out of reach, tips are pulled in from
            // it, a little at a time, so that a wide tip still holds up
            // its edge.
            let mut remaining = intersection(
                &diff(
                    overhang_raw,
                    &offset(&regular, config.line_width / 2, MITER)?,
                )?,
                &forbidden,
            )?;
            let mut accumulated: Coord = 0;
            while !remaining.is_empty() && accumulated + config.line_width / 8 < extra_outset {
                let step = (if accumulated + 2 * config.line_width > config.min_radius {
                    config.line_width / 8
                } else {
                    circle_length_to_half_linewidth_change
                })
                .min(extra_outset - accumulated);
                accumulated += step;
                let raw_collision = volumes.collision(0, layer, true);
                let offset_step = config.xy_min_distance + config.line_width;
                remaining = diff(
                    &remaining,
                    &safe_offset_inc(
                        &regular,
                        (1.5 * accumulated as f64) as Coord,
                        raw_collision,
                        offset_step,
                        0,
                        1,
                    )?,
                )?;
                regular = union2(
                    &regular,
                    &diff(
                        &safe_offset_inc(
                            &remaining,
                            accumulated,
                            raw_collision,
                            offset_step,
                            0,
                            1,
                        )?,
                        &forbidden,
                    )?,
                )?;
            }
            let mut out = Vec::new();
            if roof_enabled {
                // Wide overhangs get a roof, its lowest layer on tips.
                let roofs = diff(&union(overhang_raw)?, &forbidden)?;
                regular = diff_safe(&regular, &roofs)?;
                for part in union(&roofs)? {
                    out.push(placer.sample(
                        vec![part],
                        true,
                        layer,
                        roof_layers,
                        connect_length,
                    )?);
                }
            }
            for part in union(&regular)? {
                out.push(placer.sample(vec![part], false, layer, roof_layers, connect_length)?);
            }
            Ok(out)
        })
        .collect::<Result<_, _>>()?;

    let mut layers: Vec<Vec<Element>> = vec![Vec::new(); num_support_layers.max(1)];
    let mut roofs = Roofs::default();
    for found in placed.into_iter().flatten() {
        for tip in found.tips {
            let at = tip.state.layer;
            layers[at].push(tip);
        }
        for (polys, layer, dtt) in found.roofs {
            roofs.add(polys, layer, dtt);
        }
    }
    // finalize_tips: in an order of their own, and of tips closer than a
    // tenth of a tip's radius only the first.
    let cell = ((config.min_radius + 1) / 10).max(1);
    for layer in &mut layers {
        if layer.len() < 2 {
            continue;
        }
        let key = |e: &Element| {
            let s = &e.state;
            (
                (s.target_position.x / cell, s.target_position.y / cell),
                (s.target_position.x, s.target_position.y),
                s.supports_roof,
                s.dont_move_until,
                s.to_buildplate,
                s.to_model_gracious,
                s.can_use_safe_radius,
                s.skip_ovalisation,
            )
        };
        layer.sort_by_key(key);
        let mut last = None;
        layer.retain(|e| {
            let cell = key(e).0;
            if last == Some(cell) {
                false
            } else {
                last = Some(cell);
                true
            }
        });
    }
    Ok(Tips { layers, roofs })
}
