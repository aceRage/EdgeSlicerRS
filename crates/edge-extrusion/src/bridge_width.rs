//! `bridge_line_width`: a width of their own for everything laid over air
//! as a bridge (bridges, bridges over infill and overhang walls). The
//! reference's fork added it to `LayerRegion::bridging_flow`
//! (LayerRegion.cpp); zero, its default, leaves every width as it was.

use edge_config::{ConfigError, FloatOrPercent, Settings};

use crate::{BRIDGE_EXTRA_SPACING, Flow, FlowError};

/// What `bridge_line_width` comes to in mm, or none when it is left at
/// zero. A percentage is of the nozzle's diameter (the first nozzle's, as
/// everywhere here). A width below zero or over the nozzle's diameter is
/// refused, as the reference refuses it (PrintConfig.cpp `validate`,
/// Print.cpp `Print::validate`: "Bridge line width must not exceed nozzle
/// diameter").
pub fn bridge_line_width(settings: &Settings) -> Result<Option<f64>, ConfigError> {
    const KEY: &str = "bridge_line_width";
    let Some(set) = settings.float_or_percent(KEY)? else {
        return Ok(None);
    };
    let nozzle = settings
        .floats("nozzle_diameter")?
        .and_then(|nozzles| nozzles.first().copied().flatten())
        .unwrap_or(0.4);
    let (FloatOrPercent::Value(raw) | FloatOrPercent::Percent(raw)) = set;
    let width = set.resolve(nozzle);
    let bad = |expected| ConfigError::BadValue {
        key: KEY.into(),
        found: settings.text(KEY).unwrap_or_default().to_owned(),
        expected,
    };
    if raw.is_nan() || raw < 0.0 {
        return Err(bad("zero or more"));
    }
    if width > nozzle {
        return Err(bad("no more than the nozzle's diameter"));
    }
    Ok((width > 0.0).then_some(width))
}

/// Refuses a bridge width no flattened line of this layer height can
/// have (Print.cpp `Print::validate`, "Too small line width"): unless
/// both bridges and bridges over infill are round threads, which have
/// no height but their own, the width must be more than the layer's
/// height. `layer_height` is the print's, not the first layer's: nothing
/// bridges on the bed.
pub fn check_bridge_line_width(
    width: Option<f64>,
    layer_height: f64,
    thick_bridges: bool,
    thick_internal_bridges: bool,
) -> Result<(), FlowError> {
    match width {
        Some(width) if !(thick_bridges && thick_internal_bridges) && width <= layer_height => {
            Err(FlowError::TooNarrow {
                width,
                height: layer_height,
            })
        }
        _ => Ok(()),
    }
}

impl Flow {
    /// [`Flow::bridging`] with `bridge_line_width` (`width`, in mm) taken
    /// into account, as `LayerRegion::bridging_flow` does: a round thread
    /// is of that diameter in place of the nozzle's bore before `ratio`
    /// is applied, and a flattened line is first made that wide, at the
    /// layer's own height, and then given `ratio` times its plastic.
    /// With no width this is `bridging` itself.
    pub fn bridging_sized(
        &self,
        ratio: f64,
        thick: bool,
        width: Option<f64>,
    ) -> Result<Self, FlowError> {
        let Some(width) = width else {
            return Ok(self.bridging(ratio, thick));
        };
        if thick {
            let diameter = ratio.max(0.0).sqrt() * width;
            return Ok(Self {
                width: diameter,
                height: diameter,
                spacing: diameter + BRIDGE_EXTRA_SPACING,
                nozzle_diameter: self.nozzle_diameter,
            });
        }
        Ok(self.with_width(width)?.bridging(ratio, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(json: &str) -> Result<Option<f64>, ConfigError> {
        bridge_line_width(&Settings::from_json(json).unwrap())
    }

    #[test]
    fn zero_or_absent_is_no_width_and_a_percentage_is_of_the_nozzle() {
        assert_eq!(read("{}").unwrap(), None);
        assert_eq!(read(r#"{"bridge_line_width": "0"}"#).unwrap(), None);
        assert_eq!(read(r#"{"bridge_line_width": "0%"}"#).unwrap(), None);
        assert_eq!(read(r#"{"bridge_line_width": "0.3"}"#).unwrap(), Some(0.3));
        let share = read(r#"{"bridge_line_width": "75%", "nozzle_diameter": ["0.6"]}"#).unwrap();
        assert!((share.unwrap() - 0.45).abs() < 1e-12);
        // The whole of the nozzle is allowed, as a percentage or in mm.
        assert_eq!(read(r#"{"bridge_line_width": "100%"}"#).unwrap(), Some(0.4));
        assert_eq!(read(r#"{"bridge_line_width": "0.4"}"#).unwrap(), Some(0.4));
    }

    #[test]
    fn widths_the_reference_refuses_are_errors() {
        // Wider than the nozzle, below zero, or not a number.
        assert!(read(r#"{"bridge_line_width": "0.45"}"#).is_err());
        assert!(read(r#"{"bridge_line_width": "110%"}"#).is_err());
        assert!(read(r#"{"bridge_line_width": "0.5", "nozzle_diameter": ["0.6"]}"#).is_ok());
        assert!(read(r#"{"bridge_line_width": "-0.1"}"#).is_err());
        assert!(read(r#"{"bridge_line_width": "wide"}"#).is_err());
        // No wider than the layer is tall, with either kind of bridge flat.
        assert!(check_bridge_line_width(Some(0.2), 0.2, true, false).is_err());
        assert!(check_bridge_line_width(Some(0.2), 0.2, false, true).is_err());
        assert!(check_bridge_line_width(Some(0.21), 0.2, false, false).is_ok());
        // Round threads have no layer height to be thinner than.
        assert!(check_bridge_line_width(Some(0.1), 0.2, true, true).is_ok());
        assert!(check_bridge_line_width(None, 0.2, false, false).is_ok());
    }

    #[test]
    fn a_bridge_takes_the_width_set_for_it() {
        let solid = Flow::new(0.42, 0.2, 0.4).unwrap();
        // With none set, nothing changes.
        for thick in [false, true] {
            assert_eq!(
                solid.bridging_sized(0.8, thick, None).unwrap(),
                solid.bridging(0.8, thick)
            );
        }
        // A round thread of the width set, times the root of the ratio.
        let thread = solid.bridging_sized(0.81, true, Some(0.3)).unwrap();
        assert!((thread.width - 0.27).abs() < 1e-12);
        assert_eq!(thread.height, thread.width);
        assert!((thread.spacing - 0.32).abs() < 1e-12);
        // A flattened line that wide at the layer's height, then thinned.
        let flat = solid.bridging_sized(1.0, false, Some(0.3)).unwrap();
        assert_eq!(flat, Flow::new(0.3, 0.2, 0.4).unwrap());
        let thinned = solid.bridging_sized(0.8, false, Some(0.3)).unwrap();
        assert_eq!(
            thinned,
            Flow::new(0.3, 0.2, 0.4).unwrap().bridging(0.8, false)
        );
        assert!((thinned.mm3_per_mm() - 0.8 * flat.mm3_per_mm()).abs() < 1e-12);
        // A width no line of this height can have is an error.
        assert!(solid.bridging_sized(1.0, false, Some(0.04)).is_err());
    }
}
