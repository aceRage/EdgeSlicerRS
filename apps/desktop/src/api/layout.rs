//! The page's own layout, kept between runs in the app's settings
//! (`layout` in `settings.json`, [`super::store::Data`]) rather than in
//! the browser's storage, which the window and `--serve` do not share:
//! the sidebar's width as it was dragged, and which groups of the
//! settings pages are folded.
//!
//! Both are the owner's choices for this app and not the reference's: its
//! sidebar has a fixed width and its groups of options do not fold
//! (UI_PARITY.md section 2, "Sidebar").

use serde_json::{Value, json};

use super::{Session, lock, settings::Book};

/// The narrowest the sidebar is dragged to, its usual width in a narrow
/// window, which the pages were laid out for: an option row keeps its
/// label, a 112 px control and its reset arrow, and the pages' tabs and
/// the search their room.
pub const SIDEBAR_MIN: u32 = 300;
/// The widest. The page also keeps the stage at least
/// [`STAGE_MIN`] wide, so in a narrow window the sidebar stops sooner.
pub const SIDEBAR_MAX: u32 = 640;
/// The least the 3D stage is left beside the sidebar (`style.css` and the
/// grip in `app.js` hold to it): at 900 px with the strip at its usual
/// width that stops the sidebar at 392 px.
pub const STAGE_MIN: u32 = 420;

/// The groups folded at first, until they are unfolded by hand: what is
/// rarely touched (the reference's advanced options, G-code and notes,
/// the special modes, experimental and per-nozzle settings). Kind, page
/// and group, each by the reference's message id; an item's pages (an
/// object's, a part's, a range's) fold the process's alike, as `item`.
pub const FOLDED_AT_FIRST: &[(&str, &str, &str)] = &[
    ("process", "Quality", "Precision"),
    ("process", "Quality", "Ironing"),
    ("process", "Quality", "Z contouring"),
    ("process", "Quality", "Wall generator"),
    ("process", "Strength", "Hollowing"),
    ("process", "Strength", "Advanced"),
    ("process", "Speed", "Junction Deviation"),
    ("process", "Speed", "Jerk(XY)"),
    ("process", "Speed", "Advanced"),
    ("process", "Support", "Support ironing"),
    ("process", "Support", "Over-support surfaces"),
    ("process", "Support", "Advanced"),
    ("process", "Support", "Tree supports"),
    ("process", "Support", "Side stabilizers"),
    ("process", "Multimaterial", "Ooze prevention"),
    ("process", "Multimaterial", "Flush options"),
    ("process", "Multimaterial", "Advanced"),
    ("process", "Multimaterial", "Color Mixing (Experimental)"),
    ("process", "Others", "Special mode"),
    ("process", "Others", "G-code output"),
    ("process", "Others", "Change extrusion role G-code"),
    ("process", "Others", "Post-processing Scripts"),
    ("process", "Others", "Notes"),
    ("filament", "Cooling", "Auxiliary part cooling fan"),
    ("filament", "Cooling", "Exhaust fan"),
    ("filament", "Advanced", "Filament start G-code"),
    ("filament", "Advanced", "Change extrusion role G-code"),
    ("filament", "Advanced", "Filament end G-code"),
    (
        "filament",
        "Multimaterial",
        "Tool change parameters with single extruder MM printers",
    ),
    (
        "filament",
        "Multimaterial",
        "Tool change parameters with multi extruder MM printers",
    ),
    ("filament", "Dependencies", "Compatible printers"),
    ("filament", "Dependencies", "Compatible process profiles"),
    ("filament", "Notes", "Notes"),
    ("printer", "Basic information", "Extruder Clearance"),
    ("printer", "Basic information", "Adaptive bed mesh"),
    ("printer", "Basic information", "Accessory"),
    ("printer", "Machine G-code", "File header G-code"),
    ("printer", "Machine G-code", "Printing by object G-code"),
    ("printer", "Machine G-code", "Before layer change G-code"),
    ("printer", "Machine G-code", "Layer change G-code"),
    ("printer", "Machine G-code", "Timelapse G-code"),
    ("printer", "Machine G-code", "Change filament G-code"),
    ("printer", "Machine G-code", "Change extrusion role G-code"),
    ("printer", "Machine G-code", "Pause G-code"),
    ("printer", "Machine G-code", "Template Custom G-code"),
    (
        "printer",
        "Multimaterial",
        "Per-nozzle settings (experimental)",
    ),
    (
        "printer",
        "Multimaterial",
        "Single extruder multi-material parameters",
    ),
    ("printer", "Multimaterial", "Advanced"),
    (
        "printer",
        "Extruder %d",
        "Retraction when switching material",
    ),
    ("printer", "Motion ability", "Resonance Compensation"),
    ("printer", "Motion ability", "Jerk limitation"),
    ("printer", "Notes", "Notes"),
];

