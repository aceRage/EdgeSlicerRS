//! Text made into a part (GLGizmoEmboss.cpp, EmbossJob.cpp): the solid of
//! a text, where it goes on an object, making it again, and what a saved
//! project says of it. Checked by the solid being closed and as large as
//! its shape raised by its depth, by where it stands against the face it
//! was put on, and by the project read back.

use std::path::{Path, PathBuf};

use edge_engine::{
    Bed, Chosen, Model, Overrides, PartKind, Saving, TextConfiguration, Transform, Unwatched,
    emboss::{self, TextFonts},
};
use edge_mesh::{Mesh, primitives::cuboid};
use edge_text::Font;

const BED: Bed = Bed {
    min: [0.0, 0.0],
    max: [200.0, 200.0],
};

fn fonts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/EdgeSlicer/resources/fonts")
}

/// The pin's NanumGothic, the bundled font with a licence beside it.
fn nanum() -> Font {
    Font::open(&fonts_dir().join("NanumGothic-Regular.ttf")).unwrap()
}

fn config(text: &str) -> TextConfiguration {
    TextConfiguration {
        text: text.into(),
        style_name: "NORMAL".into(),
        font_descriptor: "NanumGothic-Regular.ttf".into(),
        descriptor_type: edge_3mf::DescriptorType::FileName,
        face_name: Some("NanumGothic".into()),
        ..TextConfiguration::default()
    }
}

/// Every edge is a side of exactly two triangles, once each way.
fn closed(mesh: &Mesh) -> bool {
    let mut edges = std::collections::HashMap::new();
    for t in &mesh.triangles {
        for k in 0..3 {
            *edges.entry((t[k], t[(k + 1) % 3])).or_insert(0) += 1;
        }
    }
    edges
        .iter()
        .all(|(&(a, b), &n)| n == 1 && edges.get(&(b, a)) == Some(&1))
}

fn cube_model(dir: &Path) -> Model {
    let path = dir.join("cube.stl");
    let cube = Mesh::from_triangle_soup(cuboid([0.0; 3], [20.0; 3]));
    std::fs::write(&path, edge_mesh::stl::to_binary(&cube)).unwrap();
    let mut model = Model::open(&path).unwrap();
    // Stood on the bed, as the scene shows it, before lines of sight are
    // taken in it.
    model.add_plate(Some(&BED)).unwrap();
    model
}

#[test]
fn the_solid_is_the_shape_raised_by_its_depth_off_the_surface() {
    let font = nanum();
    let fonts = TextFonts {
        font: &font,
        fallback: None,
    };
    let text = config("Embossed text");
    let (shape, scale, healed) = emboss::text_shape(fonts, &text).unwrap();
    assert!(healed);
    let area: f64 =
        shape.iter().map(|region| region.area_mm2()).sum::<f64>() * (scale / 1e-6).powi(2);
    // A part: from 0.015 mm under its surface, 2 mm deep.
    let made = emboss::emboss(fonts, &text, 2.0, PartKind::Normal).unwrap();
    assert_eq!(made.name, "Embossed text");
    assert!(closed(&made.mesh));
    let volume = made.mesh.signed_volume();
    assert!(
        (volume - 2.0 * area).abs() < 1e-3 * volume,
        "{volume} against {area}"
    );
    let (low, high) = made.mesh.bounds().unwrap();
    assert!((low[2] + 0.015).abs() < 1e-6 && (high[2] - 1.985).abs() < 1e-5);
    // Ten mm high letters, centred: the line's third of the ascent on the
    // baseline's other side ("Center" alignment, `ASCENT_CENTER`).
    assert!(high[0] > 0.0 && low[0] < 0.0);
    assert!((high[0] + low[0]).abs() < 0.05, "{low:?} {high:?}");
    // A negative part or a modifier goes into the object, 0.015 mm proud.
    let cut = emboss::emboss(fonts, &text, 2.0, PartKind::Negative).unwrap();
    let (low, high) = cut.mesh.bounds().unwrap();
    assert!((low[2] + 1.985).abs() < 1e-5 && (high[2] - 0.015).abs() < 1e-6);
    // White space alone is refused in the reference's words.
    let empty = emboss::emboss(fonts, &config(" \n\t"), 2.0, PartKind::Normal);
    assert_eq!(
        empty.unwrap_err().to_string(),
        "Embossed text cannot contain only white spaces."
    );
}

