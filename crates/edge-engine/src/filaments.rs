//! The project's filaments as a list: how many there are, each one's
//! colour, and what is flushed between them; adding one, taking one away
//! (and with it what printed in it), merging one into another.
//!
//! The reference keeps these in its project config and its filament
//! presets (PresetBundle.cpp `set_num_filaments`, `update_num_filaments`,
//! `update_multi_material_filament_presets`) and renumbers the model when
//! one goes (Plater.cpp `on_filaments_delete`, PartPlate.cpp
//! `on_filament_deleted`). Here they are the model's own settings: the
//! settings a project was saved with, laid out for the filaments as they
//! now are ([`edge_config::filaments::reslot`]), so that a slice, a save
//! and taking an edit back all have them with the model. A model with no
//! settings of its own is given the few that say what its filaments are
//! (their colours and flushing volumes) the first time they change.

use std::sync::Arc;

use edge_3mf::CustomKind;
use edge_config::{Settings, Value, filaments as lists};

use super::{Bed, Model, Shape, refused};
use crate::{Failure, flush};

/// The most filaments a project may have (libslic3r.h
/// `MAXIMUM_EXTRUDER_NUMBER`, `MAXIMUM_FILAMENT_NUMBER`).
pub const MAX_FILAMENTS: usize = 64;

/// A first filament's colour when the settings give none (PrintConfig.cpp
/// `filament_colour`'s default), and any other's (PresetBundle.cpp
/// `update_multi_material_filament_presets`, `resize(n, "#26A69A")`).
const FIRST_COLOUR: &str = "#F2754E";
const OTHER_COLOUR: &str = "#26A69A";

/// What working out flushing volumes from colours needs to know of each
/// filament after the edit, from the settings a slice would read: the
/// least each flushes ([`flush::min_flush_volumes`]) and whether it is a
/// support filament (`filament_is_support`).
#[derive(Clone, Debug, Default)]
pub struct FlushBasis {
    pub minimums: Vec<i32>,
    pub support: Vec<bool>,
}

/// The reference's palette for a filament added (Plater.cpp
/// `get_next_color_for_filament`), taken in turn.
pub const NEXT_COLOURS: [&str; 16] = [
    "#00C1AE", "#F4E2C1", "#ED1C24", "#00FF7F", "#F26722", "#FFEB31", "#7841CE", "#115877",
    "#ED1E79", "#2EBDEF", "#345B2F", "#800080", "#FA8173", "#800000", "#F7B763", "#A4C41E",
];

fn numbers(settings: Option<&Settings>, key: &str) -> Vec<f64> {
    let from = |settings: &Settings| {
        settings
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| match value {
                Value::One(text) => vec![text.clone()],
                Value::Many(items) => items.clone(),
            })
    };
    settings
        .and_then(from)
        .or_else(|| from(edge_config::reference_defaults()))
        .unwrap_or_default()
        .iter()
        .filter_map(|text| text.trim().parse::<f64>().ok())
        .collect()
}

/// A number as the reference writes a float of a list: the shortest
/// text that reads back to it.
fn written(number: f64) -> String {
    let text = format!("{number}");
    text.strip_suffix(".0").map(str::to_owned).unwrap_or(text)
}

impl Model {
    /// How many filaments the project has: as many as its settings
    /// describe, and at least the highest anything is printed in.
    pub fn filament_count(&self) -> usize {
        let described = self
            .underlying()
            .map_or(1, |settings| lists::count(settings));
        let used = self.filaments().into_iter().max().unwrap_or(1);
        described.max(used)
    }

    /// Each filament's colour, `#RRGGBB` (with its alpha when it has one).
    pub fn filament_colours(&self) -> Vec<String> {
        let count = self.filament_count();
        let held: Vec<String> = self
            .underlying()
            .and_then(|settings| {
                settings
                    .iter()
                    .find(|(key, _)| *key == "filament_colour")
                    .map(|(_, value)| match value {
                        Value::One(text) => vec![text.clone()],
                        Value::Many(items) => items.clone(),
                    })
            })
            .unwrap_or_default();
        (0..count)
            .map(|at| match held.get(at) {
                Some(colour) if !colour.trim().is_empty() => colour.clone(),
                _ if at == 0 => FIRST_COLOUR.to_owned(),
                _ => OTHER_COLOUR.to_owned(),
            })
            .collect()
    }

