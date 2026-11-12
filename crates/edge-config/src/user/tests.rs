use super::*;

/// A process preset as the C++ application writes it when "0.20 Standard
/// @Snapmaker U1 (0.4 nozzle)" is saved as "My Process" with three walls
/// (`save_current_preset`, `Preset::save` with the parent's config,
/// `ConfigBase::save_to_json`: the keys in order, four spaces, the new
/// line `std::endl` adds).
const CPP_PROCESS: &str = r#"{
    "from": "User",
    "inherits": "0.20 Standard @Snapmaker U1 (0.4 nozzle)",
    "is_custom_defined": "0",
    "name": "My Process",
    "print_settings_id": "My Process",
    "version": "2.3.3.14",
    "wall_loops": "3"
}
"#;

/// Its `.info` as `Preset::save_info` writes it for a preset just made
/// (`Tab::save_preset` sets "create"; nothing sets the time, which the
/// reference fills in when it next reads the file).
const CPP_INFO: &str =
    "sync_info = create\nuser_id = \nsetting_id = \nbase_id = GP004\nupdated_time = 0\n";

/// A filament preset: the id is a list, and so is every filament setting.
const CPP_FILAMENT: &str = r#"{
    "compatible_printers": [
        "Snapmaker U1 (0.4 nozzle)"
    ],
    "filament_settings_id": [
        "My PLA"
    ],
    "from": "User",
    "inherits": "Snapmaker PLA SnapSpeed @U1",
    "is_custom_defined": "0",
    "name": "My PLA",
    "nozzle_temperature": [
        "215"
    ],
    "version": "2.3.3.14"
}
"#;

