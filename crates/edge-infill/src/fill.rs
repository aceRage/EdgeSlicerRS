use std::f64::consts::FRAC_PI_2;

use edge_clip::{ClipError, Join};
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::{ExPolygon, Point2, scaled};

use crate::{Pattern, Surface, SurfaceKind, curves, lines};

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
    /// The height of the top of the layer above the bed, in mm: patterns
    /// that change from layer to layer are laid by it.
    pub z: f64,
    /// How far each set of a lateral lattice leans, in radians.
    pub lattice_leans: (f64, f64),
    /// How far a cross zag's lines move sideways every second layer, mm.
    pub cross_step: f64,
    /// How steeply a lateral honeycomb's walls lean, in radians.
    pub overhang_lean: f64,
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
    /// Which solid fill has what its lines leave bare filled, and the
    /// shortest line worth laying there, in mm.
    pub gap_fill: GapFillTarget,
    pub gap_shortest: f64,
    /// How closely solid fill inside the part is laid on the second
    /// layer, from 0 to 1, and over how many layers from there it comes
    /// back to full (`elefant_foot_layers_density`,
    /// `elefant_foot_compensation_layers`): see [`Self::solid_density`].
    pub elephant_foot_density: f64,
    pub elephant_foot_layers: usize,
}

/// Where the fill's own gap fill goes (`gap_fill_target`): solid fill
/// leaves slivers its lines cannot reach, at the ends of lines against a
/// slanting wall and in corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapFillTarget {
    /// Top and bottom surfaces and solid fill inside the part.
    Everywhere,
    /// Top and bottom surfaces only.
    TopBottom,
    Nowhere,
}

impl FillParams {
    /// The sets of lines a sparse pattern of crossing lines is made of:
    /// each set's direction and how far it is moved sideways, in mm
    /// (FillRectilinear.cpp, the `SweepParams` each pattern hands to
    /// `fill_surface_by_multilines`). Empty for the patterns that are
    /// one set of lines.
    fn sweeps(&self) -> Vec<(f64, f64)> {
        let along = self.sparse_angle;
        let third = std::f64::consts::FRAC_PI_3;
        match self.pattern {
            Pattern::Lines
            | Pattern::AlignedLines
            | Pattern::ZigZag
            | Pattern::CrossZag
            | Pattern::CrossHatch
            | Pattern::Gyroid
            | Pattern::Honeycomb
            | Pattern::Honeycomb3D
            | Pattern::TpmsD
            | Pattern::Hilbert
            | Pattern::Archimedean
            | Pattern::Octagram => Vec::new(),
            // One set of lines for a third of the height of a cell, then
            // two that part and come together again over two thirds, the
            // next cell half a pitch along (`FillLateralHoneycomb`).
            Pattern::LateralHoneycomb => {
                let half = 0.5 * self.lateral_pitch();
                let cell = 3.0 * half / self.overhang_lean.tan();
                let thirds = 3.0 * self.z / cell;
                let whole = thirds as i64;
                let offset = if ((whole + 1) / 3) % 2 == 1 {
                    half
                } else {
                    0.0
                };
                if (whole + 1) % 3 != 0 {
                    vec![(along + FRAC_PI_2, offset)]
                } else {
                    let apart = (1.0 - (thirds - whole as f64)) * half;
                    vec![
                        (along + FRAC_PI_2, offset - apart),
                        (along + FRAC_PI_2, offset + apart),
                    ]
                }
            }
            // Both sets square to the fill's direction, each moved by
            // its lean times the height.
            Pattern::LateralLattice => vec![
                (along + FRAC_PI_2, self.lattice_leans.0.tan() * self.z),
                (along + FRAC_PI_2, self.lattice_leans.1.tan() * self.z),
            ],
            Pattern::Grid => vec![(along, 0.0), (along + FRAC_PI_2, 0.0)],
            Pattern::Triangles => {
                vec![
                    (along, 0.0),
                    (along + third, 0.0),
                    (along + 2.0 * third, 0.0),
                ]
            }
            Pattern::Stars => vec![
                (along, 0.0),
                (along + third, 0.0),
                (
                    along + 2.0 * third,
                    1.5 * self.sparse.spacing / self.density,
                ),
            ],
            Pattern::Cubic => {
                let shift = std::f64::consts::FRAC_1_SQRT_2 * self.z;
                vec![
                    (along, shift),
                    (along + third, -shift),
                    (along + 2.0 * third, shift),
                ]
            }
            Pattern::QuarterCubic => {
                // In scaled whole units, as the reference works it out.
                let line = scaled(self.sparse.spacing);
                let period = (scaled(self.sparse.spacing) as f64 / self.density) as i64 * 4;
                let shift = |part: f64| {
                    let raw = (std::f64::consts::FRAC_1_SQRT_2
                        * (scaled(self.z) as f64 + part * period as f64 * 2.0))
                        as i64
                        % period.max(1);
                    let shift = raw
                        .min(period - raw)
                        .min(period / 2 - line / 2)
                        .max(line / 2);
                    shift as f64 / edge_geom::UNITS_PER_MM
                };
                let (first, second) = (shift(0.0), shift(0.5));
                vec![
                    (along, first),
                    (along, -first),
                    (along + FRAC_PI_2, second),
                    (along + FRAC_PI_2, -second),
                ]
            }
        }
    }
}

impl FillParams {
    /// The direction a bridge over infill is laid in on a layer, in
    /// radians: across the sparse lines beneath. For a grid that is along
    /// its other set; for plain lines, square to the layer below's.
    pub fn internal_bridge_angle(&self, layer_index: usize) -> f64 {
        self.sparse_angle
            + match self.pattern {
                Pattern::Lines | Pattern::ZigZag | Pattern::CrossZag => {
                    if layer_index % 2 == 1 {
                        FRAC_PI_2
                    } else {
                        0.0
                    }
                }
                Pattern::AlignedLines => FRAC_PI_2,
                _ => 0.0,
            }
    }