    /// The flushing volumes, the matrix (n × n) and the vector (2n), the
    /// size of the filament list.
    pub fn flush_volumes(&self) -> (Vec<f64>, Vec<f64>) {
        let settings = self.underlying().map(|settings| &**settings);
        flush::resized(
            &numbers(settings, "flush_volumes_matrix"),
            &numbers(settings, "flush_volumes_vector"),
            self.filament_count(),
        )
    }

    /// Whether the filaments have been changed since the file was read:
    /// its own settings, as they were, no longer say what they are.
    pub fn filaments_edited(&self) -> bool {
        self.filaments_edited
    }

    /// Lays its settings out for a new list of filaments (`sources[n]` the
    /// filament that filament n + 1 was, from 0), with their colours and
    /// flushing volumes.
    fn relay(
        &mut self,
        sources: &[usize],
        number: &dyn Fn(usize) -> usize,
        colours: &[String],
        matrix: &[f64],
        vector: &[f64],
    ) {
        let mut settings = match self.underlying() {
            Some(own) => lists::reslot(own, sources, number),
            None => Settings::default(),
        };
        settings.insert("filament_colour", Value::Many(colours.to_vec()));
        if let Some(Value::Many(multi)) = settings.get("filament_multi_colors").cloned()
            && multi.len() == colours.len()
        {
            // A plain colour is its own one colour.
            let multi = multi
                .iter()
                .zip(colours)
                .map(|(held, colour)| {
                    if held.contains(' ') {
                        held.clone()
                    } else {
                        colour.clone()
                    }
                })
                .collect();
            settings.insert("filament_multi_colors", Value::Many(multi));
        }
        settings.insert(
            "flush_volumes_matrix",
            Value::Many(matrix.iter().copied().map(written).collect()),
        );
        settings.insert(
            "flush_volumes_vector",
            Value::Many(vector.iter().copied().map(written).collect()),
        );
        if self.settings.is_some() {
            self.settings = Some(Arc::new(settings));
        } else {
            self.filament_lists = Some(Arc::new(settings));
        }
        self.filaments_edited = true;
    }

    /// Adds a filament after the last ("Add one filament", Plater.cpp
    /// l.3573): it is the last one again (its preset and settings,
    /// `filament_presets.back()`) in `colour`, and what is flushed to and
    /// from it is worked out from the colours
    /// (`auto_calc_flushing_volumes`). Answers its number.
    pub fn add_filament(
        &mut self,
        bed: Option<&Bed>,
        colour: &str,
        basis: &FlushBasis,
    ) -> Result<usize, Failure> {
        let count = self.filament_count();
        if count >= MAX_FILAMENTS {
            return Err(refused(format!(
                "a project has at most {MAX_FILAMENTS} filaments"
            )));
        }
        self.settle(bed);
        let held = self
            .underlying()
            .map_or(1, |settings| lists::count(settings));
        // A filament a part prints in but the settings had no entry for
        // reads the last, as the engine reads it.
        let mut sources: Vec<usize> = (0..count).map(|at| at.min(held - 1)).collect();
        sources.push(held - 1);
        let mut colours = self.filament_colours();
        colours.push(colour.to_owned());
        let (matrix, vector) = self.flush_volumes();
        let (mut matrix, vector) = flush::resized(&matrix, &vector, count + 1);
        flush::auto_calc(
            &mut matrix,
            &colours,
            &basis.minimums,
            &basis.support,
            count,
        );
        self.relay(&sources, &|at| at, &colours, &matrix, &vector);
        Ok(count + 1)
    }

