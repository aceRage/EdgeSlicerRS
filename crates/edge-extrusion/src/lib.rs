//! What the printer lays down: the cross-section of an extruded line and the
//! paths lines follow. Perimeters and infill produce these; G-code export
//! consumes them.

use std::f64::consts::PI;

use edge_config::FloatOrPercent;
use edge_geom::{Coord, Point2, Polygon, UNITS_PER_MM, scaled};

mod bridge_width;

pub use bridge_width::{bridge_line_width, check_bridge_line_width};

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum FlowError {
    #[error("layer height {0} mm is not positive")]
    InvalidHeight(f64),
    #[error("a {width} mm line is too narrow for a {height} mm layer")]
    TooNarrow { width: f64, height: f64 },
}

/// Air left between neighbouring bridge threads, in mm (Flow.hpp).
pub const BRIDGE_EXTRA_SPACING: f64 = 0.05;

/// Share of the nominal spacing two neighbouring lines may give up before
/// there is no room for both (libslic3r.h, `INSET_OVERLAP_TOLERANCE`).
pub const INSET_OVERLAP_TOLERANCE: f64 = 0.4;

/// The cross-section of one extruded line: a rectangle with semicircular
/// ends, `width` across and `height` tall. Formulas follow the reference's
/// Flow.cpp, computed in `f64` where it uses `f32`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flow {
    /// Width of the line in mm, edge to edge.
    pub width: f64,
    /// Layer height in mm.
    pub height: f64,
    /// Distance in mm between the centres of two adjacent lines. Less than
    /// the width, because the rounded edges of neighbours overlap.
    pub spacing: f64,
    pub nozzle_diameter: f64,
}

impl Flow {
    pub fn new(width: f64, height: f64, nozzle_diameter: f64) -> Result<Self, FlowError> {
        if height.is_nan() || height <= 0.0 {
            return Err(FlowError::InvalidHeight(height));
        }
        let spacing = width - height * (1.0 - 0.25 * PI);
        if spacing.is_nan() || spacing <= 0.0 {
            return Err(FlowError::TooNarrow { width, height });
        }
        Ok(Self {
            width,
            height,
            spacing,
            nozzle_diameter,
        })
    }

    /// The width used when a line width setting is left at zero.
    pub fn auto_width(nozzle_diameter: f64) -> f64 {
        1.125 * nozzle_diameter
    }

    /// Builds a flow from line width settings, in the reference's order of
    /// precedence (`PrintRegion::flow`, then `Flow::new_from_config_width`):
    /// the first layer's own width, the role's width, the general line
    /// width, and last `auto_width`. A width of zero means "not set"; a
    /// percentage is of the nozzle diameter.
    pub fn from_widths(
        widths: &LineWidths,
        role_width: FloatOrPercent,
        auto_width: f64,
        height: f64,
        first_layer: bool,
    ) -> Result<Self, FlowError> {
        let is_set = |width: FloatOrPercent| match width {
            FloatOrPercent::Value(value) | FloatOrPercent::Percent(value) => value > 0.0,
        };
        let chosen = if first_layer && is_set(widths.initial_layer_line_width) {
            widths.initial_layer_line_width
        } else if is_set(role_width) {
            role_width
        } else {
            widths.line_width
        };
        let width = if is_set(chosen) {
            chosen.resolve(widths.nozzle_diameter)
        } else {
            auto_width
        };
        Self::new(width, height, widths.nozzle_diameter)
    }

    /// The same layer and nozzle with lines this far apart, each as wide
    /// as fills that spacing (`Flow::with_spacing`).
    pub fn with_spacing(&self, spacing: f64) -> Result<Self, FlowError> {
        self.with_width(spacing + self.height * (1.0 - 0.25 * PI))
    }

    /// The same layer and nozzle with a different line width.
    pub fn with_width(&self, width: f64) -> Result<Self, FlowError> {
        Self::new(width, self.height, self.nozzle_diameter)
    }

    /// This line as it is laid over air (`LayerRegion::bridging_flow`).
    /// `thick` is the round thread of the nozzle's own bore, `ratio` times
    /// its section. Otherwise the line is given `ratio` times its plastic
    /// (`Flow::with_cross_section`): less by narrowing it, its spacing
    /// following its width, or by making it a round thread once it would
    /// be narrower than it is tall; more by letting it stand taller once
    /// it fills its spacing.
    pub fn bridging(&self, ratio: f64, thick: bool) -> Self {
        if thick {
            let diameter = ratio.max(0.0).sqrt() * self.nozzle_diameter;
            return Self {
                width: diameter,
                height: diameter,
                spacing: diameter + BRIDGE_EXTRA_SPACING,
                nozzle_diameter: self.nozzle_diameter,
            };
        }
        let area = self.mm3_per_mm();
        let wanted = area * ratio;
        if wanted > area + 1e-9 {
            if wanted / self.height > self.spacing {
                let height = wanted / self.spacing;
                return Self {
                    width: self.spacing + height * (1.0 - 0.25 * PI),
                    height,
                    ..*self
                };
            }
            return *self;
        }
        if wanted < area - 1e-9 {
            let width = self.width - (area - wanted) / self.height;
            if width > self.height {
                return Self {
                    width,
                    spacing: width - self.height * (1.0 - 0.25 * PI),
                    ..*self
                };
            }
            // As the reference has it, which takes the thread's radius
            // for its width.
            let thread = (wanted / PI).sqrt();
            return Self {
                width: thread,
                height: thread,
                ..*self
            };
        }
        *self
    }

