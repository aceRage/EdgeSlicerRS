//! Ironing: a finished surface gone over again with lines close together
//! and next to no plastic, the hot nozzle smoothing what is there.
//!
//! A port of the reference's `Layer::make_ironing` (Fill.cpp): which
//! areas of a layer are ironed for each `ironing_type`, how far they are
//! kept from the layer's edge, and the lines laid over them, with the
//! line it works out for them. The settings and their limits are
//! PrintConfig.cpp's.
//!
//! Not as the reference, each said where it is done:
//! - the reference irons region by region of each object; here a layer is
//!   everything on the plate, shared out among filaments, so a filament's
//!   share of the layer stands for the region;
//! - `solid_infill_rotate_template` and `align_infill_direction_to_model`
//!   are not followed, and say so in a note;
//! - the reference's flow calibration and its dithering of ironing
//!   through an image are not here.

use std::f64::consts::FRAC_PI_2;

use edge_clip::{ClipError, Join};
use edge_config::{ConfigError, Settings};
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::{Coord, ExPolygon, Point2, Polygon, scaled};

use crate::{
    Shells, Surface, SurfaceKind,
    fill::{LINES_INSET, TURN_INSET},
    lines,
    surfaces::{self, Faces},
};

/// The closest ironing lines may lie, in mm: anything tighter is an
/// unprintable number of lines (PrintConfig.hpp `IRONING_SPACING_MIN`).
const SPACING_MIN: f64 = 0.05;
/// Ironing lines are joined along the edge of their area when it runs no
/// further than this many line spacings between them (Fill.cpp
/// `make_ironing`, `link_max_length`).
const LINK_SPACINGS: f64 = 3.0;
/// Added to areas about to be united, so that edges meant to coincide
/// overlap (ClipperUtils.hpp `ClipperSafetyOffset`).
const SAFETY_OFFSET: Coord = 10;
/// libslic3r.h `SCALED_EPSILON`, by half of which the first line of a
/// solid fill is moved (FillRectilinear.cpp `fill_surface_by_lines`).
const SCALED_EPSILON: f64 = 100.0;
/// Above this sparse density, in percent, sparse fill counts as solid
/// when deciding whether a whole layer can be ironed (Fill.cpp).
const SOLID_DENSITY: f64 = 95.0;

/// Which surfaces are ironed (`ironing_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IroningType {
    /// `no ironing`.
    None,
    /// `top`: every top surface, at whatever height.
    Top,
    /// `topmost`: the last layer of each object only.
    Topmost,
    /// `solid`: every solid layer, and the solid part of the others.
    Solid,
}

/// The lines ironing is done with (`ironing_pattern`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IroningPattern {
    /// Straight lines swept from one side of the area to the other.
    Rectilinear,
    /// Loops that follow the edge of the area, from the outside in.
    Concentric,
}

/// The settings ironing depends on, read once per print. Defaults and
/// limits are the reference's (PrintConfig.cpp).
#[derive(Clone, Debug, PartialEq)]
pub struct IroningSettings {
    pub kind: IroningType,
    pub pattern: IroningPattern,
    /// Plastic in an ironing line, as a percentage of a line of the
    /// layer's height (`ironing_flow`).
    pub flow: f64,
    /// Distance between ironing lines, in mm (`ironing_spacing`).
    pub spacing: f64,
    /// How far ironing keeps from the layer's edge, in mm; zero stands
    /// for half the nozzle's diameter (`ironing_inset`).
    pub inset: f64,
    /// Degrees added to the top surface's direction, or with `angle_fixed`
    /// the direction itself (`ironing_angle`, `ironing_angle_fixed`).
    pub angle: f64,
    pub angle_fixed: bool,
    /// The direction solid lines run on even layers, in degrees, which
    /// ironing starts from (`solid_infill_direction`).
    pub solid_infill_direction: f64,
    /// A direction of its own for top surfaces, in degrees, which ironing
    /// then starts from instead and keeps on every layer
    /// (`top_layer_direction`; unset when negative).
    pub top_layer_direction: Option<f64>,
    /// Whether the lines keep one direction on every layer, as Z
    /// contouring may ask ([`crate::InfillSettings::steady_direction`]).
    pub steady_direction: bool,
    pub top_shell_layers: usize,
    pub bottom_shell_layers: usize,
    /// Sparse fill density in percent.
    pub sparse_density: f64,
    /// The nozzle each filament is printed through, in mm.
    pub nozzle_diameters: Vec<f64>,
    /// What a filament sets in place of `flow`, `spacing` and `inset`,
    /// by filament from the first; `None` where it sets nothing
    /// (`filament_ironing_flow`, `filament_ironing_spacing`,
    /// `filament_ironing_inset`). The speed's override is the writer's.
    pub filament_flow: Vec<Option<f64>>,
    pub filament_spacing: Vec<Option<f64>>,
    pub filament_inset: Vec<Option<f64>>,
    /// What the settings ask of ironing that is not done, each with what
    /// is done instead. Empty when nothing is ironed.
    pub unsupported: Vec<String>,
}

