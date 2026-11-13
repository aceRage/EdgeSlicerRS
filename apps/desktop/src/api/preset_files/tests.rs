use std::path::{Path, PathBuf};

use edge_config::bundle::{self, Entry};

use super::*;

fn reference(path: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
        .to_string_lossy()
        .into_owned()
}

/// A session that keeps its settings and presets in `dir`, on the
/// reference's profiles, with the Snapmaker U1 added and chosen.
fn opened(dir: &Path) -> Arc<Session> {
    let session = Arc::new(Session::with_settings(None, Some(dir.to_owned())));
    ask(
        &session,
        "set_profiles",
        json!({ "dir": reference("resources/profiles") }),
    );
    ask(
        &session,
        "printers_save",
        json!({ "models": [{ "vendor": "Snapmaker", "model": "Snapmaker U1" }] }),
    );
    ask(&session, "printer_select", json!({ "initial": true }));
    session
}

fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
    session
        .call(name, &args)
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn refused(session: &Arc<Session>, name: &str, args: Value) -> String {
    session
        .call(name, &args)
        .expect_err(&format!("{name} was not refused"))
}

fn user_dir(dir: &Path) -> PathBuf {
    dir.join("user").join("default")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

const U1: &str = "Snapmaker U1 (0.4 nozzle)";
const PROCESS: &str = "0.20 Standard @Snapmaker U1 (0.4 nozzle)";

/// Saves "My Process" (three walls) over the project's process preset.
fn save_process(session: &Arc<Session>) {
    ask(
        session,
        "process_set",
        json!({ "key": "wall_loops", "value": 3 }),
    );
    ask(
        session,
        "preset_save",
        json!({ "kind": "process", "name": "My Process" }),
    );
}

/// A process preset as the C++ application keeps it.
fn cpp_process(name: &str, inherits: &str, extra: &str) -> String {
    format!(
        "{{\n    \"from\": \"User\",\n    \"inherits\": \"{inherits}\",\n    \"is_custom_defined\": \"0\",\n    \"name\": \"{name}\",\n    \"print_settings_id\": \"{name}\",\n{extra}    \"version\": \"2.3.3.14\",\n    \"wall_loops\": \"4\"\n}}\n"
    )
}

#[test]
fn without_a_data_folder_nothing_is_imported_or_exported() {
    let session = Arc::new(Session::with_settings(None, None));
    assert_eq!(
        refused(&session, "preset_import", json!({ "path": "x.json" })),
        NO_FOLDER
    );
    assert_eq!(
        refused(&session, "preset_export", json!({ "type": "process" })),
        NO_FOLDER
    );
}

#[test]
fn process_presets_go_out_in_a_zip_and_come_back_as_they_were() {
    let home = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let session = opened(home.path());
    save_process(&session);
    let saved = read(
        &user_dir(home.path())
            .join("process")
            .join("My Process.json"),
    );

    let choices = ask(&session, "preset_export", json!({}));
    assert_eq!(choices["process"], json!(["My Process"]));
    assert_eq!(
        choices["types"][0]["title"],
        "Printer config bundle(.orca_printer)"
    );
    assert_eq!(
        choices["types"][0]["note"],
        "Printer and all the filament&process presets that belongs to the printer.\nCan be shared with others."
    );
    // The system printer goes in a bundle: a preset of the person's goes
    // with it.
    assert_eq!(choices["printer_bundle"], json!([U1]));

    let path = out.path().to_string_lossy().into_owned();
    let exported = ask(
        &session,
        "preset_export",
        json!({ "path": path, "type": "process" }),
    );
    assert_eq!(exported["exported"], true);
    assert_eq!(exported["message"], "Export successful");
    let zip = out.path().join("Process presets.zip");
    assert_eq!(exported["files"], json!([shown(&zip)]));
    let found = bundle::read_path(&zip).unwrap();
    assert_eq!(found.presets.len(), 1);
    assert_eq!(found.presets[0].file, "My Process.json");

    // A second export asks before clearing the zip; or takes a new name.
    let again = ask(
        &session,
        "preset_export",
        json!({ "path": path, "type": "process" }),
    );
    assert_eq!(again["exported"], false);
    assert!(
        again["ask"]
            .as_str()
            .unwrap()
            .starts_with("The 'Process presets.zip' folder already exists"),
        "{again}"
    );
    let beside = ask(
        &session,
        "preset_export",
        json!({ "path": path, "type": "process", "replace": false }),
    );
    let beside = beside["files"][0].as_str().unwrap();
    assert!(beside.contains("Process presets 2"), "{beside}");
    assert!(Path::new(beside).exists());
    let cleared = ask(
        &session,
        "preset_export",
        json!({ "path": path, "type": "process", "replace": true }),
    );
    assert_eq!(cleared["files"], json!([shown(&zip)]));

    // Into another person's folder: the same file.
    let other = tempfile::tempdir().unwrap();
    let theirs = opened(other.path());
    let imported = ask(&theirs, "preset_import", json!({ "path": shown(&zip) }));
    assert_eq!(imported["imported"], true, "{imported}");
    assert_eq!(imported["process"], json!(["My Process"]));
    assert_eq!(imported["title"], "Import result");
    assert_eq!(
        imported["message"],
        "There is 1 config imported. (Only non-system and compatible configs)"
    );
    assert_eq!(imported["presets"]["process"][0]["name"], "My Process");
    assert_eq!(
        read(
            &user_dir(other.path())
                .join("process")
                .join("My Process.json")
        ),
        saved
    );
    let info = read(
        &user_dir(other.path())
            .join("process")
            .join("My Process.info"),
    );
    assert!(info.contains("sync_info = create"), "{info}");

    // Again: the person is asked first, in the reference's words.
    let asked = ask(&theirs, "preset_import", json!({ "path": shown(&zip) }));
    assert_eq!(asked["imported"], false);
    assert_eq!(asked["title"], "Overwrite config");
    assert_eq!(
        asked["ask"],
        "A config exists with the same name: My Process. Do you want to overwrite it?"
    );
    assert_eq!(
        asked["conflicts"],
        json!([{ "kind": "process", "name": "My Process" }])
    );
    let kept = ask(
        &theirs,
        "preset_import",
        json!({ "path": shown(&zip), "overwrite": false }),
    );
    assert_eq!(kept["process"], json!([]));
    assert_eq!(kept["passed"].as_array().unwrap().len(), 1);
    assert!(
        kept["message"]
            .as_str()
            .unwrap()
            .contains("Make sure you have added the corresponding printer"),
        "{kept}"
    );
    let over = ask(
        &theirs,
        "preset_import",
        json!({ "path": shown(&zip), "overwrite": true }),
    );
    assert_eq!(over["process"], json!(["My Process"]));
    let info = read(
        &user_dir(other.path())
            .join("process")
            .join("My Process.info"),
    );
    assert!(info.contains("sync_info = update"), "{info}");
}

#[test]
fn a_printer_bundle_holds_the_printer_whole_and_its_presets() {
    let home = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let session = opened(home.path());
    save_process(&session);
    ask(
        &session,
        "settings_set",
        json!({ "kind": "printer", "key": "printable_height", "value": 250 }),
    );
    ask(
        &session,
        "preset_save",
        json!({ "kind": "printer", "name": "My U1" }),
    );
    let printer_file = read(&user_dir(home.path()).join("machine").join("My U1.json"));

    let choices = ask(&session, "preset_export", json!({}));
    assert_eq!(choices["printer_bundle"], json!([U1, "My U1"]));
    let path = out.path().to_string_lossy().into_owned();
    let exported = ask(
        &session,
        "preset_export",
        json!({ "path": path, "type": "printer_bundle", "names": ["My U1"] }),
    );
    let file = out.path().join("My U1.orca_printer");
    assert_eq!(exported["files"], json!([shown(&file)]));
    let found = bundle::read_path(&file).unwrap();
    let files: Vec<&str> = found.presets.iter().map(|p| p.file.as_str()).collect();
    assert_eq!(files, ["printer/My U1.json", "process/My Process.json"]);
    // The printer whole: far more than what it sets over the U1.
    let printer = &found.presets[0];
    assert_eq!(printer.inherits.as_deref(), Some(U1));
    assert!(printer.settings.len() > 50, "{}", printer.settings.len());
    assert_eq!(printer.settings.text("printable_height"), Some("250"));

    // Imported elsewhere, the printer is what it sets over the U1 again.
    let other = tempfile::tempdir().unwrap();
    let theirs = opened(other.path());
    let imported = ask(&theirs, "preset_import", json!({ "path": shown(&file) }));
    assert_eq!(imported["printer"], json!(["My U1"]), "{imported}");
    assert_eq!(imported["process"], json!(["My Process"]));
    assert_eq!(
        read(&user_dir(other.path()).join("machine").join("My U1.json")),
        printer_file
    );

    // The printer zip holds it whole too, by its name.
    ask(
        &session,
        "preset_export",
        json!({ "path": path, "type": "printer" }),
    );
    let found = bundle::read_path(&out.path().join("Printer presets.zip")).unwrap();
    assert_eq!(found.presets[0].file, "My U1.json");
    assert!(found.presets[0].settings.len() > 50);
}

#[test]
fn a_filament_bundle_is_written_by_filament_name() {
    let home = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let session = opened(home.path());
    ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "key": "nozzle_temperature", "value": 212 }),
    );
    ask(
        &session,
        "preset_save",
        json!({ "kind": "filament", "filament": 1, "name": "My PLA" }),
    );
    let choices = ask(&session, "preset_export", json!({}));
    assert_eq!(choices["filament"], json!(["My PLA"]));
    let groups = choices["filament_bundle"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "{choices}");
    let group = groups[0].as_str().unwrap();
    assert_eq!(group, "Generic PLA");
    let path = out.path().to_string_lossy().into_owned();
    let exported = ask(
        &session,
        "preset_export",
        json!({ "path": path, "type": "filament_bundle" }),
    );
    let file = out.path().join("Generic PLA.orca_filament");
    assert_eq!(exported["files"], json!([shown(&file)]));
    // Under the first word of the printer its system preset's name gives
    // ("Generic PLA @U1 ...": "U1").
    let found = bundle::read_path(&file).unwrap();
    assert_eq!(found.presets.len(), 1);
    assert_eq!(found.presets[0].file, "U1/My PLA.json");

    // One brought in from a file goes the same way.
    let theirs = r#"{
    "filament_settings_id": [
        "Their PA @U1"
    ],
    "from": "User",
    "inherits": "Generic PA @U1 0.4 nozzle",
    "is_custom_defined": "0",
    "name": "Their PA @U1",
    "nozzle_temperature": [
        "275"
    ],
    "version": "2.3.3.14"
}
"#;
    let json_file = home.path().join("Their PA @U1.json");
    std::fs::write(&json_file, theirs).unwrap();
    let imported = ask(
        &session,
        "preset_import",
        json!({ "path": shown(&json_file) }),
    );
    assert_eq!(imported["filament"], json!(["Their PA @U1"]), "{imported}");
    let choices = ask(&session, "preset_export", json!({}));
    assert_eq!(
        choices["filament_bundle"],
        json!(["Generic PA", "Generic PLA"])
    );
    ask(
        &session,
        "preset_export",
        json!({ "path": path, "type": "filament_bundle", "names": ["Generic PA"] }),
    );
    let file = out.path().join("Generic PA.orca_filament");
    let found = bundle::read_path(&file).unwrap();
    assert_eq!(found.presets.len(), 1);
    assert_eq!(found.presets[0].file, "U1/Their PA @U1.json");
    let other = tempfile::tempdir().unwrap();
    let elsewhere = opened(other.path());
    let imported = ask(&elsewhere, "preset_import", json!({ "path": shown(&file) }));
    assert_eq!(imported["filament"], json!(["Their PA @U1"]), "{imported}");
    assert_eq!(
        read(
            &user_dir(other.path())
                .join("filament")
                .join("Their PA @U1.json")
        ),
        read(
            &user_dir(home.path())
                .join("filament")
                .join("Their PA @U1.json")
        )
    );
}

