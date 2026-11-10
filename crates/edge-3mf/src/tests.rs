use std::io::{Cursor, Write};

use edge_mesh::{Mesh, primitives::cuboid};
use zip::{ZipWriter, write::SimpleFileOptions};

use super::*;

fn package(parts: &[(&str, &str)]) -> Cursor<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in parts {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    Cursor::new(writer.finish().unwrap().into_inner())
}

/// A `<mesh>` element holding a box from the origin to `size`.
fn box_mesh(size: [f32; 3]) -> String {
    let mesh = Mesh::from_triangle_soup(cuboid([0.0; 3], size));
    let vertices: String = mesh
        .vertices
        .iter()
        .map(|[x, y, z]| format!(r#"<vertex x="{x}" y="{y}" z="{z}"/>"#))
        .collect();
    let triangles: String = mesh
        .triangles
        .iter()
        .map(|[a, b, c]| format!(r#"<triangle v1="{a}" v2="{b}" v3="{c}"/>"#))
        .collect();
    format!("<mesh><vertices>{vertices}</vertices><triangles>{triangles}</triangles></mesh>")
}

fn model(unit: &str, resources: &str, build: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="{unit}" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"
       xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06">
 <resources>{resources}</resources><build>{build}</build></model>"#
    )
}

fn error(parts: &[(&str, &str)]) -> String {
    Project::read(package(parts)).unwrap_err().to_string()
}

#[test]
fn plain_3mf_groups_items_into_instances() {
    let resources = format!(
        r#"<object id="7" name="A &amp; B" type="model">{}</object>"#,
        box_mesh([1.0, 2.0, 3.0])
    );
    let build = r#"<item objectid="7" transform="1 0 0 0 1 0 0 0 1 10 20 0"/>
                   <item objectid="7" transform="-1 0 0 0 1 0 0 0 1 0 0 0" printable="0"/>
                   <item objectid="7" transform="-1 0 0 0 1 0 0 0 1 -5 0 0"/>"#;
    let project = Project::read(package(&[(
        "3D/3dmodel.model",
        &model("millimeter", &resources, build),
    )]))
    .unwrap();

    assert!(project.plates.is_empty() && project.settings.is_none());
    let [object] = project.objects.as_slice() else {
        panic!("expected one object")
    };
    assert_eq!((object.id, object.name.as_deref()), (7, Some("A & B")));
    assert_eq!(object.parts.len(), 1);
    assert_eq!(
        object
            .instances
            .iter()
            .map(|i| i.printable)
            .collect::<Vec<_>>(),
        [true, false, true]
    );

    // The unprintable instance is left out and the mirrored one stays a solid.
    let mesh = project.mesh(None);
    assert_eq!(mesh.signed_volume(), 12.0);
    assert_eq!(mesh.bounds(), Some(([-6.0, 0.0, 0.0], [11.0, 22.0, 3.0])));
}

#[test]
fn bambu_layout_reads_parts_roles_plates_and_settings() {
    let root = model(
        "millimeter",
        r#"<object id="4" type="model"><components>
             <component p:path="/3D/Objects/parts.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 0 0 5"/>
             <component p:path="/3D/Objects/parts.model" objectid="2" transform="1 0 0 0 1 0 0 0 1 100 0 0"/>
           </components></object>"#,
        r#"<item objectid="4" transform="1 0 0 0 1 0 0 0 1 50 50 0" printable="1"/>
           <item objectid="4" transform="1 0 0 0 1 0 0 0 1 350 50 0" printable="1"/>"#,
    );
    let sub = model(
        "millimeter",
        &format!(
            r#"<object id="1" type="model">{}</object><object id="2" type="model">{}</object>"#,
            box_mesh([2.0; 3]),
            box_mesh([1.0; 3])
        ),
        "",
    );
    let model_settings = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="4">
    <metadata key="name" value="Bracket"/>
    <part id="1" subtype="normal_part"><metadata key="name" value="Body"/></part>
    <part id="2" subtype="modifier_part"><metadata key="name" value="Dense zone"/><mesh_stat edges_fixed="0"/></part>
  </object>
  <plate>
    <metadata key="plater_id" value="1"/><metadata key="plater_name" value="First"/>
    <model_instance><metadata key="object_id" value="4"/><metadata key="instance_id" value="0"/></model_instance>
  </plate>
  <plate>
    <metadata key="plater_id" value="2"/>
    <model_instance><metadata key="object_id" value="4"/><metadata key="instance_id" value="1"/></model_instance>
    <model_instance><metadata key="object_id" value="4"/><metadata key="instance_id" value="9"/></model_instance>
    <model_instance><metadata key="object_id" value="99"/><metadata key="instance_id" value="0"/></model_instance>
  </plate>
  <assemble><assemble_item object_id="4" instance_id="0"/></assemble>
</config>"#;
    let project = Project::read(package(&[
        ("_rels/.rels", r#"<Relationships><Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#),
        ("3D/3dmodel.model", &root),
        ("3D/Objects/parts.model", &sub),
        (MODEL_SETTINGS_PART, model_settings),
        (PROJECT_SETTINGS_PART, r#"{"layer_height": "0.16", "nozzle_diameter": ["0.4"]}"#),
    ]))
    .unwrap();

    let [object] = project.objects.as_slice() else {
        panic!("expected one object")
    };
    assert_eq!(object.name.as_deref(), Some("Bracket"));
    let described: Vec<_> = object
        .parts
        .iter()
        .map(|p| (p.name.as_deref(), p.kind))
        .collect();
    assert_eq!(
        described,
        [
            (Some("Body"), PartKind::Normal),
            (Some("Dense zone"), PartKind::Modifier)
        ]
    );

    let plates: Vec<_> = project
        .plates
        .iter()
        .map(|p| (p.id, p.name.as_str(), p.instances.clone()))
        .collect();
    assert_eq!(
        plates,
        [
            (
                1,
                "First",
                vec![InstanceRef {
                    object: 0,
                    instance: 0
                }]
            ),
            (
                2,
                "",
                vec![InstanceRef {
                    object: 0,
                    instance: 1
                }]
            ),
        ]
    );
    assert_eq!(project.plate_index(2), Some(1));
    assert_eq!(
        project
            .settings
            .as_ref()
            .unwrap()
            .float("layer_height")
            .unwrap(),
        Some(0.16)
    );

    // Only the normal part is material: a 2 mm cube, lifted 5 by its
    // component and moved onto each plate by its instance.
    let second = project.mesh(Some(1));
    assert_eq!(second.signed_volume(), 8.0);
    assert_eq!(
        second.bounds(),
        Some(([350.0, 50.0, 5.0], [352.0, 52.0, 7.0]))
    );
    assert_eq!(project.mesh(None).signed_volume(), 16.0);
}

#[test]
fn units_scale_geometry_and_placement() {
    let resources = format!(r#"<object id="1">{}</object>"#, box_mesh([1.0; 3]));
    let build = r#"<item objectid="1" transform="1 0 0 0 1 0 0 0 1 2 0 0"/>"#;
    let project = Project::read(package(&[(
        "3D/3dmodel.model",
        &model("inch", &resources, build),
    )]))
    .unwrap();
    let (min, max) = project.mesh(None).bounds().unwrap();
    assert_eq!((min, max), ([50.8, 0.0, 0.0], [76.2, 25.4, 25.4]));
}

#[test]
fn nested_components_compose_inner_first() {
    let resources = format!(
        r#"<object id="1">{}</object>
           <object id="2"><components><component objectid="1" transform="2 0 0 0 2 0 0 0 2 0 0 0"/></components></object>
           <object id="3"><components><component objectid="2" transform="1 0 0 0 1 0 0 0 1 10 0 0"/></components></object>"#,
        box_mesh([1.0; 3])
    );
    let project = Project::read(package(&[(
        "3D/3dmodel.model",
        &model("millimeter", &resources, r#"<item objectid="3"/>"#),
    )]))
    .unwrap();
    assert_eq!(
        project.mesh(None).bounds(),
        Some(([10.0, 0.0, 0.0], [12.0, 2.0, 2.0]))
    );
}

#[test]
fn broken_files_say_what_is_wrong() {
    assert!(matches!(
        Project::read(Cursor::new(b"not a zip".to_vec())),
        Err(Error::Archive(_))
    ));
    assert_eq!(error(&[("other.txt", "")]), "3MF has no 3D/3dmodel.model");

    let bad_index = r#"<object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/></vertices>
        <triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>"#;
    let message = error(&[("3D/3dmodel.model", &model("millimeter", bad_index, ""))]);
    assert!(
        message.contains("vertex 1, but only 1 vertices"),
        "{message}"
    );

    let dangling = r#"<object id="1"><components><component p:path="/3D/gone.model" objectid="5"/></components></object>"#;
    assert_eq!(
        error(&[("3D/3dmodel.model", &model("millimeter", dangling, ""))]),
        "3MF has no 3D/gone.model"
    );

    let cycle = r#"<object id="1"><components><component objectid="1"/></components></object>"#;
    let message = error(&[(
        "3D/3dmodel.model",
        &model("millimeter", cycle, r#"<item objectid="1"/>"#),
    )]);
    assert!(message.contains("nest more than 16 deep"), "{message}");

    let message = error(&[(
        "3D/3dmodel.model",
        &model("millimeter", "", r#"<item objectid="8"/>"#),
    )]);
    assert!(
        message.contains("object 8 is used but not defined"),
        "{message}"
    );

    let message = error(&[("3D/3dmodel.model", &model("cubit", "", ""))]);
    assert!(message.contains("unknown unit"), "{message}");

    let message = error(&[(
        "3D/3dmodel.model",
        &model(
            "millimeter",
            "",
            r#"<item objectid="1" transform="1 0 0"/>"#,
        ),
    )]);
    assert!(message.contains("fewer than 12 numbers"), "{message}");
}

#[test]
fn negative_parts_make_their_instances_separate_bodies() {
    let root = model(
        "millimeter",
        &format!(
            r#"<object id="1">{}</object><object id="2">{}</object><object id="3">{}</object>
               <object id="4"><components><component objectid="1"/><component objectid="2"/></components></object>"#,
            box_mesh([10.0; 3]),
            box_mesh([2.0, 2.0, 20.0]),
            box_mesh([5.0; 3])
        ),
        r#"<item objectid="4"/><item objectid="4" transform="1 0 0 0 1 0 0 0 1 50 0 0"/>
           <item objectid="3" transform="1 0 0 0 1 0 0 0 1 0 50 0"/>
           <item objectid="3" transform="1 0 0 0 1 0 0 0 1 20 50 0"/>"#,
    );
    let model_settings = r#"<config><object id="4">
        <part id="1" subtype="normal_part"/><part id="2" subtype="negative_part"/></object></config>"#;
    let project = Project::read(package(&[
        ("3D/3dmodel.model", &root),
        (MODEL_SETTINGS_PART, model_settings),
    ]))
    .unwrap();

    let bodies = project.bodies(None);
    // The two plain cubes share the first body; each drilled block has its own.
    let volumes: Vec<_> = bodies
        .iter()
        .map(|b| {
            (
                b.solid.signed_volume(),
                b.negative.as_ref().map(Mesh::signed_volume),
            )
        })
        .collect();
    assert_eq!(
        volumes,
        [(250.0, None), (1000.0, Some(80.0)), (1000.0, Some(80.0))]
    );
    assert_eq!(
        bodies[2].negative.as_ref().unwrap().bounds().unwrap().0,
        [50.0, 0.0, 0.0]
    );
    // The merged mesh is the printed material only.
    assert_eq!(project.mesh(None).signed_volume(), 2250.0);
}
