//! The pages' side of a person's own presets: what is set over a preset,
//! put into the preset itself as its file holds it, and the options two
//! sets of settings differ in, by page and group, as the reference lists
//! them in "Transfer or discard changes" (UnsavedChangesDialog.cpp
//! `update_tree`) and "Compare presets" (`DiffPresetDialog::update_tree`).

use std::collections::{BTreeMap, BTreeSet};

use edge_config::{Settings, Value as Setting};
use serde_json::{Value, json};

use super::{
    Asked, Book, Session, address, held, lock, over_of, pages, preset_values, printer_rules,
    unescaped, values_in,
};

/// A preset with what is set over it put in, each value as the preset's
/// file holds it: a list with a value for each variant keeps its length,
/// an extruder's own value goes into its place of the list, a text of
/// several lines is the text itself (the reference writes a string as it
/// is, `ConfigBase::save_to_json`).
pub(in crate::api) fn with_edits(
    book: Book,
    preset: &Settings,
    own: &BTreeMap<String, String>,
) -> Settings {
    let defaults = edge_config::reference_defaults();
    let mut out = preset.clone();
    for (name, value) in own {
        let (key, extruder) = address(name);
        let Some(option) = pages(book).options.get(key) else {
            continue;
        };
        let under = preset.get(key).or_else(|| defaults.get(key));
        let mut setting = match extruder {
            Some(extruder) => {
                let mut items = match under {
                    Some(Setting::Many(items)) => items.clone(),
                    Some(Setting::One(text)) => vec![text.clone()],
                    None => Vec::new(),
                };
                if items.len() <= extruder {
                    let last = items.last().cloned().unwrap_or_default();
                    items.resize(extruder + 1, last);
                }
                items[extruder] = value.clone();
                Setting::Many(items)
            }
            None => held(option, value, under),
        };
        if option.escaped {
            setting = match setting {
                Setting::One(text) => Setting::One(unescaped(&text)),
                Setting::Many(items) => {
                    Setting::Many(items.iter().map(|item| unescaped(item)).collect())
                }
            };
        }
        out.insert(key, setting);
    }
    out
}

/// Whether a setting is one of this kind of preset's: on its pages, or
/// one of the settings that say which printers and processes it goes
/// with.
pub(in crate::api) fn is_of(book: Book, key: &str) -> bool {
    pages(book).options.contains_key(key)
        || matches!(
            key,
            "compatible_printers"
                | "compatible_printers_condition"
                | "compatible_prints"
                | "compatible_prints_condition"
        )
}

/// The kind of preset a book is of.
pub(in crate::api) fn kind(book: Book) -> edge_config::Kind {
    match book {
        Book::Process => edge_config::Kind::Process,
        Book::Filament => edge_config::Kind::Filament,
        Book::Printer => edge_config::Kind::Machine,
    }
}

/// One option that differs.
struct Row {
    key: String,
    old: String,
    new: String,
}

/// The rows that differ, by page and group in the pages' order: `pages`
/// of `{ title, groups: [{ title, rows: [{ key, label, old, new }] }] }`,
/// each title and label a message id of the reference's. An extruder's
/// page is there for each extruder (`Extruder %d` with its number).
fn by_page(
    book: Book,
    extruders: usize,
    differs: impl Fn(&str) -> Option<(String, String)>,
) -> Value {
    let described = pages(book);
    let mut out = Vec::new();
    for page in described.json["pages"].as_array().into_iter().flatten() {
        let per_extruder = page["per"].as_str() == Some("extruder");
        let copies = if per_extruder { extruders.max(1) } else { 1 };
        for extruder in 0..copies {
            let mut groups = Vec::new();
            for group in page["groups"].as_array().into_iter().flatten() {
                let mut rows: Vec<Row> = Vec::new();
                let mut seen = BTreeSet::new();
                for line in group["lines"].as_array().into_iter().flatten() {
                    for key in line["keys"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        let name = if per_extruder {
                            format!("{key}#{extruder}")
                        } else {
                            key.to_owned()
                        };
                        if !seen.insert(name.clone()) {
                            continue;
                        }
                        if let Some((old, new)) = differs(&name) {
                            rows.push(Row {
                                key: name,
                                old,
                                new,
                            });
                        }
                    }
                }
                if !rows.is_empty() {
                    groups.push(json!({
                        "title": group["title"],
                        "rows": rows
                            .into_iter()
                            .map(|row| {
                                let label = described
                                    .options
                                    .get(address(&row.key).0)
                                    .map_or(row.key.clone(), |option| option.label.clone());
                                json!({ "key": row.key, "label": label, "old": row.old, "new": row.new })
                            })
                            .collect::<Vec<_>>(),
                    }));
                }
            }
            if !groups.is_empty() {
                out.push(json!({
                    "title": page["title"],
                    "extruder": per_extruder.then_some(extruder + 1),
                    "groups": groups,
                }));
            }
        }
    }
    Value::Array(out)
}

