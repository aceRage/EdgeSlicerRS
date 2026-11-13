//! "Send to Bambu Connect" (Plater.cpp `send_to_bambu_connect`
//! l.25747): the sliced file of the plate shown, or of every plate,
//! written exactly as "Export plate sliced file" writes it into a folder
//! of the app's own, and the `bambu-connect://` URL that hands it to the
//! person's own Bambu Connect app, which prints it with their Bambu
//! account or over LAN. Nothing here talks to Bambu or to a printer.
//!
//! The session answers with the URL; only the window opens it, on the
//! person's click (main.rs `hand_off`, `crate::outside::perform`). In a
//! browser (`--serve`) and in the tests the URL is all there is.

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use edge_config::Value as Setting;
use serde_json::{Value, json};

use super::{Session, lock, pictures, suggested_sliced};
use crate::api::i18n::tr;
use crate::outside::{self, Installed};

/// The calls answered here.
pub(in crate::api) const JSON: [&str; 1] = ["bambu_connect_send"];

#[cfg(test)]
thread_local! {
    /// The folder a test hands its files to Bambu Connect in.
    pub(in crate::api) static FOLDER_IN_TEST: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// `<system temp>/EdgeSlicerRS/bambu-connect` (`prepare_temp_dir`; the
/// reference's is `.../EdgeSlicer/bambu-connect`), made when it is not
/// there, with what was handed over more than three days ago taken out.
fn folder() -> Result<PathBuf, String> {
    #[cfg(test)]
    let dir = FOLDER_IN_TEST
        .with(|dir| dir.borrow().clone())
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("EdgeSlicerRS")
                .join("bambu-connect")
        });
    #[cfg(not(test))]
    let dir = std::env::temp_dir()
        .join("EdgeSlicerRS")
        .join("bambu-connect");
    std::fs::create_dir_all(&dir)
        .ok()
        .filter(|()| dir.is_dir())
        .ok_or("Could not create a temporary folder for the file to send to Bambu Connect.")?;
    let seconds = |time: SystemTime| {
        time.duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs())
    };
    let entries: Vec<(String, u64)> = std::fs::read_dir(&dir)
        .map(|read| {
            read.flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .filter_map(|entry| {
                    let written = entry.metadata().ok()?.modified().ok()?;
                    Some((
                        entry.file_name().to_string_lossy().into_owned(),
                        seconds(written),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    for stale in outside::stale_connect_files(
        &entries,
        seconds(SystemTime::now()),
        outside::CONNECT_KEPT_SECONDS,
    ) {
        let _ = std::fs::remove_file(dir.join(stale));
    }
    Ok(dir)
}

/// Whether a project's printer is a Bambu Lab printer
/// (`PresetBundle::is_bbl_vendor`): on presets, the vendor's are "BBL";
/// on its own settings, they name a Bambu Lab model.
pub(in crate::api) fn is_bambu(
    choices: &super::super::Choices,
    file: Option<&super::super::File>,
) -> bool {
    if choices.from_presets {
        return choices.machine.is_some() && choices.vendor.as_deref() == Some("BBL");
    }
    file.and_then(|file| file.settings.as_deref())
        .and_then(|settings| settings.text("printer_model"))
        .is_some_and(|model| model.starts_with("Bambu Lab"))
}

impl Session {
    pub(in crate::api) fn connect_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "bambu_connect_send" => self.connect_send(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// "Send plate to Bambu Connect" (`all` false) and "Send all plates
    /// to Bambu Connect" (`all` true), and the print button's "Send to
    /// Bambu Connect". Writes the sliced file and answers `{ url, name,
    /// path }`; or `{ not_installed: true }` when Bambu Connect is not
    /// there (checked first, so it costs no export), with nothing written.
    /// `{ probe: true }` answers `{ probe: true, bambu }`: whether the
    /// printer is Bambu Lab's (the entries are shown for one alone).
    fn connect_send(&self, args: &Value) -> Result<Value, String> {
        let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
        let (choices, file) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (project.choices.clone(), project.file.clone())
        };
        let bambu = is_bambu(&choices, file.as_deref());
        if args.get("probe").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(json!({ "probe": true, "bambu": bambu }));
        }
        if file
            .as_ref()
            .is_none_or(|file| file.model.objects().is_empty())
        {
            return Err("There are no objects in the project.".into());
        }
        if !bambu {
            return Err(tr("Send to Bambu Connect is for a Bambu Lab printer."));
        }
        // The plate (or every plate) sliced and its G-code written.
        let ready = match self.sliced_project(args, all) {
            Ok(ready) if !all || self.all_sliced(args)? => ready,
            _ => {
                return Err(if all {
                    tr("Slice all plates first, then send them to Bambu Connect.")
                } else {
                    tr("Slice the plate first, then send it to Bambu Connect.")
                });
            }
        };
        if outside::connect_installed() == Installed::No {
            return Ok(json!({ "not_installed": true, "info_url": outside::CONNECT_INFO_URL }));
        }
        let dir = folder().map_err(|error| tr(&error))?;
        let (file_ref, name, plate) = ready.naming();
        let display = outside::connect_name(&suggested_sliced(file_ref, name, plate, all, false));
        let path = dir.join(outside::connect_file_name(&display));
        // What "Export plate sliced file" writes, the settings with
        // `print_compatible_printers` naming the printer's system preset
        // when they name none (Plater.cpp `export_3mf`,
        // `fill_print_compatible_printers_for_export`): Bambu Connect
        // checks it against the printer it sends to, and hangs on a file
        // without it.
        let mut saving = self.saving(&ready.choices, Some(&ready.file))?;
        let system = if choices.from_presets {
            choices
                .machine
                .as_deref()
                .map(|machine| self.system_printer(machine))
        } else {
            ready
                .file
                .settings
                .as_deref()
                .and_then(|settings| settings.text("printer_settings_id"))
                .map(|name| self.system_printer(name))
        };
        saving = with_compatible(saving, ready.file.settings.as_deref(), system);
        let pictures = pictures(args)?;
        ready
            .write(&path, saving, all, pictures)
            .map_err(|_| tr("Failed to export the sliced file for Bambu Connect."))?;
        if !path.exists() {
            return Err(tr("Failed to export the sliced file for Bambu Connect."));
        }
        let shown = path.to_string_lossy().into_owned();
        Ok(json!({
            "url": outside::connect_url(&shown, &display),
            "name": display,
            "path": shown,
        }))
    }

    /// Whether every plate that holds something is sliced
    /// (`is_all_slice_result_ready_for_export`).
    fn all_sliced(&self, args: &Value) -> Result<bool, String> {
        let state = lock(&self.state);
        let project = &state.projects[state.target(args)?];
        let sliceable = project.sliceable();
        Ok(!sliceable.is_empty()
            && sliceable.iter().all(|slot| {
                project
                    .results
                    .get(&super::super::slot(*slot))
                    .is_some_and(|done| done.gcode.is_ok())
            }))
    }
}

/// `saving` with `print_compatible_printers` naming the printer's system
/// preset where the settings name no printer there
/// (`fill_print_compatible_printers_for_export`); on presets chosen here
/// they already carry the process preset's list (`edge_engine::settled`).
/// A project saved on its own settings is written from them (read from
/// its file), so the name can be set.
fn with_compatible(
    saving: edge_engine::Saving,
    own: Option<&edge_config::Settings>,
    system: Option<String>,
) -> edge_engine::Saving {
    let Some(system) = system.filter(|name| !name.is_empty()) else {
        return saving;
    };
    let (mut settings, presets) = match saving {
        edge_engine::Saving::Settings { settings, presets } => (settings, presets),
        edge_engine::Saving::Own => match own {
            Some(own) => (own.clone(), true),
            None => return edge_engine::Saving::Own,
        },
        edge_engine::Saving::None => return edge_engine::Saving::None,
    };
    let named = settings
        .texts("print_compatible_printers")
        .is_some_and(|names| names.iter().any(|name| !name.is_empty()));
    if !named {
        settings.insert("print_compatible_printers", Setting::Many(vec![system]));
    }
    edge_engine::Saving::Settings { settings, presets }
}
