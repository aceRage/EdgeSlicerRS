use super::*;

/// A process preset as the C++ application keeps it on the disk.
const PROCESS: &str = r#"{
    "from": "User",
    "inherits": "0.20 Standard @Snapmaker U1 (0.4 nozzle)",
    "is_custom_defined": "0",
    "name": "My Process",
    "print_settings_id": "My Process",
    "version": "2.3.3.14",
    "wall_loops": "3"
}
"#;

/// A filament preset: its id is a list.
const FILAMENT: &str = r#"{
    "compatible_printers": [
        "My U1"
    ],
    "filament_settings_id": [
        "My PLA @Snapmaker U1"
    ],
    "from": "User",
    "inherits": "Snapmaker PLA SnapSpeed @U1",
    "is_custom_defined": "0",
    "name": "My PLA @Snapmaker U1",
    "nozzle_temperature": [
        "215"
    ],
    "version": "2.3.3.14"
}
"#;

/// A printer preset as `earse_preset_fields_for_safe` writes it for the
/// export: whole (only a few of its settings here), host left out.
const PRINTER: &str = r#"{
    "from": "User",
    "inherits": "Snapmaker U1 (0.4 nozzle)",
    "is_custom_defined": "0",
    "name": "My U1",
    "nozzle_diameter": [
        "0.4"
    ],
    "printable_height": "300",
    "printer_settings_id": "My U1",
    "version": "2.3.3.14"
}
"#;

/// An archive laid out by hand as the C++ lays it out, with miniz's
/// defaults (the `zip` crate stands in for it).
fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        if name.ends_with('/') {
            writer
                .add_directory(*name, SimpleFileOptions::default())
                .unwrap();
            continue;
        }
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// An archive's entries by name, unpacked.
fn unpacked(bytes: &[u8]) -> Vec<(String, String)> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            let mut text = String::new();
            entry.read_to_string(&mut text).unwrap();
            (entry.name().unwrap().into_owned(), text)
        })
        .collect()
}

#[test]
fn a_printer_bundle_from_the_cpp_is_read_by_kind_its_structure_passed_over() {
    let structure = r#"{"bundle_id":"offline_My U1_20261111120000","bundle_type":"printer config bundle","filament_config":["filament/My PLA @Snapmaker U1.json"],"printer_config":["printer/My U1.json"],"printer_preset_name":"My U1","process_config":["process/My Process.json"],"version":""}"#;
    let bytes = archive(&[
        ("printer/My U1.json", PRINTER.as_bytes()),
        ("filament/My PLA @Snapmaker U1.json", FILAMENT.as_bytes()),
        ("process/My Process.json", PROCESS.as_bytes()),
        (BUNDLE_STRUCTURE, structure.as_bytes()),
    ]);
    let found = read_archive(&bytes).unwrap();
    assert!(found.passed.is_empty() && found.broken.is_empty());
    let kinds: Vec<(Kind, &str)> = found
        .presets
        .iter()
        .map(|p| (p.kind, p.name.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            (Kind::Machine, "My U1"),
            (Kind::Filament, "My PLA @Snapmaker U1"),
            (Kind::Process, "My Process"),
        ]
    );
    let process = &found.presets[2];
    assert_eq!(
        process.inherits.as_deref(),
        Some("0.20 Standard @Snapmaker U1 (0.4 nozzle)")
    );
    assert_eq!(process.version, "2.3.3.14");
    assert_eq!(process.file, "process/My Process.json");
    // Only what the preset sets is left.
    let keys: Vec<&str> = process.settings.iter().map(|(key, _)| key).collect();
    assert_eq!(keys, ["wall_loops"]);
    let filament = &found.presets[1];
    assert_eq!(
        filament.settings.texts("compatible_printers").unwrap(),
        ["My U1"]
    );
}

#[test]
fn a_filament_bundle_and_a_flat_zip_are_read_the_same() {
    let bundle = archive(&[
        ("Snapmaker/", b""),
        ("Snapmaker/My PLA @Snapmaker U1.json", FILAMENT.as_bytes()),
        (BUNDLE_STRUCTURE, b"{}"),
    ]);
    let flat = archive(&[("My PLA @Snapmaker U1.json", FILAMENT.as_bytes())]);
    let (a, b) = (read_archive(&bundle).unwrap(), read_archive(&flat).unwrap());
    assert_eq!(a.presets.len(), 1);
    assert_eq!(a.presets[0].settings, b.presets[0].settings);
    assert_eq!(a.presets[0].name, b.presets[0].name);
}

#[test]
fn what_is_not_a_preset_is_passed_over_or_reported() {
    let no_version = PROCESS.replace("\"version\": \"2.3.3.14\",", "");
    let no_kind = PROCESS.replace("print_settings_id", "something_else");
    let climbing = PROCESS.replace("\"name\": \"My Process\"", "\"name\": \"../x\"");
    let bytes = archive(&[
        ("../evil.json", PROCESS.as_bytes()),
        ("__MACOSX/._My Process.json", b"\x00\x05\x16\x07"),
        ("a/My Process.json", PROCESS.as_bytes()),
        ("b/My Process.json", PROCESS.as_bytes()),
        ("no version.json", no_version.as_bytes()),
        ("no kind.json", no_kind.as_bytes()),
        ("climbing.json", climbing.as_bytes()),
        ("readme.txt", b"not json"),
    ]);
    let found = read_archive(&bytes).unwrap();
    assert_eq!(found.presets.len(), 1);
    assert_eq!(found.presets[0].file, "a/My Process.json");
    let passed: Vec<&str> = found.passed.iter().map(|p| p.file.as_str()).collect();
    assert_eq!(
        passed,
        [
            "../evil.json",
            "b/My Process.json",
            "no version.json",
            "no kind.json",
            "climbing.json"
        ]
    );
    assert_eq!(found.broken.len(), 1);
    assert_eq!(found.broken[0].file, "readme.txt");
}

