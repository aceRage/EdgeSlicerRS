//! What is set at a plate's layers from the layers slider (IMSlider.cpp):
//! "Add Pause", "Add Custom G-code", "Add Custom Template", "Change
//! Filament", and the ticks they leave, each with its own menu ("Edit
//! Custom G-code", "Delete Pause" and the rest).
//!
//! An edit sets the plate's items as a whole, as the reference sets
//! `Model::plates_custom_gcodes` from the slider's ticks (Plater.cpp,
//! `EVT_CUSTOMEVT_TICKSCHANGED`): the project is then unsaved and the
//! plate's slice is let go, to be sliced again with the item. As in the
//! reference, it is not a step of the undo history: undo and redo take
//! back what was done to the objects and leave the items as they are.

use std::sync::{Arc, atomic::Ordering};

use edge_config::Settings;
use edge_engine::{CustomGcode, CustomItem, CustomKind, CustomMode};
use serde_json::{Value, json};

use super::{File, Session, lock, needed, slot, text};

/// The calls answered here.
pub(super) const JSON: [&str; 3] = ["layer_codes", "layer_code_set", "layer_code_delete"];

/// An item's kind as the page names it.
fn kind_name(kind: CustomKind) -> &'static str {
    match kind {
        CustomKind::PausePrint => "pause",
        CustomKind::Custom => "custom",
        CustomKind::Template => "template",
        CustomKind::ToolChange => "filament",
        CustomKind::ColorChange => "color",
        CustomKind::Unknown => "unknown",
    }
}

/// How near a height must be to a layer's top to be taken for it, in mm.
const NEAR: f64 = 2e-3;

/// What the slider offers for a plate, from the settings it was sliced
/// with (else the printer's) and what stands on it.
struct Offers {
    /// The printer's pause G-code and template, as their text.
    pause: String,
    template: String,
    /// The project's filaments' colours, one for each filament.
    colours: Vec<String>,
    /// The filament the whole plate prints in, when it is one (the
    /// reference's `only_extruder`); none for a plate in several.
    only: Option<usize>,
    /// Printed by object: nothing can be set (`dmSequentialFffPrint`).
    sequential: bool,
    /// Spiral vase: no filament change (`m_is_spiral_vase`).
    spiral: bool,
    /// The layers' tops of the plate's slice, when it is sliced.
    tops: Vec<f64>,
}

impl Offers {
    /// "Change Filament" is offered with more than one filament
    /// (`extruder_num > 1`), and can be picked while the plate prints in
    /// one of them and is neither printed by object nor a spiral vase
    /// (GUI_Preview.cpp `update_layers_slider_mode`, `m_can_change_color`).
    fn change_filament(&self) -> Option<bool> {
        (self.colours.len() > 1).then_some(self.only.is_some() && !self.sequential && !self.spiral)
    }

    /// The mode the slider is in (`SetModeAndOnlyExtruder`): one filament
    /// in the project, or several with the plate printed in one.
    fn mode(&self) -> CustomMode {
        if self.colours.len() > 1 && self.only.is_some() {
            CustomMode::MultiAsSingle
        } else {
            CustomMode::SingleExtruder
        }
    }

    /// The filament an item is made with (`max(1, m_only_extruder)`).
    fn extruder(&self) -> i32 {
        self.only.map_or(1, |only| only as i32)
    }

    fn colour(&self, filament: i32) -> String {
        usize::try_from(filament - 1)
            .ok()
            .and_then(|at| self.colours.get(at))
            .cloned()
            .unwrap_or_default()
    }
}

fn template_text(settings: &Settings, key: &str) -> String {
    edge_gcode::gcode_template(settings, key).unwrap_or_default()
}

