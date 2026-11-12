//! The options beside the Arrange button (GLCanvas3D.cpp
//! `_render_arrange_menu`, `load_arrange_settings`, `ArrangeSettings`):
//! "Spacing" (0 is "auto"), "Auto rotate for arrangement", "Allow multiple
//! materials on same plate", "Avoid extrusion calibration region" (shown
//! for Bambu Lab printers that scan the first layer) and "Align to Y axis",
//! with "Reset".
//!
//! They are kept as the reference keeps them, under its keys in its
//! `arrange` section ([`super::store::Data::arrange`]): the spacing and
//! the turning each for printing by layer (`_fff`) and by object
//! (`_fff_seq_print`), the project's print sequence picking which. "Align
//! to Y axis" is not kept: the reference sets it from the printer's
//! structure whenever the printer changes (an i3's: on), and so here.

use edge_config::Settings;
use edge_engine::ArrangeOptions;
use serde_json::{Value, json};

use super::{Session, lock};

/// What the options are for: the printer and how the project prints.
struct Facts {
    /// "by object": the `_fff_seq_print` keys.
    seq_print: bool,
    /// "Avoid extrusion calibration region" is offered.
    calibration: bool,
    /// "Align to Y axis" when it has not been set: on for an i3.
    align_default: bool,
    /// The printer preset the session's "Align to Y axis" was set for.
    printer: String,
}

fn facts(global: &Settings) -> Facts {
    let text = |key: &str| global.text(key).unwrap_or_default().to_owned();
    Facts {
        seq_print: text("print_sequence") == "by object",
        calibration: text("printer_model").starts_with("Bambu Lab")
            && global.flag("scan_first_layer").ok().flatten() == Some(true),
        align_default: text("printer_structure") == "i3",
        printer: text("printer_settings_id"),
    }
}

fn keys(seq_print: bool) -> (String, String) {
    let postfix = if seq_print { "_fff_seq_print" } else { "_fff" };
    (
        format!("min_object_distance{postfix}"),
        format!("enable_rotation{postfix}"),
    )
}

const MULTI: &str = "allow_multi_materials_on_same_plate";
const AVOID: &str = "avoid_extrusion_cali_region";

fn flag(text: Option<&String>, default: bool) -> bool {
    text.map_or(default, |text| text == "1" || text == "true")
}

impl Session {
    /// The options the next arrangement of a project takes, with the
    /// settings it is made by (the project's and the presets').
    pub(super) fn arrange_choice(&self, id: u32) -> Result<(ArrangeOptions, Settings), String> {
        let global = self.items_global(id)?;
        let facts = facts(&global);
        let state = lock(&self.state);
        let kept = &state.store.data.arrange;
        let (distance, rotation) = keys(facts.seq_print);
        let spacing = kept
            .get(&distance)
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|spacing| spacing.is_finite())
            .unwrap_or(0.0)
            .max(0.0);
        let rotate = flag(kept.get(&rotation), false);
        let align = state
            .arrange_align
            .as_ref()
            .filter(|(printer, _)| *printer == facts.printer)
            .map_or(facts.align_default, |(_, align)| *align);
        Ok((
            ArrangeOptions {
                spacing,
                rotate,
                multi_materials: flag(kept.get(MULTI), true),
                // Offered only where the printer has the region; off
                // elsewhere, as the menu sets it.
                avoid_calibration: facts.calibration && flag(kept.get(AVOID), true),
                align_to_y: Some(align && !rotate),
            },
            global,
        ))
    }

    /// `arrange_options`: the options as the menu shows them.
    pub(super) fn arrange_options(&self, args: &Value) -> Result<Value, String> {
        let id = self.project_id(args)?;
        let (options, global) = self.arrange_choice(id)?;
        let facts = facts(&global);
        Ok(json!({
            "project": id,
            "spacing": options.spacing,
            "rotate": options.rotate,
            "multi_materials": options.multi_materials,
            "avoid_calibration": options.avoid_calibration,
            "calibration": facts.calibration,
            "align_to_y": options.align_to_y.unwrap_or(false),
            "seq_print": facts.seq_print,
        }))
    }

    /// `arrange_options_set`: changes the options given (`spacing`,
    /// `rotate`, `multi_materials`, `avoid_calibration`, `align_to_y`),
    /// kept at once as the menu keeps them.
    pub(super) fn arrange_options_set(&self, args: &Value) -> Result<Value, String> {
        let id = self.project_id(args)?;
        let global = self.items_global(id)?;
        let facts = facts(&global);
        let (distance, rotation) = keys(facts.seq_print);
        let spacing = match args.get("spacing") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_f64()
                    .filter(|spacing| spacing.is_finite())
                    .ok_or("spacing must be a number")?
                    // The slider's field takes no less than 0 (`max(0.f, …)`).
                    .max(0.0),
            ),
        };
        let switch = |key: &str| -> Result<Option<bool>, String> {
            match args.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(value) => value
                    .as_bool()
                    .map(Some)
                    .ok_or_else(|| format!("{key} must be true or false")),
            }
        };
        let (rotate, multi, avoid, align) = (
            switch("rotate")?,
            switch("multi_materials")?,
            switch("avoid_calibration")?,
            switch("align_to_y")?,
        );
        {
            let mut state = lock(&self.state);
            if let Some(align) = align {
                state.arrange_align = Some((facts.printer.clone(), align));
            }
            let bit = |on: bool| if on { "1" } else { "0" }.to_owned();
            state.store.change(|data| {
                if let Some(spacing) = spacing {
                    // As `float_to_string_decimal_point` writes it.
                    data.arrange.insert(distance.clone(), format!("{spacing}"));
                }
                if let Some(rotate) = rotate {
                    data.arrange.insert(rotation.clone(), bit(rotate));
                }
                if let Some(multi) = multi {
                    data.arrange.insert(MULTI.into(), bit(multi));
                }
                if let Some(avoid) = avoid {
                    data.arrange.insert(AVOID.into(), bit(avoid));
                }
            })?;
        }
        self.arrange_options(args)
    }

    /// `arrange_options_reset`: "Reset": the kept options taken away, and
    /// "Align to Y axis" the printer's own again.
    pub(super) fn arrange_options_reset(&self, args: &Value) -> Result<Value, String> {
        let id = self.project_id(args)?;
        let global = self.items_global(id)?;
        let (distance, rotation) = keys(facts(&global).seq_print);
        {
            let mut state = lock(&self.state);
            state.arrange_align = None;
            state.store.change(|data| {
                for key in [distance.as_str(), rotation.as_str(), MULTI, AVOID] {
                    data.arrange.remove(key);
                }
            })?;
        }
        self.arrange_options(args)
    }

    fn project_id(&self, args: &Value) -> Result<u32, String> {
        let state = lock(&self.state);
        let at = state.target(args)?;
        Ok(state.projects[at].id)
    }
}