#[test]
fn a_file_that_is_not_an_archive_is_refused() {
    assert!(read_archive(b"PK but not really").is_err());
}

#[test]
fn files_are_told_apart_by_their_names() {
    assert_eq!(is_archive(Path::new("a.JSON")), Some(false));
    assert_eq!(is_archive(Path::new("My U1.orca_printer")), Some(true));
    assert_eq!(is_archive(Path::new("PLA.orca_filament")), Some(true));
    assert_eq!(is_archive(Path::new("Process presets.zip")), Some(true));
    assert_eq!(is_archive(Path::new("a.3mf")), None);
}

#[test]
fn a_single_json_is_read() {
    let found = read_json(PROCESS.as_bytes(), "My Process.json");
    assert_eq!(found.presets.len(), 1);
    assert_eq!(found.presets[0].kind, Kind::Process);
    let broken = read_json(b"{", "x.json");
    assert_eq!(broken.broken.len(), 1);
}

#[test]
fn the_clock_reads_as_the_reference_stamps_it() {
    assert_eq!(clock(0), "19700101000000");
    assert_eq!(clock(1_700_000_000), "20231114221320");
    assert_eq!(clock(951_782_400), "20000229000000");
}

#[test]
fn names_split_as_the_export_dialog_splits_them() {
    let name = "Snapmaker PLA SnapSpeed @Snapmaker U1 (0.4 nozzle)";
    assert_eq!(machine_name(name), "Snapmaker U1 (0.4 nozzle)");
    assert_eq!(filament_name(name), "Snapmaker PLA SnapSpeed");
    assert_eq!(vendor_name(machine_name(name)), "Snapmaker");
    assert_eq!(filament_name("PLA@U1"), "PLA");
    assert_eq!(filament_name("Generic PLA"), "Generic PLA");
    assert_eq!(machine_name("Generic PLA"), "");
    assert_eq!(vendor_name(""), "");
}

#[test]
fn a_printer_is_written_whole_without_its_host() {
    let mut resolved = Settings::default();
    resolved.insert("printable_height", Value::One("300".into()));
    resolved.insert("print_host", Value::One("192.168.1.2".into()));
    resolved.insert("printhost_apikey", Value::One("secret".into()));
    resolved.insert("setting_id", Value::One("GM001".into()));
    resolved.insert("nozzle_diameter", Value::Many(vec!["0.4".into()]));
    let text = whole_printer(
        "My U1",
        &resolved,
        Some("Snapmaker U1 (0.4 nozzle)"),
        "User",
        "2.3.3.14",
    );
    assert_eq!(text, PRINTER);
}

#[test]
fn a_printer_bundle_is_written_as_the_reference_writes_it_and_read_back() {
    let bundle = PrinterBundle {
        printer: "My U1".into(),
        printer_file: PRINTER.as_bytes().to_vec(),
        filaments: vec![(
            "My PLA @Snapmaker U1.json".into(),
            FILAMENT.as_bytes().to_vec(),
        )],
        processes: vec![("My Process.json".into(), PROCESS.as_bytes().to_vec())],
    };
    let bytes = printer_bundle(&bundle, 1_700_000_000).unwrap();
    // The same time gives the same archive.
    assert_eq!(bytes, printer_bundle(&bundle, 1_700_000_000).unwrap());
    let entries = unpacked(&bytes);
    let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "printer/My U1.json",
            "filament/My PLA @Snapmaker U1.json",
            "process/My Process.json",
            BUNDLE_STRUCTURE
        ]
    );
    assert_eq!(entries[2].1, PROCESS);
    assert_eq!(
        entries[3].1,
        r#"{"bundle_id":"offline_My U1_20231114221320","bundle_type":"printer config bundle","filament_config":["filament/My PLA @Snapmaker U1.json"],"printer_config":["printer/My U1.json"],"printer_preset_name":"My U1","process_config":["process/My Process.json"],"version":""}"#
    );
    let found = read_archive(&bytes).unwrap();
    assert_eq!(found.presets.len(), 3);
    assert!(found.passed.is_empty() && found.broken.is_empty());
}

#[test]
fn a_filament_bundle_is_written_by_vendor_and_read_back() {
    let mut vendors = BTreeMap::new();
    vendors.insert(
        "Snapmaker".to_owned(),
        vec![(
            "My PLA @Snapmaker U1".to_owned(),
            FILAMENT.as_bytes().to_vec(),
        )],
    );
    let bytes = filament_bundle("My PLA", &vendors, 0).unwrap();
    let entries = unpacked(&bytes);
    assert_eq!(entries[0].0, "Snapmaker/My PLA @Snapmaker U1.json");
    assert_eq!(
        entries[1].1,
        r#"{"bundle_id":"offline_My PLA_19700101000000","bundle_type":"filament config bundle","filament_name":"My PLA","printer_vendor":[{"filament_path":["Snapmaker/My PLA @Snapmaker U1.json"],"vendor":"Snapmaker"}],"version":""}"#
    );
    let found = read_archive(&bytes).unwrap();
    assert_eq!(found.presets.len(), 1);
    assert_eq!(found.presets[0].name, "My PLA @Snapmaker U1");
}

#[test]
fn a_flat_zip_holds_the_files_as_they_are() {
    let bytes = zip(
        &[Entry {
            name: "My Process.json".into(),
            bytes: PROCESS.as_bytes().to_vec(),
        }],
        0,
    )
    .unwrap();
    assert_eq!(
        unpacked(&bytes),
        [("My Process.json".to_owned(), PROCESS.to_owned())]
    );
}
