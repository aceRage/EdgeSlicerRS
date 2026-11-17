use std::f64::consts::FRAC_PI_2;

use edge_clip::{ClipError, Join};
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::{Coord, ExPolygon, Point2, scaled};

use crate::rectilinear;
use crate::{NarrowFill, Pattern, Surface, SurfaceKind, curves, lines, narrow};

/// Why a layer could not be filled.
#[derive(Debug, thiserror::Error)]
pub enum FillError {
    #[error(transparent)]
    Clip(#[from] ClipError),
    /// The loops a narrow solid area is filled with are walls of
    /// changing width, and are worked out as those are.
    #[error("the loops of a narrow solid area could not be worked out: {0}")]
    Narrow(#[from] edge_arachne::Error),
}

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
    /// How far apart sparse lines are laid at full density, in mm: the
    /// spacing of `sparse` at the object's own layer height, whatever
    /// this layer's is (Fill.cpp `group_fills`: "a generic flow spacing
    /// for all layers").
    pub sparse_spacing: f64,
    /// Sparse fill density from 0 to 1.
    pub density: f64,
    pub pattern: Pattern,
    /// Direction of sparse lines, radians counter-clockwise from +x.
    pub sparse_angle: f64,
    /// Direction of solid lines on even layers; odd layers are turned 90°.
    pub solid_angle: f64,
    /// The direction set for the lines of top surfaces and of bottom
    /// surfaces, radians counter-clockwise from +x (`top_layer_direction`,
    /// `bottom_layer_direction`), the same on every layer; none keeps
    /// them to the solid fill's (Fill.cpp `group_fills`, "optional
    /// absolute direction for the top / bottom surfaces", which marks
    /// the angle a template angle so that it is not turned from layer
    /// to layer). A bottom's direction is a bridge's too where the
    /// bridge has no direction of its own: the reference counts a bridge
    /// over air as a bottom surface.
    pub top_angle: Option<f64>,
    pub bottom_angle: Option<f64>,
    /// How closely the lines of top surfaces and of bottom surfaces are
    /// laid, from 0 to 1 (`top_surface_density`, `bottom_surface_density`):
    /// below 1 they keep their width and lie that much further apart,
    /// on the object's grid as any fill short of full does; a top of
    /// none is left to its walls (Fill.cpp `group_fills`, "a 0% top
    /// surface density means walls only").
    pub top_density: f64,
    pub bottom_density: f64,
    /// How the solid fill just under a top surface is laid where
    /// `undertop_surface_pattern` sets it apart: whether its lines are
    /// joined end to end and swept in order, as for the other solid
    /// fill. None lays it as all solid fill inside the part is. See
    /// [`fill_under_tops`].
    pub under_top: Option<(bool, bool)>,
    /// How much the corners of a sparse pattern are rounded, from 0 to
    /// 1, and the G-code resolution in mm the rounded corners keep to
    /// (`sparse_infill_smooth_factor`, `resolution`): only for the
    /// patterns the reference rounds (PrintConfig.hpp
    /// `is_smoothable_infill_pattern`: Hilbert curve, octagram spiral,
    /// honeycomb, 3D honeycomb and cross hatch). None leaves them sharp.
    pub smooth: Option<(f64, f64)>,
    /// Whether lines laid in the fill's own direction turn a quarter on
    /// odd layers, as they do unless Z contouring keeps them steady
    /// ([`crate::InfillSettings::steady_direction`]).
    pub alternate: bool,
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
    /// A top surface laid with Archimedean chords, and whether in the
    /// flow rate calibration's order ([`crate::InfillSettings`]).
    pub top_archimedean: bool,
    pub flow_calib_order: bool,
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
    /// What filling the narrow parts of solid fill inside the part with
    /// loops needs, where they are told from the rest
    /// (`detect_narrow_internal_solid_infill`); none fills them with the
    /// same lines as the wide parts.
    pub narrow: Option<NarrowFill>,
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
                    1.5 * self.sparse_spacing / self.density,
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
                let line = scaled(self.sparse_spacing);
                let period = (scaled(self.sparse_spacing) as f64 / self.density) as i64 * 4;
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
                    if self.alternate && layer_index % 2 == 1 {
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
        (4.0 / 3.0) * self.sparse_spacing / self.density
    }
}

/// How a surface's lines meet the edge of its area.
#[derive(Clone, Copy)]
enum Lay {
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
    /// A top surface of Archimedean chords (FillPlanePath.cpp, with
    /// `center_of_surface_pattern` at `each_surface`): the spiral turns
    /// about the middle of the area's own bounds, cut to the area half a
    /// spacing in; the pieces are not joined.
    Spiral,
    /// Solid lines and bridges as `FillRectilinear.cpp` lays them: run
    /// to within `LINES_INSET` of the edge, and turned half a spacing
    /// inside it where they are joined; a line that does not come half
    /// a spacing inside the area is not laid. `ordered` lays them in
    /// one sweep across the area, where no line goes down between two
    /// already laid. A `density` under 1 lays them that much further
    /// apart.
    Turned {
        joined: bool,
        placed: Placed,
        ordered: bool,
        density: f64,
    },
}

/// Where the lines of an area lie across it (FillRectilinear.cpp
/// `fill_surface_by_lines`, "define flow spacing according to requested
/// density").
#[derive(Clone, Copy, PartialEq)]
enum Placed {
    /// Spread to fill the area's own width, the first line half a pitch
    /// in from its edge (`_adjust_solid_spacing`): fill at full density
    /// that the reference does not tell to keep its spacing, which is
    /// top and bottom surfaces and solid fill inside the part.
    Fitted,
    /// At the pitch set, on the lines every pitch from
    /// [`FillParams::origin`], the middle of the object
    /// (`align_to_grid` about `_infill_direction`'s point): what the
    /// reference does with fill that is not at full density, and with
    /// bridges, which it tells not to adjust (Fill.cpp
    /// `Layer::make_fills`, `params.dont_adjust = true`). At full
    /// density the lines lie half a pitch off those, as there (`x0 +=
    /// (line_spacing + SCALED_EPSILON) / 2`).
    Grid,
}

/// How far a line's centre stops short of the area's edge, as a share of
/// the line spacing (FillRectilinear.cpp: 0.5 less `INFILL_OVERLAP_OVER_SPACING`
/// for single-direction fill, a full half spacing for crossing lines).
pub(crate) const LINES_INSET: f64 = 0.05;
/// How much further in joined solid lines turn: to half a spacing from the
/// area's edge, so the turn itself lies inside the area and not on it.
pub(crate) const TURN_INSET: f64 = 0.45;
/// Fill lines for one layer's surfaces, surface by surface.
pub fn fill(
    surfaces: &[Surface],
    layer_index: usize,
    params: &FillParams,
) -> Result<Vec<ExtrusionPath>, FillError> {
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
) -> Result<Vec<ExtrusionPath>, FillError> {
    fill_within(surfaces, layer_index, params, frame_of, &[])
}

/// As [`fill_from`], knowing the layer's fill area without its overlap
/// into the walls (`no_overlap`): what a solid surface's lines leave bare
/// of it is then filled with lines of whatever width fits, where
/// [`FillParams::gap_fill`] asks (FillBase.cpp `_create_gap_fill`). With
/// no such area there is no such fill.
///
/// Where [`FillParams::narrow`] asks, the narrow parts of the solid fill
/// inside the part are told from the rest and filled with loops along
/// their outline, kept within `no_overlap` (Fill.cpp `group_fills`,
/// "detect narrow internal solid infill area and use
/// ipConcentricInternal pattern instead"). The reference tells them
/// apart over all of a layer's solid fill at once, areas that touch
/// taken as one; so it is done here, and on a layer with nothing
/// narrow the areas are filled one by one as they came.
///
/// Of what is so told apart, a patch under three line spacings across
/// both ways and under four spacings squared in area is left out, wide
/// or narrow, as in the reference ("drop degenerate internal-solid
/// slivers"): all it could hold is a dot shorter than it is wide, and
/// telling narrow from wide leaves one in a corner of every area its
/// lines cross at a slant. (The reference has a second reason, which
/// is not one here: whether such a patch came through its own clipping
/// changed from one run to the next of the same project.) With
/// narrow areas not told apart nothing is left out, as before.
pub fn fill_within(
    surfaces: &[Surface],
    layer_index: usize,
    params: &FillParams,
    frame_of: &dyn Fn(&Surface) -> Frame,
    no_overlap: &[ExPolygon],
) -> Result<Vec<ExtrusionPath>, FillError> {
    let turned = if params.alternate && layer_index % 2 == 1 {
        FRAC_PI_2
    } else {
        0.0
    };
    let mut paths = Vec::new();
    // The solid fill told apart: its wide parts stand where the first
    // of the solid areas stood, and the narrow parts are filled last.
    let told = match &params.narrow {
        Some(_) => told_apart(surfaces, params, params.solid_angle + turned)?,
        None => None,
    };
    let (surfaces, narrow_areas): (&[Surface], &[ExPolygon]) = match &told {
        Some((surfaces, narrow)) => (surfaces, narrow),
        None => (surfaces, &[]),
    };
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
            SurfaceKind::Top if params.top_density <= 0.0 => continue,
            SurfaceKind::Sparse => match params.pattern {
                Pattern::Lines => (
                    params.sparse,
                    Role::SparseInfill,
                    params.sparse_spacing / params.density,
                    Lay::Zigzag,
                    &[params.sparse_angle + turned],
                ),
                Pattern::AlignedLines => (
                    params.sparse,
                    Role::SparseInfill,
                    params.sparse_spacing / params.density,
                    Lay::Zigzag,
                    &[params.sparse_angle],
                ),
                Pattern::ZigZag | Pattern::CrossZag => (
                    params.sparse,
                    Role::SparseInfill,
                    params.sparse_spacing / params.density,
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
                    params.sparse_spacing / params.density,
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
                        sweeps.len() as f64 * params.sparse_spacing / params.density
                    },
                    Lay::Anchored,
                    &sweep_angles,
                ),
            },
            // Archimedean chords keep their direction from layer to
            // layer (FillPlanePath.hpp `_layer_angle` is 0) and are a
            // quarter turn on from the fill's (`_infill_direction`).
            SurfaceKind::Top if params.top_archimedean => (
                params.top,
                Role::TopSolidInfill,
                params.top.spacing,
                Lay::Spiral,
                &[params.top_angle.unwrap_or(params.solid_angle) + FRAC_PI_2],
            ),
            SurfaceKind::Top => (
                params.top,
                Role::TopSolidInfill,
                params.top.spacing,
                Lay::Turned {
                    joined: params.top_joined,
                    ordered: params.top_ordered,
                    placed: Placed::Fitted,
                    density: params.top_density,
                },
                &[params.top_angle.unwrap_or(params.solid_angle + turned)],
            ),
            // From support to support where that is known. A bridge is
            // laid as solid lines are (Fill.cpp `group_fills`: the
            // pattern of a solid surface that is neither a top or
            // bottom face nor solid fill inside the part is `ipMonotonic`
            // where the top surface's is `monotonic` or `monotonicline`,
            // `ipRectilinear` otherwise, and both join their lines), at
            // its own spacing over `bridge_density`, and is not fitted
            // to its area (`Layer::make_fills`, `params.dont_adjust`).
            SurfaceKind::Bridge => (
                params.bridge,
                Role::BridgeInfill,
                params.bridge.spacing,
                Lay::Turned {
                    joined: true,
                    ordered: params.top_ordered,
                    placed: Placed::Grid,
                    density: params.bridge.spacing / params.bridge_spacing,
                },
                &[surface
                    .angle
                    .or(params.bottom_angle)
                    .unwrap_or(params.solid_angle + turned)],
            ),
            // Across the sparse lines beneath, unless the surface has a
            // direction of its own: a second layer of bridge is laid
            // across the first. Laid as a bridge over air is, over
            // `internal_bridge_density`.
            SurfaceKind::InternalBridge => (
                params.internal_bridge,
                Role::InternalBridge,
                params.internal_bridge.spacing,
                Lay::Turned {
                    joined: true,
                    ordered: params.top_ordered,
                    placed: Placed::Grid,
                    density: params.internal_bridge.spacing / params.internal_bridge_spacing,
                },
                &[surface
                    .angle
                    .unwrap_or_else(|| params.internal_bridge_angle(layer_index))],
            ),
            SurfaceKind::Bottom => (
                params.solid,
                Role::BottomSurface,
                params.solid.spacing,
                Lay::Turned {
                    joined: params.bottom_joined,
                    ordered: params.bottom_ordered,
                    placed: Placed::Fitted,
                    density: params.bottom_density,
                },
                &[params.bottom_angle.unwrap_or(params.solid_angle + turned)],
            ),
            // Solid fill inside the part is fitted to its area as top
            // and bottom surfaces are: FillRectilinear.cpp
            // `fill_surface_by_lines` fits whatever is at full density
            // and not told to keep its spacing, and only bridges are
            // told. Where narrow parts have been told from the rest
            // ([`told_apart`]) what is fitted is the wide part, as
            // there. That part is cut along its own lines, a whole
            // number of spacings across from the area's edge, unless
            // the last of them reaches past the far edge and the wide
            // part is the area's whole width: so its lines mostly keep
            // the pitch set (a whole number of them fits already), and
            // an area with nothing narrow in it, or any area with
            // `detect_narrow_internal_solid_infill` off, has them
            // spread.
            SurfaceKind::InternalSolid => (
                params.solid,
                Role::SolidInfill,
                params.solid.spacing,
                Lay::Turned {
                    joined: params.solid_joined,
                    ordered: params.solid_ordered,
                    placed: Placed::Fitted,
                    density: params.solid_density(layer_index),
                },
                &[params.solid_angle + turned],
            ),
        };

        match lay {
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
                    // Corners rounded as the pattern is made, before it is
                    // cut to the area, as there.
                    let smoothing = params.smooth.filter(|_| {
                        matches!(
                            params.pattern,
                            Pattern::Hilbert
                                | Pattern::Octagram
                                | Pattern::Honeycomb
                                | Pattern::Honeycomb3D
                                | Pattern::CrossHatch
                        )
                    });
                    let mut pattern = match params.pattern {
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
                            curves::plane_path(curve, over, flow.spacing, params.density, smoothing)
                        }
                        _ => curves::crosshatch(bounds, flow.spacing, params.density, params.z),
                    };
                    // The plane paths are rounded as they are made, in
                    // units of their pitch; the others here, in whole
                    // units (FillHoneycomb.cpp, Fill3DHoneycomb.cpp,
                    // FillCrossHatch.cpp).
                    if let Some((factor, resolution)) = smoothing
                        && !matches!(params.pattern, Pattern::Hilbert | Pattern::Octagram)
                    {
                        crate::smooth::smooth_lines(
                            &mut pattern,
                            factor,
                            resolution * edge_geom::UNITS_PER_MM,
                        );
                    }
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
            Lay::Spiral => {
                let line = flow.scaled_spacing();
                let inside =
                    edge_clip::offset(std::slice::from_ref(&surface.area), -line / 2, Join::MITER)?;
                // `_infill_direction` works the angle in single precision.
                let along = params.top_angle.unwrap_or(params.solid_angle);
                let angle = f64::from(along as f32 + std::f32::consts::FRAC_PI_2);
                // Short of full density the spiral's turns lie that much
                // further apart (FillPlanePath.cpp, `spacing / density`).
                for chords in spiral_chords(
                    &inside,
                    angle,
                    (flow.spacing, params.top_density),
                    params.flow_calib_order,
                )? {
                    for (index, points) in chords.into_iter().enumerate() {
                        paths.push(ExtrusionPath {
                            points,
                            role,
                            flow,
                            // In the calibration's order every piece
                            // follows the one before, from its first
                            // point (FillBase.cpp: `no_sort`, and
                            // `set_reverse`, which forbids turning).
                            in_turn: params.flow_calib_order && index > 0,
                            part,
                        });
                    }
                }
            }
            Lay::Anchored => {
                // The reference works the spacing out in single precision
                // (`Flow::spacing`) and scales it in double: half of it is
                // what the area is taken in by for the lines' ends
                // (`fill_surface_by_multilines`, passed through a `float`),
                // the whole of it, cut to whole units, how far rows keep
                // off the area's edge, and over the density per set of
                // lines (a `float` too), how far apart the rows lie.
                let ref_spacing = if (flow.spacing - params.sparse_spacing).abs() < 1e-12 {
                    // `rounded_rectangle_extrusion_spacing`, in `float`.
                    f64::from(
                        flow.width as f32
                            - flow.height as f32 * (1.0 - 0.25 * std::f64::consts::PI) as f32,
                    )
                } else {
                    f64::from(params.sparse_spacing as f32)
                };
                let scale = |mm: f64| mm / 0.000_001;
                let inset = (scale(-0.5 * ref_spacing) as f32) as Coord;
                let line_width = scale(ref_spacing) as Coord;
                let inside =
                    edge_clip::offset(std::slice::from_ref(&surface.area), inset, Join::MITER)?;
                let line = flow.scaled_spacing();
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
                // The lines as the reference makes them, to the unit, for
                // the patterns that are rows in sets with no more to them:
                // the frame's angle added up in single precision as
                // `_infill_direction` and `fill_surface_by_multilines` do,
                // and the rows' pitch over the density shared between the
                // sets. Where the reference would give up on the area, and
                // for the lateral honeycomb, the rows above are used.
                let exact = if params.pattern == Pattern::LateralHoneycomb {
                    None
                } else {
                    let direction = params.sparse_angle as f32 + std::f32::consts::FRAC_PI_2;
                    let density = ((params.density as f32) as f64 / sweeps.len() as f64) as f32;
                    let line_spacing = (scale(ref_spacing) / f64::from(density)) as Coord;
                    let ref_sweeps: Vec<rectilinear::Sweep> = sweeps
                        .iter()
                        .map(|(angle, shift)| rectilinear::Sweep {
                            angle: direction + (angle - params.sparse_angle) as f32,
                            shift: scale(f64::from(*shift as f32)) as Coord,
                        })
                        .collect();
                    let rings: Vec<&edge_geom::Polygon> =
                        inside.iter().flat_map(ExPolygon::rings).collect();
                    rectilinear::fill_lines(
                        &surface.area,
                        &rings,
                        origin,
                        line_spacing,
                        line_width,
                        &ref_sweeps,
                    )
                };
                let made = exact.unwrap_or_else(|| {
                    lines::rows_in_order(&inside, scaled(spacing) as f64, &rows, angles)
                });
                // A path shorter than half its own width is a blob, not a
                // line, and is left out; the reference keeps them.
                let least = flow.width / 2.0;
                // A grid, a lateral lattice and a lateral honeycomb are
                // laid from the other end of every path on odd layers
                // (FillRectilinear.cpp, `FillGrid::fill_surface` and the
                // like: "if (this->layer_id % 2 == 1) std::reverse"), and
                // a grid's paths are not turned again when they are
                // ordered (`ExtrusionPath::KEPT_WAY`).
                let turned_round = layer_index % 2 == 1
                    && matches!(
                        params.pattern,
                        Pattern::Grid | Pattern::LateralLattice | Pattern::LateralHoneycomb
                    );
                let part = if params.pattern == Pattern::Grid {
                    part | ExtrusionPath::KEPT_WAY
                } else {
                    part
                };
                // Every island of the surface at once, as the reference
                // joins the lines of `polygons_outer` together.
                for mut points in lines::connected(made, &inside, scale(ref_spacing), joining) {
                    if turned_round {
                        points.reverse();
                    }
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
            Lay::Turned {
                joined,
                placed,
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
                // A join is no longer than three spacings above 80%
                // density, and as long as it likes below that and for a
                // bridge (Fill.cpp `link_max_length`). The reference
                // means the same and does not do it: it works the limit
                // out from the filler's spacing before that is set,
                // while it is still zero, and zero is no limit, so a
                // row parted by a hole is joined round the hole's side
                // however long that is and a whole extra line is laid
                // beside a fill line along a square hole. Not copied;
                // fixed there in aceRage/EdgeSlicer#968.
                let unlimited = edge_geom::Coord::MAX / 4;
                let bridging = matches!(
                    surface.kind,
                    SurfaceKind::Bridge | SurfaceKind::InternalBridge
                );
                let max_link = if !joined {
                    0
                } else if !bridging && density > 0.8 {
                    scaled(3.0 * flow.spacing)
                } else {
                    unlimited
                };
                let (mut pitch, mut flow) = (scaled(spacing) as f64, flow);
                // Lines laid thinner than solid keep their width and
                // are that much further apart, the distance cut to a
                // whole number of units as FillRectilinear.cpp
                // `fill_surface_by_lines` cuts it. They are turned and
                // joined where solid lines are: the outlines they run
                // to are drawn in by the line's own spacing there too.
                if density < 1.0 {
                    pitch = (spacing * edge_geom::UNITS_PER_MM / density).trunc();
                }
                // The reference's `FillParams::full_infill`. Fill that
                // is not full is never fitted, and lies on the grid.
                let full = density > 0.9999;
                let placed = if full { placed } else { Placed::Grid };
                // The area's reach across its lines, in whole units as
                // the reference has it (it turns the area first, which
                // rounds every corner).
                let (low, high) = (low.round(), high.round());
                if placed == Placed::Fitted {
                    let spread = fitted_pitch(high - low, pitch);
                    // Each line is made as wide as its share. A pitch no
                    // line can be made for leaves the fill as it was.
                    if spread != pitch
                        && let Ok(wider) = flow.with_spacing(spread / edge_geom::UNITS_PER_MM)
                    {
                        (pitch, flow) = (spread, wider);
                    }
                }
                // Half a pitch and a hair (FillRectilinear.cpp, `x0 +=
                // (line_spacing + SCALED_EPSILON) / 2`) from where the
                // lines are counted: the area's own edge when they are
                // fitted to it, and otherwise the grid through the
                // middle of the object, which is where the reference
                // puts any fill it does not fit (`align_to_grid` about
                // `_infill_direction`'s point, turned with the area and
                // rounded to whole units as there). Fill that is not
                // full lies on that grid itself.
                //
                // The hair matters where an area is a whole number of
                // pitches across, as the wide part of solid fill told
                // from its narrow part is: its first line and its last
                // would lie exactly where joined lines turn, which is
                // neither inside that outline nor out of it, and the
                // hair keeps the first and drops the last.
                let half = ((pitch + SCALED_EPSILON) / 2.0).floor();
                let first = match placed {
                    Placed::Fitted => low + half,
                    Placed::Grid => {
                        let (sin, cos) = angles[0].sin_cos();
                        let through = (origin.y as f64 * cos - origin.x as f64 * sin).round();
                        if full { through + half } else { through }
                    }
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
            (_, SurfaceKind::Top) if params.top_density < 1.0 => false,
            (_, SurfaceKind::Bottom) if params.bottom_density < 1.0 => false,
            (_, SurfaceKind::Top | SurfaceKind::Bottom) => true,
            (GapFillTarget::Everywhere, SurfaceKind::InternalSolid) => true,
            _ => false,
        };
        if wanted && !no_overlap.is_empty() {
            let gaps = gap_fill(surface, &paths[first_path..], &flow, params, no_overlap)?;
            // Lines laid in turn keep their order and the gap lines go in
            // among them (`insert_gap_fill`, where the reference's
            // collection is `no_sort`); otherwise the gap lines join the
            // surface's lines, to be ordered with them when the layer is
            // written.
            let in_turn = match lay {
                Lay::Turned { ordered, .. } => ordered,
                Lay::Spiral => params.flow_calib_order,
                _ => false,
            };
            if in_turn && paths.len() > first_path {
                let laid = paths.split_off(first_path);
                paths.extend(insert_gap_fill(laid, &gaps, &flow, part));
            } else {
                let mut made = edge_medial::variable_width(&gaps, Role::GapFill, &flow);
                for path in &mut made {
                    path.part = part;
                }
                paths.extend(made);
            }
        }
    }
    if let Some(narrow) = &params.narrow {
        for (number, area) in narrow_areas.iter().enumerate() {
            let part = ExtrusionPath::LOOPED | (surfaces.len() + number + 1) as u32;
            paths.extend(narrow::loops(
                area,
                no_overlap,
                &params.solid,
                narrow,
                part,
            )?);
        }
    }
    Ok(paths)
}

/// As [`fill_within`], with the solid fill inside the part that lies
/// under a top surface of the layer above (`upper_tops`) laid by
/// [`FillParams::under_top`], where that is set (Fill.cpp `group_fills`,
/// `is_undertop`: a surface of solid fill is under a top when the top's
/// outline crosses it or it lies wholly within the top). The reference
/// fills it as a group of its own, with its own pattern, and so it is
/// filled here: after the rest of the layer, its narrow parts told
/// from its wide ones by themselves.
pub fn fill_under_tops(
    surfaces: &[Surface],
    upper_tops: &[ExPolygon],
    layer_index: usize,
    params: &FillParams,
    frame_of: &dyn Fn(&Surface) -> Frame,
    no_overlap: &[ExPolygon],
) -> Result<Vec<ExtrusionPath>, FillError> {
    let Some((joined, ordered)) = params.under_top else {
        return fill_within(surfaces, layer_index, params, frame_of, no_overlap);
    };
    let mut under = Vec::new();
    let mut rest = Vec::new();
    for surface in surfaces {
        let below = surface.kind == SurfaceKind::InternalSolid
            && upper_tops
                .iter()
                .map(|top| overlaps(&surface.area, top))
                .try_fold(false, |any, found| found.map(|found| any || found))?;
        if below {
            under.push(surface.clone());
        } else {
            rest.push(surface.clone());
        }
    }
    if under.is_empty() {
        return fill_within(surfaces, layer_index, params, frame_of, no_overlap);
    }
    let mut paths = fill_within(&rest, layer_index, params, frame_of, no_overlap)?;
    let own = FillParams {
        solid_joined: joined,
        solid_ordered: ordered,
        ..*params
    };
    append_parts(
        &mut paths,
        fill_within(&under, layer_index, &own, frame_of, no_overlap)?,
    );
    Ok(paths)
}

/// Adds the paths of another fill of the same layer after `paths`, its
/// parts numbered on from theirs, so that no part of one runs into a part
/// of the other.
pub fn append_parts(paths: &mut Vec<ExtrusionPath>, more: Vec<ExtrusionPath>) {
    let flags = ExtrusionPath::LOOPED | ExtrusionPath::KEPT_WAY;
    let past = paths
        .iter()
        .map(|path| path.part & !flags)
        .max()
        .unwrap_or(0);
    for mut path in more {
        path.part = (path.part & flags) | ((path.part & !flags) + past);
        paths.push(path);
    }
}

/// Whether `other` overlaps `area` (ExPolygon.cpp `ExPolygon::overlaps`):
/// some of its outline lies inside `area`, or `area` lies within it.
fn overlaps(area: &ExPolygon, other: &ExPolygon) -> Result<bool, ClipError> {
    let rings: Vec<Vec<Point2>> = std::iter::once(&other.contour)
        .chain(&other.holes)
        .filter(|ring| !ring.points.is_empty())
        .map(|ring| {
            let mut line = ring.points.clone();
            line.push(ring.points[0]);
            line
        })
        .collect();
    if !edge_clip::clip_lines(&rings, std::slice::from_ref(area))?.is_empty() {
        return Ok(true);
    }
    Ok(area
        .contour
        .points
        .first()
        .is_some_and(|&point| expolygon_contains(other, point)))
}

/// Whether a point lies inside a region and none of its holes, by the
/// crossings of a ray (Polygon.cpp `Polygon::contains`).
fn expolygon_contains(region: &ExPolygon, point: Point2) -> bool {
    let inside = |ring: &edge_geom::Polygon| {
        let mut result = false;
        let points = &ring.points;
        for (at, &a) in points.iter().enumerate() {
            let b = points[(at + 1) % points.len()];
            if (a.y > point.y) != (b.y > point.y) {
                let cross =
                    (b.x - a.x) as f64 * (point.y - a.y) as f64 / (b.y - a.y) as f64 + a.x as f64;
                if (point.x as f64) < cross {
                    result = !result;
                }
            }
        }
        result
    };
    inside(&region.contour) && !region.holes.iter().any(inside)
}

/// A layer's surfaces to fill with lines, and its areas to fill with loops.
type Told = (Vec<Surface>, Vec<ExPolygon>);

/// A layer's surfaces with the solid fill inside the part told apart
/// into wide and narrow: the surfaces to fill with lines, the wide parts
/// among them, and the narrow areas. None when nothing is narrow.
fn told_apart(
    surfaces: &[Surface],
    params: &FillParams,
    direction: f64,
) -> Result<Option<Told>, ClipError> {
    let solid: Vec<ExPolygon> = surfaces
        .iter()
        .filter(|surface| surface.kind == SurfaceKind::InternalSolid)
        .map(|surface| surface.area.clone())
        .collect();
    // Areas that touch are one, made so with the reference's safety
    // offset; one by itself is taken as it is (`group_fills`).
    let solid = if solid.len() > 1 {
        edge_clip::offset(&solid, SAFETY_OFFSET, Join::MITER)?
    } else {
        solid
    };
    let spacing = params.solid.scaled_spacing();
    let Some(mut split) = narrow::split(&solid, spacing, direction)? else {
        return Ok(None);
    };
    // Too small to hold more than a dot (`group_fills`, "drop
    // degenerate internal-solid slivers").
    let dot = |area: &ExPolygon| {
        let points = &area.contour.points;
        let across = |of: fn(&Point2) -> edge_geom::Coord| {
            let all = points.iter().map(of);
            all.clone().max().unwrap_or(0) - all.min().unwrap_or(0)
        };
        across(|point| point.x) < 3 * spacing
            && across(|point| point.y) < 3 * spacing
            && area.area_mm2().abs() < 4.0 * params.solid.spacing * params.solid.spacing
    };
    split.wide.retain(|area| !dot(area));
    split.narrow.retain(|area| !dot(area));
    let mut wide = Some(split.wide);
    let mut told = Vec::with_capacity(surfaces.len());
    for surface in surfaces {
        if surface.kind != SurfaceKind::InternalSolid {
            told.push(surface.clone());
        } else if let Some(wide) = wide.take() {
            told.extend(
                wide.into_iter()
                    .map(|area| Surface::new(SurfaceKind::InternalSolid, area)),
            );
        }
    }
    Ok(Some((told, split.narrow)))
}

/// The pitch that fits a whole number of lines across `width`, both in
/// scaled units: the pitch set or a little more, by a fifth at most, so
/// that the first line and the last lie half a pitch inside the edges
/// with none left over (FillBase.cpp `Fill::_adjust_solid_spacing`,
/// with its rounding down to whole units).
/// A point turned by `angle` about the origin and rounded to whole units,
/// as the reference turns its polygons (MultiPoint.cpp `rotate`).
fn rotated(point: Point2, angle: f64) -> Point2 {
    let (sin, cos) = angle.sin_cos();
    let (x, y) = (point.x as f64, point.y as f64);
    Point2 {
        x: (cos * x - sin * y).round() as i64,
        y: (cos * y + sin * x).round() as i64,
    }
}

/// The pieces of an Archimedean spiral that fill each of `areas`, which
/// are already half a spacing in from the surface's edge
/// (FillPlanePath.cpp `_fill_surface_single`, for a solid surface with
/// `center_of_surface_pattern` at `each_surface`): the area is turned by
/// `-angle`, the spiral laid about the middle of its bounds (grown by
/// `SCALED_EPSILON`) and cut to it, and the pieces turned back.
///
/// With `calibration` (`calib_flowrate_topinfill_special_order`) the
/// longest piece, the spiral in the middle, comes last and runs from the
/// inside out, after the chords round it. The reference orders the
/// chords with `chain_polylines` (a greedy chain improved by exchanging
/// pairs); here each next chord is the one with an end nearest the last
/// one's end. Which chord comes when only changes the travel between
/// them, not what is laid.
fn spiral_chords(
    areas: &[ExPolygon],
    angle: f64,
    (spacing, density): (f64, f64),
    calibration: bool,
) -> Result<Vec<Vec<Vec<Point2>>>, FillError> {
    /// libslic3r's `SCALED_EPSILON`.
    const SCALED_EPSILON: i64 = 100;
    let length = |piece: &[Point2]| -> f64 {
        piece
            .windows(2)
            .map(|pair| ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64))
            .sum()
    };
    let mut out = Vec::new();
    for area in areas {
        let turn = |polygon: &edge_geom::Polygon| edge_geom::Polygon {
            points: polygon.points.iter().map(|&p| rotated(p, -angle)).collect(),
        };
        let area = ExPolygon {
            contour: turn(&area.contour),
            holes: area.holes.iter().map(turn).collect(),
        };
        let Some(&first) = area.contour.points.first() else {
            continue;
        };
        let (low, high) = area
            .contour
            .points
            .iter()
            .fold((first, first), |(low, high), p| {
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
        let bounds = (
            Point2 {
                x: low.x - SCALED_EPSILON,
                y: low.y - SCALED_EPSILON,
            },
            Point2 {
                x: high.x + SCALED_EPSILON,
                y: high.y + SCALED_EPSILON,
            },
        );
        let pattern =
            curves::plane_path(curves::Curve::Archimedean, bounds, spacing, density, None);
        if pattern.first().is_none_or(|line| line.len() < 2) {
            continue;
        }
        let mut pieces = edge_clip::clip_lines(&pattern, std::slice::from_ref(&area))?;
        pieces.retain(|piece| piece.len() >= 2);
        if pieces.is_empty() {
            continue;
        }
        if calibration {
            // The first of the longest (`std::max_element`).
            let mut longest = 0;
            for (at, piece) in pieces.iter().enumerate() {
                if length(piece) > length(&pieces[longest]) {
                    longest = at;
                }
            }
            let mut spiral = pieces.remove(longest);
            // The spiral's middle is where the pattern was laid about.
            let middle = Point2 {
                x: (bounds.0.x + bounds.1.x) / 2,
                y: (bounds.0.y + bounds.1.y) / 2,
            };
            let far = |p: Point2| {
                let (x, y) = ((p.x - middle.x) as f64, (p.y - middle.y) as f64);
                x * x + y * y
            };
            if far(spiral[0]) > far(spiral[spiral.len() - 1]) {
                spiral.reverse();
            }
            let mut chained: Vec<Vec<Point2>> = Vec::with_capacity(pieces.len() + 1);
            let mut left = pieces;
            if !left.is_empty() {
                chained.push(left.remove(0));
            }
            while !left.is_empty() {
                let last = &chained[chained.len() - 1];
                let at = last[last.len() - 1];
                let apart = |p: Point2| {
                    let (x, y) = ((p.x - at.x) as f64, (p.y - at.y) as f64);
                    x * x + y * y
                };
                let mut best = (0, false, f64::MAX);
                for (index, piece) in left.iter().enumerate() {
                    for (turned, end) in [(false, piece[0]), (true, piece[piece.len() - 1])] {
                        if apart(end) < best.2 {
                            best = (index, turned, apart(end));
                        }
                    }
                }
                let mut next = left.remove(best.0);
                if best.1 {
                    next.reverse();
                }
                chained.push(next);
            }
            chained.push(spiral);
            pieces = chained;
        }
        out.push(
            pieces
                .into_iter()
                .map(|piece| piece.into_iter().map(|p| rotated(p, angle)).collect())
                .collect(),
        );
    }
    Ok(out)
}

fn fitted_pitch(width: f64, pitch: f64) -> f64 {
    // libslic3r's `EPSILON`, taken off the width as the reference takes
    // it: a width of exactly so many pitches counts one interval fewer.
    const EPSILON: f64 = 1e-4;
    let intervals = ((width - EPSILON) / pitch).trunc();
    if intervals < 1.0 {
        return pitch;
    }
    let spread = ((width - EPSILON) / intervals).trunc();
    if spread / pitch > 1.2 {
        (1.2 * pitch + 0.5).floor()
    } else {
        spread
    }
}

/// Added to an offset so that edges meant to coincide overlap instead
/// (ClipperUtils.hpp `ClipperSafetyOffset`).
const SAFETY_OFFSET: edge_geom::Coord = 10;
/// libslic3r's `SCALED_EPSILON`: a tenth of a micron, in scaled units.
const SCALED_EPSILON: f64 = 100.0;
/// How much of its spacing a line may lose where it is squeezed
/// (libslic3r.h `INSET_OVERLAP_TOLERANCE`).
const INSET_OVERLAP_TOLERANCE: f64 = 0.4;

/// Lines down the middle of what `laid`, a solid surface's own lines,
/// leave bare of the surface's share of `no_overlap`, with the width of
/// the gap they fill (FillBase.cpp `_create_gap_fill`).
fn gap_fill(
    surface: &Surface,
    laid: &[ExtrusionPath],
    flow: &Flow,
    params: &FillParams,
    no_overlap: &[ExPolygon],
) -> Result<Vec<edge_medial::ThickPolyline>, ClipError> {
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
    Ok(lines)
}

/// The gap lines of a surface whose lines are laid in turn, each put
/// among `laid` where it adds least travel, but never before the last
/// of the surface's lines beside it; all of them after `laid` instead
/// when that travels no further (FillBase.cpp `insert_gap_fill`). The
/// surface's lines keep their order and direction, and so does every
/// gap line where it is put: all of them are laid in turn.
fn insert_gap_fill(
    laid: Vec<ExtrusionPath>,
    gaps: &[edge_medial::ThickPolyline],
    flow: &Flow,
    part: u32,
) -> Vec<ExtrusionPath> {
    let gaps = edge_medial::without_repeated_loops(gaps);
    let point = |p: Point2| (p.x as f64, p.y as f64);
    let apart = |a: Point2, b: Point2| {
        let ((ax, ay), (bx, by)) = (point(a), point(b));
        (bx - ax).hypot(by - ay)
    };
    let closed = |gap: usize| gaps[gap].points.first() == gaps[gap].points.last();
    // The point of a closed gap line nearest `to`, where it is begun.
    let nearest = |gap: usize, to: Point2| {
        *gaps[gap]
            .points
            .iter()
            .min_by(|a, b| apart(**a, to).total_cmp(&apart(**b, to)))
            .expect("a gap line has points")
    };
    #[derive(Clone, Copy)]
    struct Item {
        enter: Point2,
        leave: Point2,
        gap: Option<usize>,
        reversed: bool,
    }
    let travel = |sequence: &[Item]| {
        let mut sum = 0.0;
        let mut at = sequence[0].leave;
        for item in &sequence[1..] {
            let ring = item.gap.filter(|&gap| closed(gap));
            let enter = ring.map_or(item.enter, |gap| nearest(gap, at));
            sum += apart(at, enter);
            at = if ring.is_some() { enter } else { item.leave };
        }
        sum
    };

    // The last of the surface's lines that lies beside each gap line
    // (Orca #17223): within half the gap's widest plus a spacing of a
    // point along it, taken every half spacing.
    let spacing = flow.scaled_spacing() as f64;
    let after: Vec<Option<usize>> = gaps
        .iter()
        .map(|gap| {
            let widest = gap.widths.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if gap.widths.is_empty() {
                return None;
            }
            let reach = 0.5 * widest + spacing;
            let mut samples = Vec::new();
            for pair in gap.points.windows(2) {
                let (a, b) = (point(pair[0]), point(pair[1]));
                let length = (b.0 - a.0).hypot(b.1 - a.1);
                let steps = ((length / (0.5 * spacing)).ceil() as usize).max(1);
                for step in 0..=steps {
                    let t = step as f64 / steps as f64;
                    samples.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
                }
            }
            (0..laid.len()).rev().find(|&line| {
                laid[line].points.windows(2).any(|pair| {
                    let (a, b) = (point(pair[0]), point(pair[1]));
                    samples.iter().any(|&p| segment_distance(p, a, b) <= reach)
                })
            })
        })
        .collect();

    let mut items: Vec<Item> = laid
        .iter()
        .map(|path| Item {
            enter: path.points[0],
            leave: path.points[path.points.len() - 1],
            gap: None,
            reversed: false,
        })
        .collect();
    let mut appended = items.clone();
    for gap in 0..gaps.len() {
        let (first, last) = (
            gaps[gap].points[0],
            gaps[gap].points[gaps[gap].points.len() - 1],
        );
        appended.push(Item {
            enter: first,
            leave: last,
            gap: Some(gap),
            reversed: false,
        });
        let mut best = f64::MAX;
        let mut best_slot = 0;
        let mut best_item = None;
        let mut lines_before = 0;
        for slot in 0..=items.len() {
            if slot > 0 && items[slot - 1].gap.is_none() {
                lines_before += 1;
            }
            if after[gap].is_some_and(|after| lines_before <= after) {
                continue;
            }
            let previous = (slot > 0).then(|| items[slot - 1].leave);
            let next = items.get(slot).map(|item| item.enter);
            let mut consider = |enter: Point2, leave: Point2, reversed: bool| {
                let added = previous.map_or(0.0, |p| apart(p, enter))
                    + next.map_or(0.0, |n| apart(leave, n))
                    - match (previous, next) {
                        (Some(p), Some(n)) => apart(p, n),
                        _ => 0.0,
                    };
                if added < best {
                    best = added;
                    best_slot = slot;
                    best_item = Some(Item {
                        enter,
                        leave,
                        gap: Some(gap),
                        reversed,
                    });
                }
            };
            if closed(gap) {
                let entry = nearest(gap, previous.or(next).unwrap_or(first));
                consider(entry, entry, false);
            } else {
                consider(first, last, false);
                consider(last, first, true);
            }
        }
        if let Some(item) = best_item {
            items.insert(best_slot, item);
        }
    }
    if travel(&appended) <= travel(&items) {
        items = appended;
    }

    let mut laid = laid.into_iter();
    let mut out = Vec::with_capacity(items.len());
    for item in &items {
        match item.gap {
            None => out.extend(laid.next()),
            Some(gap) => {
                let mut line = gaps[gap].clone();
                if item.reversed {
                    line.reverse();
                }
                for mut path in
                    edge_medial::variable_width(std::slice::from_ref(&line), Role::GapFill, flow)
                {
                    // A closed line is begun where the nozzle comes in
                    // (GCode.cpp `extrude_loop` starts a loop at its
                    // point nearest the nozzle).
                    if closed(gap) && path.points.first() == path.points.last() {
                        let at = path.points.iter().position(|p| *p == item.enter);
                        if let Some(at) = at {
                            path.points.pop();
                            path.points.rotate_left(at);
                            path.points.push(path.points[0]);
                        }
                    }
                    path.part = part;
                    out.push(path);
                }
            }
        }
    }
    for (index, path) in out.iter_mut().enumerate() {
        path.in_turn = index > 0;
    }
    out
}

/// The distance from `p` to the segment from `a` to `b`.
fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

#[cfg(test)]
mod tests {
    use edge_geom::{Coord, ExPolygon, Point2, Polygon};

    use super::*;
    use crate::NarrowFill;

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

    #[test]
    fn a_grid_is_laid_the_other_way_on_odd_layers_and_kept_that_way() {
        // The same lines on an even and an odd layer (a grid does not turn
        // from layer to layer), each path run the other way on the odd one
        // (`FillGrid::fill_surface`), and marked not to be turned when
        // ordered (`params.can_reverse = false` for a grid).
        let mut grid = params(Pattern::Grid, 0.2);
        grid.sparse_link_max = 10.0;
        grid.sparse_anchor = 1.0;
        grid.origin = Point2::from_mm(20.0, 20.0);
        let square = [surface(SurfaceKind::Sparse, 40.0)];
        let even = fill(&square, 2, &grid).unwrap();
        let odd = fill(&square, 3, &grid).unwrap();
        assert!(!even.is_empty());
        assert_eq!(even.len(), odd.len());
        for (a, b) in even.iter().zip(&odd) {
            let mut turned = b.points.clone();
            turned.reverse();
            assert_eq!(a.points, turned);
        }
        assert!(even.iter().chain(&odd).all(ExtrusionPath::keeps_way));
        assert!(
            even.iter()
                .all(|path| path.part & !ExtrusionPath::KEPT_WAY == 1)
        );
        // Other patterns of crossing rows are neither turned nor kept.
        let mut triangles = grid;
        triangles.pattern = Pattern::Triangles;
        let even = fill(&square, 2, &triangles).unwrap();
        let odd = fill(&square, 3, &triangles).unwrap();
        assert_eq!(even, odd);
        assert!(!even.iter().any(ExtrusionPath::keeps_way));
    }

    #[test]
    fn a_top_of_archimedean_chords_is_a_spiral_about_its_own_middle() {
        let base = FillParams {
            top_archimedean: true,
            ..params(Pattern::Grid, 0.2)
        };
        let length = |path: &ExtrusionPath| -> f64 {
            path.points
                .windows(2)
                .map(|pair| ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64))
                .sum()
        };
        // A 30 by 10 mm top: the spiral fills the middle 10 by 10 mm whole,
        // the chords the ends.
        let top = |params: &FillParams| {
            let mut wide = surface(SurfaceKind::Top, 10.0);
            for point in &mut wide.area.contour.points {
                point.x *= 3;
            }
            fill(&[wide], 3, params).unwrap()
        };
        let any = top(&base);
        assert!(any.len() > 2, "{}", any.len());
        assert!(
            any.iter()
                .all(|path| path.role == Role::TopSolidInfill && !path.in_turn)
        );
        // On every layer the same: the chords keep their direction.
        let mut other = surface(SurfaceKind::Top, 10.0);
        for point in &mut other.area.contour.points {
            point.x *= 3;
        }
        assert_eq!(fill(&[other], 4, &base).unwrap(), any);
        // In the flow rate calibration's order: the chords, then the
        // longest piece, the spiral, last and from the middle outwards;
        // nothing reordered or turned round after.
        let calibration = top(&FillParams {
            flow_calib_order: true,
            ..base
        });
        assert_eq!(calibration.len(), any.len());
        assert!(!calibration[0].in_turn && calibration[1..].iter().all(|path| path.in_turn));
        let spiral = calibration.last().unwrap();
        let longest = calibration.iter().map(length).fold(0.0, f64::max);
        assert_eq!(length(spiral), longest);
        let middle = Point2::from_mm(15.0, 5.0);
        let off = |p: Point2| ((p.x - middle.x) as f64).hypot((p.y - middle.y) as f64) / 1e6;
        assert!(off(spiral.points[0]) < 0.5, "{}", off(spiral.points[0]));
        assert!(off(*spiral.points.last().unwrap()) > 4.0);
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
            sparse_spacing: flow.spacing,
            density,
            pattern,
            sparse_angle: 45f64.to_radians(),
            solid_angle: 45f64.to_radians(),
            top_angle: None,
            bottom_angle: None,
            top_density: 1.0,
            bottom_density: 1.0,
            under_top: None,
            smooth: None,
            alternate: true,
            sparse_link_max: 0.0,
            sparse_anchor: 0.0,
            origin: Point2::from_mm(0.0, 0.0),
            z: 0.2,
            lattice_leans: (-FRAC_PI_2 / 2.0, FRAC_PI_2 / 2.0),
            cross_step: 0.4,
            overhang_lean: 60f64.to_radians(),
            top_joined: false,
            top_archimedean: false,
            flow_calib_order: false,
            bottom_joined: true,
            solid_joined: true,
            top_ordered: true,
            bottom_ordered: true,
            solid_ordered: true,
            gap_fill: GapFillTarget::Nowhere,
            gap_shortest: 0.0,
            elephant_foot_density: 1.0,
            elephant_foot_layers: 1,
            narrow: None,
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
    fn a_direction_set_for_top_and_bottom_surfaces_holds_on_every_layer() {
        // The direction of a path's longest stretch, in degrees from 0
        // to 180.
        let direction = |paths: &[ExtrusionPath]| -> f64 {
            let mut best = (0.0, 0.0);
            for path in paths {
                for pair in path.points.windows(2) {
                    let (dx, dy) = (
                        (pair[1].x - pair[0].x) as f64,
                        (pair[1].y - pair[0].y) as f64,
                    );
                    if dx.hypot(dy) > best.0 {
                        best = (dx.hypot(dy), dy.atan2(dx).to_degrees().rem_euclid(180.0));
                    }
                }
            }
            best.1
        };
        let set = FillParams {
            top_angle: Some(70f64.to_radians()),
            bottom_angle: Some(30f64.to_radians()),
            ..params(Pattern::Grid, 0.2)
        };
        for layer in [0, 1, 2, 3] {
            let lay = |kind| fill(&[surface(kind, 20.0)], layer, &set).unwrap();
            assert!((direction(&lay(SurfaceKind::Top)) - 70.0).abs() < 0.1);
            assert!((direction(&lay(SurfaceKind::Bottom)) - 30.0).abs() < 0.1);
            // A bridge over air with no direction of its own is laid as
            // a bottom; solid fill inside keeps turning.
            assert!((direction(&lay(SurfaceKind::Bridge)) - 30.0).abs() < 0.1);
            let solid = 45.0 + if layer % 2 == 1 { 90.0 } else { 0.0 };
            assert!((direction(&lay(SurfaceKind::InternalSolid)) - solid).abs() < 0.1);
        }
        // A bridge that has a direction keeps it.
        let bridge = Surface {
            angle: Some(0.0),
            ..surface(SurfaceKind::Bridge, 20.0)
        };
        assert!(direction(&fill(&[bridge], 1, &set).unwrap()).abs() < 0.1);
        // Unset, they turn with the solid fill.
        let unset = params(Pattern::Grid, 0.2);
        let bottom = fill(&[surface(SurfaceKind::Bottom, 20.0)], 1, &unset).unwrap();
        assert!((direction(&bottom) - 135.0).abs() < 0.1);
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
    fn a_solid_join_round_a_hole_is_no_longer_than_three_spacings() {
        // A 30 mm square with a round hole 12 mm across. A row the
        // hole parts is joined round the hole's side only if that is
        // no further than three spacings. The reference joins it
        // however far, by a slip (Fill.cpp works the limit out from a
        // spacing not yet set), and so lays plastic along the hole's
        // edge beside the fill lines that end there.
        let base = params(Pattern::Grid, 0.2);
        let mut holed = surface(SurfaceKind::Bottom, 30.0);
        holed.area.holes.push(Polygon {
            points: (0..64)
                .map(|step| {
                    let turn = -(step as f64) * std::f64::consts::TAU / 64.0;
                    Point2::from_mm(15.0 + 6.0 * turn.cos(), 15.0 + 6.0 * turn.sin())
                })
                .collect(),
        });
        let lines = fill(&[holed], 0, &base).unwrap();
        let spacing = lines[0].flow.spacing;
        let length = |a: &Point2, b: &Point2| ((b.x - a.x) as f64).hypot((b.y - a.y) as f64) / 1e6;
        // The way the lines run, from the longest stretch of all.
        let (from, to) = lines
            .iter()
            .flat_map(|line| line.points.windows(2))
            .map(|pair| (pair[0], pair[1]))
            .max_by(|a, b| length(&a.0, &a.1).total_cmp(&length(&b.0, &b.1)))
            .unwrap();
        let along = (
            (to.x - from.x) as f64 / 1e6 / length(&from, &to),
            (to.y - from.y) as f64 / 1e6 / length(&from, &to),
        );
        // The longest way any path goes without running that way: a
        // join, round the square's edge or round the hole's.
        let mut longest = 0.0_f64;
        for line in &lines {
            let mut run = 0.0;
            for pair in line.points.windows(2) {
                let across = ((pair[1].x - pair[0].x) as f64 * along.1
                    - (pair[1].y - pair[0].y) as f64 * along.0)
                    / 1e6;
                if across.abs() > 1e-3 * length(&pair[0], &pair[1]) {
                    run += length(&pair[0], &pair[1]);
                    longest = longest.max(run);
                } else {
                    run = 0.0;
                }
            }
        }
        assert!(longest > 0.0);
        assert!(
            longest <= 3.0 * spacing + 1e-3,
            "{longest} mm, of {spacing}"
        );
    }

    #[test]
    fn a_bridge_is_laid_on_the_objects_own_lines_and_turns_inside_its_area() {
        // A bridge 20 mm by 3.15 mm, its lines along x, in an object
        // whose middle is the origin.
        let base = params(Pattern::Grid, 0.2);
        let spacing = base.bridge.spacing;
        let bridge = |kind| Surface {
            angle: Some(0.0),
            ..strip(kind, 0.0, 0.0, 20.0, 3.15)
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
        // At four fifths the density the lines are that much further
        // apart and lie on the lines through the object's middle. The
        // sixth is inside the area but not half a spacing inside it,
        // and is not laid (laid edge to edge, as bridges were, it was).
        let thin = FillParams {
            bridge_spacing: spacing / 0.8,
            ..base
        };
        let lines = fill(&[bridge(SurfaceKind::Bridge)], 0, &thin).unwrap();
        let pitch = (spacing * 1e6 / 0.8).trunc() / 1e6;
        let rows = rows_of(&lines);
        assert_eq!(rows.len(), 5);
        assert!(6.0 * pitch < 3.15 - 0.05 * spacing && 6.0 * pitch > 3.15 - 0.5 * spacing);
        for (index, row) in rows.iter().enumerate() {
            assert!((row - (index + 1) as f64 * pitch).abs() < 2e-6, "{row}");
        }
        // The width set, one path, and joined: it turns half a spacing
        // inside the area and its two ends reach to a twentieth of one.
        assert!(lines.iter().all(|line| line.flow == base.bridge));
        assert_eq!(lines.len(), 1);
        let points = &lines[0].points;
        let (turn, reach) = (0.5 * spacing, 0.05 * spacing);
        let at = |x: Coord, inset: f64| {
            let x = x as f64 / 1e6;
            (x - inset).abs() < 2e-6 || (x - (20.0 - inset)).abs() < 2e-6
        };
        assert!(points[1..points.len() - 1].iter().all(|p| at(p.x, turn)));
        assert!(at(points[0].x, reach) && at(points[points.len() - 1].x, reach));
        // With the object's middle elsewhere the lines move with it.
        let moved = FillParams {
            origin: Point2::from_mm(3.0, 0.1),
            ..thin
        };
        let elsewhere = rows_of(&fill(&[bridge(SurfaceKind::Bridge)], 0, &moved).unwrap());
        assert!(
            (elsewhere[0] - (0.1 + pitch)).abs() < 2e-6,
            "{}",
            elsewhere[0]
        );

        // At full density the lines lie half a pitch and a hair off
        // those through the middle: the first is the one exactly half
        // a spacing in, which the hair keeps, and seven fit.
        for kind in [SurfaceKind::Bridge, SurfaceKind::InternalBridge] {
            let lines = fill(&[bridge(kind)], 0, &base).unwrap();
            let spacing = lines[0].flow.spacing;
            let rows = rows_of(&lines);
            assert_eq!(
                rows.len(),
                ((3.15 - spacing) / spacing).floor() as usize + 1
            );
            for (index, row) in rows.iter().enumerate() {
                let expected = (index as f64 + 0.5) * spacing + 0.5e-4;
                // To within the whole units the pitch is counted in.
                assert!((row - expected).abs() < 1e-5, "{kind:?} {row} {expected}");
            }
        }

        // Not swept in order where the top surface is not (the
        // reference lays a bridge in the top surface's kind of lines),
        // and joined all the same.
        let loose = FillParams {
            top_ordered: false,
            ..thin
        };
        let lines = fill(&[bridge(SurfaceKind::Bridge)], 0, &loose).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(rows_of(&lines).len(), 5);
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
        // nothing is left unfilled along either edge: to within the
        // hair every fitted area's lines are now moved by, as the
        // reference's are (a twentieth of a micron; it was only the
        // wide part of solid fill told apart that had it), and the
        // whole units the pitch is now cut to, as there.
        let hair = 0.5e-4;
        assert!((rows[0] - (low + pitch / 2.0 + hair)).abs() < 1e-5);
        assert!((rows[rows.len() - 1] - (high - pitch / 2.0)).abs() < 1e-4);
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

        // Solid fill inside the part is fitted the same way (it kept
        // the pitch set, which the reference's does only where the
        // area is a whole number of pitches across already).
        let inside = fill(&[surface(SurfaceKind::InternalSolid, 10.0)], 0, &base).unwrap();
        assert!(inside.iter().all(|line| line.flow == flow));
        assert_eq!(rows_of(&inside), rows);
        // An area a whole number of pitches and a little across, as the
        // wide part of solid fill told from its narrow part is, keeps
        // its pitch: there is nothing to spread.
        let whole = |extra: f64| Surface {
            area: ExPolygon {
                contour: Polygon {
                    points: {
                        let high = 20.0 * base.solid.spacing + extra;
                        [(0.0, 0.0), (10.0, 0.0), (10.0, high), (0.0, high)]
                            .map(|(x, y)| Point2::from_mm(x, y))
                            .to_vec()
                    },
                },
                holes: Vec::new(),
            },
            ..surface(SurfaceKind::InternalSolid, 1.0)
        };
        let lines = fill(&[whole(1e-5)], 0, &base).unwrap();
        assert_eq!(lines[0].flow, base.solid);
        // The hair keeps the first line, half a pitch in, and drops
        // the one that would lie half a pitch from the far edge, where
        // joined lines turn.
        let kept = rows_of(&lines);
        assert_eq!(kept.len(), 19);
        assert!((kept[0] - 0.5 * base.solid.spacing).abs() < 1e-4);
        // Short of a whole number by as little, the nineteen are
        // spread over the width, each that much wider.
        let lines = fill(&[whole(-1e-5)], 0, &base).unwrap();
        assert_eq!(rows_of(&lines).len(), 19);
        assert!((lines[0].flow.spacing / base.solid.spacing - 20.0 / 19.0).abs() < 1e-4);
    }

    #[test]
    fn the_fitted_pitch_is_the_references() {
        // FillBase.cpp `_adjust_solid_spacing`, in scaled units.
        // 24.56 pitches across: 24 intervals, each a little wider.
        assert_eq!(fitted_pitch(10_000_000.0, 407_124.0), 416_666.0);
        // Exactly 20 across counts 19 intervals, and one unit over 20.
        assert_eq!(fitted_pitch(8_000_000.0, 400_000.0), 421_052.0);
        assert_eq!(fitted_pitch(8_000_001.0, 400_000.0), 400_000.0);
        // A few units over a whole number, as the wide part of solid
        // fill told apart is, changes nothing unless the lines are few.
        assert_eq!(fitted_pitch(8_000_020.0, 400_000.0), 400_000.0);
        assert_eq!(fitted_pitch(2_000_020.0, 400_000.0), 400_003.0);
        // By a fifth at most; and too narrow for two lines, as it was.
        assert_eq!(fitted_pitch(700_000.0, 400_000.0), 480_000.0);
        assert_eq!(fitted_pitch(390_000.0, 400_000.0), 400_000.0);
        assert_eq!(fitted_pitch(0.0, 400_000.0), 400_000.0);
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
    fn gap_lines_go_in_among_lines_laid_in_turn_where_they_add_least() {
        let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
        let laid = |x: f64| ExtrusionPath {
            points: vec![Point2::from_mm(x, 0.0), Point2::from_mm(x, 10.0)],
            role: Role::TopSolidInfill,
            flow,
            in_turn: x > 0.0,
            part: 1,
        };
        // Three lines up the area and a gap line beside the second, laid
        // down: it goes after the second (the last of the lines beside
        // it), turned to start where that one ends.
        let gap = edge_medial::ThickPolyline {
            points: vec![Point2::from_mm(5.3, 1.0), Point2::from_mm(5.3, 9.0)],
            widths: vec![0.2e6, 0.2e6],
            open_ends: (true, true),
        };
        let out = insert_gap_fill(vec![laid(0.0), laid(5.0), laid(30.0)], &[gap], &flow, 1);
        let roles: Vec<Role> = out.iter().map(|path| path.role).collect();
        assert_eq!(
            roles,
            [
                Role::TopSolidInfill,
                Role::TopSolidInfill,
                Role::GapFill,
                Role::TopSolidInfill
            ]
        );
        assert_eq!(out[2].points[0], Point2::from_mm(5.3, 9.0));
        assert!(!out[0].in_turn && out[1..].iter().all(|path| path.in_turn && path.part == 1));
    }

    #[test]
    fn gap_lines_go_after_lines_laid_in_turn_when_that_travels_no_further() {
        // A gap line past the end of the last line: put anywhere else it
        // would add travel, so all gap lines come after the lines, and
        // a gap line is begun at its end nearest where the lines finish
        // only when it goes in among them.
        let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
        let laid = |x: f64, up: bool| ExtrusionPath {
            points: if up {
                vec![Point2::from_mm(x, 0.0), Point2::from_mm(x, 10.0)]
            } else {
                vec![Point2::from_mm(x, 10.0), Point2::from_mm(x, 0.0)]
            },
            role: Role::TopSolidInfill,
            flow,
            in_turn: x > 0.0,
            part: 3,
        };
        let gap = edge_medial::ThickPolyline {
            points: vec![Point2::from_mm(10.3, 1.0), Point2::from_mm(10.3, 9.0)],
            widths: vec![0.2e6, 0.2e6],
            open_ends: (true, true),
        };
        let out = insert_gap_fill(
            vec![laid(0.0, true), laid(5.0, false), laid(10.0, true)],
            &[gap],
            &flow,
            3,
        );
        let roles: Vec<Role> = out.iter().map(|path| path.role).collect();
        assert_eq!(
            roles,
            [
                Role::TopSolidInfill,
                Role::TopSolidInfill,
                Role::TopSolidInfill,
                Role::GapFill
            ]
        );
        assert!(out.iter().all(|path| path.part == 3));
        assert!(out[1..].iter().all(|path| path.in_turn));
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
        // the surface they belong to, in turn with its lines.
        for line in &found {
            assert_eq!(line.part, 1);
            assert!(line.in_turn);
            assert!(line.flow.width < 0.4, "{}", line.flow.width);
            assert!(line.points.iter().all(|point| {
                point.x >= 0
                    && point.y >= 0
                    && point.y as f64 <= 4e6 - point.x as f64 * 4.0 / 30.0 + 1e3
            }));
        }
        // Where the surface's lines may be reordered, they are of its
        // part, to be ordered with them.
        let swept = FillParams {
            gap_fill: GapFillTarget::TopBottom,
            top_ordered: false,
            ..params(Pattern::Lines, 0.2)
        };
        let surface = wedge(SurfaceKind::Top);
        let inside = [surface.area.clone()];
        let swept = fill_within(&[surface], 0, &swept, &frame, &inside).unwrap();
        assert!(swept.iter().any(|path| path.role == Role::GapFill));
        assert!(swept.iter().all(|path| path.part == 1));
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
    fn the_corners_of_the_patterns_the_reference_rounds_are_rounded() {
        let area = [surface(SurfaceKind::Sparse, 30.0)];
        let points = |paths: &[ExtrusionPath]| paths.iter().map(|p| p.points.len()).sum::<usize>();
        for pattern in [
            Pattern::Honeycomb,
            Pattern::Hilbert,
            Pattern::Octagram,
            Pattern::CrossHatch,
            Pattern::Honeycomb3D,
            Pattern::Gyroid,
        ] {
            let sharp = FillParams {
                z: 2.3,
                ..params(pattern, 0.2)
            };
            let round = FillParams {
                smooth: Some((0.6, 0.01)),
                ..sharp
            };
            let (before, after) = (
                fill(&area, 3, &sharp).unwrap(),
                fill(&area, 3, &round).unwrap(),
            );
            if pattern == Pattern::Gyroid {
                // Not one the reference rounds.
                assert_eq!(before, after);
                continue;
            }
            // More vertices, cutting each corner: shorter (the octagram
            // spiral, all short legs, by near a fifth, as there).
            assert!(points(&after) > points(&before), "{pattern:?}");
            let share = length(&after) / length(&before);
            assert!(share < 1.0 && share > 0.75, "{pattern:?} {share}");
        }
    }

    #[test]
    fn solid_fill_under_a_top_is_laid_by_its_own_pattern() {
        let frame = |_: &Surface| Frame {
            origin: Point2::from_mm(0.0, 0.0),
            bounds: None,
        };
        // Separate lines, swept in order, under a top.
        let params = FillParams {
            under_top: Some((false, true)),
            ..params(Pattern::Grid, 0.2)
        };
        let solid = [surface(SurfaceKind::InternalSolid, 20.0)];
        let joined = fill(&solid, 1, &params).unwrap();
        let shifted = |dx: f64, side: f64| {
            let mut top = surface(SurfaceKind::Top, side).area;
            for point in &mut top.contour.points {
                point.x += scaled(dx);
            }
            top
        };
        let lay =
            |tops: &[ExPolygon]| fill_under_tops(&solid, tops, 1, &params, &frame, &[]).unwrap();
        // A top crossing it, and a top it lies wholly within.
        for top in [shifted(10.0, 20.0), shifted(-5.0, 40.0)] {
            let apart = lay(std::slice::from_ref(&top));
            assert!(
                apart.len() > 2 * joined.len(),
                "{} {}",
                apart.len(),
                joined.len()
            );
            assert!((length(&apart) - length(&joined)).abs() / length(&joined) < 0.05);
        }
        // A top beside it, or none, leaves it as it was.
        assert_eq!(lay(&[shifted(30.0, 5.0)]), joined);
        assert_eq!(lay(&[]), joined);
        // Unset, a top above changes nothing.
        let plain = FillParams {
            under_top: None,
            ..params
        };
        let unset =
            fill_under_tops(&solid, &[shifted(10.0, 20.0)], 1, &plain, &frame, &[]).unwrap();
        assert_eq!(unset, joined);
    }

    #[test]
    fn top_and_bottom_surfaces_short_of_full_lie_further_apart() {
        let full = params(Pattern::Lines, 0.2);
        let thin = FillParams {
            top_density: 0.6,
            bottom_density: 0.5,
            gap_fill: GapFillTarget::Everywhere,
            ..full
        };
        for (kind, share, flow) in [
            (SurfaceKind::Top, 0.6, full.top),
            (SurfaceKind::Bottom, 0.5, full.solid),
        ] {
            let area = [surface(kind, 20.0)];
            let usual = fill(&area, 1, &full).unwrap();
            let thinned = fill(&area, 1, &thin).unwrap();
            let angle = 135f64.to_radians();
            let (pitch, _) = pitch_of(&thinned, angle);
            // On the grid at the pitch over the density, each line as
            // wide as ever.
            assert!((pitch - flow.spacing / share).abs() < 1e-3, "{pitch}");
            assert!(thinned.iter().all(|line| line.flow == flow));
            let laid = length(&thinned) / length(&usual);
            assert!((laid - share).abs() < 0.04, "{kind:?} {laid}");
            // What thinned lines leave bare is not gap filled.
            let bare = fill_within(
                &area,
                1,
                &thin,
                &|_| Frame {
                    origin: thin.origin,
                    bounds: None,
                },
                &[area[0].area.clone()],
            )
            .unwrap();
            assert!(bare.iter().all(|path| path.role != Role::GapFill));
        }
        // A top of none is left to its walls.
        let none = FillParams {
            top_density: 0.0,
            ..full
        };
        assert!(
            fill(&[surface(SurfaceKind::Top, 20.0)], 1, &none)
                .unwrap()
                .is_empty()
        );
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
        // At full density the lines are fitted to the square's 28.28 mm
        // across them, 69 where 69.5 of the pitch set would go (they
        // kept the pitch set before solid fill inside the part was
        // fitted); thinned they are not.
        let across = 20.0 * 2f64.sqrt();
        let fitted = across / (across / full.solid.spacing).floor();
        assert!((usual_pitch - fitted).abs() < 1e-3, "{usual_pitch}");
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
    fn strip(kind: SurfaceKind, x0: f64, y0: f64, x1: f64, y1: f64) -> Surface {
        Surface {
            kind,
            area: ExPolygon {
                contour: Polygon {
                    points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                        .map(|(x, y)| Point2::from_mm(x, y))
                        .to_vec(),
                },
                holes: Vec::new(),
            },
            angle: None,
        }
    }

    fn telling_narrow() -> FillParams {
        FillParams {
            narrow: Some(NarrowFill {
                nozzle: 0.4,
                seam_gap: 0.06,
            }),
            ..params(Pattern::Lines, 0.2)
        }
    }

    #[test]
    fn a_narrow_solid_area_is_filled_with_loops_where_that_is_asked() {
        let narrow = [strip(SurfaceKind::InternalSolid, 0.0, 0.0, 20.0, 1.2)];
        // Left alone, the same lines as a wide area: short strokes
        // across the strip, joined at both sides.
        let lines = fill(&narrow, 0, &params(Pattern::Lines, 0.2)).unwrap();
        assert!(lines.iter().all(|path| !path.looped()));
        let strokes = lines.iter().flat_map(|path| path.points.windows(2)).count();
        assert!(strokes > 60, "{strokes}");
        // Told apart, loops along it: solid fill still, a part of
        // their own, in far fewer moves and with as much plastic.
        let loops = fill(&narrow, 0, &telling_narrow()).unwrap();
        assert!(loops.iter().all(ExtrusionPath::looped));
        assert!(loops.iter().all(|path| path.role == Role::SolidInfill));
        let moves = loops.iter().flat_map(|path| path.points.windows(2)).count();
        assert!(moves < 20, "{moves}");
        let volume =
            |paths: &[ExtrusionPath]| -> f64 { paths.iter().map(ExtrusionPath::volume_mm3).sum() };
        assert!((volume(&loops) / volume(&lines) - 1.0).abs() < 0.15);
        // Along the strip, not across it.
        let long = loops
            .iter()
            .flat_map(|path| path.points.windows(2))
            .filter(|pair| (pair[1].x - pair[0].x).abs() > 15_000_000)
            .count();
        assert!(long >= 3, "{long}");

        // A wide area is laid as it was, and so is everything that is
        // not solid fill inside the part, however narrow.
        let wide = [surface(SurfaceKind::InternalSolid, 20.0)];
        for layer in [0, 1] {
            let told = fill(&wide, layer, &telling_narrow()).unwrap();
            let lines: Vec<ExtrusionPath> =
                told.iter().filter(|path| !path.looped()).cloned().collect();
            // All but a corner, which is the next test's.
            assert!(
                length(&lines)
                    > 0.98 * length(&fill(&wide, layer, &params(Pattern::Lines, 0.2)).unwrap())
            );
        }
        for kind in [
            SurfaceKind::Top,
            SurfaceKind::Bottom,
            SurfaceKind::Bridge,
            SurfaceKind::InternalBridge,
            SurfaceKind::Sparse,
        ] {
            let other = [strip(kind, 0.0, 0.0, 20.0, 1.2)];
            assert_eq!(
                fill(&other, 0, &telling_narrow()).unwrap(),
                fill(&other, 0, &params(Pattern::Lines, 0.2)).unwrap(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn slanting_lines_leave_one_corner_of_a_solid_square_to_loops() {
        // Lines at 45 degrees across a square: the corner they end in
        // is narrow and gets a small triangle of loops, one inside the
        // other (the reference's triangle is a stroke smaller, one loop
        // and a dot: `narrow::goes`); the tip of the corner they begin
        // in is too small to hold more than a dot and gets nothing, as
        // in the reference.
        let square = [surface(SurfaceKind::InternalSolid, 20.0)];
        let told = fill(&square, 0, &telling_narrow()).unwrap();
        let loops: Vec<&ExtrusionPath> = told.iter().filter(|path| path.looped()).collect();
        assert!(!loops.is_empty());
        assert!(
            loops
                .iter()
                .flat_map(|path| &path.points)
                .all(|point| point.x > 17_500_000 && point.y < 2_500_000)
        );
        let laid: f64 = loops.iter().map(|path| path.length_mm()).sum();
        assert!((8.0..12.0).contains(&laid), "{laid}");
        // The lines keep out of that corner, and nothing else changes.
        let lines: Vec<&ExtrusionPath> = told.iter().filter(|path| !path.looped()).collect();
        assert!(
            lines
                .iter()
                .flat_map(|path| &path.points)
                .all(|point| point.x < 18_500_000 || point.y > 1_500_000)
        );
        // The wide part is a whole number of spacings across, cut along
        // its own lines: a line lies half a spacing in from where it
        // was cut off the narrow corner, and none half a spacing in
        // from its other end, as the reference has them.
        let full = telling_narrow();
        let spacing = full.solid.spacing;
        let wide = narrow::split(
            &[square[0].area.clone()],
            full.solid.scaled_spacing(),
            full.solid_angle,
        )
        .unwrap()
        .unwrap()
        .wide;
        let (low, high) = lines::extent_across(&wide[0], full.solid_angle).unwrap();
        let (sin, cos) = full.solid_angle.sin_cos();
        let mut across: Vec<f64> = lines
            .iter()
            .flat_map(|path| path.points.windows(2))
            .filter(|pair| {
                ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64) > 1e6
            })
            .map(|pair| (pair[0].y as f64 * cos - pair[0].x as f64 * sin) / 1e6)
            .collect();
        across.sort_by(f64::total_cmp);
        let (first, last) = (across[0], across[across.len() - 1]);
        assert!(((high - low) / 1e6 / spacing).fract() < 1e-3);
        assert!(
            (first - (low / 1e6 + 0.5 * spacing)).abs() < 1e-3,
            "{first}"
        );
        assert!((last - (high / 1e6 - 1.5 * spacing)).abs() < 1e-3, "{last}");
        // The loops are a part of their own, after the lines.
        assert!(lines.iter().all(|path| path.part == 1));
        assert!(
            loops
                .iter()
                .all(|path| path.part == (ExtrusionPath::LOOPED | 2))
        );
    }

    #[test]
    fn solid_areas_that_touch_are_told_apart_as_one() {
        // Two strips 2 mm wide side by side are 4 mm of solid fill,
        // which slanting lines cross in strokes 5 mm long: wide, and
        // each is filled with lines as it came. One by itself is
        // crossed in strokes of little over 2 mm, each as short as the
        // last, and is narrow.
        let both = [
            strip(SurfaceKind::InternalSolid, 0.0, 0.0, 20.0, 2.0),
            strip(SurfaceKind::InternalSolid, 0.0, 2.0, 20.0, 4.0),
        ];
        // Layer 0 has its lines at 45 degrees; the two strips' ends
        // are corners like any other, so only what is not a loop is
        // compared.
        let told = fill(&both, 0, &telling_narrow()).unwrap();
        let lines = length(
            &told
                .iter()
                .filter(|path| !path.looped())
                .cloned()
                .collect::<Vec<_>>(),
        );
        let loops = length(
            &told
                .iter()
                .filter(|path| path.looped())
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert!(lines > 10.0 * loops, "{lines} {loops}");
        let alone = fill(&both[..1], 0, &telling_narrow()).unwrap();
        assert!(alone.iter().all(ExtrusionPath::looped));
    }

    #[test]
    fn loops_keep_their_density_over_the_first_layer() {
        // Solid fill laid thinner over the first layer is the wide
        // part's lines; a narrow area's loops are laid as ever.
        let narrow = [strip(SurfaceKind::InternalSolid, 0.0, 0.0, 20.0, 1.2)];
        let thin = FillParams {
            elephant_foot_density: 0.5,
            ..telling_narrow()
        };
        assert_eq!(
            fill(&narrow, 1, &thin).unwrap(),
            fill(&narrow, 1, &telling_narrow()).unwrap()
        );
    }
}