impl Default for IroningSettings {
    fn default() -> Self {
        Self {
            kind: IroningType::None,
            pattern: IroningPattern::Rectilinear,
            flow: 10.0,
            spacing: 0.1,
            inset: 0.0,
            angle: 0.0,
            angle_fixed: false,
            solid_infill_direction: 45.0,
            top_layer_direction: None,
            steady_direction: false,
            top_shell_layers: 4,
            bottom_shell_layers: 3,
            sparse_density: 20.0,
            nozzle_diameters: vec![0.4],
            filament_flow: Vec::new(),
            filament_spacing: Vec::new(),
            filament_inset: Vec::new(),
            unsupported: Vec::new(),
        }
    }
}

/// A number within the limits the reference gives its setting.
fn limited(key: &str, found: f64, most: f64, expected: &'static str) -> Result<f64, ConfigError> {
    if (0.0..=most).contains(&found) {
        Ok(found)
    } else {
        Err(ConfigError::BadValue {
            key: key.into(),
            found: found.to_string(),
            expected,
        })
    }
}

impl IroningSettings {
    /// Reads what is present; anything absent keeps its default. A value
    /// outside the limits the reference gives a setting is an error.
    pub fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let kind = match settings.text("ironing_type") {
            None => defaults.kind,
            Some("no ironing") => IroningType::None,
            Some("top") => IroningType::Top,
            Some("topmost") => IroningType::Topmost,
            Some("solid") => IroningType::Solid,
            Some(other) => {
                return Err(ConfigError::BadValue {
                    key: "ironing_type".into(),
                    found: other.to_owned(),
                    expected: "no ironing, top, topmost or solid",
                });
            }
        };
        let pattern = match settings.text("ironing_pattern") {
            None => defaults.pattern,
            // "zig-zag" is how other slicers' files spell it
            // (PrintConfig.cpp `handle_legacy`).
            Some("rectilinear" | "zig-zag") => IroningPattern::Rectilinear,
            Some("concentric") => IroningPattern::Concentric,
            Some(other) => {
                return Err(ConfigError::BadValue {
                    key: "ironing_pattern".into(),
                    found: other.to_owned(),
                    expected: "rectilinear or concentric",
                });
            }
        };
        // A percentage whether or not the sign is written.
        let percent = |key: &str, text: &str| -> Result<f64, ConfigError> {
            let found = text
                .trim()
                .trim_end_matches('%')
                .parse::<f64>()
                .ok()
                .filter(|found| found.is_finite())
                .ok_or_else(|| ConfigError::BadValue {
                    key: key.into(),
                    found: text.to_owned(),
                    expected: "a percentage",
                })?;
            limited(key, found, 100.0, "a percentage from 0 to 100")
        };
        let flow = match settings.text("ironing_flow") {
            Some(text) => percent("ironing_flow", text)?,
            None => defaults.flow,
        };
        let spacing = match settings.float("ironing_spacing")? {
            Some(found) => limited("ironing_spacing", found, 1.0, "from 0 to 1 mm")?,
            None => defaults.spacing,
        };
        let inset = match settings.float("ironing_inset")? {
            Some(found) => limited("ironing_inset", found, 100.0, "from 0 to 100 mm")?,
            None => defaults.inset,
        };
        // Before it was an angle added to the top surface's, a negative
        // angle stood for "as the fill"; it reads as none added
        // (PrintConfig.cpp `handle_legacy`).
        let angle = settings
            .float("ironing_angle")?
            .map_or(defaults.angle, |found| found.max(0.0));
        // What a filament sets in place of the process's value; `nil`
        // where it sets nothing.
        let overrides = |key: &str,
                         read: &dyn Fn(&str) -> Result<f64, ConfigError>|
         -> Result<Vec<Option<f64>>, ConfigError> {
            settings
                .texts(key)
                .unwrap_or(&[])
                .iter()
                .map(|text| match text.trim() {
                    "nil" | "" => Ok(None),
                    text => read(text).map(Some),
                })
                .collect()
        };
        let number = |key: &'static str, most: f64, expected: &'static str| {
            move |text: &str| -> Result<f64, ConfigError> {
                let found = text
                    .parse::<f64>()
                    .ok()
                    .filter(|found| found.is_finite())
                    .ok_or_else(|| ConfigError::BadValue {
                        key: key.into(),
                        found: text.to_owned(),
                        expected: "a number",
                    })?;
                limited(key, found, most, expected)
            }
        };
        let count = |key: &str, default: usize| -> Result<usize, ConfigError> {
            settings.integer(key)?.map_or(Ok(default), |found| {
                usize::try_from(found).map_err(|_| ConfigError::BadValue {
                    key: key.into(),
                    found: found.to_string(),
                    expected: "zero or more",
                })
            })
        };
        let mut unsupported = Vec::new();
        if kind != IroningType::None {
            // The reference turns ironing with the solid fill where that
            // is turned by a template; solid fill is not turned by one
            // here, and neither is ironing.
            if settings
                .text("solid_infill_rotate_template")
                .is_some_and(|template| !template.trim().is_empty())
            {
                unsupported.push(
                    "solid_infill_rotate_template: ironing starts from solid_infill_direction and turns a quarter on every other layer"
                        .to_owned(),
                );
            }
            if settings.flag("align_infill_direction_to_model")? == Some(true) {
                unsupported.push(
                    "align_infill_direction_to_model \"1\": ironing is turned by the bed's axes, not the model's"
                        .to_owned(),
                );
            }
        }
        Ok(Self {
            kind,
            pattern,
            flow,
            spacing,
            inset,
            angle,
            angle_fixed: settings
                .flag("ironing_angle_fixed")?
                .unwrap_or(defaults.angle_fixed),
            solid_infill_direction: settings
                .float("solid_infill_direction")?
                .unwrap_or(defaults.solid_infill_direction),
            top_layer_direction: settings
                .float("top_layer_direction")?
                .filter(|direction| *direction >= 0.0),
            steady_direction: crate::steady_direction(settings)?,
            top_shell_layers: count("top_shell_layers", defaults.top_shell_layers)?,
            bottom_shell_layers: count("bottom_shell_layers", defaults.bottom_shell_layers)?,
            sparse_density: match settings.float_or_percent("sparse_infill_density")? {
                Some(
                    edge_config::FloatOrPercent::Percent(percent)
                    | edge_config::FloatOrPercent::Value(percent),
                ) => percent,
                None => defaults.sparse_density,
            },
            nozzle_diameters: settings
                .floats("nozzle_diameter")?
                .map(|nozzles| nozzles.into_iter().flatten().collect::<Vec<f64>>())
                .filter(|nozzles| !nozzles.is_empty())
                .unwrap_or(defaults.nozzle_diameters),
            filament_flow: overrides("filament_ironing_flow", &|text| {
                percent("filament_ironing_flow", text)
            })?,
            filament_spacing: overrides(
                "filament_ironing_spacing",
                &number("filament_ironing_spacing", 1.0, "from 0 to 1 mm"),
            )?,
            filament_inset: overrides(
                "filament_ironing_inset",
                &number("filament_ironing_inset", 100.0, "from 0 to 100 mm"),
            )?,
            unsupported,
        })
    }

    /// Whether anything is ironed at all.
    pub fn enabled(&self) -> bool {
        self.kind != IroningType::None
    }

    /// Ironing as filament `filament` (from 1) does it on a print whose
    /// layers are `layer_height` thick: its own spacing, inset and flow
    /// where it sets them, else the process's (PrintConfig.hpp
    /// `ironing_filament_override`), and the line that follows from them.
    ///
    /// The line is the reference's (Fill.cpp `make_ironing`): as tall as
    /// `flow` percent of the print's layer height, whichever layer it is
    /// on, shared out over the line spacing by the nozzle's bore
    /// (`height * spacing / nozzle`), and as wide as leaves it the
    /// nozzle's bore between its rounded edges, so that it carries
    /// `nozzle * that height` of plastic a millimetre.
    pub fn for_filament(&self, filament: usize, layer_height: f64) -> Ironing {
        let own = |overrides: &[Option<f64>], process: f64| {
            filament
                .checked_sub(1)
                .and_then(|index| overrides.get(index).copied().flatten())
                .unwrap_or(process)
        };
        let nozzle = filament
            .checked_sub(1)
            .and_then(|index| self.nozzle_diameters.get(index))
            .or(self.nozzle_diameters.first())
            .copied()
            .unwrap_or(0.4);
        let spacing = own(&self.filament_spacing, self.spacing).max(SPACING_MIN);
        let inset = own(&self.filament_inset, self.inset);
        let height = layer_height * 0.01 * own(&self.filament_flow, self.flow) * spacing / nozzle;
        // Put together by hand: a flow of nothing at all is allowed, the
        // nozzle then only being drawn over the surface, and `Flow::new`
        // has no such line.
        let flow = Flow {
            width: nozzle + height * (1.0 - 0.25 * std::f64::consts::PI),
            height,
            spacing: nozzle,
            nozzle_diameter: nozzle,
        };
        let top_set = self.top_layer_direction.is_some();
        let base = if self.angle_fixed {
            0.0
        } else {
            self.top_layer_direction
                .unwrap_or(self.solid_infill_direction)
        };
        Ironing {
            kind: self.kind,
            pattern: self.pattern,
            spacing,
            // "Trim the top surfaces with half the nozzle diameter."
            edge: if inset == 0.0 { 0.5 * nozzle } else { inset },
            flow,
            angle: (base + self.angle).to_radians(),
            alternate: !(self.angle_fixed || top_set || self.steady_direction),
            top_shell_layers: self.top_shell_layers,
            bottom_shell_layers: self.bottom_shell_layers,
            sparse_density: self.sparse_density,
        }
    }
}