fn write(dir: &Path, path: &str, text: &str) {
    let path = dir.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn system(name: &str) -> Result<Settings, ConfigError> {
    let text = match name {
        "0.20 Standard @Snapmaker U1 (0.4 nozzle)" => {
            r#"{"name": "0.20 Standard @Snapmaker U1 (0.4 nozzle)", "inherits": "fdm_process_common", "setting_id": "GP004", "wall_loops": "2", "layer_height": "0.2", "compatible_printers": ["Snapmaker U1 (0.4 nozzle)"]}"#
        }
        "Snapmaker PLA SnapSpeed @U1" => {
            r#"{"name": "Snapmaker PLA SnapSpeed @U1", "nozzle_temperature": ["220"], "filament_type": ["PLA"]}"#
        }
        _ => {
            return Err(ConfigError::UnknownPreset {
                vendor: "Snapmaker".into(),
                kind: Kind::Process,
                name: name.into(),
            });
        }
    };
    Ok(Settings::from_json(text).unwrap())
}

#[test]
fn a_saved_preset_is_written_as_the_reference_writes_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut presets = UserPresets::empty(dir.path());
    let mut own = Settings::default();
    own.insert("wall_loops", Value::One("3".into()));
    presets
        .save(
            Kind::Process,
            "My Process",
            ToSave {
                own,
                inherits: Some("0.20 Standard @Snapmaker U1 (0.4 nozzle)".into()),
                version: "2.3.3.14".into(),
                base_id: Some("GP004".into()),
            },
        )
        .unwrap();
    let file = dir.path().join("process").join("My Process.json");
    assert_eq!(fs::read_to_string(&file).unwrap(), CPP_PROCESS);
    let info = Info::parse(&fs::read_to_string(file.with_extension("info")).unwrap());
    assert_eq!(info.sync_info, "create");
    assert_eq!(info.base_id, "GP004");
    assert!(info.updated_time > 0, "the time it was saved");
    // Nothing is left beside it.
    let names: Vec<String> = fs::read_dir(dir.path().join("process"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");

    let mut own = Settings::default();
    own.insert("nozzle_temperature", Value::Many(vec!["215".into()]));
    own.insert(
        "compatible_printers",
        Value::Many(vec!["Snapmaker U1 (0.4 nozzle)".into()]),
    );
    presets
        .save(
            Kind::Filament,
            "My PLA",
            ToSave {
                own,
                inherits: Some("Snapmaker PLA SnapSpeed @U1".into()),
                version: "2.3.3.14".into(),
                base_id: None,
            },
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("filament").join("My PLA.json")).unwrap(),
        CPP_FILAMENT
    );
}

#[test]
fn presets_the_reference_wrote_are_read_and_resolved() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "process/My Process.json", CPP_PROCESS);
    write(dir.path(), "process/My Process.info", CPP_INFO);
    write(dir.path(), "filament/My PLA.json", CPP_FILAMENT);
    let presets = UserPresets::open(dir.path());
    assert!(presets.unread().is_empty(), "{:?}", presets.unread());

    let process = presets.get(Kind::Process, "My Process").unwrap();
    assert_eq!(
        process.inherits.as_deref(),
        Some("0.20 Standard @Snapmaker U1 (0.4 nozzle)")
    );
    assert_eq!(process.info.base_id, "GP004");
    assert_eq!(process.info.updated_time, 0);
    assert_eq!(process.version.as_deref(), Some("2.3.3.14"));
    // What says what the file is is not a setting.
    assert_eq!(process.own.get("from"), None);
    assert_eq!(process.own.get("version"), None);

    let resolved = presets
        .resolve(Kind::Process, "My Process", &system)
        .unwrap();
    assert_eq!(resolved.text("wall_loops"), Some("3"));
    assert_eq!(resolved.text("layer_height"), Some("0.2"));
    assert_eq!(resolved.text("print_settings_id"), Some("My Process"));
    assert_eq!(resolved.text("name"), Some("My Process"));
    assert_eq!(
        resolved.text("inherits"),
        Some("0.20 Standard @Snapmaker U1 (0.4 nozzle)")
    );
    // The system preset's own id is not the user preset's.
    assert_eq!(resolved.get("setting_id"), None);
    assert_eq!(
        resolved.texts("compatible_printers").unwrap(),
        ["Snapmaker U1 (0.4 nozzle)"]
    );
    let pla = presets.resolve(Kind::Filament, "My PLA", &system).unwrap();
    assert_eq!(pla.texts("nozzle_temperature").unwrap(), ["215"]);
    assert_eq!(pla.texts("filament_type").unwrap(), ["PLA"]);
    assert_eq!(pla.texts("filament_settings_id").unwrap(), ["My PLA"]);
    assert_eq!(
        presets.system_parent(Kind::Filament, "My PLA").as_deref(),
        Some("Snapmaker PLA SnapSpeed @U1")
    );
}

#[test]
fn a_file_that_cannot_be_read_is_left_alone_and_said() {
    let dir = tempfile::tempdir().unwrap();
    let broken = "{ \"name\": \"Broken\", \"wall_loops\": ";
    write(dir.path(), "process/Broken.json", broken);
    write(dir.path(), "process/Broken.info", CPP_INFO);
    // A filament preset in the process folder, and a second file of a
    // name already read: the reference passes over both.
    write(
        dir.path(),
        "process/Stray.json",
        r#"{"type": "filament", "name": "Stray"}"#,
    );
    write(dir.path(), "process/My Process.json", CPP_PROCESS);
    write(
        dir.path(),
        "process/Second.json",
        &CPP_PROCESS.replace("\"version\": \"2.3.3.14\",\n", ""),
    );
    let presets = UserPresets::open(dir.path());
    let unread: Vec<String> = presets
        .unread()
        .iter()
        .map(|unread| {
            unread
                .file
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(unread, ["Broken.json", "Second.json", "Stray.json"]);
    assert_eq!(presets.of(Kind::Process).count(), 1);
    // Still there, as it was.
    assert_eq!(
        fs::read_to_string(dir.path().join("process").join("Broken.json")).unwrap(),
        broken
    );
    assert!(dir.path().join("process").join("Broken.info").exists());
}

#[test]
fn a_preset_without_a_version_is_read() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "process/Old.json",
        r#"{"name": "Old", "inherits": "0.20 Standard @Snapmaker U1 (0.4 nozzle)", "wall_loops": "4"}"#,
    );
    let presets = UserPresets::open(dir.path());
    assert!(presets.get(Kind::Process, "Old").is_some());
    assert_eq!(presets.get(Kind::Process, "Old").unwrap().version, None);
}

