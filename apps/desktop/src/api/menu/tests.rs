//! The menu bar's description, what it answers, the recent projects and
//! the STL exports.

use std::{collections::HashSet, path::Path, sync::Arc};

use serde_json::{Value, json};

use super::*;
use crate::api::i18n;

const THREE_PLATES: &str = "tests/mcbrim_test.3mf";
const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";

fn reference(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
    session
        .call(name, &args)
        .unwrap_or_else(|error| panic!("{name} {args}: {error}"))
}

/// The entries of a menu answer, flattened, by id.
fn flat(menus: &Value) -> Vec<Value> {
    fn walk(items: &Value, out: &mut Vec<Value>) {
        for item in items.as_array().into_iter().flatten() {
            out.push(item.clone());
            walk(&item["items"], out);
        }
    }
    let mut out = Vec::new();
    walk(&menus["menus"], &mut out);
    out
}

fn find(menus: &Value, id: &str) -> Value {
    flat(menus)
        .into_iter()
        .find(|item| item["id"] == id)
        .unwrap_or_else(|| panic!("no entry {id}"))
}

#[test]
fn every_id_is_one_entrys_on_each_system() {
    for mac in [true, false] {
        let mut seen = HashSet::new();
        for (_, item) in entries() {
            if shows(item.os, mac) {
                assert!(seen.insert(item.id), "{} twice (mac: {mac})", item.id);
            }
        }
    }
    // What the page carries out is named by these ids; a few it must find.
    let ids: HashSet<&str> = entries().iter().map(|(_, item)| item.id).collect();
    for id in [
        "file.new",
        "file.open",
        "file.save",
        "file.save_as",
        "import.model",
        "export.stl",
        "export.stls",
        "export.gcode",
        "edit.undo",
        "edit.redo",
        "view.default",
        "help.shortcuts",
        "app.about",
        "app.preferences",
        "file.quit",
    ] {
        assert!(ids.contains(id), "{id}");
    }
}

/// Every label and tooltip is a message id of the reference's catalog,
/// or one of its words newer than the catalog (`wording.txt`); what an
/// entry waits for is this app's own and listed there too.
#[test]
fn every_word_of_the_bar_is_a_message_id_or_listed() {
    let ids = i18n::tests::message_ids();
    let (newer, own) = i18n::tests::listed();
    let known = |words: &str| ids.contains(words) || newer.iter().any(|listed| listed == words);
    let mut unknown = Vec::new();
    for (_, item) in entries() {
        if !known(item.label) {
            unknown.push(item.label.to_owned());
        }
        if !item.tip.is_empty() && !known(item.tip) {
            unknown.push(item.tip.to_owned());
        }
        if let Some(waits) = item.waits
            && !own.iter().any(|listed| listed == waits)
        {
            unknown.push(waits.to_owned());
        }
    }
    fn menus(nodes: &[Node], out: &mut Vec<&'static str>) {
        for node in nodes {
            if let Node::Menu {
                label, items, id, ..
            } = node
            {
                if *id != "app" {
                    out.push(label);
                }
                menus(items, out);
            }
        }
    }
    let mut labels = Vec::new();
    menus(BAR, &mut labels);
    unknown.extend(
        labels
            .into_iter()
            .filter(|label| !known(label))
            .map(str::to_owned),
    );
    unknown.sort();
    unknown.dedup();
    assert!(
        unknown.is_empty(),
        "not a message id nor listed: {unknown:#?}"
    );
}

#[test]
fn labels_lose_their_marks_and_keep_their_dots() {
    assert_eq!(shown_label("Show &G-code Window"), "Show G-code Window");
    assert_eq!(
        shown_label("Export && Open in Bambu Studio"),
        "Export & Open in Bambu Studio"
    );
    let english = i18n::english();
    let about = entries()
        .into_iter()
        .find(|(_, item)| item.id == "app.about")
        .unwrap()
        .1;
    let open = entries()
        .into_iter()
        .find(|(_, item)| item.id == "file.open")
        .unwrap()
        .1;
    i18n::with(english, || {
        assert_eq!(label_of(&about), "About EdgeSlicerRS");
        assert_eq!(label_of(&open), "Open Project...");
    });
    let german = i18n::catalog(i18n::find("de_DE").unwrap());
    i18n::with(german, || {
        assert_eq!(label_of(&open), format!("{}...", tr("Open Project")));
        assert_ne!(tr("Open Project"), "Open Project");
    });
}

