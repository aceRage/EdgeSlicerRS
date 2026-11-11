//! How thick each layer is and where its top lies: the first layer at its
//! own height and the rest at the layer height, but for the last five
//! where `precise_z_height` has tuned them to bring the print to the
//! exact height of what is printed.

use edge_config::{ConfigError, Settings};
use edge_extrusion::{ExtrusionPath, Flow, FlowError};
use edge_slice::{ADJUSTED_LAYERS, Aligned};

/// `EPSILON` (libslic3r.h), which the reference compares heights by.
const SAME: f64 = 1e-4;

/// Every layer's height and top, in mm from the bottom of the model.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayerHeights {
    first: f64,
    rest: f64,
    count: usize,
    /// The height and top of each of the last layers, where they were
    /// changed; empty where every layer is as the settings have it.
    tail: Vec<(f64, f64)>,
}

impl LayerHeights {
    /// `count` layers, none of them changed.
    pub(crate) fn uniform(first: f64, rest: f64, count: usize) -> Self {
        Self {
            first,
            rest,
            count,
            tail: Vec::new(),
        }
    }

    fn changed(&self, index: usize) -> Option<(f64, f64)> {
        let from = self.count - self.tail.len();
        self.tail.get(index.checked_sub(from)?).copied()
    }

    /// How thick layer `index` is.
    pub(crate) fn height_of(&self, index: usize) -> f64 {
        match self.changed(index) {
            Some((height, _)) => height,
            None if index == 0 => self.first,
            None => self.rest,
        }
    }

    /// The top of layer `index`.
    pub(crate) fn top_of(&self, index: usize) -> f64 {
        match self.changed(index) {
            Some((_, top)) => top,
            None => self.first + index as f64 * self.rest,
        }
    }

    /// Every layer's height, from the bed up.
    pub(crate) fn all(&self) -> Vec<f64> {
        (0..self.count).map(|index| self.height_of(index)).collect()
    }

    /// Gives the support lines on a changed layer that layer's height:
    /// supports are laid out for layers all of one height, and a line
    /// keeps its place and its width. (How many layers a support keeps
    /// below what it holds up is counted at the usual height too; on
    /// the changed layers the gap is off by what they were changed by.)
    pub(crate) fn fit(&self, lines: &mut [Vec<ExtrusionPath>]) -> Result<(), FlowError> {
        for (index, layer) in lines.iter_mut().enumerate() {
            let Some((height, _)) = self.changed(index) else {
                continue;
            };
            for path in layer
                .iter_mut()
                .filter(|path| path.flow.height == self.rest)
            {
                path.flow = Flow::new(path.flow.width, height, path.flow.nozzle_diameter)?;
            }
        }
        Ok(())
    }
}

/// The plate as the layer heights are settled for it.
pub(crate) struct Stack<'a> {
    /// The bottom and the top of everything printed, in mm.
    pub(crate) span: (f64, f64),
    pub(crate) first_layer_height: f64,
    pub(crate) layer_height: f64,
    /// The filaments the bodies are printed in, from 1.
    pub(crate) filaments: &'a [usize],
    /// The top of each object on the plate, in mm.
    pub(crate) tops: &'a [f64],
}

/// The layers' heights for `planes`, the heights each layer is cut at.
/// With `precise_z_height` the last five are tuned so that the print
/// ends at the top of what is printed, and their planes are moved to the
/// middle of each as it is then
/// (Slicing.cpp `generate_object_layers`). Returned with the heights is
/// a note for each thing asked that is not done.
///
/// The reference settles each object's layers by that object's height.
/// Here everything on the plate is cut at the same heights, so it is the
/// tallest object that is brought to its height; a note says so where
/// another is not as tall.
pub(crate) fn settle(
    settings: &Settings,
    planes: &mut [f64],
    stack: &Stack<'_>,
) -> Result<(LayerHeights, Vec<String>), ConfigError> {
    let (first, rest) = (stack.first_layer_height, stack.layer_height);
    let mut heights = LayerHeights::uniform(first, rest, planes.len());
    let mut notes = Vec::new();
    if settings.flag("precise_z_height")? != Some(true) || planes.is_empty() {
        return Ok((heights, notes));
    }
    let (bottom, top) = stack.span;
    let (min, max) = limits(settings, rest, stack.filaments)?;
    let as_set = heights.all();
    let ends = heights.top_of(planes.len() - 1);
    let wanted = top - bottom;
    match edge_slice::align_to_height(&as_set, ends, wanted, min, max) {
        Aligned::Exact => {}
        Aligned::TooFew => notes.push(format!(
            "precise_z_height \"1\": the print has {} layer(s) and six are needed to tune the last five; \
             it ends at {ends:.3} mm where the part is {wanted:.3} mm tall",
            planes.len()
        )),
        Aligned::OutOfRange => notes.push(format!(
            "precise_z_height \"1\": the last five layers cannot take up {:.3} mm between them and stay \
             within {min} to {max} mm; the print ends at {ends:.3} mm where the part is {wanted:.3} mm tall",
            (ends - wanted).abs()
        )),
        Aligned::Adjusted(last) => {
            let from = planes.len() - ADJUSTED_LAYERS;
            let mut top = heights.top_of(from - 1);
            for (height, plane) in last.iter().zip(&mut planes[from..]) {
                top += height;
                heights.tail.push((*height, top));
                *plane = bottom + top - height / 2.0;
            }
        }
    }
    if stack.tops.iter().any(|own| (top - own).abs() >= SAME) {
        notes.push(
            "precise_z_height \"1\": everything on the plate is cut at the same heights, so only the \
             tallest object is brought to its exact height"
                .to_owned(),
        );
    }
    Ok((heights, notes))
}