/// The book whose pages a kind's groups are of: an item's are the process's.
fn book_of(kind: &str) -> Option<Book> {
    match kind {
        "process" | "item" => Some(Book::Process),
        "filament" => Some(Book::Filament),
        "printer" => Some(Book::Printer),
        _ => None,
    }
}

/// Every group folded at first, an item's with the process's.
fn folded_at_first() -> Vec<Value> {
    FOLDED_AT_FIRST
        .iter()
        .flat_map(|&(kind, page, group)| {
            let item = (kind == "process").then_some(("item", page, group));
            std::iter::once((kind, page, group)).chain(item)
        })
        .map(|(kind, page, group)| json!({ "kind": kind, "page": page, "group": group }))
        .collect()
}

impl Session {
    /// `layout`: what the page keeps of its layout, with the limits it
    /// holds to. `sidebar_width` is null for the usual width (the style's
    /// own, which follows the window).
    pub(super) fn layout(&self) -> Value {
        let state = lock(&self.state);
        let data = &state.store.data;
        let groups: Vec<Value> = data
            .groups
            .iter()
            .map(|((kind, page, group), open)| {
                json!({ "kind": kind, "page": page, "group": group, "open": open })
            })
            .collect();
        json!({
            "sidebar_width": data.sidebar_width,
            "sidebar_min": SIDEBAR_MIN,
            "sidebar_max": SIDEBAR_MAX,
            "stage_min": STAGE_MIN,
            "groups": groups,
            "folded_at_first": folded_at_first(),
            "columns": data.columns,
        })
    }