    /// Takes filament `gone` (from 1) away ("Delete", "Remove last
    /// filament"), or merges it into filament `into` ("Merge with"), as
    /// the reference does (Plater.cpp `on_filaments_delete`, l.28076):
    /// the filaments after it are numbered one less wherever they are
    /// named; what was in it prints in `into`, or, with none, in its
    /// object's filament (a part), in filament 1 (an object) or as it
    /// would unpainted (a painted colour); a plate's own order of
    /// filaments, and its filament map, lose it; a filament change to it
    /// at a layer is taken out. Its flushing volumes go with it.
    pub fn remove_filament(
        &mut self,
        bed: Option<&Bed>,
        gone: usize,
        into: Option<usize>,
    ) -> Result<(), Failure> {
        let count = self.filament_count();
        if count <= 1 {
            return Err(refused("the project's last filament cannot be deleted"));
        }
        if gone == 0 || gone > count {
            return Err(refused(format!("there is no filament {gone}")));
        }
        if let Some(into) = into
            && (into == 0 || into > count || into == gone)
        {
            return Err(refused(format!(
                "filament {gone} cannot be merged into filament {into}"
            )));
        }
        self.settle(bed);
        let renumber = |old: usize| if old > gone { old - 1 } else { old };
        // Old number from 1 to new, 0 for none.
        let map = move |old: usize| match old {
            0 => 0,
            old if old == gone => into.map_or(0, renumber),
            old => renumber(old),
        };
        let held = self
            .underlying()
            .map_or(1, |settings| lists::count(settings));
        let sources: Vec<usize> = (0..count)
            .filter(|at| *at + 1 != gone)
            .map(|at| at.min(held - 1))
            .collect();
        let mut colours = self.filament_colours();
        colours.remove(gone - 1);
        let (matrix, vector) = self.flush_volumes();
        let (matrix, vector) = flush::without(&matrix, &vector, gone - 1);
        self.relay(&sources, &map, &colours, &matrix, &vector);
        self.renumber_filaments(gone, &map);
        Ok(())
    }

    /// Sets a filament's colour (from 1), and with it what is flushed to
    /// and from it when `basis` is given (Plater.cpp
    /// `on_filament_color_changed`: "Flushing volumes: Auto-calculate every
    /// time the color changed", on unless turned off).
    pub fn set_filament_colour(
        &mut self,
        bed: Option<&Bed>,
        filament: usize,
        colour: &str,
        basis: Option<&FlushBasis>,
    ) -> Result<(), Failure> {
        let count = self.filament_count();
        if filament == 0 || filament > count {
            return Err(refused(format!("there is no filament {filament}")));
        }
        if !is_colour(colour) {
            return Err(refused(format!("{colour} is not a colour (#RRGGBB)")));
        }
        self.settle(bed);
        let held = self
            .underlying()
            .map_or(1, |settings| lists::count(settings));
        let sources: Vec<usize> = (0..count).map(|at| at.min(held - 1)).collect();
        let mut colours = self.filament_colours();
        colours[filament - 1] = colour.to_ascii_uppercase();
        let (mut matrix, vector) = self.flush_volumes();
        if let Some(basis) = basis {
            flush::auto_calc(
                &mut matrix,
                &colours,
                &basis.minimums,
                &basis.support,
                filament - 1,
            );
        }
        self.relay(&sources, &|at| at, &colours, &matrix, &vector);
        Ok(())
    }