#[test]
fn a_chain_of_ones_own_presets_is_followed() {
    let dir = tempfile::tempdir().unwrap();
    // A preset that stands alone lives in base/, and one laid over it.
    write(
        dir.path(),
        "process/base/Root.json",
        r#"{"name": "Root", "from": "User", "inherits": "", "version": "2.3.3.14", "wall_loops": "5", "layer_height": "0.3"}"#,
    );
    write(
        dir.path(),
        "process/Leaf.json",
        r#"{"name": "Leaf", "from": "User", "inherits": "Root", "version": "2.3.3.14", "layer_height": "0.1"}"#,
    );
    let presets = UserPresets::open(dir.path());
    let leaf = presets.resolve(Kind::Process, "Leaf", &system).unwrap();
    assert_eq!(leaf.text("wall_loops"), Some("5"));
    assert_eq!(leaf.text("layer_height"), Some("0.1"));
    assert_eq!(presets.children(Kind::Process, "Root"), ["Leaf"]);
    assert_eq!(presets.system_parent(Kind::Process, "Leaf"), None);
    assert!(
        presets
            .get(Kind::Process, "Root")
            .unwrap()
            .inherits
            .is_none()
    );

    write(
        dir.path(),
        "process/Loop.json",
        r#"{"name": "Loop", "inherits": "Loop2"}"#,
    );
    write(
        dir.path(),
        "process/Loop2.json",
        r#"{"name": "Loop2", "inherits": "Loop"}"#,
    );
    let presets = UserPresets::open(dir.path());
    assert!(matches!(
        presets.resolve(Kind::Process, "Loop", &system),
        Err(ConfigError::InheritanceCycle { .. })
    ));
}

#[test]
fn a_preset_is_renamed_and_what_is_laid_over_it_follows() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "process/base/Root.json",
        r#"{"name": "Root", "inherits": "", "version": "2.3.3.14", "wall_loops": "5"}"#,
    );
    write(dir.path(), "process/base/Root.info", CPP_INFO);
    write(
        dir.path(),
        "process/Leaf.json",
        r#"{"name": "Leaf", "inherits": "Root", "version": "2.3.3.14", "layer_height": "0.1"}"#,
    );
    let mut presets = UserPresets::open(dir.path());
    presets.rename(Kind::Process, "Root", "Trunk").unwrap();
    let base = dir.path().join("process").join("base");
    assert!(!base.join("Root.json").exists());
    assert!(!base.join("Root.info").exists());
    let trunk = fs::read_to_string(base.join("Trunk.json")).unwrap();
    assert!(trunk.contains("\"name\": \"Trunk\""), "{trunk}");
    assert!(
        trunk.contains("\"print_settings_id\": \"Trunk\""),
        "{trunk}"
    );
    assert!(trunk.contains("\"wall_loops\": \"5\""), "{trunk}");
    assert_eq!(
        Info::parse(&fs::read_to_string(base.join("Trunk.info")).unwrap()).base_id,
        "GP004"
    );
    let leaf = fs::read_to_string(dir.path().join("process").join("Leaf.json")).unwrap();
    assert!(leaf.contains("\"inherits\": \"Trunk\""), "{leaf}");
    // Read again, it is all there.
    let again = UserPresets::open(dir.path());
    assert!(again.get(Kind::Process, "Trunk").is_some());
    assert_eq!(
        again
            .resolve(Kind::Process, "Leaf", &system)
            .unwrap()
            .text("wall_loops"),
        Some("5")
    );

    // A name that only changes case.
    presets.rename(Kind::Process, "Leaf", "LEAF").unwrap();
    let names: Vec<String> = fs::read_dir(dir.path().join("process"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json"))
        .collect();
    assert_eq!(names, ["LEAF.json"]);
}

#[test]
fn a_preset_is_deleted_with_its_info() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "process/My Process.json", CPP_PROCESS);
    write(dir.path(), "process/My Process.info", CPP_INFO);
    let mut presets = UserPresets::open(dir.path());
    presets.delete(Kind::Process, "My Process").unwrap();
    assert!(!dir.path().join("process").join("My Process.json").exists());
    assert!(!dir.path().join("process").join("My Process.info").exists());
    assert!(presets.get(Kind::Process, "My Process").is_none());
    assert!(presets.delete(Kind::Process, "My Process").is_err());
}