#[test]
fn keys_are_shown_as_each_system_shows_them_and_bound_once() {
    i18n::with(i18n::english(), || {
        assert_eq!(key_text(&command_shift("S"), true), "⇧⌘S");
        assert_eq!(key_text(&command_shift("S"), false), "Ctrl+Shift+S");
        assert_eq!(key_text(&plain("Delete"), false), "Del");
        assert_eq!(key_text(&plain("Escape"), true), "Esc");
    });
    let by_id = |id: &str| {
        entries()
            .into_iter()
            .find(|(_, item)| item.id == id)
            .unwrap()
            .1
    };
    assert_eq!(
        accelerator(&by_id("file.save_as")).as_deref(),
        Some("CmdOrCtrl+Shift+S")
    );
    assert_eq!(
        accelerator(&by_id("window.next")).as_deref(),
        Some("CmdOrCtrl+Shift+BracketRight")
    );
    // The clipboard's keys stay the system's; keys without the command
    // key are the page's (a text field keeps Del, Esc, C and Z).
    for id in [
        "edit.cut",
        "edit.copy",
        "edit.paste",
        "edit.delete",
        "edit.deselect_all",
        "view.fit",
        "view.gcode_window",
        "help.shortcuts",
    ] {
        assert_eq!(accelerator(&by_id(id)), None, "{id}");
    }
    // No two entries of one system share a key.
    for mac in [true, false] {
        let mut keys = HashSet::new();
        for (_, item) in entries() {
            if let Some(key) = item.key
                && shows(item.os, mac)
            {
                assert!(
                    keys.insert((key.command, key.shift, key.key)),
                    "{} (mac: {mac})",
                    item.id
                );
            }
        }
    }
}

#[test]
fn entries_follow_the_state_and_what_is_not_built_stays_off() {
    let page = PageState {
        tab: "prepare".into(),
        ..PageState::default()
    };
    let mut session = SessionState::default();
    let tree_now = |page: &PageState, session: &SessionState| {
        i18n::with(i18n::english(), || tree(page, session, false))
    };
    let menus = tree_now(&page, &session);
    assert_eq!(find(&menus, "file.new")["enabled"], true);
    assert_eq!(find(&menus, "file.save")["enabled"], false);
    assert_eq!(find(&menus, "export.stl")["enabled"], false);
    assert_eq!(find(&menus, "edit.undo")["enabled"], false);
    assert_eq!(find(&menus, "edit.undo")["label"], "Undo");
    assert_eq!(find(&menus, "file.recent")["enabled"], false);
    assert_eq!(find(&menus, "import.configs")["enabled"], false);

    session.objects = true;
    session.dirty = true;
    session.undo = Some("Delete".into());
    session.recent = vec![("/a/b.3mf".into(), false), ("/gone.3mf".into(), true)];
    session.calls = vec!["preset_import"];
    let menus = tree_now(&page, &session);
    assert_eq!(find(&menus, "file.save")["enabled"], true);
    assert_eq!(find(&menus, "export.stl")["enabled"], true);
    assert_eq!(find(&menus, "edit.undo")["label"], "Undo: Delete");
    assert_eq!(find(&menus, "edit.undo")["enabled"], true);
    assert_eq!(find(&menus, "import.configs")["enabled"], true);
    assert_eq!(find(&menus, "export.preset_bundle")["enabled"], false);
    assert_eq!(find(&menus, "file.recent")["enabled"], true);
    assert_eq!(find(&menus, "recent:0")["label"], "1 /a/b.3mf");
    assert_eq!(find(&menus, "recent:1")["missing"], true);
    // Undo is Prepare's.
    let preview = PageState {
        tab: "preview".into(),
        gcode_window: true,
        ..PageState::default()
    };
    let menus = tree_now(&preview, &session);
    assert_eq!(find(&menus, "edit.undo")["enabled"], false);
    assert_eq!(find(&menus, "view.gcode_window")["checked"], true);
    // A slice running: no new project, no opening, no saving.
    session.slicing = true;
    let menus = tree_now(&page, &session);
    for id in ["file.new", "file.open", "file.save", "import.model"] {
        assert_eq!(find(&menus, id)["enabled"], false, "{id}");
    }
    // What is not built is off whatever the state, and says why.
    for (_, item) in entries() {
        if item.waits.is_some() && shows(item.os, false) {
            let shown = find(&menus, item.id);
            assert_eq!(shown["enabled"], false, "{}", item.id);
            assert_eq!(shown["waits"], true, "{}", item.id);
            assert!(
                shown["tip"].as_str().unwrap().starts_with("not yet"),
                "{}",
                item.id
            );
        }
    }
}