    /// How closely solid fill inside the part is laid on a layer, from 0
    /// to 1 (Fill.cpp `Layer::make_fills`, "Elephant foot compensation
    /// for solid layers above the bottommost one by infill density
    /// manipulation"). The first layer is squeezed onto the bed and
    /// comes out wider than modelled; with less plastic in the layers
    /// resting on it, what is squeezed out has somewhere to go. The
    /// second layer gets the density set and each layer after it more,
    /// evenly, the last of `elephant_foot_layers` being the nearest to
    /// full; the first layer has no such fill, it is all bottom surface.
    ///
    /// The reference does this only for a part that stands on the bed,
    /// not on a raft. No raft is printed here, so every part does.
    pub fn solid_density(&self, layer_index: usize) -> f64 {
        let (density, layers) = (self.elephant_foot_density, self.elephant_foot_layers.max(1));
        // libslic3r's `is_approx`: within `EPSILON` of full is full.
        if (density - 1.0).abs() < 1e-4 || layer_index == 0 || layer_index > layers {
            return 1.0;
        }
        1.0 - (1.0 - density) * (layers - (layer_index - 1)) as f64 / layers as f64
    }

    /// The distance between a lateral honeycomb's lines where they run
    /// singly: its cells are two thirds single wall and a third double,
    /// so it is laid a third further apart than its density says.
    fn lateral_pitch(&self) -> f64 {
        (4.0 / 3.0) * self.sparse.spacing / self.density
    }
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
    /// A sparse pattern that is not straight rows: laid over the area,
    /// cut to it half a spacing in, and joined and anchored along that
    /// outline like the crossing lines.
    Curved,
    /// Solid lines as `FillRectilinear.cpp` lays them: run to within
    /// `LINES_INSET` of the edge, and turned half a spacing inside it
    /// where they are joined. `fitted` spreads them to fill the width.
    /// `ordered` lays them in one sweep across the area, where no line
    /// goes down between two already laid. A `density` under 1 lays them
    /// that much further apart, as sparse lines are placed: through
    /// [`FillParams::origin`], not from the area's own edge.
    Turned {
        joined: bool,
        fitted: bool,
        ordered: bool,
        density: f64,
    },
}

/// How far a line's centre stops short of the area's edge, as a share of
/// the line spacing (FillRectilinear.cpp: 0.5 less `INFILL_OVERLAP_OVER_SPACING`
/// for single-direction fill, a full half spacing for crossing lines).
pub(crate) const LINES_INSET: f64 = 0.05;
/// How much further in joined solid lines turn: to half a spacing from the
/// area's edge, so the turn itself lies inside the area and not on it.
pub(crate) const TURN_INSET: f64 = 0.45;
/// Solid lines are joined along the outline when it runs no further than
/// this many line spacings between them (Fill.cpp, `link_max_length`).
const SOLID_LINK_SPACINGS: f64 = 3.0;

/// Fill lines for one layer's surfaces, surface by surface.
pub fn fill(
    surfaces: &[Surface],
    layer_index: usize,
    params: &FillParams,
) -> Result<Vec<ExtrusionPath>, ClipError> {
    fill_from(surfaces, layer_index, params, &|_| Frame {
        origin: params.origin,
        bounds: None,
    })
}

/// The object a surface belongs to, as far as laying a sparse pattern in
/// it goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The middle of the object across the bed: the point its pattern is
    /// laid through.
    pub origin: Point2,
    /// The least and greatest corners of the object across the bed, for
    /// the patterns laid over the whole object and cut to each area.
    /// Without them such a pattern is laid over the area's own bounds.
    pub bounds: Option<(Point2, Point2)>,
}

/// As [`fill`], with the object each surface belongs to given by
/// `frame_of`: the reference lays every object's pattern through that
/// object's own middle, so the pattern sits the same way in a part
/// wherever the part is and whatever stands beside it.
pub fn fill_from(
    surfaces: &[Surface],
    layer_index: usize,
    params: &FillParams,
    frame_of: &dyn Fn(&Surface) -> Frame,
) -> Result<Vec<ExtrusionPath>, ClipError> {
    fill_within(surfaces, layer_index, params, frame_of, &[])
}