    /// Plastic laid down per mm of travel, in mm³.
    pub fn mm3_per_mm(&self) -> f64 {
        self.height * (self.width - self.height * (1.0 - 0.25 * PI))
    }

    pub fn scaled_width(&self) -> Coord {
        scaled(self.width)
    }

    pub fn scaled_spacing(&self) -> Coord {
        scaled(self.spacing)
    }
}

/// The width settings every role falls back on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineWidths {
    /// Used wherever a role's own width is not set.
    pub line_width: FloatOrPercent,
    /// Replaces every width on the first layer when set.
    pub initial_layer_line_width: FloatOrPercent,
    pub nozzle_diameter: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// The wall that forms the visible surface.
    ExternalPerimeter,
    /// Walls behind the external one.
    Perimeter,
    /// A stretch of wall with nothing under it, printed as a bridge.
    OverhangPerimeter,
    /// Partial fill inside the part.
    SparseInfill,
    /// Full fill inside the part, backing up a top or bottom surface.
    SolidInfill,
    /// The topmost, visible fill.
    TopSolidInfill,
    /// Fill resting on the bed.
    BottomSurface,
    /// Lines laid again over a finished surface, close together and with
    /// next to no plastic, to smooth it (the reference's `erIroning`).
    Ironing,
    /// Fill laid over air.
    BridgeInfill,
    /// Solid fill laid across the sparse fill beneath it, the first layer
    /// of a shell that closes over it.
    InternalBridge,
    /// A line of whatever width fills a gap too narrow for a wall.
    GapFill,
    /// Loops around the print that prime the nozzle and show the bed is level.
    Skirt,
    /// Loops against the first layer's outline that hold the part down.
    Brim,
    /// Scaffolding under an overhang, made to break away.
    Support,
    /// The dense top of a support, which the part rests on.
    SupportInterface,
    /// The prime tower: where a nozzle just brought into use is primed.
    PrimeTower,
    /// The first line a fresh nozzle lays on the tower, put down slowly.
    PrimeLine,
}

/// An open line: the centre line the nozzle follows and what it extrudes.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtrusionPath {
    pub points: Vec<Point2>,
    pub role: Role,
    pub flow: Flow,
    /// Laid straight after the path before it in its list, and from its
    /// first point to its last: where the order lines go down in is part
    /// of the pattern, as in a surface swept from one side to the other.
    /// A path without this may be taken in any order and from either end.
    pub in_turn: bool,
    /// Which area of its layer the path helps to fill, from 1; 0 for a
    /// path that stands by itself. Paths with the same number, next to
    /// each other in a list, are laid together: the nozzle finishes one
    /// area before it goes to another (the reference's collection of
    /// extrusions for a surface). Numbers from [`Self::LOOPED`] up are
    /// kept for areas filled with loops.
    pub part: u32,
}

impl ExtrusionPath {
    /// Part numbers from here up are areas filled with loops along
    /// their outline, not with straight lines across them: the narrow
    /// solid areas of `detect_narrow_internal_solid_infill`. Their
    /// paths are solid fill in everything but how short a stretch of
    /// one may be, which says nothing of the size of the area.
    pub const LOOPED: u32 = 1 << 31;

    /// Whether the path is one of the loops an area is filled with
    /// ([`Self::LOOPED`]).
    pub fn looped(&self) -> bool {
        self.part >= Self::LOOPED
    }

    pub fn length_mm(&self) -> f64 {
        self.points
            .windows(2)
            .map(|pair| ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64))
            .sum::<f64>()
            / UNITS_PER_MM
    }

    /// Plastic in the path, in mm³.
    pub fn volume_mm3(&self) -> f64 {
        self.length_mm() * self.flow.mm3_per_mm()
    }
}

