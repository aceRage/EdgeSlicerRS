use std::f64::consts::FRAC_PI_2;

use edge_clip::{ClipError, Join};
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::scaled;

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
}

/// How far a line's centre stops short of the area's edge, as a share of
/// the line spacing (FillRectilinear.cpp: 0.5 less `INFILL_OVERLAP_OVER_SPACING`
/// for single-direction fill, a full half spacing for crossing lines).
const LINES_INSET: f64 = 0.05;
const GRID_INSET: f64 = 0.5;
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
        // (flow, role, line spacing in mm, inset share, directions)
        let (flow, role, spacing, inset, angles): (Flow, Role, f64, f64, &[f64]) =
            match surface.kind {
                SurfaceKind::Sparse if params.density <= 0.0 => continue,
                SurfaceKind::Sparse => match params.pattern {
                    Pattern::Lines => (
                        params.sparse,
                        Role::SparseInfill,
                        params.sparse.spacing / params.density,
                        LINES_INSET,
                        &[params.sparse_angle + turned],
                    ),
                    // Two sets of lines share the density between them.
                    Pattern::Grid => (
                        params.sparse,
                        Role::SparseInfill,
                        2.0 * params.sparse.spacing / params.density,
                        GRID_INSET,
                        &[params.sparse_angle, params.sparse_angle + FRAC_PI_2],
                    ),
                },
                SurfaceKind::Top => (
                    params.top,
                    Role::TopSolidInfill,
                    params.top.spacing,
                    LINES_INSET,
                    &[params.solid_angle + turned],
                ),
                // From support to support where that is known.
                SurfaceKind::Bridge => (
                    params.bridge,
                    Role::BridgeInfill,
                    params.bridge_spacing,
                    LINES_INSET,
                    &[surface.angle.unwrap_or(params.solid_angle + turned)],
                ),
                // Across the sparse lines beneath: for a grid, along its
                // other set; for plain lines, square to the layer below's.
                SurfaceKind::InternalBridge => (
                    params.internal_bridge,
                    Role::InternalBridge,
                    params.internal_bridge_spacing,
                    LINES_INSET,
                    &[params.sparse_angle
                        + match params.pattern {
                            Pattern::Grid => 0.0,
                            Pattern::Lines => turned,
                        }],
                ),
                kind => {
                    let role = match kind {
                        SurfaceKind::Bottom => Role::BottomSurface,
                        _ => Role::SolidInfill,
                    };
                    (
                        params.solid,
                        role,
                        params.solid.spacing,
                        LINES_INSET,
                        &[params.solid_angle + turned],
                    )
                }
            };

        let inside = edge_clip::offset(
            std::slice::from_ref(&surface.area),
            -scaled(inset * flow.spacing),
            Join::MITER,
        )?;
        let max_link = match role {
            Role::SparseInfill => params.sparse_link_max,
            _ => SOLID_LINK_SPACINGS * flow.spacing,
        };
        // Solid fill that is not a bridge is fitted to the area: its
        // lines are spread, by up to a fifth, so that a whole number of
        // them fills the width with none left over, and each is made as
        // wide as its share (FillBase.cpp `_adjust_solid_spacing`).
        let fitted = matches!(
            surface.kind,
            SurfaceKind::Top | SurfaceKind::Bottom | SurfaceKind::InternalSolid
        );
        // All of a surface's directions are joined together, so a line can
        // link to a crossing one as readily as to its own neighbour.
        for area in &inside {
            let (mut pitch, mut first, mut flow) = (scaled(spacing) as f64, 0.0, flow);
            if fitted && let Some((low, high)) = lines::extent_across(area, angles[0]) {
                let intervals = ((high - low) / pitch).floor();
                if intervals >= 1.0 {
                    pitch = ((high - low) / intervals).min((1.2 * pitch + 0.5).floor());
                }
                first = low + pitch / 2.0;
                // A pitch no line can be made for leaves the fill as it was.
                match flow.with_spacing(pitch / edge_geom::UNITS_PER_MM) {
                    Ok(wider) => flow = wider,
                    Err(_) => (pitch, first) = (scaled(spacing) as f64, 0.0),
                }
            }
            // A stub shorter than half its own width is a blob, not a line.
            let min_length = flow.scaled_width() / 2;
            for points in
                lines::paths_from(area, pitch, first, angles, min_length, scaled(max_link))
            {
                paths.push(ExtrusionPath { points, role, flow });
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
        for pattern in [Pattern::Grid, Pattern::Lines] {
            let params = params(pattern, 0.25);
            let paths = fill(&[surface(SurfaceKind::Sparse, 60.0)], 0, &params).unwrap();
            let covered = length(&paths) * params.sparse.spacing;
            assert!(
                (covered / (0.25 * 3600.0) - 1.0).abs() < 0.03,
                "{pattern:?} covered {covered}"
            );
            // A grid has both directions on one layer; lines have one.
            assert_eq!(
                paths.iter().any(rises) && !paths.iter().all(rises),
                pattern == Pattern::Grid
            );
        }
        // Joining sparse lines along the walls makes fewer, longer paths.
        let separate = fill(
            &[surface(SurfaceKind::Sparse, 60.0)],
            0,
            &params(Pattern::Lines, 0.25),
        )
        .unwrap();
        let linked = FillParams {
            sparse_link_max: 20.0,
            ..params(Pattern::Lines, 0.25)
        };
        let joined = fill(&[surface(SurfaceKind::Sparse, 60.0)], 0, &linked).unwrap();
        assert!(
            joined.len() < separate.len() / 4,
            "{} paths joined from {}",
            joined.len(),
            separate.len()
        );
        assert!(length(&joined) > length(&separate));

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
        let ratio = length(&lines) / length(&plain);
        assert!((ratio - 1.0).abs() < 0.03, "{ratio}");
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
        let lines = fill(&[surface(SurfaceKind::InternalSolid, 10.0)], 0, &base).unwrap();
        let flow = lines[0].flow;
        let mut rows: Vec<f64> = lines
            .iter()
            .flat_map(|line| line.points.windows(2))
            .filter(|pair| pair[0].y == pair[1].y && pair[0].x != pair[1].x)
            .map(|pair| pair[0].y as f64 / 1e6)
            .collect();
        rows.sort_by(f64::total_cmp);
        rows.dedup();
        // The area is filled from 0.02 mm inside each edge: 24 lines fit
        // where 24.5 of the width set would, each a little wider for it.
        let (low, high) = (0.05 * base.solid.spacing, 10.0 - 0.05 * base.solid.spacing);
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
            ..surface(SurfaceKind::InternalSolid, 1.0)
        };
        let lines = fill(&[sliver], 0, &base).unwrap();
        assert!((lines[0].flow.spacing / base.solid.spacing - 1.2).abs() < 1e-3);
    }
}