/// As [`fill_from`], knowing the layer's fill area without its overlap
/// into the walls (`no_overlap`): what a solid surface's lines leave bare
/// of it is then filled with lines of whatever width fits, where
/// [`FillParams::gap_fill`] asks (FillBase.cpp `_create_gap_fill`). With
/// no such area there is no such fill.
pub fn fill_within(
    surfaces: &[Surface],
    layer_index: usize,
    params: &FillParams,
    frame_of: &dyn Fn(&Surface) -> Frame,
    no_overlap: &[ExPolygon],
) -> Result<Vec<ExtrusionPath>, ClipError> {
    let turned = if layer_index % 2 == 1 { FRAC_PI_2 } else { 0.0 };
    let mut paths = Vec::new();
    // How sparse lines' ends are joined along the outline they are cut
    // to. Nothing is joined when the profile allows no join at all
    // (`infill_anchor_max` of zero), nor anchored.
    let sparse_joining = |line: edge_geom::Coord| {
        let max_link = scaled(params.sparse_link_max);
        lines::Joining {
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
        }
    };
    let sweeps = params.sweeps();
    let sweep_angles: Vec<f64> = sweeps.iter().map(|sweep| sweep.0).collect();
    for (number, surface) in surfaces.iter().enumerate() {
        // Each surface's lines are one part, laid together.
        let part = number as u32 + 1;
        let first_path = paths.len();
        let frame = frame_of(surface);
        let origin = frame.origin;
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
                Pattern::AlignedLines => (
                    params.sparse,
                    Role::SparseInfill,
                    params.sparse.spacing / params.density,
                    Lay::Zigzag,
                    &[params.sparse_angle],
                ),
                Pattern::ZigZag | Pattern::CrossZag => (
                    params.sparse,
                    Role::SparseInfill,
                    params.sparse.spacing / params.density,
                    Lay::Zigzag,
                    &[params.sparse_angle + turned],
                ),
                Pattern::CrossHatch
                | Pattern::Gyroid
                | Pattern::TpmsD
                | Pattern::Honeycomb3D
                | Pattern::Honeycomb
                | Pattern::Hilbert
                | Pattern::Archimedean
                | Pattern::Octagram => (
                    params.sparse,
                    Role::SparseInfill,
                    params.sparse.spacing / params.density,
                    Lay::Curved,
                    &[params.sparse_angle],
                ),
                // The sets of lines share the density between them.
                _ => (
                    params.sparse,
                    Role::SparseInfill,
                    if params.pattern == Pattern::LateralHoneycomb {
                        params.lateral_pitch()
                    } else {
                        sweeps.len() as f64 * params.sparse.spacing / params.density
                    },
                    Lay::Anchored,
                    &sweep_angles,
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
                    density: 1.0,
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
            // Across the sparse lines beneath, unless the surface has a
            // direction of its own: a second layer of bridge is laid
            // across the first.
            SurfaceKind::InternalBridge => (
                params.internal_bridge,
                Role::InternalBridge,
                params.internal_bridge_spacing,
                Lay::Cut(LINES_INSET),
                &[surface
                    .angle
                    .unwrap_or_else(|| params.internal_bridge_angle(layer_index))],
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
                    density: 1.0,
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
                    density: params.solid_density(layer_index),
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
                            part,
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
                // A cross zag's lines move a step every second layer,
                // one way on even layers and the other on odd (Fill.cpp,
                // `horiz_move`).
                let moved = if params.pattern == Pattern::CrossZag {
                    let steps = (layer_index / 2) as f64 * params.cross_step;
                    if layer_index.is_multiple_of(2) {
                        -steps
                    } else {
                        steps
                    }
                } else {
                    0.0
                };
                let first =
                    origin.y as f64 * cos - origin.x as f64 * sin + moved * edge_geom::UNITS_PER_MM;
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
                    // The zigzags keep their joins on the same side
                    // from layer to layer.
                    if matches!(params.pattern, Pattern::ZigZag | Pattern::CrossZag) {
                        lines::Order::Consistent
                    } else {
                        lines::Order::Sweep
                    },
                ) {
                    paths.push(ExtrusionPath {
                        points,
                        role,
                        flow,
                        in_turn: false,
                        part,
                    });
                }
            }
            Lay::Curved => {
                let line = flow.scaled_spacing();
                let inside =
                    edge_clip::offset(std::slice::from_ref(&surface.area), -line / 2, Join::MITER)?;
                // The reference turns the area so the pattern lies square
                // to the axes, about the middle of what is printed; the
                // gyroid a further 45° back (`CorrectionAngle`).
                // Honeycomb is laid without turning the area: it turns
                // its own lines.
                // The curves that fill the plane are a quarter turn on
                // (`_infill_direction`).
                let angle = match params.pattern {
                    Pattern::Gyroid | Pattern::TpmsD => angles[0] - std::f64::consts::FRAC_PI_4,
                    Pattern::Honeycomb => 0.0,
                    Pattern::Hilbert | Pattern::Archimedean | Pattern::Octagram => {
                        angles[0] + FRAC_PI_2
                    }
                    _ => angles[0],
                };
                let (sin, cos) = angle.sin_cos();
                let turned = |p: Point2| {
                    let (x, y) = ((p.x - origin.x) as f64, (p.y - origin.y) as f64);
                    Point2 {
                        x: (x * cos + y * sin).round() as i64,
                        y: (y * cos - x * sin).round() as i64,
                    }
                };
                let back = |p: Point2| {
                    let (x, y) = (p.x as f64, p.y as f64);
                    Point2 {
                        x: (x * cos - y * sin).round() as i64 + origin.x,
                        y: (x * sin + y * cos).round() as i64 + origin.y,
                    }
                };
                // A piece shorter than this is not worth laying.
                let least = 0.8 * flow.spacing;
                for area in &inside {
                    let area = edge_geom::ExPolygon {
                        contour: edge_geom::Polygon {
                            points: area.contour.points.iter().copied().map(turned).collect(),
                        },
                        holes: area
                            .holes
                            .iter()
                            .map(|hole| edge_geom::Polygon {
                                points: hole.points.iter().copied().map(turned).collect(),
                            })
                            .collect(),
                    };
                    let points = &area.contour.points;
                    let Some(&first) = points.first() else {
                        continue;
                    };
                    let bounds = points.iter().fold((first, first), |(low, high), p| {
                        (
                            Point2 {
                                x: low.x.min(p.x),
                                y: low.y.min(p.y),
                            },
                            Point2 {
                                x: high.x.max(p.x),
                                y: high.y.max(p.y),
                            },
                        )
                    });
                    let pattern = match params.pattern {
                        Pattern::Gyroid => {
                            curves::gyroid(bounds, flow.spacing, params.density, params.z)
                        }
                        Pattern::TpmsD => {
                            curves::tpms_d(bounds, flow.spacing, params.density, params.z)
                        }
                        Pattern::Honeycomb3D => curves::honeycomb_3d(
                            bounds,
                            flow.spacing,
                            params.density,
                            params.z,
                            flow.height,
                        ),
                        // A third of a turn on from each layer to the
                        // next, and a quarter on from the fill's
                        // direction (`_layer_angle`, `_infill_direction`).
                        Pattern::Honeycomb => curves::honeycomb(
                            bounds,
                            flow.spacing,
                            params.density,
                            angles[0]
                                + std::f64::consts::FRAC_PI_3 * (layer_index % 3) as f64
                                + FRAC_PI_2,
                        ),
                        Pattern::Hilbert | Pattern::Archimedean | Pattern::Octagram => {
                            // Laid over the whole object, turned as the
                            // area is.
                            let over = match frame.bounds {
                                Some((low, high)) => {
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
                                    .map(turned);
                                    corners.iter().fold(
                                        (corners[0], corners[0]),
                                        |(low, high), p| {
                                            (
                                                Point2 {
                                                    x: low.x.min(p.x),
                                                    y: low.y.min(p.y),
                                                },
                                                Point2 {
                                                    x: high.x.max(p.x),
                                                    y: high.y.max(p.y),
                                                },
                                            )
                                        },
                                    )
                                }
                                None => bounds,
                            };
                            let curve = match params.pattern {
                                Pattern::Hilbert => curves::Curve::Hilbert,
                                Pattern::Archimedean => curves::Curve::Archimedean,
                                _ => curves::Curve::Octagram,
                            };
                            curves::plane_path(curve, over, flow.spacing, params.density)
                        }
                        _ => curves::crosshatch(bounds, flow.spacing, params.density, params.z),
                    };
                    // Honeycomb's pieces are all kept; the others drop
                    // the scraps.
                    let least = match params.pattern {
                        Pattern::CrossHatch
                        | Pattern::Gyroid
                        | Pattern::TpmsD
                        | Pattern::Honeycomb3D => least,
                        _ => 0.0,
                    };
                    let mut pieces = edge_clip::clip_lines(&pattern, std::slice::from_ref(&area))?;
                    pieces.retain(|piece| {
                        let length: f64 = piece
                            .windows(2)
                            .map(|pair| {
                                ((pair[1].x - pair[0].x) as f64)
                                    .hypot((pair[1].y - pair[0].y) as f64)
                            })
                            .sum();
                        length >= scaled(least) as f64
                    });
                    // The plane-filling curves are joined only up to half
                    // density (FillPlanePath.cpp).
                    let joining = match params.pattern {
                        Pattern::Hilbert | Pattern::Archimedean | Pattern::Octagram
                            if params.density > 0.5 =>
                        {
                            lines::Joining::default()
                        }
                        _ => sparse_joining(line),
                    };
                    for points in lines::paths_joined(&area, pieces, joining) {
                        paths.push(ExtrusionPath {
                            points: points.into_iter().map(back).collect(),
                            role,
                            flow,
                            in_turn: false,
                            part,
                        });
                    }
                }
            }
            Lay::Anchored => {
                let line = flow.scaled_spacing();
                let inside =
                    edge_clip::offset(std::slice::from_ref(&surface.area), -line / 2, Join::MITER)?;
                let joining = sparse_joining(line);
                // Each direction's lines pass through the origin. None
                // lies within a line's spacing of the area's own edge,
                // where it would only retrace the wall (`make_fill_lines`).
                let rows: Vec<lines::Rows> = sweeps
                    .iter()
                    .map(|(angle, shift)| {
                        let (sin, cos) = angle.sin_cos();
                        // A set moved sideways is moved to the right of
                        // the way its lines run (`make_fill_lines`).
                        let first = origin.y as f64 * cos
                            - origin.x as f64 * sin
                            - shift * edge_geom::UNITS_PER_MM;
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
                            part,
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
                density,
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
                // Lines laid thinner than solid keep their width and
                // are that much further apart, the distance cut to a
                // whole number of units as FillRectilinear.cpp
                // `fill_surface_by_lines` cuts it. They are turned and
                // joined where solid lines are: the outlines they run
                // to are drawn in by the line's own spacing there too.
                let thinned = density < 1.0;
                if thinned {
                    pitch = (spacing * edge_geom::UNITS_PER_MM / density).trunc();
                }
                let intervals = ((high - low - 1.0) / pitch).floor();
                if fitted && !thinned && intervals >= 1.0 {
                    let spread = ((high - low) / intervals).min((1.2 * pitch + 0.5).floor());
                    // A pitch no line can be made for leaves the fill as it was.
                    if let Ok(wider) = flow.with_spacing(spread / edge_geom::UNITS_PER_MM) {
                        (pitch, flow) = (spread, wider);
                    }
                }
                // The first line lies half a pitch in from the area's own
                // edge, not on a grid shared with other areas. Thinned
                // lines are on the grid through the middle of the object,
                // which is where the reference puts any fill that is not
                // solid (`align_to_grid` about `_infill_direction`'s point).
                let first = if thinned {
                    let (sin, cos) = angles[0].sin_cos();
                    origin.y as f64 * cos - origin.x as f64 * sin
                } else {
                    low + pitch / 2.0
                };
                // Swept lines are one run: each after the one before.
                let lines = lines::paths_turned(
                    &outer,
                    &inner,
                    pitch,
                    first,
                    angles[0],
                    max_link,
                    if ordered {
                        lines::Order::Monotonic
                    } else {
                        lines::Order::Sweep
                    },
                );
                for (index, points) in lines.into_iter().enumerate() {
                    paths.push(ExtrusionPath {
                        points,
                        role,
                        flow,
                        in_turn: ordered && index > 0,
                        part,
                    });
                }
            }
        }
        let wanted = match (params.gap_fill, surface.kind) {
            (GapFillTarget::Nowhere, _) => false,
            // Fill laid thinner than solid has no gaps filled
            // (FillBase.cpp `_create_gap_fill`, `params.density >= 1`).
            (_, SurfaceKind::InternalSolid) if params.solid_density(layer_index) < 1.0 => false,
            (_, SurfaceKind::Top | SurfaceKind::Bottom) => true,
            (GapFillTarget::Everywhere, SurfaceKind::InternalSolid) => true,
            _ => false,
        };
        if wanted && !no_overlap.is_empty() {
            let gaps = gap_fill(
                surface,
                &paths[first_path..],
                &flow,
                params,
                no_overlap,
                part,
            )?;
            paths.extend(gaps);
        }
    }
    Ok(paths)
}

/// Added to an offset so that edges meant to coincide overlap instead
/// (ClipperUtils.hpp `ClipperSafetyOffset`).
const SAFETY_OFFSET: edge_geom::Coord = 10;
/// How much of its spacing a line may lose where it is squeezed
/// (libslic3r.h `INSET_OVERLAP_TOLERANCE`).
const INSET_OVERLAP_TOLERANCE: f64 = 0.4;

/// Lines down the middle of what `laid`, a solid surface's own lines,
/// leave bare of the surface's share of `no_overlap`, each as wide as the
/// gap it fills (FillBase.cpp `_create_gap_fill`).
///
/// Different from the reference in where they come in the order: there,
/// the gap lines of a surface swept from side to side are slipped in
/// among its lines wherever that saves travel (`insert_gap_fill`); here
/// they follow the surface's lines as a group of their own.
fn gap_fill(
    surface: &Surface,
    laid: &[ExtrusionPath],
    flow: &Flow,
    params: &FillParams,
    no_overlap: &[ExPolygon],
    part: u32,
) -> Result<Vec<ExtrusionPath>, ClipError> {
    // What the lines cover, each by its spacing and a little over.
    let mut covered = Vec::new();
    for path in laid {
        covered.extend(edge_clip::thicken(
            [path.points.as_slice()],
            path.flow.scaled_spacing() / 2 + SAFETY_OFFSET,
        )?);
    }
    let own = edge_clip::intersection(
        no_overlap,
        &edge_clip::offset(
            std::slice::from_ref(&surface.area),
            SAFETY_OFFSET,
            Join::MITER,
        )?,
    )?;
    let bare = edge_clip::difference(&own, &edge_clip::union_ex(&covered)?)?;
    if bare.is_empty() {
        return Ok(Vec::new());
    }
    let spacing = flow.scaled_spacing() as f64;
    let thinnest = 0.2 * spacing * (1.0 - INSET_OVERLAP_TOLERANCE);
    let widest = 2.0 * spacing;
    // Without the slivers too thin to print and whatever a line of the
    // fill itself could have had.
    let gaps = edge_clip::difference(
        &edge_clip::opening(&bare, (thinnest / 2.0) as edge_geom::Coord, Join::MITER)?,
        &edge_clip::offset2(
            &bare,
            -((widest / 2.0) as edge_geom::Coord),
            (widest / 2.0) as edge_geom::Coord + SAFETY_OFFSET,
            Join::MITER,
        )?,
    )?;
    let mut lines = Vec::new();
    for gap in &gaps {
        // A tenth of the reference's `SCALED_RESOLUTION`.
        for gap in edge_clip::simplify(std::slice::from_ref(gap), 1_250)? {
            // A gap whose middle cannot be found is left bare.
            if let Ok(found) = edge_medial::medial_axis(&gap, thinnest, widest) {
                lines.extend(found);
            }
        }
    }
    lines.retain(|line| line.length() >= scaled(params.gap_shortest) as f64);
    let mut made = edge_medial::variable_width(&lines, Role::GapFill, flow);
    for path in &mut made {
        path.part = part;
    }
    Ok(made)
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
            z: 0.2,
            lattice_leans: (-FRAC_PI_2 / 2.0, FRAC_PI_2 / 2.0),
            cross_step: 0.4,
            overhang_lean: 60f64.to_radians(),
            top_joined: false,
            bottom_joined: true,
            solid_joined: true,
            top_ordered: true,
            bottom_ordered: true,
            solid_ordered: true,
            gap_fill: GapFillTarget::Nowhere,
            gap_shortest: 0.0,
            elephant_foot_density: 1.0,
            elephant_foot_layers: 1,
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
        // Every pattern lays the share of plastic the density asks for,
        // on whatever layer: counted whole here, joins and all, which a
        // large area keeps small beside the lines.
        for pattern in [
            Pattern::AlignedLines,
            Pattern::Triangles,
            Pattern::Stars,
            Pattern::Cubic,
            Pattern::QuarterCubic,
            Pattern::CrossHatch,
            Pattern::Gyroid,
        ] {
            for layer in [0, 1, 7, 12] {
                let params = FillParams {
                    z: 0.2 * (layer + 1) as f64,
                    ..params(pattern, 0.2)
                };
                let paths = fill(&[surface(SurfaceKind::Sparse, 100.0)], layer, &params).unwrap();
                let covered = length(&paths) * params.sparse.spacing;
                // Cross hatch is laid 8% further apart than its density
                // says, and on the layers where it turns its zigzags
                // are longer than straight lines, as in the reference.
                let (least, most) = match pattern {
                    Pattern::CrossHatch => (0.9 / 1.08, 1.45 / 1.08),
                    _ => (0.88, 1.12),
                };
                let share = covered / (0.2 * 10_000.0);
                assert!(
                    (least..=most).contains(&share),
                    "{pattern:?} on layer {layer} covered {share} of what its density asks"
                );
                assert!(paths.iter().all(|path| path.role == Role::SparseInfill));
            }
        }
        // Lines that keep their direction lie the same way on every
        // layer; plain lines turn.
        let ways = |pattern: Pattern, layer: usize| -> Vec<bool> {
            let paths = fill(
                &[surface(SurfaceKind::Sparse, 60.0)],
                layer,
                &params(pattern, 0.25),
            )
            .unwrap();
            paths
                .iter()
                .flat_map(|path| path.points.windows(2))
                .filter(|pair| pair[0].x != pair[1].x && pair[0].y != pair[1].y)
                .map(|pair| ((pair[1].x - pair[0].x) > 0) == ((pair[1].y - pair[0].y) > 0))
                .collect()
        };
        assert_eq!(
            ways(Pattern::AlignedLines, 0),
            ways(Pattern::AlignedLines, 1)
        );
        assert_ne!(ways(Pattern::Lines, 0)[0], ways(Pattern::Lines, 1)[0]);
        // Cubic lines move sideways from layer to layer; triangles stay.
        let at = |pattern: Pattern, z: f64| {
            let params = FillParams {
                z,
                ..params(pattern, 0.2)
            };
            fill(&[surface(SurfaceKind::Sparse, 60.0)], 0, &params).unwrap()
        };
        assert_eq!(at(Pattern::Triangles, 0.2), at(Pattern::Triangles, 3.0));
        assert_ne!(at(Pattern::Cubic, 0.2), at(Pattern::Cubic, 3.0));
        assert_ne!(at(Pattern::Gyroid, 0.2), at(Pattern::Gyroid, 3.0));

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
    fn the_other_sparse_patterns_lay_their_share_too() {
        // (pattern, least and most of what the density asks, over these
        // layers.) Honeycomb's walls are two lines side by side for a
        // third of their length, 3D honeycomb's zigzags open and close,
        // and a lateral honeycomb's lines are one set where its walls
        // are upright and two where they part: all as in the reference.
        for (pattern, least, most) in [
            (Pattern::Honeycomb, 1.2, 1.32),
            (Pattern::Honeycomb3D, 0.75, 1.05),
            (Pattern::LateralLattice, 0.95, 1.05),
            (Pattern::LateralHoneycomb, 0.7, 1.55),
            (Pattern::ZigZag, 0.97, 1.08),
            (Pattern::CrossZag, 0.97, 1.08),
            (Pattern::Hilbert, 0.95, 1.05),
            (Pattern::Archimedean, 0.95, 1.05),
            (Pattern::Octagram, 0.95, 1.05),
            (Pattern::TpmsD, 0.88, 1.16),
        ] {
            for layer in [0usize, 1, 2, 3, 7, 12, 20, 33] {
                let params = FillParams {
                    z: 0.2 * (layer + 1) as f64,
                    ..params(pattern, 0.2)
                };
                let paths = fill(&[surface(SurfaceKind::Sparse, 100.0)], layer, &params).unwrap();
                let share = length(&paths) * params.sparse.spacing / (0.2 * 10_000.0);
                assert!(
                    (least..=most).contains(&share),
                    "{pattern:?} on layer {layer} covered {share} of what its density asks"
                );
                assert!(paths.iter().all(|path| path.role == Role::SparseInfill));
                // Nothing is laid outside the area.
                assert!(paths.iter().flat_map(|path| &path.points).all(|p| {
                    (0..=100_000_000).contains(&p.x) && (0..=100_000_000).contains(&p.y)
                }));
            }
        }
        let at = |pattern: Pattern, layer: usize| {
            let params = FillParams {
                z: 0.2 * (layer + 1) as f64,
                ..params(pattern, 0.2)
            };
            fill(&[surface(SurfaceKind::Sparse, 60.0)], layer, &params).unwrap()
        };
        // A zigzag lies the same on every second layer, turning between;
        // a cross zag has moved on by then; honeycomb comes round every
        // third.
        assert_eq!(at(Pattern::ZigZag, 1), at(Pattern::ZigZag, 5));
        assert_ne!(at(Pattern::ZigZag, 1), at(Pattern::ZigZag, 2));
        assert_ne!(at(Pattern::CrossZag, 1), at(Pattern::CrossZag, 5));
        assert_ne!(at(Pattern::Honeycomb, 0), at(Pattern::Honeycomb, 1));
        assert_eq!(at(Pattern::Honeycomb, 0), at(Pattern::Honeycomb, 3));
        // The curves that fill a plane stay put; the rest change with height.
        for pattern in [Pattern::Hilbert, Pattern::Archimedean, Pattern::Octagram] {
            assert_eq!(at(pattern, 0), at(pattern, 9), "{pattern:?}");
        }
        for pattern in [
            Pattern::Honeycomb3D,
            Pattern::LateralLattice,
            Pattern::LateralHoneycomb,
            Pattern::TpmsD,
        ] {
            assert_ne!(at(pattern, 0), at(pattern, 9), "{pattern:?}");
        }
    }

    #[test]
    fn a_curve_over_the_plane_is_laid_over_the_whole_object() {
        // A 20 mm square in the corner of an object 100 mm across: told
        // of the object, the curve is the object's, cut to the square.
        let mut square = surface(SurfaceKind::Sparse, 20.0);
        for point in &mut square.area.contour.points {
            *point = Point2 {
                x: point.x + 70_000_000,
                y: point.y + 70_000_000,
            };
        }
        let params = params(Pattern::Archimedean, 0.2);
        let origin = Point2::from_mm(50.0, 50.0);
        let laid = |bounds| {
            fill_from(std::slice::from_ref(&square), 0, &params, &|_| Frame {
                origin,
                bounds,
            })
            .unwrap()
        };
        let own = laid(None);
        let whole = laid(Some((
            Point2::from_mm(0.0, 0.0),
            Point2::from_mm(100.0, 100.0),
        )));
        assert_ne!(own, whole);
        // Every piece of it is an arc about the object's middle, not
        // the square's, and about as much is laid either way.
        let far = |path: &ExtrusionPath| {
            path.points
                .iter()
                .map(|p| ((p.x - origin.x) as f64).hypot((p.y - origin.y) as f64) / 1e6)
                .fold(0.0, f64::max)
        };
        assert!(whole.iter().all(|path| far(path) > 20.0 * 2f64.sqrt()));
        assert!((length(&whole) / length(&own) - 1.0).abs() < 0.15);
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
        // The way it would be laid on a layer is the same on every
        // layer for a grid, and turns with the layers for plain lines.
        assert_eq!(base.internal_bridge_angle(3), base.sparse_angle);
        let plain = params(Pattern::Lines, 0.2);
        assert_eq!(plain.internal_bridge_angle(2), plain.sparse_angle);
        assert_eq!(
            plain.internal_bridge_angle(3),
            plain.sparse_angle + FRAC_PI_2
        );
        // One with a direction of its own, as a second layer of bridge
        // has, is laid that way: here along x.
        let across = Surface {
            angle: Some(0.0),
            ..surface(SurfaceKind::InternalBridge, 20.0)
        };
        let lines = fill(&[across], 3, &base).unwrap();
        let longest = lines
            .iter()
            .flat_map(|line| line.points.windows(2))
            .max_by_key(|pair| (pair[1].x - pair[0].x).abs() + (pair[1].y - pair[0].y).abs())
            .unwrap();
        assert_eq!(longest[0].y, longest[1].y, "along x");
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
        // And all the lines of one surface are one part.
        assert!(top.iter().all(|line| line.part == 1));
        let two = fill(
            &[
                surface(SurfaceKind::Top, 10.0),
                surface(SurfaceKind::InternalSolid, 10.0),
            ],
            0,
            &base,
        )
        .unwrap();
        assert_eq!(two.iter().filter(|line| line.part == 1).count(), top.len());
        assert!(two[top.len()..].iter().all(|line| line.part == 2));
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

    #[test]
    fn what_solid_lines_leave_bare_is_gap_filled_where_asked() {
        // A long thin wedge: lines across it stop short of its point,
        // where it is too narrow for a line of the fill's own width.
        let wedge = |kind| Surface {
            kind,
            area: ExPolygon {
                contour: Polygon {
                    points: [(0.0, 0.0), (30.0, 0.0), (0.0, 4.0)]
                        .map(|(x, y)| Point2::from_mm(x, y))
                        .to_vec(),
                },
                holes: Vec::new(),
            },
            angle: None,
        };
        let frame = |_: &Surface| Frame {
            origin: Point2::from_mm(0.0, 0.0),
            bounds: None,
        };
        let gaps = |kind, target, inside: bool| -> Vec<ExtrusionPath> {
            let surface = wedge(kind);
            let asked = FillParams {
                gap_fill: target,
                ..params(Pattern::Lines, 0.2)
            };
            let inside = if inside {
                vec![surface.area.clone()]
            } else {
                Vec::new()
            };
            fill_within(&[surface], 0, &asked, &frame, &inside)
                .unwrap()
                .into_iter()
                .filter(|path| path.role == Role::GapFill)
                .collect()
        };
        let found = gaps(SurfaceKind::Top, GapFillTarget::TopBottom, true);
        assert!(!found.is_empty());
        // In the wedge, thinner than the fill's own lines, and laid with
        // the surface they belong to.
        for line in &found {
            assert_eq!(line.part, 1);
            assert!(line.flow.width < 0.4, "{}", line.flow.width);
            assert!(line.points.iter().all(|point| {
                point.x >= 0
                    && point.y >= 0
                    && point.y as f64 <= 4e6 - point.x as f64 * 4.0 / 30.0 + 1e3
            }));
        }
        // The plain fill is what it is without.
        let plain = |target| {
            let asked = FillParams {
                gap_fill: target,
                ..params(Pattern::Lines, 0.2)
            };
            let surface = wedge(SurfaceKind::Top);
            let inside = [surface.area.clone()];
            fill_within(&[surface], 0, &asked, &frame, &inside)
                .unwrap()
                .into_iter()
                .filter(|path| path.role != Role::GapFill)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            plain(GapFillTarget::TopBottom),
            plain(GapFillTarget::Nowhere)
        );

        // Nowhere, when that is asked; not without knowing what is
        // inside the walls; and inside the part only for `everywhere`.
        assert!(gaps(SurfaceKind::Top, GapFillTarget::Nowhere, true).is_empty());
        assert!(gaps(SurfaceKind::Top, GapFillTarget::TopBottom, false).is_empty());
        assert!(!gaps(SurfaceKind::Bottom, GapFillTarget::TopBottom, true).is_empty());
        assert!(gaps(SurfaceKind::InternalSolid, GapFillTarget::TopBottom, true).is_empty());
        assert!(!gaps(SurfaceKind::InternalSolid, GapFillTarget::Everywhere, true).is_empty());
        assert!(gaps(SurfaceKind::Sparse, GapFillTarget::Everywhere, true).is_empty());
    }

    #[test]
    fn solid_fill_is_thinner_over_the_first_layer_and_fills_out_again() {
        // The second layer at the density set, then evenly back to full
        // over as many layers as are shrunk for the elephant's foot.
        let one = FillParams {
            elephant_foot_density: 0.7,
            ..params(Pattern::Lines, 0.2)
        };
        let density = |params: &FillParams| -> Vec<f64> {
            (0..6).map(|layer| params.solid_density(layer)).collect()
        };
        assert_eq!(density(&one), [1.0, 0.7, 1.0, 1.0, 1.0, 1.0]);
        let three = FillParams {
            elephant_foot_layers: 3,
            ..one
        };
        let found = density(&three);
        for (found, expected) in found.iter().zip([1.0, 0.7, 0.8, 0.9, 1.0, 1.0]) {
            assert!((found - expected).abs() < 1e-12, "{found} for {expected}");
        }
        // At full density, and as near it as makes no difference, no
        // layer is thinned.
        let full = FillParams {
            elephant_foot_density: 0.99995,
            elephant_foot_layers: 3,
            ..one
        };
        assert_eq!(density(&full), [1.0; 6]);
        assert_eq!(density(&params(Pattern::Lines, 0.2)), [1.0; 6]);
    }

    /// How far apart the long lines of some fill are, in mm, and how
    /// many of them there are.
    fn pitch_of(paths: &[ExtrusionPath], angle: f64) -> (f64, usize) {
        let (sin, cos) = angle.sin_cos();
        let mut across: Vec<f64> = paths
            .iter()
            .flat_map(|path| path.points.windows(2))
            .filter(|pair| {
                ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64) > 2e6
            })
            .map(|pair| (pair[0].y as f64 * cos - pair[0].x as f64 * sin) / 1e6)
            .collect();
        across.sort_by(f64::total_cmp);
        let mut gaps: Vec<f64> = across.windows(2).map(|pair| pair[1] - pair[0]).collect();
        gaps.sort_by(f64::total_cmp);
        (gaps[gaps.len() / 2], across.len())
    }

    #[test]
    fn thinned_solid_lines_keep_their_width_and_lie_further_apart() {
        let solid = [surface(SurfaceKind::InternalSolid, 20.0)];
        let full = params(Pattern::Lines, 0.2);
        let thin = FillParams {
            elephant_foot_density: 0.7,
            origin: Point2::from_mm(10.0, 10.0),
            ..full
        };
        // Layer 1 runs at 135 degrees: 45 and a quarter turn.
        let angle = 135f64.to_radians();
        let usual = fill(&solid, 1, &full).unwrap();
        let thinned = fill(&solid, 1, &thin).unwrap();
        let (usual_pitch, usual_count) = pitch_of(&usual, angle);
        let (pitch, count) = pitch_of(&thinned, angle);
        assert!(
            (usual_pitch - full.solid.spacing).abs() < 1e-3,
            "{usual_pitch}"
        );
        assert!((pitch - full.solid.spacing / 0.7).abs() < 1e-3, "{pitch}");
        assert!(count < usual_count);
        // The same line, so seven tenths of the plastic to within the
        // joins, and still solid fill in its own order.
        assert!(thinned.iter().all(|line| line.flow == full.solid));
        assert!(thinned.iter().all(|line| line.role == Role::SolidInfill));
        let share = length(&thinned) / length(&usual);
        assert!((share - 0.7).abs() < 0.03, "{share}");
        // A line passes through the middle of the object, wherever the
        // area's own edge is: the lines of a square moved half a pitch
        // lie where the first one's do.
        let across = |paths: &[ExtrusionPath]| -> f64 {
            let point = paths[0].points[0];
            let (sin, cos) = angle.sin_cos();
            let from_middle = (point.y as f64 - 10e6) * cos - (point.x as f64 - 10e6) * sin;
            (from_middle / (full.solid.spacing * 1e6 / 0.7)).rem_euclid(1.0)
        };
        let on_grid = |offset: f64| offset < 0.01 || offset > 0.99;
        assert!(on_grid(across(&thinned)), "{}", across(&thinned));
        let mut moved = solid.clone();
        for point in &mut moved[0].area.contour.points {
            point.x += 123_456;
        }
        let elsewhere = fill(&moved, 1, &thin).unwrap();
        assert!(on_grid(across(&elsewhere)), "{}", across(&elsewhere));

        // Only the layers named, and only solid fill inside the part.
        assert_eq!(
            fill(&solid, 0, &thin).unwrap(),
            fill(&solid, 0, &full).unwrap()
        );
        assert_eq!(
            fill(&solid, 2, &thin).unwrap(),
            fill(&solid, 2, &full).unwrap()
        );
        for kind in [
            SurfaceKind::Top,
            SurfaceKind::Bottom,
            SurfaceKind::Bridge,
            SurfaceKind::InternalBridge,
            SurfaceKind::Sparse,
        ] {
            let other = [surface(kind, 20.0)];
            let with_origin = FillParams {
                origin: thin.origin,
                ..full
            };
            assert_eq!(
                fill(&other, 1, &thin).unwrap(),
                fill(&other, 1, &with_origin).unwrap(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn thinned_solid_fill_has_no_gaps_filled() {
        // A wedge, whose point no line of the fill reaches.
        let wedge = Surface {
            kind: SurfaceKind::InternalSolid,
            area: ExPolygon {
                contour: Polygon {
                    points: [(0.0, 0.0), (30.0, 0.0), (0.0, 4.0)]
                        .map(|(x, y)| Point2::from_mm(x, y))
                        .to_vec(),
                },
                holes: Vec::new(),
            },
            angle: None,
        };
        let frame = |_: &Surface| Frame {
            origin: Point2::from_mm(0.0, 0.0),
            bounds: None,
        };
        let gaps = |density: f64, layer: usize| {
            let asked = FillParams {
                gap_fill: GapFillTarget::Everywhere,
                elephant_foot_density: density,
                ..params(Pattern::Lines, 0.2)
            };
            let inside = [wedge.area.clone()];
            fill_within(std::slice::from_ref(&wedge), layer, &asked, &frame, &inside)
                .unwrap()
                .into_iter()
                .filter(|path| path.role == Role::GapFill)
                .count()
        };
        assert!(gaps(1.0, 1) > 0);
        assert_eq!(gaps(0.7, 1), 0);
        assert!(gaps(0.7, 2) > 0);
    }
}