/// What is set over a preset, against the preset's own values: the list
/// of "Transfer or discard changes".
pub(in crate::api) fn changes(
    book: Book,
    preset_values: &BTreeMap<String, String>,
    own: &BTreeMap<String, String>,
    extruders: usize,
) -> Value {
    by_page(book, extruders, |name| {
        let new = own.get(name)?;
        let old = preset_values.get(name).cloned().unwrap_or_default();
        Some((old, new.clone()))
    })
}

/// Where two presets of a kind differ, each as the pages show it: the
/// list of "Compare presets". Of a filament, the first filament's.
pub(in crate::api) fn differences(book: Book, left: &Settings, right: &Settings) -> Value {
    let asked = Asked { book, filament: 0 };
    let (old, new) = (values_in(asked, Some(left)), values_in(asked, Some(right)));
    let extruders = printer_rules::extruders(left).max(printer_rules::extruders(right));
    let pages_kind = pages(book);
    by_page(book, extruders, |name| {
        pages_kind.options.get(address(name).0)?;
        let (a, b) = (old.get(name), new.get(name));
        if a == b {
            return None;
        }
        Some((
            a.cloned().unwrap_or_default(),
            b.cloned().unwrap_or_default(),
        ))
    })
}

/// Of a list of settings found by page, how many rows there are.
pub(in crate::api) fn count(listed: &Value) -> usize {
    listed
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|page| page["groups"].as_array().into_iter().flatten())
        .map(|group| group["rows"].as_array().map_or(0, Vec::len))
        .sum()
}

impl Session {
    /// What is set over a project's preset of one kind (and filament,
    /// from 1), against the preset's values: `{ kind, filament, name,
    /// count, pages }` for "Transfer or discard changes".
    pub(in crate::api) fn preset_changes(&self, args: &Value, book: Book) -> Result<Value, String> {
        let (id, choices, file, filaments) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (
                project.id,
                project.choices.clone(),
                project.file.clone(),
                project.filament_count(),
            )
        };
        let asked = Self::asked(args, book, filaments)?;
        let basis = self.basis(&choices, file.as_deref())?;
        let own = over_of(&choices, book, asked.filament)
            .cloned()
            .unwrap_or_default();
        let values = preset_values(asked, &choices, &basis);
        let extruders = basis
            .chosen
            .machine
            .as_ref()
            .map_or(1, |(_, printer)| printer_rules::extruders(printer));
        let listed = changes(book, &values, &own, extruders);
        let name = match book {
            Book::Process => choices.process.clone(),
            Book::Printer => choices.machine.clone(),
            Book::Filament => choices
                .filaments
                .get(asked.filament)
                .or(choices.filaments.last())
                .cloned(),
        };
        Ok(json!({
            "project": id,
            "kind": book.name(),
            "filament": (book == Book::Filament).then_some(asked.filament + 1),
            "name": name,
            "count": count(&listed),
            "pages": listed,
        }))
    }
}
