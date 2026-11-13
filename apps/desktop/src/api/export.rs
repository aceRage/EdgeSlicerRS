//! File > Export's project files: "Export plate sliced file..." [Ctrl+G]
//! and "Export all plate sliced file..." (the sliced file, `.gcode.3mf`:
//! Plater.cpp `export_gcode_3mf`), and "Export Generic 3MF..." (the
//! project with every mesh in the one model file: `export_core_3mf`).
//! The print button offers the sliced files too, for a printer that is
//! not Bambu Lab's (MainFrame.cpp `available_print_actions`).
//!
//! The sliced file's pictures are drawn by the page (its 3D view), as the
//! reference draws them with its own (`Plater::export_3mf`), and handed
//! here as PNG files in base64.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::{Value, json};

use super::{Done, Session, lock, shown};

/// The calls answered here.
pub(super) const JSON: [&str; 5] = [
    "export_sliced",
    "export_generic",
    "export_sliced_plates",
    "bambu_connect_send",
    "export_bambu",
];

mod bambu;
mod connect;
pub(super) use connect::is_bambu;

/// Bytes from base64, as a data URL's or bare.
fn from_base64(text: &str) -> Result<Vec<u8>, String> {
    let text = text.split_once(',').map_or(text, |(_, data)| data);
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut word = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return Err("a picture is not in base64".into()),
        };
        word = (word << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((word >> bits) as u8);
            word &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// A path with `extension` put on it when it has not that one already
/// (Plater.cpp: "if (ext != ".3mf") output_path += ".3mf"").
fn with_extension(path: &str, extension: &str) -> PathBuf {
    let path = path.trim();
    if path.to_ascii_lowercase().ends_with(extension) {
        PathBuf::from(path)
    } else {
        PathBuf::from(format!("{path}{extension}"))
    }
}

/// The pictures the page drew, by plate: `{ "2": { plate, small,
/// no_light, top, pick } }`.
fn pictures(args: &Value) -> Result<BTreeMap<u32, edge_engine::Pictures>, String> {
    let mut out = BTreeMap::new();
    let Some(given) = args.get("pictures").and_then(Value::as_object) else {
        return Ok(out);
    };
    for (plate, drawn) in given {
        let plate: u32 = plate
            .parse()
            .map_err(|_| format!("there is no plate {plate}"))?;
        let one = |key: &str| -> Result<Vec<u8>, String> {
            drawn
                .get(key)
                .and_then(Value::as_str)
                .map_or(Ok(Vec::new()), from_base64)
        };
        out.insert(
            plate,
            edge_engine::Pictures {
                plate: one("plate")?,
                small: one("small")?,
                no_light: one("no_light")?,
                top: one("top")?,
                pick: one("pick")?,
            },
        );
    }
    Ok(out)
}

/// A project ready to be written as a sliced file.
struct SlicedProject {
    file: Arc<super::File>,
    choices: super::Choices,
    name: String,
    plate: Option<u32>,
    slices: Vec<(u32, Arc<Done>)>,
}

impl SlicedProject {
    fn naming(&self) -> (&super::File, &str, Option<u32>) {
        (&self.file, &self.name, self.plate)
    }

    /// Writes the sliced file to `path` with `saving`'s settings.
    fn write(
        &self,
        path: &Path,
        saving: edge_engine::Saving,
        all: bool,
        pictures: BTreeMap<u32, edge_engine::Pictures>,
    ) -> Result<edge_engine::Saved, String> {
        let mut parts = Vec::new();
        for (id, done) in &self.slices {
            let gcode = done.gcode.as_ref().map_err(String::clone)?;
            parts.push(edge_engine::sliced_file::PlateSlice {
                plate: *id,
                sliced: &done.sliced,
                gcode: &gcode.text,
                seconds: gcode.summary.seconds,
            });
        }
        let shown_plate = if all {
            None
        } else {
            Some(self.plate.unwrap_or(1))
        };
        let mut model = self.file.model.clone();
        model.set_inside(self.choices.inside.clone());
        model
            .save_sliced(path, &saving, &parts, shown_plate, pictures)
            .map_err(|error| error.to_string())
    }
}

/// The name a sliced file is offered under: the project's, and the
/// plate's for one plate of several (Plater.cpp
/// `get_export_gcode_filename`), with `.gcode.3mf`; in the project's
/// folder when `in_folder`.
fn suggested_sliced(
    file: &super::File,
    name: &str,
    plate: Option<u32>,
    all: bool,
    in_folder: bool,
) -> String {
    let stem = file
        .path
        .as_deref()
        .and_then(Path::file_stem)
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_owned());
    let stem = match plate {
        Some(id) if !all && file.plates.len() > 1 => {
            let named = file
                .plates
                .iter()
                .find(|one| one.id == id)
                .map(|one| one.name.clone())
                .filter(|name| !name.is_empty());
            match named {
                Some(named) => format!("{stem}_{named}"),
                None => format!("{stem}_plate_{id}"),
            }
        }
        _ => stem,
    };
    let named = format!("{stem}.gcode.3mf");
    match file
        .path
        .as_deref()
        .and_then(Path::parent)
        .filter(|dir| in_folder && !dir.as_os_str().is_empty())
    {
        Some(dir) => shown(&dir.join(named)),
        None => named,
    }
}