#[test]
fn options_change_the_letters_as_the_reference_does() {
    let font = nanum();
    let fonts = TextFonts {
        font: &font,
        fallback: None,
    };
    let width = |text: &TextConfiguration| {
        let made = emboss::emboss(fonts, text, 1.0, PartKind::Normal).unwrap();
        let (low, high) = made.mesh.bounds().unwrap();
        (high[0] - low[0]) as f64
    };
    let plain = config("AB");
    let mut gap = plain.clone();
    // 100 font units more after each letter: one gap between two letters,
    // 100 / 1000 em of 10 mm.
    gap.char_gap = Some(100);
    let grown = width(&gap) - width(&plain);
    assert!(
        (grown - 100.0 / f64::from(font.units_per_em) * 10.0).abs() < 1e-3,
        "{grown}"
    );
    // Bold grows every outline.
    let mut bold = plain.clone();
    bold.boldness = Some(20.0);
    assert!(width(&bold) > width(&plain));
    // Two lines are twice the line height apart less one.
    let two = emboss::emboss(fonts, &config("A\nA"), 1.0, PartKind::Normal).unwrap();
    let one = emboss::emboss(fonts, &config("A"), 1.0, PartKind::Normal).unwrap();
    let height = |mesh: &Mesh| {
        let (low, high) = mesh.bounds().unwrap();
        (high[1] - low[1]) as f64
    };
    let line =
        f64::from(font.ascent - font.descent + font.line_gap) / f64::from(font.units_per_em) * 10.0;
    assert!((height(&two.mesh) - height(&one.mesh) - line).abs() < 1e-3);
}

#[test]
fn text_added_on_a_face_stands_on_it_and_is_made_again_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = cube_model(dir.path());
    let object = model.objects()[0].id();
    let copy = model.objects()[0].copies()[0].id();
    let font = nanum();
    let fonts = TextFonts {
        font: &font,
        fallback: None,
    };
    let made = emboss::emboss(fonts, &config("Hi"), 1.0, PartKind::Normal).unwrap();
    // The front face of the cube, where it stands in the scene.
    let (scene, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let point = [(low[0] + high[0]) / 2.0, low[1], (low[2] + high[2]) / 2.0];
    let part = model
        .add_text(
            Some(&BED),
            object,
            copy,
            Some((point, [0.0, -1.0, 0.0])),
            None,
            None,
            PartKind::Normal,
            made,
        )
        .unwrap();
    let held = &model.objects()[0].shape().parts()[1];
    assert!(held.text.is_some() && held.decoration);
    // Its frame: on the face, Z out of it, Y up.
    let placed = held.transform.then(&scene);
    let back = placed.apply([0.0; 3]);
    assert!(
        (0..3).all(|i| (back[i] - point[i]).abs() < 1e-6),
        "{back:?} {point:?}"
    );
    assert!((placed.rows[2][1] + 1.0).abs() < 1e-9 && (placed.rows[1][2] - 1.0).abs() < 1e-9);
    // Made again with other words: a new mesh in the same frame.
    let frame = held.transform;
    let again = emboss::emboss(fonts, &config("Hello"), 1.0, PartKind::Negative).unwrap();
    model
        .set_text(
            Some(&BED),
            object,
            part,
            again,
            None,
            Some(PartKind::Negative),
        )
        .unwrap();
    let held = &model.objects()[0].shape().parts()[1];
    assert_eq!(held.transform, frame);
    assert_eq!(held.kind, PartKind::Negative);
    assert_eq!(held.name.as_deref(), Some("Hello"));
    // The cube's only printed part cannot be made a negative part, and a
    // part that is not text is not made again.
    let cube = model.objects()[0].shape().part_ids()[0];
    let other = emboss::emboss(fonts, &config("x"), 1.0, PartKind::Normal).unwrap();
    assert!(
        model
            .set_text(Some(&BED), object, cube, other, None, None)
            .is_err()
    );
}