#[test]
fn saving_over_a_preset_keeps_its_place_and_its_info() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "process/My Process.json", CPP_PROCESS);
    write(
        dir.path(),
        "process/My Process.info",
        "sync_info = \nuser_id = 42\nsetting_id = PUS1\nbase_id = GP004\nupdated_time = 1700000000\n",
    );
    let mut presets = UserPresets::open(dir.path());
    let mut own = Settings::default();
    own.insert("wall_loops", Value::One("4".into()));
    presets
        .save(
            Kind::Process,
            "My Process",
            ToSave {
                own,
                inherits: Some("0.20 Standard @Snapmaker U1 (0.4 nozzle)".into()),
                version: "2.3.3.14".into(),
                base_id: Some("GP004".into()),
            },
        )
        .unwrap();
    let file = dir.path().join("process").join("My Process.json");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        CPP_PROCESS.replace("\"wall_loops\": \"3\"", "\"wall_loops\": \"4\"")
    );
    let info = Info::parse(&fs::read_to_string(file.with_extension("info")).unwrap());
    assert_eq!(info.sync_info, "update");
    assert_eq!(info.user_id, "42");
    assert_eq!(info.setting_id, "PUS1");
    assert!(info.updated_time > 1_700_000_000);
}

#[test]
fn what_differs_from_the_parent_is_what_is_written() {
    let parent = Settings::from_json(
        r#"{"name": "P", "wall_loops": "2", "layer_height": "0.20", "process_flow_support": ["Standard", "High Flow"], "outer_wall_speed": ["200", "300"]}"#,
    )
    .unwrap();
    let full = Settings::from_json(
        r#"{"name": "C", "inherits": "P", "print_settings_id": "C", "wall_loops": "3", "layer_height": "0.2", "process_flow_support": ["Standard", "High Flow"], "outer_wall_speed": ["200", "300"], "seam_position": "aligned", "not_a_setting": "x"}"#,
    )
    .unwrap();
    let own = differences(&full, Some(&parent), Kind::Process);
    let keys: Vec<&str> = own.iter().map(|(key, _)| key).collect();
    // 0.2 is 0.20; the default seam is "aligned"; the flow list stays.
    assert_eq!(keys, ["process_flow_support", "wall_loops"]);
    let all = differences(&full, None, Kind::Process);
    assert!(all.get("layer_height").is_some());
    assert!(all.get("name").is_none() && all.get("not_a_setting").is_none());
}

#[test]
fn names_are_checked_as_the_reference_checks_them() {
    assert_eq!(check_name("A/B"), Err(NameProblem::IllegalCharacters));
    assert_eq!(check_name("A: B"), Err(NameProblem::IllegalCharacters));
    assert_eq!(check_name("x (modified)"), Err(NameProblem::IllegalSuffix));
    assert_eq!(check_name("Default Setting"), Err(NameProblem::Reserved));
    assert_eq!(check_name(""), Err(NameProblem::Empty));
    assert_eq!(check_name(" a"), Err(NameProblem::LeadingSpace));
    assert_eq!(check_name("a "), Err(NameProblem::TrailingSpace));
    assert_eq!(check_name("0.20 Standard @U1 - Copy"), Ok(()));
}

#[test]
fn an_info_reads_null_as_nothing() {
    let info = Info::parse(
        "sync_info = hold\nuser_id = \nsetting_id = null\nbase_id = null\nupdated_time = 12\nother = 1\n",
    );
    assert_eq!(info.setting_id, "");
    assert_eq!(info.base_id, "");
    assert_eq!(info.updated_time, 12);
    // "hold" is not written back.
    assert!(info.text().starts_with("sync_info = \n"));
}