/// A closed line: the centre line the nozzle follows and what it extrudes.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtrusionLoop {
    /// Counter-clockwise around material, clockwise around a hole.
    pub polygon: Polygon,
    pub role: Role,
    /// Whether the loop walls a hole rather than an outer boundary.
    pub is_hole: bool,
    /// How many walls lie between this one and the surface; 0 is outermost.
    pub depth: usize,
    /// What the loop is laid with; for a loop whose width changes, at its
    /// average width.
    pub flow: Flow,
    /// For a loop whose width changes along it: the width at each point
    /// of `polygon`, in scaled units, as the wall generator gives it (the
    /// line that lays it is wider by what its rounded edges leave empty).
    /// Empty for a loop of one width.
    pub widths: Vec<Coord>,
    /// Laid clockwise where walls otherwise go counter-clockwise: set
    /// over a steep overhang on every other layer, so that a wall with
    /// little under it is not always pulled the same way
    /// (`overhang_reverse`). Which way round `polygon` is held does not
    /// change.
    pub clockwise: bool,
    /// The wall goes round a circle that the auto circle compensation
    /// resized, one small enough to be printed at the speed the
    /// compensation was measured at (the reference's
    /// `ExtrusionLoop::circle_compensation`).
    pub circle_compensation: bool,
}

impl ExtrusionLoop {
    pub fn length_mm(&self) -> f64 {
        self.polygon.perimeter_mm()
    }

    /// Plastic in the loop, in mm³.
    pub fn volume_mm3(&self) -> f64 {
        self.length_mm() * self.flow.mm3_per_mm()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_and_volume_follow_the_rounded_rectangle() {
        let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
        assert!((flow.spacing - 0.407_079_632_679).abs() < 1e-12);
        // Area of a 0.45 x 0.2 rectangle whose ends are semicircles.
        let area = 0.2 * (0.45 - 0.2) + PI * 0.1 * 0.1;
        assert!((flow.mm3_per_mm() - area).abs() < 1e-12);
        assert_eq!(
            (flow.scaled_width(), flow.scaled_spacing()),
            (450_000, 407_080)
        );
        assert_eq!(Flow::auto_width(0.4), 0.45);
    }

    #[test]
    fn impossible_flows_are_rejected() {
        assert_eq!(Flow::new(0.4, 0.0, 0.4), Err(FlowError::InvalidHeight(0.0)));
        assert!(matches!(
            Flow::new(0.4, f64::NAN, 0.4),
            Err(FlowError::InvalidHeight(_))
        ));
        // A line cannot be narrower than about a fifth of its height.
        assert_eq!(
            Flow::new(0.04, 0.2, 0.4),
            Err(FlowError::TooNarrow {
                width: 0.04,
                height: 0.2
            })
        );
        assert!(Flow::new(0.45, 0.2, 0.4).unwrap().with_width(0.01).is_err());
    }

    #[test]
    fn widths_fall_back_from_first_layer_to_role_to_general_to_auto() {
        let unset = FloatOrPercent::Value(0.0);
        let mut widths = LineWidths {
            line_width: unset,
            initial_layer_line_width: unset,
            nozzle_diameter: 0.4,
        };
        let width = |widths: &LineWidths, role, first| {
            Flow::from_widths(widths, role, 0.4, 0.2, first)
                .unwrap()
                .width
        };
        assert_eq!(width(&widths, unset, false), 0.4);
        widths.line_width = FloatOrPercent::Percent(150.0);
        assert!((width(&widths, unset, false) - 0.6).abs() < 1e-12);
        assert_eq!(width(&widths, FloatOrPercent::Value(0.42), false), 0.42);
        widths.initial_layer_line_width = FloatOrPercent::Value(0.5);
        assert_eq!(width(&widths, FloatOrPercent::Value(0.42), true), 0.5);
        assert_eq!(width(&widths, FloatOrPercent::Value(0.42), false), 0.42);
    }

    #[test]
    fn path_length_follows_every_segment() {
        let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
        let points = [(0.0, 0.0), (3.0, 4.0), (3.0, 10.0)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec();
        let path = ExtrusionPath {
            points,
            role: Role::SolidInfill,
            flow,
            in_turn: false,
            part: 0,
        };
        assert_eq!(path.length_mm(), 11.0);
        assert!((path.volume_mm3() - 11.0 * flow.mm3_per_mm()).abs() < 1e-12);
    }

    #[test]
    fn loop_volume_is_length_times_cross_section() {
        let flow = Flow::new(0.5, 0.2, 0.4).unwrap();
        let square = Polygon {
            points: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        };
        let wall = ExtrusionLoop {
            widths: Vec::new(),
            clockwise: false,
            circle_compensation: false,
            polygon: square,
            role: Role::ExternalPerimeter,
            is_hole: false,
            depth: 0,
            flow,
        };
        assert_eq!(wall.length_mm(), 40.0);
        assert!((wall.volume_mm3() - 40.0 * flow.mm3_per_mm()).abs() < 1e-12);
    }
}