#[test]
fn a_text_with_no_object_is_an_object_of_its_own_on_the_bed() {
    let mut model = Model::new();
    let font = nanum();
    let fonts = TextFonts {
        font: &font,
        fallback: None,
    };
    let made = emboss::emboss(fonts, &config("Tag"), 2.0, PartKind::Normal).unwrap();
    model.add_plate(Some(&BED)).ok();
    let copy = model
        .add_text_object(Some(&BED), 0, None, None, made)
        .unwrap();
    let object = &model.objects()[0];
    assert_eq!(object.name(), "Tag");
    assert_eq!(object.copies()[0].id(), copy);
    let [low, high] = object.copies()[0].reach().unwrap();
    // On the bed, in its middle.
    assert!(low[2].abs() < 1e-6, "{low:?}");
    assert!(((low[0] + high[0]) / 2.0 - 100.0).abs() < 1e-3);
}

fn sliced(model: &Model) -> String {
    let job = edge_engine::prepare(
        model.place(Some(model.plates()[0].id())).unwrap(),
        &Chosen::default(),
        &Overrides::default(),
    )
    .unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    unnamed(&String::from_utf8(sliced.gcode("2001-01-01 at 00:00:00").unwrap().text).unwrap())
}

/// The file with the objects' names taken out of the `; printing object`
/// comments: the name is the file's the part was read from, which a test
/// writes under a name of its own each time, or loses saving the part
/// to a project and reading it back.
fn unnamed(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            for start in ["; printing object ", "; stop printing object "] {
                if line.starts_with(start)
                    && let Some(at) = line.rfind(" id:")
                {
                    return format!("{}{}", start.trim_end(), &line[at..]);
                }
            }
            line.to_owned()
        })
        .collect()
}

/// A text part saved and read back is the same text, with its frame; a
/// project the reference wrote, centred, is made again where it was.
#[test]
fn text_is_saved_read_back_and_made_again_where_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = cube_model(dir.path());
    let object = model.objects()[0].id();
    let copy = model.objects()[0].copies()[0].id();
    let font = nanum();
    let fonts = TextFonts {
        font: &font,
        fallback: None,
    };
    let (scene, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let point = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, high[2]];
    let made = emboss::emboss(fonts, &config("Top"), 1.5, PartKind::Normal).unwrap();
    let part = model
        .add_text(
            Some(&BED),
            object,
            copy,
            Some((point, [0.0, 0.0, 1.0])),
            Some(0.3),
            None,
            PartKind::Normal,
            made,
        )
        .unwrap();
    let frame = model.text_frame(object, part).unwrap();
    let path = dir.path().join("text.3mf");
    model.save(&path, &Saving::None, Some(&BED)).unwrap();
    if let Some(keep) = std::env::var_os("EDGE_KEEP_SAVED") {
        std::fs::copy(&path, Path::new(&keep).join("text.3mf")).unwrap();
    }
    let again = Model::open(&path).unwrap();
    let object_again = again.objects()[0].id();
    let part_again = again.objects()[0].shape().part_ids()[1];
    let read = again.text_of(object_again, part_again).unwrap().unwrap();
    let was = model.text_of(object, part).unwrap().unwrap();
    assert_eq!(read.text, was.text);
    assert_eq!(read.shape.depth, 1.5);
    // The fix is the move to the middle of the mesh's box, which this
    // engine keeps where it was: the frame read back is the frame saved.
    let frame_again = again.text_frame(object_again, part_again).unwrap();
    for (a, b) in frame
        .rows
        .iter()
        .flatten()
        .zip(frame_again.rows.iter().flatten())
    {
        assert!((a - b).abs() < 1e-5, "{frame:?} {frame_again:?}");
    }
    // Saved again unchanged, its description is written as it was read.
    let path2 = dir.path().join("text2.3mf");
    again.save(&path2, &Saving::None, Some(&BED)).unwrap();
    let settings = |path: &Path| {
        let file = std::fs::File::open(path).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("Metadata/model_settings.config").unwrap(),
            &mut text,
        )
        .unwrap();
        text
    };
    let first = settings(&path);
    assert!(first.contains("<slic3rpe:shape "), "{first}");
    assert!(first.contains(r#"<slic3rpe:text text="Top""#), "{first}");
    let lines = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|line| line.contains("slic3rpe:"))
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(lines(&first), lines(&settings(&path2)));
    // It slices as it did before it was saved.
    assert_eq!(sliced(&model), sliced(&again));
    let _ = scene;
}