#[test]
fn lines_never_lead_trail_or_double() {
    for mac in [true, false] {
        let menus = i18n::with(i18n::english(), || {
            tree(&PageState::default(), &SessionState::default(), mac)
        });
        fn check(items: &Value) {
            let items = items.as_array().unwrap();
            if items.is_empty() {
                return;
            }
            assert_ne!(items[0]["line"], true);
            assert_ne!(items[items.len() - 1]["line"], true);
            for pair in items.windows(2) {
                assert!(!(pair[0]["line"] == true && pair[1]["line"] == true));
            }
            for item in items {
                if item.get("items").is_some() {
                    check(&item["items"]);
                }
            }
        }
        for menu in menus["menus"].as_array().unwrap() {
            check(&menu["items"]);
        }
        let names: Vec<&str> = menus["menus"]
            .as_array()
            .unwrap()
            .iter()
            .map(|menu| menu["id"].as_str().unwrap())
            .collect();
        if mac {
            assert_eq!(
                names,
                [
                    "app",
                    "file",
                    "edit",
                    "view",
                    "calibration",
                    "window",
                    "help"
                ]
            );
        } else {
            assert_eq!(names, ["file", "edit", "view", "calibration", "help"]);
        }
    }
}

#[test]
fn the_bar_reads_in_german() {
    let german = i18n::catalog(i18n::find("de_DE").unwrap());
    let menus = i18n::with(german, || {
        tree(&PageState::default(), &SessionState::default(), false)
    });
    let file = &menus["menus"][0];
    assert_eq!(file["label"], "Datei");
    assert_ne!(find(&menus, "file.new")["label"], "New Project");
    assert!(
        find(&menus, "file.save_as")["key"]
            .as_str()
            .unwrap()
            .contains('S')
    );
}

#[test]
fn recent_projects_put_the_latest_first_once_and_keep_eighteen() {
    let mut recent: Vec<String> = (0..20).map(|n| format!("/p/{n}.3mf")).collect();
    recent = with_recent(&recent, "/p/5.3mf");
    assert_eq!(recent[0], "/p/5.3mf");
    assert_eq!(recent.iter().filter(|path| *path == "/p/5.3mf").count(), 1);
    assert_eq!(recent.len(), RECENT_MOST);
}