/// Ironing as one filament does it: see [`IroningSettings::for_filament`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ironing {
    pub kind: IroningType,
    pub pattern: IroningPattern,
    /// Distance between lines, in mm.
    pub spacing: f64,
    /// How far the ironed area keeps from the layer's edge, in mm.
    pub edge: f64,
    /// The line laid.
    pub flow: Flow,
    /// The direction lines run on even layers, in radians
    /// counter-clockwise from +x.
    pub angle: f64,
    /// Whether odd layers are turned a quarter on from that, as the
    /// solid fill is. Not with a fixed angle or a top surface direction.
    pub alternate: bool,
    pub top_shell_layers: usize,
    pub bottom_shell_layers: usize,
    /// Sparse fill density in percent.
    pub sparse_density: f64,
}

impl Ironing {
    /// The direction lines run on layer `layer_index`, in radians
    /// (Fill.cpp `make_ironing` with FillBase.cpp `_infill_direction`).
    pub fn direction(&self, layer_index: usize) -> f64 {
        if self.alternate && layer_index % 2 == 1 {
            self.angle + FRAC_PI_2
        } else {
            self.angle
        }
    }
}

/// Where each layer of a filament's share of the print faces up with
/// nothing over it, and where it faces down: what the reference types
/// its regions' slices as, walls and all, before any thought of how many
/// shell layers there are (`PrintObject::detect_surfaces_type`). `slices`
/// and `outlines` are as for [`crate::classify`], and `opening` its
/// [`Shells::opening`].
pub fn exposed(
    slices: &[&[ExPolygon]],
    outlines: &[&[ExPolygon]],
    opening: Coord,
) -> Result<Vec<Faces>, ClipError> {
    surfaces::faces(
        slices,
        outlines,
        &Shells {
            top_layers: 1,
            bottom_layers: 1,
            top_thickness: 0.0,
            bottom_thickness: 0.0,
            sparse_density: 0.0,
            minimum_sparse_area: 0.0,
            opening,
            ensure: None,
            outer_spacing: 0,
            second_bridge: None,
        },
    )
}