    /// Every filament named in the model renumbered by `map` (old number
    /// from 1 to new, 0 for none).
    fn renumber_filaments(&mut self, gone: usize, map: &dyn Fn(usize) -> usize) {
        for object in &mut self.objects {
            let own = object.filament.map(map).filter(|filament| *filament > 0);
            let default = own.unwrap_or(1);
            object.filament = own;
            if let Some(kept) = &object.kept {
                object.kept = Some(Arc::new(kept.with_filaments(map)));
            }
            let parts: Vec<_> = object
                .shape
                .parts
                .iter()
                .map(|part| {
                    let mut part = part.clone();
                    part.own_filament = part.own_filament.map(map).filter(|filament| *filament > 0);
                    part.filament = part.own_filament.unwrap_or(default);
                    if let Some(paint) = &part.paint
                        && paint.iter().any(|facet| {
                            facet.filament > 0
                                && map(usize::from(facet.filament)) != usize::from(facet.filament)
                        })
                    {
                        part.paint = Some(Arc::new(
                            paint
                                .iter()
                                .map(|facet| {
                                    let mut facet = *facet;
                                    if facet.filament > 0 {
                                        facet.filament =
                                            u8::try_from(map(usize::from(facet.filament)))
                                                .unwrap_or(0);
                                    }
                                    facet
                                })
                                .collect(),
                        ));
                    }
                    // Nothing painted left is no paint, as a file without
                    // it reads.
                    if part
                        .paint
                        .as_ref()
                        .is_some_and(|paint| paint.iter().all(|facet| facet.filament == 0))
                    {
                        part.paint = None;
                    }
                    if let Some(kept) = &part.kept {
                        let renumbered = kept.with_filaments(map);
                        if renumbered != **kept {
                            part.kept = Some(Arc::new(renumbered));
                        }
                    }
                    part
                })
                .collect();
            let same = parts.iter().zip(&object.shape.parts).all(|(now, was)| {
                now.filament == was.filament
                    && now.own_filament == was.own_filament
                    && option_ptr_eq(&now.paint, &was.paint)
                    && option_ptr_eq(&now.kept, &was.kept)
            });
            if !same {
                object.shape = Shape::keeping(parts, object.shape.ids.clone());
            }
        }
        for at in 0..self.plates.len() {
            let mut own = self.plates[at].settings();
            let before = own.clone();
            // PartPlate.cpp `update_first_layer_print_sequence_when_delete_filament`:
            // the filament goes from each order and those after it move up
            // one; merged, it goes too (the reference is not told where).
            let strip = |order: &mut Vec<u32>| {
                order.retain(|filament| *filament as usize != gone);
                for filament in order.iter_mut() {
                    if *filament as usize > gone {
                        *filament -= 1;
                    }
                }
            };
            strip(&mut own.first_layer_sequence);
            for range in &mut own.other_layers_sequence {
                strip(&mut range.order);
            }
            if own != before {
                // A range whose order is left empty is gone: the reference
                // keeps it empty, which its dialog then refuses to take.
                own.other_layers_sequence
                    .retain(|range| !range.order.is_empty());
                let id = self.plates[at].id;
                // What was valid stays valid with a filament taken out.
                let _ = self.set_plate_settings(id, &own);
            }
            // PartPlate.cpp `on_filament_deleted`: its filament map loses
            // the filament's entry.
            let plate = &mut self.plates[at];
            if let Some(kept) = &plate.kept
                && let Some(maps) = kept.value("filament_maps")
            {
                let mut entries: Vec<&str> = maps.split_whitespace().collect();
                if gone <= entries.len() {
                    entries.remove(gone - 1);
                    let joined = entries.join(" ");
                    plate.kept = Some(Arc::new(kept.with("filament_maps", Some(&joined))));
                }
            }
            if let Some(info) = &mut plate.custom_gcode {
                renumber_changes(info, map);
            }
        }
        if let Some(info) = &mut self.loose_custom_gcode {
            renumber_changes(info, map);
        }
    }
}

/// A plate's filament changes at layers after a filament is taken away
/// (Plater.cpp `on_filaments_delete`, "update customize gcode"): one to
/// it is taken out, or, merged, goes to the filament it was merged into;
/// one to a later filament goes to its new number. What else is set at
/// layers names the filament in hand when it was made and is left as it
/// is, as there.
///
/// The reference, merging, sets the target on the first change to the
/// filament gone only, leaves the others in the unspecified tail
/// `std::remove_if` leaves, and then numbers that first one down once more
/// (Plater.cpp l.28187–28198); not followed.
fn renumber_changes(info: &mut edge_3mf::CustomGcode, map: &dyn Fn(usize) -> usize) {
    info.items.retain_mut(|item| {
        if item.kind != CustomKind::ToolChange || item.extruder <= 0 {
            return true;
        }
        match map(item.extruder as usize) {
            0 => false,
            new => {
                item.extruder = new as i32;
                true
            }
        }
    });
}

fn option_ptr_eq<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// Whether a text is a colour as the page sets one: `#RRGGBB`.
fn is_colour(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}