#[test]
fn the_shortcuts_window_lists_the_references_keys() {
    let lists = i18n::with(i18n::english(), || shortcuts(false));
    let sections = lists["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 5);
    assert_eq!(sections[0]["title"], "Global shortcuts");
    for section in sections {
        for row in section["rows"].as_array().unwrap() {
            assert!(!row["key"].as_str().unwrap().is_empty(), "{row}");
        }
    }
    assert_eq!(sections[0]["rows"][0]["key"], "Ctrl+N");
    let mac = i18n::with(i18n::english(), || shortcuts(true));
    assert_eq!(mac["sections"][0]["rows"][3]["key"], "⇧⌘S");
    // The words of the window are message ids or listed.
    let ids = i18n::tests::message_ids();
    let (newer, _) = i18n::tests::listed();
    for (title, rows) in sections_of_both() {
        for words in std::iter::once(title).chain(rows) {
            assert!(
                ids.contains(words) || newer.iter().any(|listed| listed == words),
                "{words}"
            );
        }
    }
}

fn sections_of_both() -> Vec<(&'static str, Vec<&'static str>)> {
    let mut out = Vec::new();
    for mac in [true, false] {
        for (title, rows) in sections(mac) {
            let mut words: Vec<&'static str> = rows.iter().map(|row| row.does).collect();
            for row in &rows {
                if let Keyed::Words(words_of)
                | Keyed::Command(words_of)
                | Keyed::Shift(words_of)
                | Keyed::Alt(words_of) = row.key
                    && words_of.chars().count() > 1
                {
                    words.push(words_of);
                }
            }
            out.push((title, words));
        }
    }
    out
}

// ---- through the session ----

fn session_in(dir: &Path) -> Arc<Session> {
    Arc::new(Session::with_settings(None, Some(dir.join("settings"))))
}

#[test]
fn a_project_opened_or_saved_is_the_latest_recent_one_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("plates.3mf");
    std::fs::copy(reference(THREE_PLATES), &copy).unwrap();
    let small = dir.path().join("small.3mf");
    std::fs::copy(reference(SMALL), &small).unwrap();
    {
        let session = session_in(dir.path());
        assert!(
            ask(&session, "menu", json!({}))["recent"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        ask(&session, "project_open", json!({ "path": copy }));
        ask(&session, "project_open", json!({ "path": small }));
        let menu = ask(&session, "menu", json!({}));
        let recent: Vec<&str> = menu["recent"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect();
        assert_eq!(recent, [shown(&small), shown(&copy)]);
        // Saved as another file: that one first.
        let other = dir.path().join("saved.3mf");
        let id = ask(&session, "projects", json!({}))["projects"][0]["id"].clone();
        let saved = ask(
            &session,
            "project_save",
            json!({ "project": id, "path": other, "accept": true }),
        );
        assert_eq!(saved["saved"], true);
        let menu = ask(&session, "menu", json!({}));
        assert_eq!(menu["recent"][0]["path"], shown(&other));
        assert_eq!(menu["recent"].as_array().unwrap().len(), 3);
        // An STL opened as a project is not a recent project.
        ask(
            &session,
            "project_open",
            json!({ "path": reference("resources/handy_models/OrcaToleranceTest.stl") }),
        );
        assert_eq!(
            ask(&session, "menu", json!({}))["recent"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
    // Kept for the next run, and a file no longer there is marked.
    std::fs::remove_file(&small).unwrap();
    let session = session_in(dir.path());
    let menu = ask(&session, "menu", json!({}));
    let missing: Vec<bool> = menu["recent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["missing"].as_bool().unwrap())
        .collect();
    assert_eq!(missing, [false, true, false]);
    let left = ask(&session, "recent_remove", json!({ "path": shown(&small) }));
    assert_eq!(left["recent"].as_array().unwrap().len(), 2);
    let again = session_in(dir.path());
    assert_eq!(
        ask(&again, "menu", json!({}))["recent"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn objects_export_as_one_stl_or_one_for_each_and_nothing_is_written_over() {
    let dir = tempfile::tempdir().unwrap();
    let session = session_in(dir.path());
    // Nothing to export in an empty project.
    let error = session
        .call("export_stl", &json!({ "path": dir.path().join("x.stl") }))
        .unwrap_err();
    assert_eq!(error, "There are no objects in the project.");
    ask(
        &session,
        "project_open",
        json!({ "path": reference(THREE_PLATES) }),
    );
    let suggested = ask(&session, "export_stl", json!({ "suggest": true }));
    assert!(
        suggested["suggested"]
            .as_str()
            .unwrap()
            .ends_with("mcbrim_test.stl")
    );
    let path = dir.path().join("all");
    let written = ask(&session, "export_stl", json!({ "path": path }));
    let file = dir.path().join("all.stl");
    assert_eq!(written["path"], shown(&file));
    edge_engine::open(&file, None).unwrap();
    let triangles = written["triangles"].as_u64().unwrap() as usize;
    assert!(triangles > 0);
    assert_eq!(
        std::fs::metadata(&file).unwrap().len(),
        84 + 50 * triangles as u64
    );
    // There already: asked first, written only when told to.
    let asked = ask(&session, "export_stl", json!({ "path": file }));
    assert_eq!(asked["exists"], true);
    ask(
        &session,
        "export_stl",
        json!({ "path": file, "replace": true }),
    );
    // One file for each object; a second export does not write over them.
    let out = dir.path().join("each");
    std::fs::create_dir(&out).unwrap();
    let first = ask(&session, "export_stls", json!({ "dir": out }));
    let second = ask(&session, "export_stls", json!({ "dir": out }));
    let count = first["files"].as_array().unwrap().len();
    assert!(count >= 1);
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 2 * count);
    for path in second["files"].as_array().unwrap() {
        assert!(
            !first["files"].as_array().unwrap().contains(path),
            "{path} written over"
        );
    }
    let objects: usize = first["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| ((std::fs::metadata(path.as_str().unwrap()).unwrap().len() - 84) / 50) as usize)
        .sum();
    assert_eq!(
        objects, triangles,
        "the one file is every object's together"
    );
    // Nothing is left beside them.
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[test]
fn an_obj_is_added_to_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let obj = dir.path().join("cube.obj");
    std::fs::write(
        &obj,
        "v 0 0 0\nv 10 0 0\nv 10 10 0\nv 0 10 0\nv 0 0 10\nv 10 0 10\nv 10 10 10\nv 0 10 10\nf 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n",
    )
    .unwrap();
    let session = session_in(dir.path());
    ask(
        &session,
        "project_open",
        json!({ "path": reference(THREE_PLATES) }),
    );
    let before = ask(&session, "scene", json!({}))["objects"]
        .as_array()
        .unwrap()
        .len();
    let added = ask(&session, "scene_edit", json!({ "op": "add", "path": obj }));
    let objects = added["scene"]["objects"].as_array().unwrap();
    assert_eq!(objects.len(), before + 1);
    assert_eq!(objects.last().unwrap()["triangles"], 12);
}

#[test]
fn no_preset_calls_are_found_on_this_branch() {
    let dir = tempfile::tempdir().unwrap();
    let session = session_in(dir.path());
    assert_eq!(ask(&session, "menu", json!({}))["preset_calls"], json!([]));
}
