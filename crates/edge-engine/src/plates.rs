//! What is done to a model's plates as plates: a name, a lock, a plate's
//! own settings ("Customize current plate"), moving one to the front and
//! duplicating one with what stands on it. Adding and removing plates, and
//! arranging them, are in the model's own module.
//!
//! A plate keeps its own settings where its file keeps them, in its
//! [`PlateKept`] (`Metadata/model_settings.config`), so that a plate
//! nothing was done to is written back as it was read, and one that was
//! is written as the reference writes it (bbs_3mf.cpp
//! `_add_model_config_file_to_archive`). The reference's own: PartPlate.cpp
//! (`set_bed_type`, `set_print_seq`, `set_spiral_vase_mode`,
//! `set_first_layer_print_sequence`, `set_other_layers_print_sequence`,
//! `lock`), `PartPlateList::move_plate_to_index`, `duplicate_plate`.

use std::sync::Arc;

use edge_3mf::PlateKept;

use super::{Bed, Model, ModelCopy, ModelObject, ModelPlate, fresh, refused};
use crate::{Failure, load};

/// The longest name a plate may have (PlateSettingsDialog.cpp
/// `PlateNameEditDialog`: `SetMaxLength(250)`).
pub const LONGEST_PLATE_NAME: usize = 250;

/// The layer that stands for "End" in a range of later layers
/// (PlateSettingsDialog.cpp `MAX_LAYER_VALUE`).
pub const LAST_LAYER: u32 = i32::MAX as u32 - 1;

/// The first layer a range of later layers may start at
/// (PlateSettingsDialog.cpp `MIN_LAYER_VALUE`).
pub const FIRST_LATER_LAYER: u32 = 2;

/// A range of layers above the first and the order its filaments are
/// printed in (the reference's `LayerPrintSequence`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerSequence {
    /// The first layer, from 1 (at least [`FIRST_LATER_LAYER`]).
    pub begin: u32,
    /// The last layer; [`LAST_LAYER`] for "End".
    pub end: u32,
    /// Filament numbers from 1, in the order they are printed.
    pub order: Vec<u32>,
}

/// What a plate sets for itself over the project ("Customize current
/// plate", PlateSettingsDialog.cpp). Each left at none (or empty) is the
/// project's: "Same as Global Plate Type", "Same as Global Print
/// Sequence", "Same as Global", "Auto".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlateSettings {
    /// A bed type by the reference's name for it ("Textured PEI Plate").
    pub bed_type: Option<String>,
    /// "by layer" or "by object".
    pub print_sequence: Option<String>,
    pub spiral_mode: Option<bool>,
    /// Filament numbers from 1, in the order the first layer prints them.
    pub first_layer_sequence: Vec<u32>,
    pub other_layers_sequence: Vec<LayerSequence>,
}

impl ModelPlate {
    /// Whether the plate is locked: arranging leaves what stands on it
    /// where it is.
    pub fn locked(&self) -> bool {
        self.kept
            .as_deref()
            .and_then(|kept| kept.value("locked"))
            .is_some_and(|value| value == "true" || value == "1")
    }

    /// What the plate sets for itself.
    pub fn settings(&self) -> PlateSettings {
        let kept = self.kept.as_deref();
        let value = |key: &str| kept.and_then(|kept| kept.value(key));
        let mut settings = PlateSettings {
            first_layer_sequence: self.first_layer_sequence.clone(),
            ..PlateSettings::default()
        };
        for (key, value) in load::plate_config(kept) {
            match key {
                "curr_bed_type" => settings.bed_type = Some(value),
                "print_sequence" => settings.print_sequence = Some(value),
                "spiral_mode" => settings.spiral_mode = Some(value == "1"),
                _ => {}
            }
        }
        // The later layers' ranges, each its first and last layer and then
        // its order, as many as the count says (ParameterUtils.cpp
        // `get_other_layers_print_sequence`); a count that does not divide
        // the numbers is no order, as there.
        if let (Some(numbers), Some(count)) = (
            value("other_layers_print_sequence"),
            value("other_layers_print_sequence_nums"),
        ) {
            let numbers: Vec<u32> = numbers
                .split_whitespace()
                .filter_map(|number| number.parse().ok())
                .collect();
            let count: usize = count.trim().parse().unwrap_or(0);
            if count > 0 && numbers.len().is_multiple_of(count) && numbers.len() / count >= 3 {
                settings.other_layers_sequence = numbers
                    .chunks(numbers.len() / count)
                    .map(|range| LayerSequence {
                        begin: range[0],
                        end: range[1],
                        order: range[2..].to_vec(),
                    })
                    .collect();
            }
        }
        settings
    }

