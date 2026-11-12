//! Every project the reference ships or tests with, written back and read
//! again, is the same project: its objects, parts, meshes, paint, copies,
//! plates and settings; and writing it again gives the same bytes.

use std::{
    io::Cursor,
    path::{Path, PathBuf},
};

use edge_3mf::{Project, Save, write_to};

fn reference() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/EdgeSlicer")
}

fn projects_under(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            projects_under(&path, found);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
        {
            found.push(path);
        }
    }
}

/// How many filaments a project's settings have.
fn filaments(project: &Project) -> usize {
    project
        .settings
        .as_ref()
        .and_then(|settings| settings.texts("filament_colour").map(<[String]>::len))
        .unwrap_or(1)
}

/// The project written as it was read, with its own settings.
pub fn saved(project: &Project) -> Vec<u8> {
    let settings = project
        .kept
        .as_ref()
        .and_then(|kept| kept.settings_text())
        .map(|text| edge_3mf::own_settings_text(text).unwrap());
    let save = Save {
        project,
        settings,
        presets: true,
        filaments: filaments(project),
    };
    let mut out = Cursor::new(Vec::new());
    write_to(&save, &mut out).unwrap();
    out.into_inner()
}

/// Says how two projects differ, if they do.
pub fn differences(a: &Project, b: &Project) -> Vec<String> {
    let mut found = Vec::new();
    let mut differ = |what: String| found.push(what);
    if a.custom_gcode != b.custom_gcode {
        differ("the G-code set at layers".into());
    }
    if a.objects.len() != b.objects.len() {
        differ(format!(
            "{} objects, then {}",
            a.objects.len(),
            b.objects.len()
        ));
        return found;
    }
    for (at, (x, y)) in a.objects.iter().zip(&b.objects).enumerate() {
        // An object the file names nowhere is named as the reference names it.
        let named = |object: &edge_3mf::Object| {
            object
                .name
                .clone()
                .unwrap_or_else(|| format!("Object_{}", at + 1))
        };
        if named(x) != named(y) {
            differ(format!("object {at}: name {:?}, then {:?}", x.name, y.name));
        }
        if x.instances.len() != y.instances.len() {
            differ(format!("object {at}: copies"));
        }
        for (i, (p, q)) in x.instances.iter().zip(&y.instances).enumerate() {
            if p.transform != q.transform || p.printable != q.printable {
                differ(format!("object {at} copy {i}: {:?} then {:?}", p, q));
            }
        }
        if x.parts.len() != y.parts.len() {
            differ(format!("object {at}: parts"));
            continue;
        }
        for (k, (p, q)) in x.parts.iter().zip(&y.parts).enumerate() {
            let what = format!("object {at} part {k}");
            if p.name != q.name {
                differ(format!("{what}: name {:?} then {:?}", p.name, q.name));
            }
            if p.kind != q.kind || p.filament != q.filament || p.decoration != q.decoration {
                differ(format!(
                    "{what}: {:?}/{}/{} then {:?}/{}/{}",
                    p.kind, p.filament, p.decoration, q.kind, q.filament, q.decoration
                ));
            }
            if p.transform != q.transform {
                differ(format!("{what}: placed otherwise"));
            }
            if p.mesh.vertices != q.mesh.vertices || p.mesh.triangles != q.mesh.triangles {
                differ(format!("{what}: mesh"));
            }
            let facets = |part: &edge_3mf::Part| {
                part.paint.as_ref().map(|paint| {
                    paint
                        .iter()
                        .map(|f| (f.corners, f.filament))
                        .collect::<Vec<_>>()
                })
            };
            if facets(p) != facets(q) {
                differ(format!("{what}: colour paint"));
            }
            let seams = |part: &edge_3mf::Part| {
                part.seam
                    .as_ref()
                    .map(|seam| seam.iter().map(|f| (f.corners, f.kind)).collect::<Vec<_>>())
            };
            if seams(p) != seams(q) {
                differ(format!("{what}: seam paint"));
            }
            let extras = |part: &edge_3mf::Part| {
                part.kept
                    .as_ref()
                    .and_then(|kept| kept.extras().map(|e| e.to_vec()))
            };
            if extras(p) != extras(q) {
                differ(format!("{what}: what its triangles carry"));
            }
        }
    }
    let plates = |project: &Project| {
        project
            .plates
            .iter()
            .map(|plate| {
                (
                    plate.id,
                    plate.name.clone(),
                    plate.first_layer_sequence.clone(),
                    plate.instances.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    if plates(a) != plates(b) {
        differ(format!("plates {:?} then {:?}", plates(a), plates(b)));
    }
    // What would reach a printer is not written.
    let open = |project: &Project| {
        project.settings.clone().map(|mut settings| {
            for key in edge_3mf::SECRETS {
                settings.remove(key);
            }
            settings
        })
    };
    if open(a) != open(b) {
        differ("settings".into());
    }
    if a.unread_paint != b.unread_paint {
        differ(format!(
            "paint not read: {:?} then {:?}",
            a.unread_paint, b.unread_paint
        ));
    }
    found
}

#[test]
fn every_reference_project_is_itself_once_written_and_read_again() {
    let mut paths = Vec::new();
    for dir in ["tests", "resources"] {
        projects_under(&reference().join(dir), &mut paths);
    }
    paths.sort();
    let mut failures = Vec::new();
    let mut checked = 0;
    for path in &paths {
        let Ok(project) = Project::open(path) else {
            continue;
        };
        let bytes = saved(&project);
        let again = Project::read(Cursor::new(bytes.clone()))
            .unwrap_or_else(|e| panic!("{}: written, it reads as {e}", path.display()));
        for difference in differences(&project, &again) {
            failures.push(format!("{}: {difference}", path.display()));
        }
        // Written again, it is the same file to the byte.
        if saved(&again) != bytes {
            failures.push(format!(
                "{}: written twice, the files differ",
                path.display()
            ));
        }
        checked += 1;
    }
    assert!(checked >= 30, "only {checked} projects checked");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The reference's own file of G-code at layers (the pressure advance
/// pattern's, every number at 17 digits and its G-code over many lines in
/// one attribute) is written back as the reference wrote it, to the byte;
/// and items edited are written in the same layout.
#[test]
fn custom_gcode_at_layers_is_written_as_the_reference_writes_it() {
    use std::io::Read;
    let path = reference().join("resources/calib/pressure_advance/pa_pattern.3mf");
    let entry = |bytes: &[u8]| {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
        let mut text = String::new();
        archive
            .by_name("Metadata/custom_gcode_per_layer.xml")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    };
    let original = entry(&std::fs::read(&path).unwrap());
    let mut project = Project::open(&path).unwrap();
    let plate = &project.custom_gcode[&1];
    assert_eq!(plate.items.len(), 4);
    assert_eq!(plate.items[1].print_z, 0.45);
    assert_eq!(plate.mode, Some(edge_3mf::CustomMode::SingleExtruder));
    assert!(plate.items[0].extra.lines().count() > 900);
    assert_eq!(entry(&saved(&project)), original);

    // A pause added to plate 2, which had none, and the last item taken
    // from plate 1.
    project.custom_gcode.get_mut(&1).unwrap().items.pop();
    project.custom_gcode.insert(
        2,
        edge_3mf::CustomGcode {
            mode: Some(edge_3mf::CustomMode::MultiAsSingle),
            items: vec![edge_3mf::CustomItem {
                print_z: 1.05,
                kind: edge_3mf::CustomKind::PausePrint,
                extruder: 1,
                color: String::new(),
                extra: String::new(),
                gcode: "M600".into(),
            }],
        },
    );
    let text = entry(&saved(&project));
    assert!(text.ends_with(
        "<plate>\n<plate_info id=\"2\"/>\n<layer top_z=\"1.05\" type=\"1\" extruder=\"1\" color=\"\" \
         extra=\"\" gcode=\"M600\"/>\n<mode value=\"MultiAsSingle\"/>\n</plate>\n</custom_gcodes_per_layer>"
    ));
    let again = Project::read(Cursor::new(saved(&project))).unwrap();
    assert_eq!(again.custom_gcode, project.custom_gcode);
}

/// A project that names a printer's address and keys is written without
/// them, as the reference writes one (PresetBundle.cpp
/// `full_config_secure`); everything else of its settings stays.
#[test]
fn what_would_reach_a_printer_is_not_written() {
    let path = reference().join("tests/data/untrusted_3mf/post_process_project.3mf");
    let project = Project::open(&path).unwrap();
    let settings = project.settings.as_ref().unwrap();
    assert_eq!(settings.text("print_host"), Some("http://attacker.invalid"));
    let bytes = saved(&project);
    let text = {
        use std::io::Read;
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
        let mut text = String::new();
        archive
            .by_name("Metadata/project_settings.config")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    };
    assert!(!text.contains("\"print_host\""));
    assert!(!text.contains("http://attacker.invalid"));
    let again = Project::read(Cursor::new(bytes)).unwrap();
    let written = again.settings.unwrap();
    for key in edge_3mf::SECRETS {
        assert!(written.get(key).is_none(), "{key} was written");
    }
    for (key, value) in settings.iter() {
        if key != "print_host" {
            assert_eq!(written.get(key), Some(value), "{key}");
        }
    }
    // Four spaces to a level, as the reference indents it.
    assert!(text.starts_with("{\n    \""), "{}", &text[..40]);
}

/// A package of a few parts, by name and text.
fn package(parts: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in parts {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// An object with two copies, each with its place in the assembly view.
fn two_copies() -> Project {
    let model = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
 <resources><object id="2" type="model"><mesh>
  <vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="0" y="0" z="1"/></vertices>
  <triangles><triangle v1="0" v2="2" v3="1" paint_supports="4"/><triangle v1="0" v2="1" v3="3"/><triangle v1="1" v2="2" v3="3" paint_color="8"/><triangle v1="0" v2="3" v3="2"/></triangles>
 </mesh></object></resources>
 <build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 10 10 0"/><item objectid="2" transform="1 0 0 0 1 0 0 0 1 20 10 0"/></build>
</model>"#;
    let settings = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="2">
    <metadata key="name" value="Tetra"/>
    <metadata key="extruder" value="2"/>
    <metadata key="wall_loops" value="4"/>
  </object>
  <plate>
    <metadata key="plater_id" value="1"/>
    <metadata key="plater_name" value="First"/>
    <metadata key="locked" value="true"/>
    <model_instance><metadata key="object_id" value="2"/><metadata key="instance_id" value="0"/></model_instance>
    <model_instance><metadata key="object_id" value="2"/><metadata key="instance_id" value="1"/></model_instance>
  </plate>
  <assemble>
   <assemble_item object_id="2" instance_id="0" transform="1 0 0 0 1 0 0 0 1 0 0 0" offset="0 0 0" />
   <assemble_item object_id="2" instance_id="1" transform="1 0 0 0 1 0 0 0 1 5 0 0" offset="0 0 0" />
  </assemble>
</config>"#;
    Project::read(Cursor::new(package(&[
        ("3D/3dmodel.model", model),
        ("Metadata/model_settings.config", settings),
        ("Metadata/plate_1.png", "not a picture"),
        ("Metadata/plate_1.gcode", "G1 X1"),
        ("Auxiliaries/../escape.txt", "out"),
        ("Auxiliaries/notes.txt", "kept"),
    ])))
    .unwrap()
}

#[test]
fn what_a_plain_file_carries_is_written_in_the_references_layout() {
    let project = two_copies();
    let bytes = saved(&project);
    let again = Project::read(Cursor::new(bytes.clone())).unwrap();
    assert!(differences(&project, &again).is_empty());
    let names: Vec<String> = zip::ZipArchive::new(Cursor::new(bytes.clone()))
        .unwrap()
        .file_names()
        .filter_map(Result::ok)
        .map(|name| name.into_owned())
        .collect();
    // The slice is not saved, nor a name that reaches outside the file;
    // the plate's picture and the rest are.
    assert!(!names.iter().any(|name| name.ends_with(".gcode")));
    assert!(!names.iter().any(|name| name.contains("..")));
    for wanted in [
        "Metadata/plate_1.png",
        "Auxiliaries/notes.txt",
        "3D/3dmodel.model",
        "3D/Objects/Tetra_1.model",
        "Metadata/model_settings.config",
        "Metadata/slice_info.config",
        "_rels/.rels",
        "[Content_Types].xml",
    ] {
        assert!(
            names.iter().any(|name| name == wanted),
            "{wanted}: {names:?}"
        );
    }
    let text = |name: &str| {
        use std::io::Read;
        let mut text = String::new();
        zip::ZipArchive::new(Cursor::new(bytes.clone()))
            .unwrap()
            .by_name(name)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    };
    let model = text("3D/3dmodel.model");
    assert!(model.contains(r#"<metadata name="Application">EdgeSlicer-"#));
    assert!(model.contains(r#"<metadata name="BambuStudio:3mfVersion">1</metadata>"#));
    let settings = text("Metadata/model_settings.config");
    for kept in [
        r#"<metadata key="wall_loops" value="4"/>"#,
        r#"<metadata key="locked" value="true"/>"#,
        r#"<metadata key="plater_name" value="First"/>"#,
        r#"<metadata key="thumbnail_file" value="Metadata/plate_1.png"/>"#,
        r#"instance_id="1" transform="1 0 0 0 1 0 0 0 1 5 0 0""#,
    ] {
        assert!(settings.contains(kept), "{kept}\n{settings}");
    }
    // Every kind of paint as it was, on the triangle it was on.
    let object = text("3D/Objects/Tetra_1.model");
    assert!(object.contains(r#"<triangle v1="0" v2="2" v3="1" paint_supports="4"/>"#));
    assert!(object.contains(r#"<triangle v1="1" v2="2" v3="3" paint_color="8"/>"#));
    // What is left out, and what would be, is said.
    assert_eq!(edge_3mf::losses(&project, true).len(), 1);
}

#[test]
fn a_copy_deleted_takes_the_assembly_view_of_its_object_with_it_and_says_so() {
    let mut project = two_copies();
    project.objects[0].instances.pop();
    project.plates[0].instances.pop();
    let losses = edge_3mf::losses(&project, true);
    assert!(
        losses.iter().any(|loss| loss.contains("assembly view")),
        "{losses:?}"
    );
    let again = Project::read(Cursor::new(saved(&project))).unwrap();
    assert_eq!(again.objects[0].instances.len(), 1);
}

/// The version a file claims is the pinned reference's.
#[test]
fn the_version_claimed_is_the_pinned_references() {
    let header =
        std::fs::read_to_string(reference().join("src/common_func/common_func.hpp")).unwrap();
    let line = header
        .lines()
        .find(|line| line.starts_with("#define Snapmaker_VERSION"))
        .unwrap();
    assert!(
        line.contains(&format!("\"{}\"", edge_3mf::REFERENCE_VERSION)),
        "{line}"
    );
}

/// An object of three parts (a tetrahedron, a connector, a tetrahedron),
/// cut, its second part a cut connector.
fn cut_object() -> Project {
    let tetra = |id: u32, x: u32| {
        format!(
            r#"<object id="{id}" type="model"><mesh>
  <vertices><vertex x="{x}" y="0" z="0"/><vertex x="{}" y="0" z="0"/><vertex x="{x}" y="1" z="0"/><vertex x="{x}" y="0" z="1"/></vertices>
  <triangles><triangle v1="0" v2="2" v3="1"/><triangle v1="0" v2="1" v3="3"/><triangle v1="1" v2="2" v3="3"/><triangle v1="0" v2="3" v3="2"/></triangles>
 </mesh></object>"#,
            x + 1
        )
    };
    let model = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
 <resources>{}{}{}<object id="4" type="model"><components>
  <component objectid="1"/><component objectid="2"/><component objectid="3"/>
 </components></object></resources>
 <build><item objectid="4" transform="1 0 0 0 1 0 0 0 1 10 10 0"/></build>
</model>"#,
        tetra(1, 0),
        tetra(2, 2),
        tetra(3, 4)
    );
    let settings = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="4">
    <metadata key="name" value="Half"/>
    <part id="1" subtype="normal_part"><metadata key="name" value="A"/></part>
    <part id="2" subtype="negative_part"><metadata key="name" value="Plug"/></part>
    <part id="3" subtype="normal_part"><metadata key="name" value="B"/></part>
  </object>
</config>"#;
    let cut = r#"<?xml version="1.0" encoding="utf-8"?>
<objects>
 <object id="1"><cut_id id="7" check_sum="1" connectors_cnt="1"/><connectors><connector volume_id="1" type="0" r_tolerance="0" h_tolerance="0"/></connectors></object>
</objects>
"#;
    Project::read(Cursor::new(package(&[
        ("3D/3dmodel.model", &model),
        ("Metadata/model_settings.config", settings),
        ("Metadata/cut_information.xml", cut),
    ])))
    .unwrap()
}

fn cut_text(project: &Project) -> Option<String> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(Cursor::new(saved(project))).unwrap();
    let mut text = String::new();
    archive
        .by_name("Metadata/cut_information.xml")
        .ok()?
        .read_to_string(&mut text)
        .unwrap();
    Some(text)
}

#[test]
fn a_cut_connector_follows_its_part_and_one_gone_is_asked_about() {
    let project = cut_object();
    assert!(
        cut_text(&project)
            .unwrap()
            .contains(r#"<connector volume_id="1""#)
    );
    // The part before the connector deleted: the connector is the first.
    let mut fewer = project.clone();
    fewer.objects[0].parts.remove(0);
    assert!(edge_3mf::losses(&fewer, true).is_empty());
    let text = cut_text(&fewer).unwrap();
    assert!(text.contains(r#"<connector volume_id="0""#), "{text}");
    assert!(text.contains(r#"<cut_id id="7""#));
    // A part added after it: as it was.
    let mut more = project.clone();
    let added = more.objects[0].parts[0].clone();
    more.objects[0].parts.push(edge_3mf::Part {
        kept: None,
        ..added
    });
    assert!(
        cut_text(&more)
            .unwrap()
            .contains(r#"<connector volume_id="1""#)
    );
    // The connector deleted: the cut information would name another part,
    // so it is not written, and that is asked about first.
    let mut gone = project.clone();
    gone.objects[0].parts.remove(1);
    let losses = edge_3mf::losses(&gone, true);
    assert!(
        losses.iter().any(|loss| loss.contains("cut information")),
        "{losses:?}"
    );
    assert!(cut_text(&gone).is_none());
}