/// A text the reference wrote and read back: its mesh centred about the
/// middle of its box, its part moved there, the shape's `transform` the
/// move back (`fix_3mf_tr`). Its frame here is where the reference would
/// make it again (`volume matrix * fix⁻¹`).
#[test]
fn a_text_the_reference_centred_is_made_again_in_its_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = cube_model(dir.path());
    let object = model.objects()[0].id();
    let copy = model.objects()[0].copies()[0].id();
    let font = nanum();
    let fonts = TextFonts {
        font: &font,
        fallback: None,
    };
    let made = emboss::emboss(fonts, &config("Ref"), 1.0, PartKind::Normal).unwrap();
    let part = model
        .add_text(
            Some(&BED),
            object,
            copy,
            None,
            None,
            None,
            PartKind::Normal,
            made.clone(),
        )
        .unwrap();
    let frame = Transform::translation([10.0, 0.0, 10.0]);
    let (low, high) = made.mesh.bounds().unwrap();
    let centre: [f64; 3] = std::array::from_fn(|i| (f64::from(low[i]) + f64::from(high[i])) / 2.0);
    let mut centred = made.mesh.clone();
    for v in &mut centred.vertices {
        for i in 0..3 {
            v[i] = (f64::from(v[i]) - centre[i]) as f32;
        }
    }
    let mut volume = made.volume.clone();
    volume.shape.fix = Some(Transform::translation(centre));
    let read = emboss::NewText {
        volume,
        mesh: centred,
        name: "Ref".into(),
    };
    model
        .set_text(
            Some(&BED),
            object,
            part,
            read,
            Some(Transform::translation(centre).then(&frame)),
            None,
        )
        .unwrap();
    let found = model.text_frame(object, part).unwrap();
    for (a, b) in found.rows.iter().flatten().zip(frame.rows.iter().flatten()) {
        assert!((a - b).abs() < 1e-5, "{found:?}");
    }
}

/// "From surface" measures the text's distance off the face along its own
/// Z, and "Set text to face camera" turns its Z to the eye.
#[test]
fn the_distance_from_the_surface_and_facing_the_camera() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = cube_model(dir.path());
    let object = model.objects()[0].id();
    let copy = model.objects()[0].copies()[0].id();
    let font = nanum();
    let fonts = TextFonts {
        font: &font,
        fallback: None,
    };
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let point = [(low[0] + high[0]) / 2.0, low[1], (low[2] + high[2]) / 2.0];
    let made = emboss::emboss(fonts, &config("D"), 1.0, PartKind::Normal).unwrap();
    let part = model
        .add_text(
            Some(&BED),
            object,
            copy,
            Some((point, [0.0, -1.0, 0.0])),
            None,
            Some(1.5),
            PartKind::Normal,
            made,
        )
        .unwrap();
    let distance = model
        .text_distance(Some(&BED), object, part, copy)
        .unwrap()
        .unwrap();
    assert!((distance - 1.5).abs() < 1e-6, "{distance}");
    // Seen from the right (+X), facing the camera turns its Z to +X.
    let frame = model.text_frame(object, part).unwrap();
    let eye = [high[0] + 100.0, point[1] - 1.5, point[2]];
    let turned = model
        .text_facing(
            Some(&BED),
            object,
            copy,
            &frame,
            Some(eye),
            [-1.0, 0.0, 0.0],
            true,
        )
        .unwrap();
    let z = turned.rows[2];
    assert!(
        (z[0] - 1.0).abs() < 1e-6 && z[1].abs() < 1e-6 && z[2].abs() < 1e-6,
        "{z:?}"
    );
    // Its up stays the world's Z, and it stays where it was.
    assert!((turned.rows[1][2] - 1.0).abs() < 1e-6, "{:?}", turned.rows);
    for i in 0..3 {
        assert!((turned.rows[3][i] - frame.rows[3][i]).abs() < 1e-6);
    }
}