    /// Its own settings and metadata to change: what its file held, else
    /// what is written for a plate the file did not have.
    fn kept_mut(&mut self) -> &mut PlateKept {
        let (id, first) = (self.id, self.first_layer_sequence.clone());
        let kept = self
            .kept
            .get_or_insert_with(|| Arc::new(PlateKept::fresh(id, &first)));
        Arc::make_mut(kept)
    }

    fn set(&mut self, key: &str, value: Option<&str>) {
        let kept = self.kept_mut();
        *kept = kept.with(key, value);
    }
}

/// Filament numbers written as the reference writes a list of them.
fn listed(numbers: impl IntoIterator<Item = u32>) -> String {
    numbers
        .into_iter()
        .map(|number| number.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether filament numbers are each from 1 and none twice.
fn an_order(order: &[u32]) -> bool {
    let mut seen = order.to_vec();
    seen.sort_unstable();
    seen.dedup();
    seen.len() == order.len() && order.iter().all(|filament| *filament >= 1)
}

impl Model {
    fn plate_at(&self, plate: u32) -> Result<usize, Failure> {
        self.plate_index(plate)
            .ok_or_else(|| refused(format!("there is no plate {plate}")))
    }

    /// Names a plate (Plater.cpp `rename_plate`); an empty name is none.
    pub fn rename_plate(&mut self, plate: u32, name: &str) -> Result<(), Failure> {
        let at = self.plate_at(plate)?;
        if name.chars().count() > LONGEST_PLATE_NAME {
            return Err(refused(format!(
                "a plate's name is at most {LONGEST_PLATE_NAME} characters"
            )));
        }
        self.plates[at].name = name.to_owned();
        Ok(())
    }

    /// Locks a plate, or unlocks it (PartPlateList.cpp `lock_plate`).
    pub fn lock_plate(&mut self, plate: u32, locked: bool) -> Result<(), Failure> {
        let at = self.plate_at(plate)?;
        if self.plates[at].locked() != locked {
            self.plates[at].set("locked", Some(if locked { "true" } else { "false" }));
        }
        Ok(())
    }

    /// Sets what a plate sets for itself ("Customize current plate"):
    /// each as PartPlate.cpp sets it, a setting left to the project taken
    /// out of the plate's own.
    pub fn set_plate_settings(
        &mut self,
        plate: u32,
        settings: &PlateSettings,
    ) -> Result<(), Failure> {
        let at = self.plate_at(plate)?;
        if let Some(bed) = &settings.bed_type
            && !load::BED_TYPES.contains(&bed.as_str())
        {
            return Err(refused(format!("there is no bed type named {bed}")));
        }
        if let Some(sequence) = &settings.print_sequence
            && !matches!(sequence.as_str(), "by layer" | "by object")
        {
            return Err(refused(format!(
                "a print sequence is \"by layer\" or \"by object\", not {sequence}"
            )));
        }
        if !an_order(&settings.first_layer_sequence) {
            return Err(refused(
                "the first layer's order names each filament once, from 1",
            ));
        }
        for range in &settings.other_layers_sequence {
            if range.begin < FIRST_LATER_LAYER || range.end < FIRST_LATER_LAYER {
                return Err(refused("Please input layer value (>= 2)."));
            }
            if range.end < range.begin {
                return Err(refused(format!(
                    "layer {} comes before layer {}",
                    range.end, range.begin
                )));
            }
            if range.order.is_empty() || !an_order(&range.order) {
                return Err(refused(
                    "a range of layers' order names each filament once, from 1",
                ));
            }
        }
        // The ranges in order of their first layer, then their last
        // (PlateSettingsDialog.cpp `LayerSeqInfo::operator<`).
        let mut ranges = settings.other_layers_sequence.clone();
        ranges.sort_by_key(|range| (range.begin, range.end));
        let held = &mut self.plates[at];
        held.set("bed_type", settings.bed_type.as_deref());
        held.set("print_sequence", settings.print_sequence.as_deref());
        held.first_layer_sequence = settings.first_layer_sequence.clone();
        let first = (!settings.first_layer_sequence.is_empty())
            .then(|| listed(settings.first_layer_sequence.iter().copied()));
        held.set("first_layer_print_sequence", first.as_deref());
        let (numbers, count) = if ranges.is_empty() {
            (None, None)
        } else {
            let numbers = ranges.iter().flat_map(|range| {
                [range.begin, range.end]
                    .into_iter()
                    .chain(range.order.iter().copied())
            });
            (Some(listed(numbers)), Some(ranges.len().to_string()))
        };
        held.set("other_layers_print_sequence", numbers.as_deref());
        held.set("other_layers_print_sequence_nums", count.as_deref());
        // Written as the reference writes a bool there, after `locked`
        // left the stream in `boolalpha`.
        let spiral = settings
            .spiral_mode
            .map(|on| if on { "true" } else { "false" });
        held.set("spiral_mode", spiral);
        Ok(())
    }

    /// Takes a plate's own bed type away, on every plate whose bed type
    /// `allowed` does not take (Plater.cpp `normalize_bed_types`). Answers
    /// whether one was.
    pub fn reset_bed_types(&mut self, allowed: impl Fn(&str) -> bool) -> bool {
        let mut reset = false;
        for plate in &mut self.plates {
            if let Some(bed) = plate.settings().bed_type
                && !allowed(&bed)
            {
                plate.set("bed_type", None);
                reset = true;
            }
        }
        reset
    }

    /// Moves a plate to the front, with what stands on it; the plates
    /// before it each go one back, and every plate is numbered anew
    /// (PartPlateList.cpp `move_plate_to_index`, to 0).
    pub fn move_plate_to_front(&mut self, bed: Option<&Bed>, plate: u32) -> Result<(), Failure> {
        let at = self.plate_at(plate)?;
        if at == 0 {
            return Err(refused("the plate is at the front already"));
        }
        self.settle(bed);
        let moved = self.plates.remove(at);
        self.plates.insert(0, moved);
        for (at, plate) in self.plates.iter_mut().enumerate() {
            plate.id = at as u32 + 1;
        }
        Ok(())
    }

    /// Adds a plate after the last with a copy of everything on `plate`,
    /// each where it stood on its plate (PartPlateList.cpp
    /// `duplicate_plate`). Answers the new plate's number and the copies
    /// made. The new plate is a new plate: its name and its own settings
    /// are not the old one's, as in the reference.
    ///
    /// An object with copies on other plates too is copied with the
    /// copies on this plate only; the reference copies the whole object,
    /// which puts the other copies off every plate (see the pass's notes).
    pub fn duplicate_plate(
        &mut self,
        bed: Option<&Bed>,
        plate: u32,
    ) -> Result<(u32, Vec<u64>), Failure> {
        let from = self.plate_at(plate)?;
        let on: Vec<u64> = self.plates[from].order.clone();
        let id = self.add_plate(bed)?;
        let to = self.plate_at(id)?;
        let origin = self.plates[to].origin.unwrap_or((0.0, 0.0));
        let mut made = Vec::new();
        // One new object for each object with copies there, in the list's
        // order, its copies in the plate's order.
        let mut objects: Vec<usize> = Vec::new();
        for copy in &on {
            if let Some((object, _)) = self.find(*copy)
                && !objects.contains(&object)
            {
                objects.push(object);
            }
        }
        objects.sort_unstable();
        for object in objects {
            let held = &self.objects[object];
            let copies: Vec<ModelCopy> = on
                .iter()
                .filter_map(|copy| held.copies.iter().find(|own| own.id == *copy))
                .map(|copy| ModelCopy {
                    id: fresh(),
                    world: copy.local.then(&edge_mesh::Transform::translation([
                        origin.0, origin.1, 0.0,
                    ])),
                    local: copy.local,
                    reach: copy.reach,
                    printable: copy.printable,
                })
                .collect();
            let new = ModelObject {
                id: fresh(),
                name: held.name.clone(),
                filament: held.filament,
                shape: held.shape.clone(),
                copies,
                kept: held.kept.clone(),
            };
            for copy in &new.copies {
                self.plates[to].order.push(copy.id);
                made.push(copy.id);
            }
            self.objects.push(new);
        }
        Ok((id, made))
    }
}