    /// `layout_set`: `sidebar_width`, a whole number of pixels from
    /// [`SIDEBAR_MIN`] to [`SIDEBAR_MAX`], or null for the usual width;
    /// `groups`, a list of `{ kind, page, group, open }`, each a group of
    /// that kind's pages by its message ids, `open` true or false, or null
    /// to have it as it is at first. Kept at once, all or nothing; the
    /// answer is `layout`'s.
    pub(super) fn layout_set(&self, args: &Value) -> Result<Value, String> {
        let mut folds = Vec::new();
        if let Some(groups) = args.get("groups") {
            let groups = groups.as_array().ok_or("groups must be a list")?;
            for each in groups {
                let field = |name: &str| {
                    each.get(name)
                        .and_then(Value::as_str)
                        .ok_or_else(|| format!("a group needs its {name}"))
                };
                let (kind, page, group) = (field("kind")?, field("page")?, field("group")?);
                let book = book_of(kind).ok_or_else(|| format!("there are no {kind} pages"))?;
                if !Session::has_group(book, page, group) {
                    return Err(format!(
                        "the {kind} pages have no group {group:?} on {page:?}"
                    ));
                }
                let open = match each.get("open") {
                    None | Some(Value::Null) => None,
                    Some(open) => Some(open.as_bool().ok_or("open must be true, false or null")?),
                };
                folds.push(((kind.to_owned(), page.to_owned(), group.to_owned()), open));
            }
        }
        if let Some(width) = args.get("sidebar_width") {
            let width = match width {
                Value::Null => None,
                width => Some(
                    width
                        .as_u64()
                        .and_then(|width| u32::try_from(width).ok())
                        .filter(|width| (SIDEBAR_MIN..=SIDEBAR_MAX).contains(width))
                        .ok_or_else(|| {
                            format!(
                                "sidebar_width must be a whole number from {SIDEBAR_MIN} to {SIDEBAR_MAX}, or null"
                            )
                        })?,
                ),
            };
            lock(&self.state)
                .store
                .change(|data| data.sidebar_width = width)?;
        }
        // `columns`: { table: [width, …] | null }, the widths a table's
        // columns were dragged to, as the reference's tables keep them
        // (data_table.js `saveWidths`, under `storageKey`), null for the
        // widths it starts with. Tables: the Filament and the Process
        // manager.
        if let Some(columns) = args.get("columns") {
            let columns = columns.as_object().ok_or("columns must be an object")?;
            let mut kept = Vec::new();
            for (table, widths) in columns {
                if !matches!(table.as_str(), "filament_manager" | "process_manager") {
                    return Err(format!("there is no table {table}"));
                }
                let widths = match widths {
                    Value::Null => None,
                    widths => Some(
                        widths
                            .as_array()
                            .ok_or("widths must be a list")?
                            .iter()
                            .map(|width| {
                                width
                                    .as_u64()
                                    .and_then(|width| u32::try_from(width).ok())
                                    .filter(|width| (16..=2000).contains(width))
                                    .ok_or_else(|| {
                                        "a width must be a whole number from 16 to 2000".to_owned()
                                    })
                            })
                            .collect::<Result<Vec<u32>, String>>()?,
                    ),
                };
                kept.push((table.clone(), widths));
            }
            lock(&self.state).store.change(|data| {
                for (table, widths) in kept {
                    match widths {
                        Some(widths) => {
                            data.columns.insert(table, widths);
                        }
                        None => {
                            data.columns.remove(&table);
                        }
                    }
                }
            })?;
        }
        if !folds.is_empty() {
            lock(&self.state).store.change(|data| {
                for (key, open) in folds {
                    match open {
                        Some(open) => {
                            data.groups.insert(key, open);
                        }
                        None => {
                            data.groups.remove(&key);
                        }
                    }
                }
            })?;
        }
        Ok(self.layout())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn the_sidebar_width_is_kept_between_runs_within_its_limits() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
        let answer = session.call("layout", &json!({})).unwrap();
        assert_eq!(answer["sidebar_width"], Value::Null);
        assert_eq!(answer["sidebar_min"], SIDEBAR_MIN);
        assert_eq!(answer["sidebar_max"], SIDEBAR_MAX);
        assert_eq!(answer["stage_min"], STAGE_MIN);

        let answer = session
            .call("layout_set", &json!({ "sidebar_width": 452 }))
            .unwrap();
        assert_eq!(answer["sidebar_width"], 452);
        // Another run finds it.
        let again = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
        assert_eq!(
            again.call("layout", &json!({})).unwrap()["sidebar_width"],
            452
        );

        // Outside the limits, or not a whole number: refused, and nothing changes.
        for wrong in [
            json!(SIDEBAR_MIN - 1),
            json!(SIDEBAR_MAX + 1),
            json!(400.5),
            json!("400"),
            json!(-1),
        ] {
            let said = again
                .call("layout_set", &json!({ "sidebar_width": wrong }))
                .unwrap_err();
            assert!(said.contains("sidebar_width"), "{said}");
        }
        assert_eq!(
            again.call("layout", &json!({})).unwrap()["sidebar_width"],
            452
        );

        // Null is the usual width again, and nothing is kept for it.
        let answer = again
            .call("layout_set", &json!({ "sidebar_width": null }))
            .unwrap();
        assert_eq!(answer["sidebar_width"], Value::Null);
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
        )
        .unwrap();
        assert!(written.get("layout").is_none(), "{written}");
    }

    /// Every group folded at first is a group of its kind's pages, as the
    /// pages are made from the reference (a pin bump that renames one
    /// fails here).
    #[test]
    fn the_groups_folded_at_first_are_groups_of_the_pages() {
        for &(kind, page, group) in FOLDED_AT_FIRST {
            let book = book_of(kind).unwrap();
            assert!(
                Session::has_group(book, page, group),
                "{kind}: no group {group:?} on {page:?}"
            );
        }
        let all = folded_at_first();
        assert!(all.contains(&json!({ "kind": "item", "page": "Others", "group": "Notes" })));
        assert!(
            !all.iter()
                .any(|each| each["kind"] == "item" && each["page"] == "Notes")
        );
    }

    #[test]
    fn a_groups_state_is_kept_between_runs_by_kind_page_and_group() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
        let answer = session.call("layout", &json!({})).unwrap();
        assert_eq!(answer["groups"], json!([]));
        assert!(
            answer["folded_at_first"]
                .as_array()
                .unwrap()
                .contains(&json!({ "kind": "process", "page": "Others", "group": "Special mode" }))
        );

        // "Advanced" is on several pages and in several kinds: each its own.
        let answer = session
            .call(
                "layout_set",
                &json!({ "groups": [
                    { "kind": "process", "page": "Strength", "group": "Advanced", "open": true },
                    { "kind": "process", "page": "Quality", "group": "Seam", "open": false },
                    { "kind": "printer", "page": "Extruder %d", "group": "Retraction", "open": false },
                    { "kind": "item", "page": "Quality", "group": "Seam", "open": true },
                ] }),
            )
            .unwrap();
        assert_eq!(answer["groups"].as_array().unwrap().len(), 4);
        let again = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
        let groups = again.call("layout", &json!({})).unwrap()["groups"].clone();
        let open = |kind: &str, page: &str, group: &str| {
            groups
                .as_array()
                .unwrap()
                .iter()
                .find(|each| each["kind"] == kind && each["page"] == page && each["group"] == group)
                .map(|each| each["open"].clone())
        };
        assert_eq!(open("process", "Strength", "Advanced"), Some(json!(true)));
        assert_eq!(open("process", "Speed", "Advanced"), None);
        assert_eq!(open("process", "Quality", "Seam"), Some(json!(false)));
        assert_eq!(open("item", "Quality", "Seam"), Some(json!(true)));
        assert_eq!(
            open("printer", "Extruder %d", "Retraction"),
            Some(json!(false))
        );
        assert_eq!(open("filament", "Setting Overrides", "Retraction"), None);

        // A group the pages do not have, a kind there is none of, an
        // open that is not true, false or null: refused, nothing kept.
        for wrong in [
            json!([{ "kind": "process", "page": "Quality", "group": "Nope", "open": true }]),
            json!([{ "kind": "process", "page": "Qualität", "group": "Seam", "open": true }]),
            json!([{ "kind": "machine", "page": "Notes", "group": "Notes", "open": true }]),
            json!([{ "kind": "process", "page": "Quality", "group": "Seam", "open": "yes" }]),
            json!([
                { "kind": "process", "page": "Quality", "group": "Ironing", "open": true },
                { "kind": "process", "page": "Quality" },
            ]),
            json!({ "kind": "process" }),
        ] {
            assert!(
                again
                    .call("layout_set", &json!({ "groups": wrong }))
                    .is_err(),
                "{wrong}"
            );
        }
        assert_eq!(again.call("layout", &json!({})).unwrap()["groups"], groups);

        // Null puts a group back as it is at first.
        let answer = again
            .call(
                "layout_set",
                &json!({ "groups": [{ "kind": "process", "page": "Quality", "group": "Seam", "open": null }] }),
            )
            .unwrap();
        assert_eq!(answer["groups"].as_array().unwrap().len(), 3);
    }
}