/// One layer of one filament's share of the print, as ironing sees it.
#[derive(Clone, Copy)]
pub struct IronedLayer<'a> {
    /// The share's cross-section as printed, walls and all.
    pub slices: &'a [ExPolygon],
    /// Where it faces up and where it faces down ([`exposed`]).
    pub faces: &'a Faces,
    /// Whether it lies on the bed: only there is a face down a bottom
    /// surface, and anywhere else a bridge, which is not ironed "on
    /// purpose, as it would likely destroy the bridges".
    pub on_bed: bool,
    /// What its fill is made of, as it is filled.
    pub surfaces: &'a [Surface],
    /// The whole of its area inside the walls.
    pub infill: &'a [ExPolygon],
    /// The outline of everything on the layer, in whatever filament, as
    /// modelled: ironing keeps back from the edge of this.
    pub outline: &'a [ExPolygon],
    /// Whether the object standing at a point has this for its last
    /// layer. The reference irons object by object, and `topmost` is the
    /// last layer of each; here a layer holds every object on the plate.
    pub ends_here: &'a dyn Fn(Point2) -> bool,
}

/// The areas of a layer to iron (Fill.cpp `make_ironing`, from "Create
/// the ironing extrusions for regions" on).
///
/// With `top` and `topmost`, where the share faces up; with `solid`,
/// the whole cross-section of a share that is solid right through, and
/// otherwise where it faces up, where it lies on the bed, and whatever
/// of its fill is solid. All of it is kept the inset from the edge of
/// the layer, and from nothing else: a top surface is ironed right up to
/// what stands on it.
pub fn ironing_areas(
    layer: &IronedLayer<'_>,
    ironing: &Ironing,
) -> Result<Vec<ExPolygon>, ClipError> {
    let (top, bottom) = layer.faces;
    if layer.slices.is_empty() {
        return Ok(Vec::new());
    }
    let everything = ironing.kind == IroningType::Solid;
    let mut areas: Vec<ExPolygon> = Vec::new();
    let mut solid_fill: Vec<ExPolygon> = Vec::new();
    match ironing.kind {
        IroningType::None => return Ok(Vec::new()),
        // "top_shell_layers > 0": with no top shell there is no top
        // surface to iron. (The reference also irons a vase's floor; no
        // vase is printed here.)
        IroningType::Top | IroningType::Topmost if ironing.top_shell_layers == 0 => {
            return Ok(Vec::new());
        }
        IroningType::Top => areas.extend(top.iter().cloned()),
        // On an object's last layer everything it has faces up.
        IroningType::Topmost => areas.extend(
            top.iter()
                .filter(|face| {
                    face.contour
                        .points
                        .first()
                        .is_some_and(|&point| (layer.ends_here)(point))
                })
                .cloned(),
        ),
        IroningType::Solid => {
            // "Check whether there is any non-solid hole in the regions."
            let sparse_is_solid = ironing.sparse_density > SOLID_DENSITY;
            let mut hollow = layer.surfaces.iter().any(|surface| match surface.kind {
                SurfaceKind::Sparse => !sparse_is_solid,
                SurfaceKind::InternalBridge => true,
                _ => false,
            });
            // At no density at all the sparse areas are not among the
            // surfaces, being left empty: they show as fill area that
            // no surface covers.
            if !hollow && ironing.sparse_density <= 0.0 {
                let covered: Vec<ExPolygon> = layer
                    .surfaces
                    .iter()
                    .map(|surface| surface.area.clone())
                    .collect();
                let bare: f64 = edge_clip::difference(layer.infill, &covered)?
                    .iter()
                    .map(ExPolygon::area_mm2)
                    .sum();
                hollow = bare > 0.01;
            }
            if !hollow {
                // "Iron everything. This is likely only good for solid
                // transparent objects." Everything is everything: a
                // bridge on such a layer is ironed with the rest, as in
                // the reference, though on any other layer it is left
                // alone.
                areas.extend(layer.slices.iter().cloned());
            } else {
                if ironing.top_shell_layers > 0 {
                    areas.extend(top.iter().cloned());
                }
                if layer.on_bed && ironing.bottom_shell_layers > 0 {
                    areas.extend(bottom.iter().cloned());
                }
                // "Add solid fill surfaces. This may not be ideal, as one
                // will not iron perimeters touching these solid fill
                // surfaces, but it is likely better than nothing."
                solid_fill.extend(
                    layer
                        .surfaces
                        .iter()
                        .filter(|surface| surface.kind == SurfaceKind::InternalSolid)
                        .map(|surface| surface.area.clone()),
                );
            }
        }
    }
    debug_assert!(everything || solid_fill.is_empty());
    if !solid_fill.is_empty() {
        areas.extend(solid_fill);
        areas = edge_clip::union_ex(&edge_clip::offset(&areas, SAFETY_OFFSET, Join::MITER)?)?;
    }
    if areas.is_empty() {
        return Ok(areas);
    }
    let within = edge_clip::offset(layer.outline, -scaled(ironing.edge), Join::MITER)?;
    edge_clip::intersection(&areas, &within)
}