/// The least and the greatest height a layer may be given: what the
/// nozzles of the filaments in use allow, and those of the supports'
/// filaments where there are supports, never narrower than the layer
/// height itself (Slicing.cpp `SlicingParameters::create_from_config`).
fn limits(
    settings: &Settings,
    layer_height: f64,
    filaments: &[usize],
) -> Result<(f64, f64), ConfigError> {
    let list = |key: &str| -> Result<Vec<f64>, ConfigError> {
        Ok(settings
            .floats(key)?
            .unwrap_or_default()
            .into_iter()
            .map(|value| value.unwrap_or(0.0))
            .collect())
    };
    let (least, greatest, nozzles) = (
        list("min_layer_height")?,
        list("max_layer_height")?,
        list("nozzle_diameter")?,
    );
    let (map, assigned) = (list("filament_map")?, list("filament_nozzle_assignment")?);
    let filament_count = settings
        .texts("filament_diameter")
        .map_or(0, <[String]>::len);
    // The nozzle a filament is printed through; filament 0 is "whichever
    // is in hand" (PrintConfig.cpp `physical_extruder_for_filament`).
    let nozzle_of = |filament: usize| -> usize {
        let Some(direct) = filament.checked_sub(1) else {
            return 0;
        };
        let named = |tools: &[f64]| {
            tools
                .get(direct)
                .map(|tool| *tool as usize)
                .filter(|tool| (1..=nozzles.len()).contains(tool))
                .map(|tool| tool - 1)
        };
        if nozzles.is_empty() {
            0
        } else if let Some(tool) = named(&assigned) {
            tool
        } else if filament_count > nozzles.len()
            && let Some(tool) = named(&map)
        {
            tool
        } else if direct < nozzles.len() {
            direct
        } else {
            0
        }
    };
    // A nozzle with no entry of its own takes the first.
    let at = |values: &[f64], nozzle: usize, otherwise: f64| {
        values
            .get(nozzle)
            .or(values.first())
            .copied()
            .unwrap_or(otherwise)
    };
    let min_of = |filament: usize| match at(&least, nozzle_of(filament), 0.0) {
        0.0 => 0.07,
        set => set.max(0.01),
    };
    let max_of = |filament: usize| {
        let nozzle = nozzle_of(filament);
        let set = match at(&greatest, nozzle, 0.0) {
            0.0 => 0.75 * at(&nozzles, nozzle, 0.4),
            set => set,
        };
        set.max(min_of(filament))
    };
    let (mut min, mut max) = (0.01_f64, f64::MAX);
    let mut narrow = |filament: usize| {
        min = min.max(min_of(filament));
        max = max.min(max_of(filament));
    };
    let supported = settings.flag("enable_support")? == Some(true)
        || settings.integer("raft_layers")?.unwrap_or(0) > 0
        || settings.integer("enforce_support_layers")?.unwrap_or(0) > 0;
    if supported {
        for key in ["support_filament", "support_interface_filament"] {
            narrow(usize::try_from(settings.integer(key)?.unwrap_or(0)).unwrap_or(0));
        }
    }
    if filaments.is_empty() {
        narrow(0);
    }
    for filament in filaments {
        narrow(*filament);
    }
    Ok((min.min(layer_height), max.max(layer_height)))
}

#[cfg(test)]
mod tests {
    use edge_extrusion::Role;
    use edge_geom::Point2;

    use super::*;

    fn read(json: &str) -> Settings {
        Settings::from_json(json).unwrap()
    }

