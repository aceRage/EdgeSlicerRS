use std::f64::consts::FRAC_PI_2;

use edge_clip::{ClipError, Join};
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::{Point2, scaled};

use crate::{Pattern, Surface, SurfaceKind, lines};

/// What filling one layer needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FillParams {
    /// Internal solid and bottom fill.
    pub solid: Flow,
    /// Fill laid over air, and the distance between its lines in mm.
    pub bridge: Flow,
    pub bridge_spacing: f64,
    /// Fill laid over sparse fill, and the distance between its lines.
    pub internal_bridge: Flow,
    pub internal_bridge_spacing: f64,
    /// The visible top fill.
    pub top: Flow,
    pub sparse: Flow,
    /// Sparse fill density from 0 to 1.
    pub density: f64,
    pub pattern: Pattern,
    /// Direction of sparse lines, radians counter-clockwise from +x.
    pub sparse_angle: f64,
    /// Direction of solid lines on even layers; odd layers are turned 90°.
    pub solid_angle: f64,
    /// Longest stretch of outline, in mm, that two sparse lines may be
    /// joined along; zero leaves sparse lines separate.
    pub sparse_link_max: f64,
    /// How far along the outline, in mm, a sparse line's end runs when it
    /// is joined to no other; zero leaves it bare.
    pub sparse_anchor: f64,
    /// A point the sparse pattern is laid through, in each of its
    /// directions: the middle of what is printed, as in the reference,
    /// so the pattern sits the same way in a part wherever the part is.
    pub origin: Point2,
    /// Whether the lines of a top surface, of a bottom surface and of
    /// solid fill inside the part are joined end to end. The reference's
    /// `monotonicline` pattern leaves them separate.
    pub top_joined: bool,
    pub bottom_joined: bool,
    pub solid_joined: bool,
    /// Whether each of those is swept from one side to the other so that
    /// no line is laid between two already down (the reference's
    /// `monotonic` and `monotonicline`; `rectilinear` is not).
    pub top_ordered: bool,
    pub bottom_ordered: bool,
    pub solid_ordered: bool,
}

/// How a surface's lines meet the edge of its area.
#[derive(Clone, Copy)]
enum Lay {
    /// Cut to the area drawn in by this share of the line spacing, and
    /// joined along that outline.
    Cut(f64),
    /// Sparse lines in one direction, as FillRectilinear.cpp
    /// `fill_surface_by_lines` lays them when they are not solid: through
    /// [`FillParams::origin`], every line joined to the next however far
    /// along the wall that is, turning half a spacing inside the area.
    Zigzag,
    /// Sparse lines as FillRectilinear.cpp `fill_surface_by_multilines`
    /// lays them: cut half a spacing inside the area, laid through
    /// [`FillParams::origin`], joined along that outline, and anchored
    /// where they are not.
    Anchored,
    /// Solid lines as `FillRectilinear.cpp` lays them: run to within
    /// `LINES_INSET` of the edge, and turned half a spacing inside it
    /// where they are joined. `fitted` spreads them to fill the width.
    /// `ordered` lays them in one sweep across the area, where no line
    /// goes down between two already laid.
    Turned {
        joined: bool,
        fitted: bool,
        ordered: bool,
    },
}

/// How far a line's centre stops short of the area's edge, as a share of
/// the line spacing (FillRectilinear.cpp: 0.5 less `INFILL_OVERLAP_OVER_SPACING`
/// for single-direction fill, a full half spacing for crossing lines).
const LINES_INSET: f64 = 0.05;
/// How much further in joined solid lines turn: to half a spacing from the
/// area's edge, so the turn itself lies inside the area and not on it.
const TURN_INSET: f64 = 0.45;
/// Solid lines are joined along the outline when it runs no further than
/// this many line spacings between them (Fill.cpp, `link_max_length`).
const SOLID_LINK_SPACINGS: f64 = 3.0;