#[test]
fn an_archive_as_the_cpp_writes_it_is_imported_parents_first() {
    let home = tempfile::tempdir().unwrap();
    let session = opened(home.path());
    let child = cpp_process("Child", "Parent", "");
    let parent = cpp_process("Parent", PROCESS, "");
    let orphan = cpp_process("Orphan", "Nobody's Process", "");
    let system = cpp_process(PROCESS, "", "");
    let bytes = bundle::zip(
        &[
            Entry {
                name: "process/Child.json".into(),
                bytes: child.into_bytes(),
            },
            Entry {
                name: "process/Parent.json".into(),
                bytes: parent.into_bytes(),
            },
            Entry {
                name: "process/Orphan.json".into(),
                bytes: orphan.into_bytes(),
            },
            Entry {
                name: "process/System.json".into(),
                bytes: system.into_bytes(),
            },
            Entry {
                name: bundle::BUNDLE_STRUCTURE.into(),
                bytes: b"{}".to_vec(),
            },
        ],
        0,
    )
    .unwrap();
    let file = home.path().join("from cpp.orca_printer");
    std::fs::write(&file, bytes).unwrap();
    let imported = ask(&session, "preset_import", json!({ "path": shown(&file) }));
    assert_eq!(
        imported["process"],
        json!(["Parent", "Child"]),
        "{imported}"
    );
    let passed: Vec<&str> = imported["passed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["file"].as_str().unwrap())
        .collect();
    assert_eq!(passed, ["process/System.json", "process/Orphan.json"]);
    // The child sets nothing its parent does not.
    let child = read(&user_dir(home.path()).join("process").join("Child.json"));
    assert!(!child.contains("wall_loops"), "{child}");
    assert!(child.contains("\"inherits\": \"Parent\""), "{child}");
    let parent = read(&user_dir(home.path()).join("process").join("Parent.json"));
    assert!(parent.contains("\"wall_loops\": \"4\""), "{parent}");
}

#[test]
fn scripts_an_imported_preset_brings_are_asked_about_and_removed() {
    let home = tempfile::tempdir().unwrap();
    let session = opened(home.path());
    let text = cpp_process(
        "Scripted",
        PROCESS,
        "    \"post_process\": [\n        \"/usr/bin/evil --now\"\n    ],\n",
    );
    let file = home.path().join("Scripted.json");
    std::fs::write(&file, text).unwrap();
    let asked = ask(&session, "preset_import", json!({ "path": shown(&file) }));
    assert_eq!(asked["imported"], false);
    assert_eq!(asked["title"], "Untrusted settings in file");
    let message = asked["ask"].as_str().unwrap();
    assert!(
        message.starts_with("\"Scripted\" contains settings"),
        "{message}"
    );
    assert!(message.contains("/usr/bin/evil --now"), "{message}");
    assert_eq!(
        asked["untrusted"],
        json!([{ "name": "Scripted", "key": "post_process", "value": "/usr/bin/evil --now" }])
    );
    assert!(!user_dir(home.path()).join("process").exists());

    let removed = ask(
        &session,
        "preset_import",
        json!({ "path": shown(&file), "keep_untrusted": false }),
    );
    assert_eq!(removed["process"], json!(["Scripted"]));
    assert_eq!(
        removed["removed"],
        json!([{ "name": "Scripted", "key": "post_process" }])
    );
    let saved = read(&user_dir(home.path()).join("process").join("Scripted.json"));
    assert!(!saved.contains("evil"), "{saved}");

    // Kept when the person says so; once it is theirs it is not asked
    // about again.
    let kept = ask(
        &session,
        "preset_import",
        json!({ "path": shown(&file), "keep_untrusted": true, "overwrite": true }),
    );
    assert_eq!(kept["process"], json!(["Scripted"]));
    let saved = read(&user_dir(home.path()).join("process").join("Scripted.json"));
    assert!(saved.contains("evil"), "{saved}");
    let again = ask(
        &session,
        "preset_import",
        json!({ "path": shown(&file), "overwrite": true }),
    );
    assert_eq!(again["imported"], true, "{again}");
}

#[test]
fn a_file_that_is_not_a_preset_stops_the_import() {
    let home = tempfile::tempdir().unwrap();
    let session = opened(home.path());
    let good = cpp_process("Good", PROCESS, "");
    let bytes = bundle::zip(
        &[
            Entry {
                name: "Good.json".into(),
                bytes: good.into_bytes(),
            },
            Entry {
                name: "Bad.json".into(),
                bytes: b"{ not json".to_vec(),
            },
        ],
        0,
    )
    .unwrap();
    let file = home.path().join("mixed.zip");
    std::fs::write(&file, bytes).unwrap();
    let error = refused(&session, "preset_import", json!({ "path": shown(&file) }));
    assert!(error.contains("Bad.json"), "{error}");
    assert!(!user_dir(home.path()).join("process").exists());
    let error = refused(
        &session,
        "preset_import",
        json!({ "path": shown(&home.path().join("a.3mf")) }),
    );
    assert!(error.contains("not a .json"), "{error}");
}

#[test]
fn an_export_names_only_what_there_is() {
    let home = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let session = opened(home.path());
    let path = out.path().to_string_lossy().into_owned();
    assert_eq!(
        refused(
            &session,
            "preset_export",
            json!({ "path": path, "type": "process" })
        ),
        "Please select at least one process preset."
    );
    save_process(&session);
    let error = refused(
        &session,
        "preset_export",
        json!({ "path": path, "type": "process", "names": ["Nope"] }),
    );
    assert!(error.contains("Nope"), "{error}");
    let error = refused(
        &session,
        "preset_export",
        json!({ "path": shown(&out.path().join("missing")), "type": "process" }),
    );
    assert!(error.contains("is not a folder"), "{error}");
}

/// What the reference's import (PresetBundle.cpp `import_presets`,
/// `import_json_presets`, l.1227–1433) makes of an archive, entry by
/// entry in the archive's order, as its code reads: only the entry's own
/// name counts and `bundle_structure.json` is passed over; a file is
/// taken when its version reads, it carries a kind's id, and its
/// `inherits` is empty, a preset already there (`known`) or one taken
/// before it in the same archive. The names it would import.
fn as_the_reference_imports(archive: &[u8], known: &[&str]) -> Vec<String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).unwrap();
    let mut taken: Vec<String> = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).unwrap();
        let leaf = entry.name().unwrap().rsplit('/').next().unwrap().to_owned();
        if leaf == bundle::BUNDLE_STRUCTURE {
            continue;
        }
        let mut text = String::new();
        std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
        let json: Value = serde_json::from_str(&text).unwrap();
        let name = json["name"].as_str().unwrap().to_owned();
        let version = json["version"].as_str().unwrap_or_default();
        assert!(
            version.split('.').all(|part| part.parse::<u32>().is_ok()),
            "{leaf}: version {version:?} does not read"
        );
        assert!(
            [
                "printer_settings_id",
                "print_settings_id",
                "filament_settings_id"
            ]
            .iter()
            .any(|key| json[*key].as_str() == Some(name.as_str())),
            "{leaf}: no id of a kind"
        );
        let inherits = json["inherits"].as_str().unwrap_or_default();
        if inherits.is_empty() || known.contains(&inherits) || taken.iter().any(|t| t == inherits) {
            taken.push(name);
        }
    }
    taken
}