/// The ironing lines over `areas`, each area's laid together and in the
/// order given (the reference's unsorted collection of extrusions for an
/// area). `origin_of` gives the middle of the object an area belongs to,
/// which its lines are placed by, as a solid fill's that is not fitted
/// to its area are. Areas are numbered from `first_part` for the writer.
pub fn iron(
    areas: &[ExPolygon],
    layer_index: usize,
    ironing: &Ironing,
    origin_of: &dyn Fn(&ExPolygon) -> Point2,
    first_part: u32,
) -> Result<Vec<ExtrusionPath>, ClipError> {
    let mut paths = Vec::new();
    for (number, area) in areas.iter().enumerate() {
        let part = first_part + number as u32;
        let origin = origin_of(area);
        let lines = match ironing.pattern {
            IroningPattern::Rectilinear => rectilinear(
                area,
                ironing.spacing,
                ironing.direction(layer_index),
                origin,
            )?,
            IroningPattern::Concentric => concentric(area, ironing.spacing, origin)?,
        };
        // "Don't sort the ironing infill lines as they are monotonicly
        // ordered."
        for (index, points) in lines.into_iter().enumerate() {
            paths.push(ExtrusionPath {
                points,
                role: Role::Ironing,
                flow: ironing.flow,
                in_turn: index > 0,
                part,
            });
        }
    }
    Ok(paths)
}