impl Session {
    pub(super) fn export_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "export_sliced" => self.export_sliced(args),
            "export_sliced_plates" => self.sliced_plates(args),
            "export_generic" => self.export_generic(args),
            name if connect::JSON.contains(&name) => self.connect_json(name, args),
            name if bambu::JSON.contains(&name) => self.bambu_json(name, args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// The plates a sliced file of the project would hold, and what the
    /// page draws their pictures with: `{ plates: [{ plate, copies:
    /// [{ copy, number }] }], all_ready }`. `all`: every plate sliced
    /// ("Export all plate sliced file"), else the plate shown.
    /// `all_ready`: every plate that holds something is sliced and its
    /// G-code written (`is_all_slice_results_ready_for_print`).
    fn sliced_plates(&self, args: &Value) -> Result<Value, String> {
        let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
        let state = lock(&self.state);
        let project = &state.projects[state.target(args)?];
        let ready = |slot: Option<u32>| {
            project
                .results
                .get(&super::slot(slot))
                .is_some_and(|done| done.gcode.is_ok())
        };
        let sliceable = project.sliceable();
        let all_ready = !sliceable.is_empty() && sliceable.iter().all(|slot| ready(*slot));
        let wanted: Vec<Option<u32>> = if all {
            sliceable.into_iter().filter(|slot| ready(*slot)).collect()
        } else {
            vec![project.plate]
                .into_iter()
                .filter(|slot| ready(*slot))
                .collect()
        };
        // Each copy's number, which its pick picture is drawn in.
        let numbers = project.file.as_ref().map(|file| file.model.copy_numbers());
        Ok(json!({
            "plates": wanted.iter().map(|slot| slot.unwrap_or(1)).collect::<Vec<_>>(),
            "all_plates": project
                .file
                .as_ref()
                .map(|file| file.plates.iter().map(|plate| plate.id).collect::<Vec<_>>())
                .unwrap_or_default(),
            "copies": numbers
                .unwrap_or_default()
                .into_iter()
                .map(|(copy, number)| json!({ "copy": copy, "number": number }))
                .collect::<Vec<_>>(),
            "all_ready": all_ready,
        }))
    }

    /// "Export plate sliced file..." (`all` false) and "Export all plate
    /// sliced file..." (`all` true): the sliced file of the plate shown,
    /// or of every plate sliced, to `path` (".3mf" put on a name without
    /// it), with `pictures` the page drew. Not over a file that is there
    /// unless `replace` (the save dialog asked); else `{ exists, path }`.
    /// With `suggest: true` only `{ suggested }`, the name the dialog
    /// offers. Answers `{ path, bytes, entries }`.
    fn export_sliced(&self, args: &Value) -> Result<Value, String> {
        let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
        if args
            .get("suggest")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let ready = self.sliced_project(args, all)?;
            let (file, name, plate) = ready.naming();
            return Ok(json!({ "suggested": suggested_sliced(file, name, plate, all, true) }));
        }
        let path = with_extension(super::needed(args, "path")?, ".3mf");
        let replace = args
            .get("replace")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if path.exists() && !replace {
            return Ok(json!({ "exists": true, "path": shown(&path) }));
        }
        let pictures = pictures(args)?;
        let ready = self.sliced_project(args, all)?;
        let saving = self.saving(&ready.choices, Some(&ready.file))?;
        let saved = ready.write(&path, saving, all, pictures)?;
        Ok(json!({
            "path": shown(&path),
            "bytes": saved.bytes,
            "entries": saved.entries,
        }))
    }

    /// The project a sliced file is written of, with the plates sliced
    /// that go in it: the plate shown, or (`all`) every plate sliced.
    fn sliced_project(&self, args: &Value, all: bool) -> Result<SlicedProject, String> {
        let state = lock(&self.state);
        let project = &state.projects[state.target(args)?];
        let file = project
            .file
            .clone()
            .filter(|file| !file.model.objects().is_empty())
            .ok_or("There are no objects in the project.")?;
        let mut slices: Vec<(u32, Arc<Done>)> = Vec::new();
        let wanted: Vec<Option<u32>> = if all {
            project.sliceable()
        } else {
            vec![project.plate]
        };
        for slot in wanted {
            if let Some(done) = project.results.get(&super::slot(slot))
                && done.gcode.is_ok()
            {
                slices.push((slot.unwrap_or(1), done.clone()));
            }
        }
        if slices.is_empty() {
            return Err("nothing has been sliced yet".into());
        }
        Ok(SlicedProject {
            file,
            choices: project.choices.clone(),
            name: project.name.clone(),
            plate: project.plate,
            slices,
        })
    }

    /// "Export Generic 3MF...": the project as "Save Project" writes it,
    /// with every mesh in the one model file and without the production
    /// extension, to `path`; the project is not then that file (the
    /// reference saves it `Silence`d). `suggest`, `replace` and the
    /// answer as [`Session::export_sliced`]'s.
    fn export_generic(&self, args: &Value) -> Result<Value, String> {
        let (file, choices, name) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            let file = project
                .file
                .clone()
                .filter(|file| !file.model.objects().is_empty())
                .ok_or("There are no objects in the project.")?;
            (file, project.choices.clone(), project.name.clone())
        };
        if args
            .get("suggest")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(json!({ "suggested": super::save::suggested(Some(&file), &name) }));
        }
        let path = with_extension(super::needed(args, "path")?, ".3mf");
        let replace = args
            .get("replace")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if path.exists() && !replace {
            return Ok(json!({ "exists": true, "path": shown(&path) }));
        }
        let saving = self.saving(&choices, Some(&file))?;
        let bed = self
            .printer_of(&choices, Some(&file))?
            .and_then(|(_, printer)| super::bed_of(&printer))
            .map(edge_engine::Bed::of);
        let saved = file
            .model
            .save_as(&path, &saving, bed.as_ref(), edge_engine::Layout::Core)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "path": shown(&path),
            "bytes": saved.bytes,
            "entries": saved.entries,
        }))
    }
}

#[cfg(test)]
mod bambu_tests;
#[cfg(test)]
mod connect_tests;
#[cfg(test)]
mod tests;