/// Fill lines for one layer's surfaces, surface by surface.
pub fn fill(
    surfaces: &[Surface],
    layer_index: usize,
    params: &FillParams,
) -> Result<Vec<ExtrusionPath>, ClipError> {
    let turned = if layer_index % 2 == 1 { FRAC_PI_2 } else { 0.0 };
    let mut paths = Vec::new();
    for surface in surfaces {
        // (flow, role, line spacing in mm, how the lines end, directions)
        let (flow, role, spacing, lay, angles): (Flow, Role, f64, Lay, &[f64]) = match surface.kind
        {
            SurfaceKind::Sparse if params.density <= 0.0 => continue,
            SurfaceKind::Sparse => match params.pattern {
                Pattern::Lines => (
                    params.sparse,
                    Role::SparseInfill,
                    params.sparse.spacing / params.density,
                    Lay::Zigzag,
                    &[params.sparse_angle + turned],
                ),
                // Two sets of lines share the density between them.
                Pattern::Grid => (
                    params.sparse,
                    Role::SparseInfill,
                    2.0 * params.sparse.spacing / params.density,
                    Lay::Anchored,
                    &[params.sparse_angle, params.sparse_angle + FRAC_PI_2],
                ),
            },
            SurfaceKind::Top => (
                params.top,
                Role::TopSolidInfill,
                params.top.spacing,
                Lay::Turned {
                    joined: params.top_joined,
                    ordered: params.top_ordered,
                    fitted: true,
                },
                &[params.solid_angle + turned],
            ),
            // From support to support where that is known.
            SurfaceKind::Bridge => (
                params.bridge,
                Role::BridgeInfill,
                params.bridge_spacing,
                Lay::Cut(LINES_INSET),
                &[surface.angle.unwrap_or(params.solid_angle + turned)],
            ),
            // Across the sparse lines beneath: for a grid, along its
            // other set; for plain lines, square to the layer below's.
            SurfaceKind::InternalBridge => (
                params.internal_bridge,
                Role::InternalBridge,
                params.internal_bridge_spacing,
                Lay::Cut(LINES_INSET),
                &[params.sparse_angle
                    + match params.pattern {
                        Pattern::Grid => 0.0,
                        Pattern::Lines => turned,
                    }],
            ),
            // The reference's output has top and bottom lines spread to
            // fit and solid lines inside the part at their own pitch,
            // on every part compared; the source gives no reason seen.
            SurfaceKind::Bottom => (
                params.solid,
                Role::BottomSurface,
                params.solid.spacing,
                Lay::Turned {
                    joined: params.bottom_joined,
                    ordered: params.bottom_ordered,
                    fitted: true,
                },
                &[params.solid_angle + turned],
            ),
            SurfaceKind::InternalSolid => (
                params.solid,
                Role::SolidInfill,
                params.solid.spacing,
                Lay::Turned {
                    joined: params.solid_joined,
                    ordered: params.solid_ordered,
                    fitted: false,
                },
                &[params.solid_angle + turned],
            ),
        };

        match lay {
            Lay::Cut(inset) => {
                let inside = edge_clip::offset(
                    std::slice::from_ref(&surface.area),
                    -scaled(inset * flow.spacing),
                    Join::MITER,
                )?;
                let max_link = match role {
                    Role::SparseInfill => params.sparse_link_max,
                    _ => SOLID_LINK_SPACINGS * flow.spacing,
                };
                // A stub shorter than half its own width is a blob, not a line.
                let min_length = flow.scaled_width() / 2;
                // All of a surface's directions are joined together, so a
                // line can link to a crossing one as readily as to its
                // own neighbour.
                for area in &inside {
                    let spacing = scaled(spacing);
                    for points in lines::paths(area, spacing, angles, min_length, scaled(max_link))
                    {
                        paths.push(ExtrusionPath {
                            points,
                            role,
                            flow,
                            in_turn: false,
                        });
                    }
                }
            }
            Lay::Zigzag => {
                let outer = edge_clip::offset(
                    std::slice::from_ref(&surface.area),
                    -scaled(LINES_INSET * flow.spacing),
                    Join::MITER,
                )?;
                let inner =
                    edge_clip::offset(&outer, -scaled(TURN_INSET * flow.spacing), Join::MITER)?;
                let (sin, cos) = angles[0].sin_cos();
                let first = params.origin.y as f64 * cos - params.origin.x as f64 * sin;
                // No join is too long: sparse lines have no limit there
                // (Fill.cpp sets `link_max_length` only above 80% density).
                let unlimited = edge_geom::Coord::MAX / 4;
                for points in lines::paths_turned(
                    &outer,
                    &inner,
                    scaled(spacing) as f64,
                    first,
                    angles[0],
                    unlimited,
                    false,
                ) {
                    paths.push(ExtrusionPath {
                        points,
                        role,
                        flow,
                        in_turn: false,
                    });
                }
            }
            Lay::Anchored => {
                let line = flow.scaled_spacing();
                let inside =
                    edge_clip::offset(std::slice::from_ref(&surface.area), -line / 2, Join::MITER)?;
                // Nothing is joined when the profile allows no join at
                // all (`infill_anchor_max` of zero), nor anchored.
                let max_link = scaled(params.sparse_link_max);
                let joining = lines::Joining {
                    max_link,
                    anchor: if max_link > 0 {
                        scaled(params.sparse_anchor)
                    } else {
                        0
                    },
                    half_width: line / 2,
                    // FillBase.cpp `create_boundary_infill_graph`.
                    beside: (0.8 * line as f64) as i64,
                    beside_clip: (1.7 * line as f64) as i64,
                    order: lines::Order::Nearest,
                };
                // Each direction's lines pass through the origin. None
                // lies within a line's spacing of the area's own edge,
                // where it would only retrace the wall (`make_fill_lines`).
                let rows: Vec<lines::Rows> = angles
                    .iter()
                    .map(|angle| {
                        let (sin, cos) = angle.sin_cos();
                        let first = params.origin.y as f64 * cos - params.origin.x as f64 * sin;
                        let within = match lines::extent_across(&surface.area, *angle) {
                            Some((low, high)) => {
                                (low + line as f64 + 1.0, high - line as f64 - 1.0)
                            }
                            None => (f64::NEG_INFINITY, f64::INFINITY),
                        };
                        lines::Rows { first, within }
                    })
                    .collect();
                // A path shorter than half its own width is a blob, not a
                // line, and is left out; the reference keeps them.
                let least = flow.width / 2.0;
                for area in &inside {
                    for points in
                        lines::paths_anchored(area, scaled(spacing) as f64, &rows, angles, joining)
                    {
                        let path = ExtrusionPath {
                            points,
                            role,
                            flow,
                            in_turn: false,
                        };
                        if path.length_mm() >= least {
                            paths.push(path);
                        }
                    }
                }
            }
            Lay::Turned {
                joined,
                fitted,
                ordered,
            } => {
                let Some((low, high)) = lines::extent_across(&surface.area, angles[0]) else {
                    continue;
                };
                let outer = edge_clip::offset(
                    std::slice::from_ref(&surface.area),
                    -scaled(LINES_INSET * flow.spacing),
                    Join::MITER,
                )?;
                let inner =
                    edge_clip::offset(&outer, -scaled(TURN_INSET * flow.spacing), Join::MITER)?;
                // No join is too long. Fill.cpp means to stop at three
                // spacings above 80% density, but works that out from the
                // filler's spacing before it is set, while it is still
                // zero, and zero is no limit.
                let max_link = if joined { edge_geom::Coord::MAX / 4 } else { 0 };
                // Fitted lines are spread, by up to a fifth, so that a
                // whole number of them fills the area's width with none
                // left over, and each is made as wide as its share
                // (FillBase.cpp `_adjust_solid_spacing`).
                let (mut pitch, mut flow) = (scaled(spacing) as f64, flow);
                let intervals = ((high - low - 1.0) / pitch).floor();
                if fitted && intervals >= 1.0 {
                    let spread = ((high - low) / intervals).min((1.2 * pitch + 0.5).floor());
                    // A pitch no line can be made for leaves the fill as it was.
                    if let Ok(wider) = flow.with_spacing(spread / edge_geom::UNITS_PER_MM) {
                        (pitch, flow) = (spread, wider);
                    }
                }
                // The first line lies half a pitch in from the area's own
                // edge, not on a grid shared with other areas.
                let first = low + pitch / 2.0;
                // Swept lines are one run: each after the one before.
                let lines =
                    lines::paths_turned(&outer, &inner, pitch, first, angles[0], max_link, ordered);
                for (index, points) in lines.into_iter().enumerate() {
                    paths.push(ExtrusionPath {
                        points,
                        role,
                        flow,
                        in_turn: ordered && index > 0,
                    });
                }
            }
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use edge_geom::{ExPolygon, Point2, Polygon};

    use super::*;

    fn surface(kind: SurfaceKind, side: f64) -> Surface {
        let points = [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec();
        Surface {
            kind,
            area: ExPolygon {
                contour: Polygon { points },
                holes: Vec::new(),
            },
            angle: None,
        }
    }

    fn params(pattern: Pattern, density: f64) -> FillParams {
        let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
        let top = Flow::new(0.4, 0.2, 0.4).unwrap();
        FillParams {
            solid: flow,
            bridge: flow,
            bridge_spacing: flow.spacing,
            internal_bridge: flow.bridging(1.0, true),
            internal_bridge_spacing: 0.45,
            top,
            sparse: flow,
            density,
            pattern,
            sparse_angle: 45f64.to_radians(),
            solid_angle: 45f64.to_radians(),
            sparse_link_max: 0.0,
            sparse_anchor: 0.0,
            origin: Point2::from_mm(0.0, 0.0),
            top_joined: false,
            bottom_joined: true,
            solid_joined: true,
            top_ordered: true,
            bottom_ordered: true,
            solid_ordered: true,
        }
    }

    fn length(paths: &[ExtrusionPath]) -> f64 {
        paths.iter().map(ExtrusionPath::length_mm).sum()
    }

    /// Whether a path runs up to the right (+45°) or up to the left (135°).
    fn rises(path: &ExtrusionPath) -> bool {
        let (a, b) = (path.points[0], path.points[1]);
        ((b.x - a.x) > 0) == ((b.y - a.y) > 0)
    }

    #[test]
    fn solid_fill_lays_about_the_area_in_plastic() {
        let params = params(Pattern::Grid, 0.2);
        let paths = fill(&[surface(SurfaceKind::InternalSolid, 40.0)], 0, &params).unwrap();
        // Every line is as wide as the others, and no narrower than set.
        let flow = paths[0].flow;
        assert!(
            paths
                .iter()
                .all(|path| path.role == Role::SolidInfill && path.flow == flow)
        );
        assert!(flow.width >= params.solid.width && flow.width < 1.2 * params.solid.width);
        // Lines one spacing apart cover the area once; the short links
        // joining them along the edges add a little.
        let covered = length(&paths) * flow.spacing;
        assert!(
            (covered / 1600.0 - 1.0).abs() < 0.03,
            "covered {covered} of 1600 mm²"
        );
        // Joined, the whole square is a handful of zigzags, not a hundred lines.
        assert!(paths.len() <= 4, "{} separate paths", paths.len());
    }

    #[test]
    fn solid_lines_turn_ninety_degrees_each_layer() {
        let params = params(Pattern::Grid, 0.2);
        let even = fill(&[surface(SurfaceKind::InternalSolid, 20.0)], 0, &params).unwrap();
        let odd = fill(&[surface(SurfaceKind::InternalSolid, 20.0)], 1, &params).unwrap();
        assert!(even.iter().all(rises) && !odd.iter().any(rises));
    }

    #[test]
    fn roles_and_flows_follow_the_surface() {
        let params = params(Pattern::Grid, 0.2);
        let role = |kind| fill(&[surface(kind, 10.0)], 0, &params).unwrap()[0].clone();
        let top = role(SurfaceKind::Top);
        assert_eq!(top.role, Role::TopSolidInfill);
        // The top's own width, spread a little to fit the area.
        assert!(top.flow.width >= params.top.width && top.flow.width < params.top.width * 1.05);
        assert_eq!(role(SurfaceKind::Sparse).flow, params.sparse);
        assert_eq!(role(SurfaceKind::Bottom).role, Role::BottomSurface);
        assert_eq!(role(SurfaceKind::Bridge).role, Role::BridgeInfill);
        assert_eq!(role(SurfaceKind::Sparse).role, Role::SparseInfill);
    }

    #[test]
    fn sparse_patterns_lay_the_density_share_of_plastic() {
        // The lines themselves, leaving out the runs along the walls
        // that join or anchor them.
        let slanted = |paths: &[ExtrusionPath]| -> f64 {
            paths
                .iter()
                .flat_map(|path| path.points.windows(2))
                .filter(|pair| pair[0].x != pair[1].x && pair[0].y != pair[1].y)
                .map(|pair| ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64))
                .sum::<f64>()
                / 1e6
        };
        for pattern in [Pattern::Grid, Pattern::Lines] {
            let params = params(pattern, 0.25);
            let paths = fill(&[surface(SurfaceKind::Sparse, 60.0)], 0, &params).unwrap();
            let covered = slanted(&paths) * params.sparse.spacing;
            assert!(
                (covered / (0.25 * 3600.0) - 1.0).abs() < 0.03,
                "{pattern:?} covered {covered}"
            );
            // A grid has both directions on one layer; lines have one.
            let crossing = paths
                .iter()
                .flat_map(|path| path.points.windows(2))
                .filter(|pair| pair[0].x != pair[1].x && pair[0].y != pair[1].y)
                .map(|pair| ((pair[1].x - pair[0].x) > 0) == ((pair[1].y - pair[0].y) > 0))
                .collect::<Vec<bool>>();
            assert_eq!(
                crossing.iter().any(|up| *up) && !crossing.iter().all(|up| *up),
                pattern == Pattern::Grid
            );
        }
        // Lines in one direction are one zigzag, however far apart they are.
        let zigzag = fill(
            &[surface(SurfaceKind::Sparse, 60.0)],
            0,
            &params(Pattern::Lines, 0.25),
        )
        .unwrap();
        assert_eq!(zigzag.len(), 1);

        // A grid's lines are joined along the walls where that is allowed,
        // which makes fewer, longer paths; and an end left over is anchored.
        let grid = |sparse_link_max: f64, sparse_anchor: f64| {
            let params = FillParams {
                sparse_link_max,
                sparse_anchor,
                // Off the square's own lines of symmetry, so that the two
                // directions do not meet the walls at the same points.
                origin: Point2::from_mm(1.15, 0.0),
                ..params(Pattern::Grid, 0.25)
            };
            fill(&[surface(SurfaceKind::Sparse, 60.0)], 0, &params).unwrap()
        };
        let (separate, joined, anchored) = (grid(0.0, 0.0), grid(20.0, 0.0), grid(0.01, 1.5));
        assert!(
            joined.len() < separate.len() / 4,
            "{} paths joined from {}",
            joined.len(),
            separate.len()
        );
        assert!(length(&joined) > length(&separate));
        // Too far apart to join, each end runs a little way along the wall.
        assert!(anchored.len() > joined.len());
        assert!(length(&anchored) > length(&separate) + 0.5 * separate.len() as f64);

        // The pattern is laid through the origin: moved with the part, it
        // sits in the part as it did.
        let moved = |by: f64| {
            let params = FillParams {
                origin: Point2::from_mm(30.0 + by, 30.0 + by),
                ..params(Pattern::Grid, 0.25)
            };
            let mut surface = surface(SurfaceKind::Sparse, 60.0);
            for point in &mut surface.area.contour.points {
                *point = Point2::from_mm(point.x as f64 / 1e6 + by, point.y as f64 / 1e6 + by);
            }
            length(&fill(&[surface], 0, &params).unwrap())
        };
        assert!((moved(0.0) - moved(7.3)).abs() < 1e-3);

        let empty = params(Pattern::Grid, 0.0);
        assert!(
            fill(&[surface(SurfaceKind::Sparse, 60.0)], 0, &empty)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn bridges_are_laid_in_their_own_flow() {
        let base = params(Pattern::Grid, 0.2);
        // Over air: four fifths of the plastic, by a narrower line whose
        // spacing follows its width; at four fifths the density, that
        // comes to the same number of lines as the solid fill has.
        let thin = base.solid.bridging(0.8, false);
        assert!(thin.width < base.solid.width && thin.spacing < base.solid.spacing);
        assert!((thin.mm3_per_mm() / base.solid.mm3_per_mm() - 0.8).abs() < 1e-9);
        let over_air = FillParams {
            bridge: thin,
            bridge_spacing: thin.spacing / 0.8,
            ..base
        };
        let lines = fill(&[surface(SurfaceKind::Bridge, 20.0)], 0, &over_air).unwrap();
        assert!(
            lines
                .iter()
                .all(|line| line.role == Role::BridgeInfill && line.flow == thin)
        );
        let plain = fill(&[surface(SurfaceKind::InternalSolid, 20.0)], 0, &base).unwrap();
        // The solid lines turn inside their area, so they come a little shorter.
        let ratio = length(&lines) / length(&plain);
        assert!((ratio - 1.03).abs() < 0.04, "{ratio}");
        // A bridge with a direction of its own is laid along it.
        let turned = Surface {
            angle: Some(0.0),
            ..surface(SurfaceKind::Bridge, 20.0)
        };
        let lines = fill(&[turned], 0, &over_air).unwrap();
        let longest = lines
            .iter()
            .flat_map(|line| line.points.windows(2))
            .max_by_key(|pair| (pair[1].x - pair[0].x).abs() + (pair[1].y - pair[0].y).abs())
            .unwrap();
        assert_eq!(longest[0].y, longest[1].y, "along x");

        // Over sparse fill: round threads of the nozzle's bore, along the
        // grid's own direction so they cross its other lines.
        let lines = fill(&[surface(SurfaceKind::InternalBridge, 20.0)], 3, &base).unwrap();
        assert!(lines.iter().all(|line| line.role == Role::InternalBridge));
        let thread = lines[0].flow;
        assert_eq!((thread.width, thread.height), (0.4, 0.4));
        assert!((thread.mm3_per_mm() - std::f64::consts::PI * 0.04).abs() < 1e-9);
        let first = &lines[0].points;
        let (dx, dy) = (first[1].x - first[0].x, first[1].y - first[0].y);
        assert_eq!(dx.abs(), dy.abs(), "at 45 degrees");
    }

    #[test]
    fn solid_lines_are_spread_to_fill_the_width_exactly() {
        // Lines along x across a 10 mm square, 0.407 mm apart as set.
        let base = FillParams {
            solid_angle: 0.0,
            ..params(Pattern::Grid, 0.2)
        };
        let rows_of = |lines: &[ExtrusionPath]| {
            let mut rows: Vec<f64> = lines
                .iter()
                .flat_map(|line| line.points.windows(2))
                .filter(|pair| pair[0].y == pair[1].y && pair[0].x != pair[1].x)
                .map(|pair| pair[0].y as f64 / 1e6)
                .collect();
            rows.sort_by(f64::total_cmp);
            rows.dedup();
            rows
        };
        let lines = fill(&[surface(SurfaceKind::Bottom, 10.0)], 0, &base).unwrap();
        let flow = lines[0].flow;
        let rows = rows_of(&lines);
        // The fit is to the area's own width: 24 lines go where 24.6 of
        // the width set would, each a little wider for it.
        let (low, high) = (0.0, 10.0);
        assert_eq!(
            rows.len(),
            ((high - low) / base.solid.spacing).floor() as usize
        );
        let pitch = (high - low) / rows.len() as f64;
        assert!(
            (flow.spacing - pitch).abs() < 1e-6,
            "{} against {pitch}",
            flow.spacing
        );
        assert!(pitch > base.solid.spacing && pitch < 1.2 * base.solid.spacing);
        // The first and last lines lie half a pitch inside the area, so
        // nothing is left unfilled along either edge.
        assert!((rows[0] - (low + pitch / 2.0)).abs() < 1e-5);
        assert!((rows[rows.len() - 1] - (high - pitch / 2.0)).abs() < 1e-5);
        assert!(
            rows.windows(2)
                .all(|pair| (pair[1] - pair[0] - pitch).abs() < 1e-5)
        );

        // An area too narrow for the fit to hold is spread by a fifth and
        // no more.
        let sliver = Surface {
            area: ExPolygon {
                contour: Polygon {
                    points: [(0.0, 0.0), (10.0, 0.0), (10.0, 0.55), (0.0, 0.55)]
                        .map(|(x, y)| Point2::from_mm(x, y))
                        .to_vec(),
                },
                holes: Vec::new(),
            },
            ..surface(SurfaceKind::Bottom, 1.0)
        };
        let lines = fill(&[sliver], 0, &base).unwrap();
        assert!((lines[0].flow.spacing / base.solid.spacing - 1.2).abs() < 1e-3);

        // Solid fill inside the part keeps the pitch set, starting half
        // of it in from the area's edge.
        let lines = fill(&[surface(SurfaceKind::InternalSolid, 10.0)], 0, &base).unwrap();
        assert!(lines.iter().all(|line| line.flow == base.solid));
        let rows = rows_of(&lines);
        assert!((rows[0] - base.solid.spacing / 2.0).abs() < 1e-5);
        assert!(
            rows.windows(2)
                .all(|pair| (pair[1] - pair[0] - base.solid.spacing).abs() < 1e-5)
        );
    }

    #[test]
    fn joined_lines_turn_inside_and_separate_ones_run_edge_to_edge() {
        let base = FillParams {
            solid_angle: 0.0,
            ..params(Pattern::Grid, 0.2)
        };
        let xs = |lines: &[ExtrusionPath]| -> Vec<f64> {
            let mut xs: Vec<f64> = lines
                .iter()
                .flat_map(|line| &line.points)
                .map(|p| p.x as f64 / 1e6)
                .collect();
            xs.sort_by(f64::total_cmp);
            xs.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
            xs
        };
        // A top surface in separate lines: each runs to within a twentieth
        // of a spacing of both edges, and none is joined to the next.
        let top = fill(&[surface(SurfaceKind::Top, 10.0)], 0, &base).unwrap();
        assert!(top.len() > 20 && top.iter().all(|line| line.points.len() == 2));
        let reach = 0.05 * base.top.spacing;
        let ends = xs(&top);
        assert_eq!(ends.len(), 2);
        assert!((ends[0] - reach).abs() < 1e-5 && (ends[1] - (10.0 - reach)).abs() < 1e-5);

        // Swept in order, each line is laid after the one before it;
        // laid in any order, none is tied to another.
        assert!(!top[0].in_turn && top[1..].iter().all(|line| line.in_turn));
        let loose = FillParams {
            top_ordered: false,
            ..base
        };
        let any = fill(&[surface(SurfaceKind::Top, 10.0)], 0, &loose).unwrap();
        assert_eq!(any.len(), top.len());
        assert!(any.iter().all(|line| !line.in_turn));

        // Joined, the lines are one path that turns half a spacing inside
        // the edge; only its two loose ends reach as far as separate lines.
        let joined = FillParams {
            top_joined: true,
            ..base
        };
        let top = fill(&[surface(SurfaceKind::Top, 10.0)], 0, &joined).unwrap();
        assert_eq!(top.len(), 1);
        let turn = 0.5 * base.top.spacing;
        let points = &top[0].points;
        assert!(points[1..points.len() - 1].iter().all(|p| {
            let x = p.x as f64 / 1e6;
            (x - turn).abs() < 1e-5 || (x - (10.0 - turn)).abs() < 1e-5
        }));
        let loose = [points[0].x, points[points.len() - 1].x].map(|x| x as f64 / 1e6);
        assert!(
            loose
                .iter()
                .all(|x| (x - reach).abs() < 1e-5 || (x - (10.0 - reach)).abs() < 1e-5)
        );
    }
}
