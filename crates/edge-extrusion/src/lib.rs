//! What the printer lays down: the cross-section of an extruded line and the
//! paths lines follow. Perimeters and infill produce these; G-code export
//! consumes them.

use std::f64::consts::PI;

use edge_geom::{Coord, Polygon, scaled};

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum FlowError {
    #[error("layer height {0} mm is not positive")]
    InvalidHeight(f64),
    #[error("a {width} mm line is too narrow for a {height} mm layer")]
    TooNarrow { width: f64, height: f64 },
}

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

    /// The same layer and nozzle with a different line width.
    pub fn with_width(&self, width: f64) -> Result<Self, FlowError> {
        Self::new(width, self.height, self.nozzle_diameter)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// The wall that forms the visible surface.
    ExternalPerimeter,
    /// Walls behind the external one.
    Perimeter,
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
    pub flow: Flow,
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
    fn loop_volume_is_length_times_cross_section() {
        let flow = Flow::new(0.5, 0.2, 0.4).unwrap();
        let square = Polygon {
            points: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
                .map(|(x, y)| edge_geom::Point2::from_mm(x, y))
                .to_vec(),
        };
        let wall = ExtrusionLoop {
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