/// The project's process preset, which "My Process" is laid over.
const SYSTEM_PROCESS: &str = "0.20mm Standard @Snapmaker U1 (0.4 nozzle)";

#[test]
fn an_archive_written_here_is_read_by_the_reference_parents_first() {
    let home = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    // A preset standing alone and one laid over it, named so that the
    // child comes first by name.
    let folder = user_dir(home.path()).join("process");
    std::fs::create_dir_all(folder.join("base")).unwrap();
    std::fs::write(
        folder.join("base").join("Z root.json"),
        cpp_process("Z root", "", ""),
    )
    .unwrap();
    std::fs::write(
        folder.join("A leaf.json"),
        cpp_process("A leaf", "Z root", ""),
    )
    .unwrap();
    let session = opened(home.path());
    save_process(&session);
    let path = out.path().to_string_lossy().into_owned();
    ask(
        &session,
        "preset_export",
        json!({ "path": path, "type": "process" }),
    );
    let archive = std::fs::read(out.path().join("Process presets.zip")).unwrap();
    let names: Vec<String> = {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&archive)).unwrap();
        (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned())
            .collect()
    };
    let root = names.iter().position(|n| n == "Z root.json").unwrap();
    let leaf = names.iter().position(|n| n == "A leaf.json").unwrap();
    assert!(root < leaf, "{names:?}");
    let mut imported = as_the_reference_imports(&archive, &[SYSTEM_PROCESS]);
    imported.sort();
    assert_eq!(imported, ["A leaf", "My Process", "Z root"]);

    // A printer bundle: the printer whole first, then its presets, with
    // the structure the reference writes (`archive_preset_bundle_to_file`).
    ask(
        &session,
        "preset_export",
        json!({ "path": path, "type": "printer_bundle", "names": [U1] }),
    );
    let archive = std::fs::read(out.path().join(format!("{U1}.orca_printer"))).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&archive)).unwrap();
    let entries: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned())
        .collect();
    assert_eq!(entries[0], format!("printer/{U1}.json"));
    assert_eq!(entries.last().unwrap(), bundle::BUNDLE_STRUCTURE);
    let mut text = String::new();
    std::io::Read::read_to_string(
        &mut zip.by_name(bundle::BUNDLE_STRUCTURE).unwrap(),
        &mut text,
    )
    .unwrap();
    let structure: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(structure["bundle_type"], "printer config bundle");
    assert_eq!(structure["printer_preset_name"], U1);
    assert_eq!(structure["version"], "");
    assert!(
        structure["bundle_id"]
            .as_str()
            .unwrap()
            .starts_with(&format!("offline_{U1}_"))
    );
    assert_eq!(structure["printer_config"], json!([entries[0]]));
    let listed: Vec<String> = structure["process_config"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        listed,
        entries
            .iter()
            .filter(|e| e.starts_with("process/"))
            .cloned()
            .collect::<Vec<_>>()
    );
    // The system printer goes in whole, without the print host.
    let mut printer = String::new();
    std::io::Read::read_to_string(&mut zip.by_index(0).unwrap(), &mut printer).unwrap();
    let printer: Value = serde_json::from_str(&printer).unwrap();
    assert_eq!(printer["from"], "System");
    assert_eq!(printer["printer_settings_id"], U1);
    assert!(printer.get("print_host").is_none());
    // The reference passes the system printer over (it has it) and takes
    // the rest, the parent before its child.
    let imported = as_the_reference_imports(&archive, &[SYSTEM_PROCESS]);
    assert!(imported.contains(&"A leaf".to_owned()), "{imported:?}");
}