    /// Planes for `count` layers of 0.2 mm on a first of 0.25 mm.
    fn planes(count: usize) -> Vec<f64> {
        (0..count)
            .map(|index| match index {
                0 => 0.125,
                _ => 0.25 + (index as f64 - 0.5) * 0.2,
            })
            .collect()
    }

    fn stack<'a>(top: f64, tops: &'a [f64]) -> Stack<'a> {
        Stack {
            span: (0.0, top),
            first_layer_height: 0.25,
            layer_height: 0.2,
            filaments: &[1],
            tops,
        }
    }

    const U1: &str = r#""min_layer_height": ["0.08", "0.08"], "max_layer_height": ["0.32", "0.32"],
        "nozzle_diameter": ["0.4", "0.4"]"#;

    #[test]
    fn layers_are_as_set_unless_precise_z_is_asked_for() {
        let mut cut = planes(100);
        let (heights, notes) = settle(&read("{}"), &mut cut, &stack(20.0, &[20.0])).unwrap();
        assert_eq!(heights, LayerHeights::uniform(0.25, 0.2, 100));
        assert_eq!(cut, planes(100));
        assert!(notes.is_empty());
        assert_eq!(heights.height_of(0), 0.25);
        assert_eq!(heights.height_of(99), 0.2);
        // Worked out from the layer's number, as the planes are.
        assert_eq!(heights.top_of(99), 0.25 + 99.0 * 0.2);

        let off = read(r#"{"precise_z_height": "0"}"#);
        let (heights, _) = settle(&off, &mut cut, &stack(20.0, &[20.0])).unwrap();
        assert_eq!(heights, LayerHeights::uniform(0.25, 0.2, 100));
    }

    #[test]
    fn the_last_five_layers_bring_a_cube_to_its_height() {
        // 100 layers end at 20.05 mm; the cube is 20 mm.
        let on = read(&format!(r#"{{"precise_z_height": "1", {U1}}}"#));
        let mut cut = planes(100);
        let (heights, notes) = settle(&on, &mut cut, &stack(20.0, &[20.0])).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        for index in 0..95 {
            assert_eq!(
                heights.height_of(index),
                if index == 0 { 0.25 } else { 0.2 }
            );
            assert_eq!(heights.top_of(index), 0.25 + index as f64 * 0.2);
        }
        assert_eq!(cut[..95], planes(100)[..95]);
        for (step, index) in (95..100).enumerate() {
            let top = 19.05 + 0.19 * (step + 1) as f64;
            assert!((heights.height_of(index) - 0.19).abs() < 1e-9);
            assert!((heights.top_of(index) - top).abs() < 1e-9);
            // Cut through the middle of the layer as it now is.
            assert!((cut[index] - (top - 0.095)).abs() < 1e-9);
        }
        assert!((heights.top_of(99) - 20.0).abs() < 1e-9);
        assert_eq!(heights.all().len(), 100);
    }

    #[test]
    fn planes_are_measured_from_the_bottom_of_the_model() {
        let on = read(&format!(r#"{{"precise_z_height": "1", {U1}}}"#));
        let mut cut: Vec<f64> = planes(100).iter().map(|plane| plane + 3.0).collect();
        let lifted = Stack {
            span: (3.0, 23.1),
            ..stack(0.0, &[23.1])
        };
        let (heights, _) = settle(&on, &mut cut, &lifted).unwrap();
        // 20.1 mm tall: the layers grow by 0.01 mm each.
        assert!((heights.height_of(99) - 0.21).abs() < 1e-9);
        assert!((heights.top_of(99) - 20.1).abs() < 1e-9);
        assert!((cut[99] - (23.1 - 0.105)).abs() < 1e-9);
    }

    #[test]
    fn what_cannot_be_done_is_said() {
        let on = read(&format!(r#"{{"precise_z_height": "1", {U1}}}"#));
        // Five layers: too few.
        let mut cut = planes(5);
        let (heights, notes) = settle(&on, &mut cut, &stack(1.0, &[1.0])).unwrap();
        assert_eq!(heights, LayerHeights::uniform(0.25, 0.2, 5));
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("six are needed"), "{}", notes[0]);

        // A nozzle that lays 0.2 mm at most cannot make up 0.09 mm.
        let narrow = read(
            r#"{"precise_z_height": "1", "min_layer_height": ["0.2"], "max_layer_height": ["0.2"],
                "nozzle_diameter": ["0.4"]}"#,
        );
        let mut cut = planes(100);
        let (heights, notes) = settle(&narrow, &mut cut, &stack(20.14, &[20.14])).unwrap();
        assert_eq!(heights, LayerHeights::uniform(0.25, 0.2, 100));
        assert_eq!(cut, planes(100));
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("0.090 mm"), "{}", notes[0]);

        // Two objects, one shorter: only the taller comes out exact.
        let mut cut = planes(100);
        let (_, notes) = settle(&on, &mut cut, &stack(20.0, &[20.0, 12.0])).unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("tallest object"), "{}", notes[0]);
    }

    #[test]
    fn the_range_of_heights_comes_from_the_nozzles_in_use() {
        let two = read(
            r#"{"min_layer_height": ["0.08", "0.1"], "max_layer_height": ["0.32", "0.28"],
                "nozzle_diameter": ["0.4", "0.4"], "filament_diameter": ["1.75", "1.75"]}"#,
        );
        assert_eq!(limits(&two, 0.2, &[1]).unwrap(), (0.08, 0.32));
        assert_eq!(limits(&two, 0.2, &[2]).unwrap(), (0.1, 0.28));
        // The narrowest of those in use.
        assert_eq!(limits(&two, 0.2, &[1, 2]).unwrap(), (0.1, 0.28));
        // Never narrower than the layer height itself.
        assert_eq!(limits(&two, 0.3, &[2]).unwrap(), (0.1, 0.3));
        assert_eq!(limits(&two, 0.06, &[1]).unwrap(), (0.06, 0.32));

        // Nothing set: 0.07 mm to three quarters of the nozzle.
        let bare = read(r#"{"nozzle_diameter": ["0.6"]}"#);
        let (min, max) = limits(&bare, 0.2, &[1]).unwrap();
        assert_eq!(min, 0.07);
        assert!((max - 0.45).abs() < 1e-12);
        assert_eq!(limits(&read("{}"), 0.2, &[]).unwrap().0, 0.07);

        // With supports their filaments' nozzles count too.
        let supported = read(
            r#"{"min_layer_height": ["0.08", "0.1"], "max_layer_height": ["0.32", "0.28"],
                "nozzle_diameter": ["0.4", "0.4"], "filament_diameter": ["1.75", "1.75"],
                "enable_support": "1", "support_filament": "2", "support_interface_filament": "0"}"#,
        );
        assert_eq!(limits(&supported, 0.2, &[1]).unwrap(), (0.1, 0.28));

        // Four filaments through two nozzles go by `filament_map`, and a
        // filament put on a tool by hand goes there.
        let shared = read(
            r#"{"min_layer_height": ["0.08", "0.1"], "max_layer_height": ["0.32", "0.28"],
                "nozzle_diameter": ["0.4", "0.4"], "filament_diameter": ["1.75", "1.75", "1.75", "1.75"],
                "filament_map": ["1", "2", "2", "1"]}"#,
        );
        assert_eq!(limits(&shared, 0.2, &[3]).unwrap(), (0.1, 0.28));
        assert_eq!(limits(&shared, 0.2, &[4]).unwrap(), (0.08, 0.32));
        let by_hand = read(
            r#"{"min_layer_height": ["0.08", "0.1"], "max_layer_height": ["0.32", "0.28"],
                "nozzle_diameter": ["0.4", "0.4"], "filament_diameter": ["1.75", "1.75"],
                "filament_nozzle_assignment": ["2", "1"]}"#,
        );
        assert_eq!(limits(&by_hand, 0.2, &[1]).unwrap(), (0.1, 0.28));
    }

    #[test]
    fn support_lines_on_a_changed_layer_take_its_height() {
        let on = read(&format!(r#"{{"precise_z_height": "1", {U1}}}"#));
        let mut cut = planes(100);
        let (heights, _) = settle(&on, &mut cut, &stack(20.0, &[20.0])).unwrap();
        let line = |height: f64| ExtrusionPath {
            points: vec![Point2::from_mm(0.0, 0.0), Point2::from_mm(10.0, 0.0)],
            role: Role::Support,
            flow: Flow::new(0.42, height, 0.4).unwrap(),
            in_turn: false,
            part: 0,
        };
        let mut lines = vec![Vec::new(); 100];
        lines[50] = vec![line(0.2)];
        // The last layer: a line at the usual height and a thick one
        // that is its own height whatever the layer's.
        lines[99] = vec![line(0.2), line(0.4)];
        heights.fit(&mut lines).unwrap();
        assert_eq!(lines[50][0], line(0.2));
        assert!((lines[99][0].flow.height - 0.19).abs() < 1e-9);
        assert_eq!(lines[99][0].flow.width, 0.42);
        assert_eq!(lines[99][1], line(0.4));
    }
}
