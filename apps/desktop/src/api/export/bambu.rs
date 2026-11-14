//! "Export Bambu 3MF..." and "Export && Open in Bambu Studio..."
//! (Plater.cpp `export_bambu_3mf`, `export_and_open_in_bambu_studio`):
//! the project as "Save Project" writes it, in the form Bambu Studio reads
//! as a project of its own (`edge_engine::Model::save_bambu`, the settings
//! translated by `edge_config::bambu`), and the reference's one-line
//! account of what was left out.
//!
//! The second also hands the file to Bambu Studio. The session never
//! starts it: it answers with how it would be started, and only the
//! window starts it, on the person's click (main.rs `hand_off`,
//! `crate::outside::perform`).

use serde_json::{Value, json};

use super::{Session, lock, shown, with_extension};
use crate::api::i18n::tr;

/// The calls answered here.
pub(in crate::api) const JSON: [&str; 1] = ["export_bambu"];

impl Session {
    pub(in crate::api) fn bambu_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "export_bambu" => self.export_bambu(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// Writes the Bambu form of the project to `path` (".3mf" put on a
    /// name without it); not over a file that is there unless `replace`,
    /// else `{ exists, path }`. `suggest: true` answers `{ suggested }`
    /// alone. Answers `{ path, bytes, entries, dropped, renamed,
    /// converted, notes, said }`, `said` the reference's notification;
    /// with `open: true` also `open`, the ways Bambu Studio would be
    /// started with it on this system (what the window does).
    fn export_bambu(&self, args: &Value) -> Result<Value, String> {
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
            return Ok(json!({ "suggested": super::super::save::suggested(Some(&file), &name) }));
        }
        let path = with_extension(super::super::needed(args, "path")?, ".3mf");
        let replace = args
            .get("replace")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if path.exists() && !replace {
            return Ok(json!({ "exists": true, "path": shown(&path) }));
        }
        let failed = || tr("Failed to export the project for Bambu Studio.");
        let saving = self.saving(&choices, Some(&file)).map_err(|_| failed())?;
        let bed = self
            .printer_of(&choices, Some(&file))?
            .and_then(|(_, printer)| super::super::bed_of(&printer))
            .map(edge_engine::Bed::of);
        let mut model = file.model.clone();
        model.set_inside(choices.inside.clone());
        let (saved, report) = model
            .save_bambu(&path, &saving, bed.as_ref())
            .map_err(|error| format!("{}\n{error}", failed()))?;
        let said = if report.dropped.is_empty() {
            tr("Exported for Bambu Studio.")
        } else {
            tr("Exported for Bambu Studio: %d settings not supported by Bambu Studio were left out.")
                .replace("%d", &report.dropped.len().to_string())
        };
        let mut answer = json!({
            "path": shown(&path),
            "bytes": saved.bytes,
            "entries": saved.entries,
            "dropped": report.dropped,
            "renamed": report.renamed,
            "converted": report.converted,
            "notes": report.notes,
            "summary": report.summary(),
            "said": said,
        });
        if args.get("open").and_then(Value::as_bool).unwrap_or(false) {
            answer["open"] = json!(crate::outside::bambu_studio_tries(&shown(&path)));
        }
        Ok(answer)
    }
}