impl Session {
    /// The plate's file, its slice's settings and layers, and what the
    /// slider offers on it.
    fn offers(&self, args: &Value) -> Result<(u32, Option<u32>, Arc<File>, Offers), String> {
        let stage = self.stage_of(args)?;
        let file = stage
            .file
            .clone()
            .ok_or("the project is empty: there is nothing to set G-code on")?;
        let plate = match args.get("plate").and_then(Value::as_u64) {
            Some(plate) => u32::try_from(plate).ok(),
            None => stage.plate,
        };
        let done = self
            .done(&json!({ "project": stage.id, "plate": plate }))
            .ok();
        let sliced = done.as_ref().map(|(done, _)| &done.sliced);
        let settings: Option<&Settings> = sliced
            .map(|sliced| &sliced.settings)
            .or(stage.printer.as_deref());
        let texts = |key: &str| settings.map_or_else(String::new, |s| template_text(s, key));
        let on_plate: Vec<usize> =
            match plate.and_then(|id| file.plates.iter().find(|held| held.id == id)) {
                Some(held) => held.filaments.clone(),
                None => file.used.clone(),
            };
        let colours: Vec<String> = settings
            .and_then(|settings| settings.texts("filament_colour"))
            .map(<[String]>::to_vec)
            .or_else(|| {
                file.model
                    .underlying()
                    .map(|_| file.model.filament_colours())
            })
            .unwrap_or_else(|| vec!["#FFFFFF".into()]);
        let flag = |key: &str| {
            settings
                .and_then(|s| s.flag(key).ok().flatten())
                .unwrap_or(false)
        };
        let offers = Offers {
            pause: texts("machine_pause_gcode"),
            template: texts("template_custom_gcode"),
            only: match on_plate.as_slice() {
                [one] if colours.len() > 1 => Some(*one),
                _ => None,
            },
            colours,
            sequential: settings.and_then(|s| s.text("print_sequence")) == Some("by object"),
            spiral: flag("spiral_mode"),
            tops: sliced
                .map(|sliced| {
                    (0..sliced.layers.len())
                        .map(|at| sliced.top_of(at))
                        .collect()
                })
                .unwrap_or_default(),
        };
        Ok((stage.id, plate, file, offers))
    }

    /// The plate's items and what the slider offers on it.
    fn layer_codes(&self, args: &Value) -> Result<Value, String> {
        let (id, plate, file, offers) = self.offers(args)?;
        let info = file.model.custom_gcode(plate).cloned().unwrap_or_default();
        let items: Vec<Value> = info
            .items
            .iter()
            .map(|item| {
                json!({
                    "print_z": item.print_z,
                    "kind": kind_name(item.kind),
                    "extruder": item.extruder,
                    "color": item.color,
                    "extra": item.extra,
                })
            })
            .collect();
        Ok(json!({
            "project": id,
            "plate": plate,
            "items": items,
            "pause": offers.pause,
            "template": offers.template,
            "colours": offers.colours,
            "change_filament": offers.change_filament(),
            "sequential": offers.sequential,
        }))
    }

    /// Adds an item at a layer, or changes the one there (IMSlider.cpp
    /// `add_code_as_tick`, `add_custom_gcode`): custom G-code takes the
    /// layer's place whatever was there; a filament change changes a
    /// filament change there to the filament picked; a pause or a template
    /// is added only where there is nothing.
    fn layer_code_set(&self, args: &Value) -> Result<Value, String> {
        let (id, plate, file, offers) = self.offers(args)?;
        if offers.sequential {
            return Err("nothing can be set at a layer of a plate printed by object".into());
        }
        let print_z = self.layer_top(args, &offers)?;
        let mut info = file.model.custom_gcode(plate).cloned().unwrap_or_default();
        let at = info
            .items
            .iter()
            .position(|item| (item.print_z - print_z).abs() < NEAR);
        let extruder = offers.extruder();
        let made =
            |kind: CustomKind, extruder: i32, color: String, extra: String, gcode: String| {
                CustomItem {
                    print_z,
                    kind,
                    extruder,
                    color,
                    extra,
                    gcode,
                }
            };
        match needed(args, "kind")? {
            "custom" => {
                let extra = text(args, "extra").unwrap_or_default();
                if extra.is_empty() {
                    return Err("Enter Custom G-code used on current layer:".into());
                }
                if let Some(at) = at {
                    info.items.remove(at);
                }
                info.items.push(made(
                    CustomKind::Custom,
                    extruder,
                    String::new(),
                    extra.into(),
                    extra.into(),
                ));
            }
            kind @ ("pause" | "template") => {
                if at.is_some() {
                    return Err("there is already something set at this layer".into());
                }
                if kind == "pause" {
                    info.items.push(made(
                        CustomKind::PausePrint,
                        extruder,
                        String::new(),
                        String::new(),
                        offers.pause.clone(),
                    ));
                } else {
                    if offers.template.is_empty() {
                        return Err("the printer has no template (template_custom_gcode)".into());
                    }
                    info.items.push(made(
                        CustomKind::Template,
                        extruder,
                        offers.colour(extruder),
                        String::new(),
                        offers.template.clone(),
                    ));
                }
            }
            "filament" => {
                let filament = args
                    .get("extruder")
                    .and_then(Value::as_i64)
                    .ok_or("extruder was not given")? as i32;
                match offers.change_filament() {
                    Some(true) => {}
                    Some(false) => return Err("Change Filament is not available: the plate prints in more than one filament, or by object, or as a spiral vase".into()),
                    None => return Err("Change Filament needs more than one filament".into()),
                }
                if filament < 1 || filament as usize > offers.colours.len() {
                    return Err(format!("there is no filament {filament}"));
                }
                let colour = offers.colour(filament);
                match at.map(|at| &mut info.items[at]) {
                    Some(item)
                        if matches!(
                            item.kind,
                            CustomKind::ToolChange | CustomKind::ColorChange
                        ) =>
                    {
                        item.kind = CustomKind::ToolChange;
                        item.extruder = filament;
                        item.color = colour;
                    }
                    Some(_) => return Err("there is already something set at this layer".into()),
                    None => info.items.push(made(
                        CustomKind::ToolChange,
                        filament,
                        colour,
                        String::new(),
                        "tool_change".into(),
                    )),
                }
            }
            other => return Err(format!("there is no item named {other}")),
        }
        info.items.sort_by(|a, b| a.print_z.total_cmp(&b.print_z));
        info.mode = Some(offers.mode());
        self.set_codes(id, plate, &file, info)
    }