/// Straight lines `spacing` mm apart over `area`, running at `angle`,
/// swept from one side to the other so that no line is laid between two
/// already down, and joined along the area's edge where that runs no
/// further than three spacings (FillRectilinear.cpp
/// `fill_surface_by_lines`, as `make_ironing` calls it: full density,
/// monotonic, no overlap, and the spacing left as it is set, where a
/// surface's own fill is spread to fit).
///
/// The lines lie on a grid through `origin`, half a spacing off it, as
/// the reference aligns fill it does not fit to its area.
fn rectilinear(
    area: &ExPolygon,
    spacing: f64,
    angle: f64,
    origin: Point2,
) -> Result<Vec<Vec<Point2>>, ClipError> {
    let outer = edge_clip::offset(
        std::slice::from_ref(area),
        -scaled(LINES_INSET * spacing),
        Join::MITER,
    )?;
    let inner = edge_clip::offset(&outer, -scaled(TURN_INSET * spacing), Join::MITER)?;
    // "Not a single infill line fits."
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let pitch = scaled(spacing) as f64;
    let (sin, cos) = angle.sin_cos();
    let first = origin.y as f64 * cos - origin.x as f64 * sin + (pitch + SCALED_EPSILON) / 2.0;
    Ok(lines::paths_turned(
        &outer,
        &inner,
        pitch,
        first,
        angle,
        scaled(LINK_SPACINGS * spacing),
        lines::Order::Monotonic,
    ))
}