/// Every file under a folder with its bytes.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(at) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_malformed_file_is_refused_and_nothing_changes() {
    let home = tempfile::tempdir().unwrap();
    let session = opened(home.path());
    save_process(&session);
    let before = snapshot(&user_dir(home.path()));
    let listed = ask(&session, "user_presets", Value::Null);
    let good = std::fs::read(
        user_dir(home.path())
            .join("process")
            .join("My Process.json"),
    )
    .unwrap();
    let whole = bundle::zip(
        &[Entry {
            name: "Other.json".into(),
            bytes: String::from_utf8(good)
                .unwrap()
                .replace("My Process", "Other")
                .into_bytes(),
        }],
        0,
    )
    .unwrap();
    // A zip cut short, a zip under another name, and a .json that is not.
    let cases = [
        ("cut.zip", whole[..whole.len() / 2].to_vec()),
        ("cut.orca_printer", whole[..20].to_vec()),
        (
            "broken.json",
            b"{\"name\": \"Broken\", \"version\": ".to_vec(),
        ),
    ];
    for (file, bytes) in cases {
        let path = home.path().join(file);
        std::fs::write(&path, bytes).unwrap();
        let error = refused(&session, "preset_import", json!({ "path": shown(&path) }));
        assert!(!error.is_empty(), "{file}");
        assert_eq!(snapshot(&user_dir(home.path())), before, "{file}");
        assert_eq!(ask(&session, "user_presets", Value::Null), listed, "{file}");
    }
}