    /// Takes away the item at a layer (`delete_tick`).
    fn layer_code_delete(&self, args: &Value) -> Result<Value, String> {
        let (id, plate, file, offers) = self.offers(args)?;
        let print_z = self.layer_top(args, &offers)?;
        let mut info = file.model.custom_gcode(plate).cloned().unwrap_or_default();
        let before = info.items.len();
        info.items
            .retain(|item| (item.print_z - print_z).abs() >= NEAR);
        if info.items.len() == before {
            return Err("nothing is set at this layer".into());
        }
        info.mode = Some(offers.mode());
        self.set_codes(id, plate, &file, info)
    }

    /// The top of the layer a call names by its height (`print_z`), as the
    /// slice has it, so an item is at the layer's own height; the height
    /// as given where the plate is not sliced.
    fn layer_top(&self, args: &Value, offers: &Offers) -> Result<f64, String> {
        let print_z = args
            .get("print_z")
            .and_then(Value::as_f64)
            .filter(|z| z.is_finite() && *z > 0.0)
            .ok_or("print_z was not given")?;
        if offers.tops.is_empty() {
            return Ok(print_z);
        }
        offers
            .tops
            .iter()
            .copied()
            .find(|top| (top - print_z).abs() < NEAR)
            .ok_or_else(|| format!("no layer of the slice is at {print_z} mm"))
    }

    /// Puts a plate's items in place: the project is then unsaved, and
    /// the plate's slice is let go (PartPlate.cpp
    /// `update_slice_result_valid_state(false)` for these types).
    fn set_codes(
        &self,
        id: u32,
        plate: Option<u32>,
        file: &Arc<File>,
        info: CustomGcode,
    ) -> Result<Value, String> {
        let mut model = file.model.clone();
        model
            .set_custom_gcode(plate, info)
            .map_err(|e| e.to_string())?;
        let next = Arc::new(File::of(model));
        {
            let mut state = lock(&self.state);
            let at = state
                .index_of(id)
                .ok_or_else(|| format!("there is no project {id}"))?;
            let project = &mut state.projects[at];
            if !project
                .file
                .as_ref()
                .is_some_and(|now| Arc::ptr_eq(now, file))
            {
                return Err("the project changed meanwhile; nothing was done".into());
            }
            project.file = Some(next);
            project.results.remove(&slot(plate));
            project.failures.remove(&slot(plate));
            if let Some(work) = &project.work
                && lock(&work.progress).plate == plate
            {
                work.cancel.store(true, Ordering::Relaxed);
            }
        }
        let mut answer = self.layer_codes(&json!({ "project": id, "plate": plate }))?;
        let state = lock(&self.state);
        if let Some(at) = state.index_of(id) {
            answer["detail"] = state.projects[at].detail();
        }
        Ok(answer)
    }

    pub(super) fn ticks_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "layer_codes" => self.layer_codes(args),
            "layer_code_set" => self.layer_code_set(args),
            "layer_code_delete" => self.layer_code_delete(args),
            other => Err(format!("there is no call named {other}")),
        }
    }
}

/// A model with the items of every plate of `from` put on the plates of
/// the same number: what undo and redo do, the items not being in the
/// history. None when they are the same already.
pub(super) fn carried(from: &File, onto: &File) -> Option<File> {
    let ids: Vec<Option<u32>> = if onto.plates.is_empty() {
        vec![None]
    } else {
        onto.plates.iter().map(|plate| Some(plate.id)).collect()
    };
    let differs = ids
        .iter()
        .any(|id| from.model.custom_gcode(*id) != onto.model.custom_gcode(*id));
    if !differs {
        return None;
    }
    let mut model = onto.model.clone();
    for id in ids {
        let held = from.model.custom_gcode(id).cloned();
        if let Some(info) = held {
            model.set_custom_gcode(id, info).ok()?;
        }
    }
    Some(File::of(model))
}

#[cfg(test)]
mod tests;