/// Loops `spacing` mm apart that follow the edge of `area`, the first
/// half a spacing inside it, from the outside in, each begun at the
/// corner nearest where the one before ended (FillConcentric.cpp
/// `_fill_surface_single`, with FillBase.cpp `fill_surface` before it).
/// The first is begun nearest `origin`, the middle of the object, which
/// is where the reference's own coordinates have their zero.
///
/// Not as the reference in one thing: where the loops fall into several
/// nests, as round two holes, the reference takes the nests in the order
/// of a shortest path through their first corners; here each is taken
/// nearest first from where the nozzle is.
fn concentric(
    area: &ExPolygon,
    spacing: f64,
    origin: Point2,
) -> Result<Vec<Vec<Point2>>, ClipError> {
    let step = scaled(spacing);
    if step <= 0 {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for start in edge_clip::offset(std::slice::from_ref(area), -step / 2, Join::MITER)? {
        // The edge itself, then what is left a spacing further in, for
        // as long as anything is: narrow necks are dropped on the way,
        // as there, by shrinking half a spacing too far and growing back.
        let mut loops: Vec<Polygon> = start.rings().cloned().collect();
        let mut last = vec![start];
        while !last.is_empty() {
            last = edge_clip::offset2(&last, -(step + step / 2), step / 2, Join::MITER)?;
            loops.extend(last.iter().flat_map(ExPolygon::rings).cloned());
        }
        loops.retain(|ring| ring.points.len() >= 3);
        // Which loop lies inside which: the smallest that holds it.
        let sizes: Vec<f64> = loops
            .iter()
            .map(|ring| ring.signed_area_mm2().abs())
            .collect();
        let parent: Vec<Option<usize>> = (0..loops.len())
            .map(|inner| {
                (0..loops.len())
                    .filter(|&outer| {
                        outer != inner
                            && sizes[outer] > sizes[inner]
                            && holds(&loops[outer], loops[inner].points[0])
                    })
                    .min_by(|&a, &b| sizes[a].total_cmp(&sizes[b]).then(a.cmp(&b)))
            })
            .collect();
        // Outside in: a loop, then all that lies within it.
        let mut at = origin;
        let mut waiting: Vec<Vec<usize>> = vec![
            (0..loops.len())
                .filter(|&ring| parent[ring].is_none())
                .collect(),
        ];
        while let Some(siblings) = waiting.last_mut() {
            if siblings.is_empty() {
                waiting.pop();
                continue;
            }
            let (nearest, corner) = siblings
                .iter()
                .enumerate()
                .map(|(place, &ring)| {
                    let (corner, apart) = nearest_corner(&loops[ring], at);
                    (apart, place, corner)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, place, corner)| (place, corner))
                .expect("a nest that is not empty");
            let ring = siblings.remove(nearest);
            // Every loop is laid counter-clockwise, from its corner
            // round to that corner again.
            let mut points = loops[ring].points.clone();
            if loops[ring].signed_area_mm2() < 0.0 {
                let kept = points[corner];
                points.reverse();
                let moved = points.iter().position(|&point| point == kept).unwrap_or(0);
                points.rotate_left(moved);
            } else {
                points.rotate_left(corner);
            }
            points.push(points[0]);
            at = points[0];
            paths.push(points);
            waiting.push(
                (0..loops.len())
                    .filter(|&inner| parent[inner] == Some(ring))
                    .collect(),
            );
        }
    }
    Ok(paths)
}

/// The corner of `ring` nearest `to`, and how far off it is.
fn nearest_corner(ring: &Polygon, to: Point2) -> (usize, f64) {
    ring.points
        .iter()
        .enumerate()
        .map(|(corner, point)| {
            (
                corner,
                ((point.x - to.x) as f64).hypot((point.y - to.y) as f64),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .unwrap_or((0, f64::INFINITY))
}

/// Whether `point` lies inside `ring`, whichever way round the ring goes.
fn holds(ring: &Polygon, point: Point2) -> bool {
    let points = &ring.points;
    let mut inside = false;
    for (index, &a) in points.iter().enumerate() {
        let b = points[(index + 1) % points.len()];
        if (a.y > point.y) != (b.y > point.y) {
            let cross =
                (b.x - a.x) as f64 * (point.y - a.y) as f64 / (b.y - a.y) as f64 + a.x as f64;
            if (point.x as f64) < cross {
                inside = !inside;
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests;
