//! The paint the reference wrote into its projects, read into the selector
//! (`edge_mesh::selector`) and written back: every code comes back as it
//! was, and a part given back its own paint writes the same file.

use std::{
    fs,
    path::{Path, PathBuf},
};

use edge_3mf::{PaintKind, Project, Save};
use edge_mesh::selector::{Selector, Topology};

fn projects_under(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
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

fn painted_projects() -> Vec<PathBuf> {
    let reference = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/EdgeSlicer");
    assert!(
        reference.join("tests").is_dir(),
        "reference slicer missing; run `git submodule update --init`"
    );
    let mut paths = Vec::new();
    for dir in ["tests", "resources"] {
        projects_under(&reference.join(dir), &mut paths);
    }
    paths.sort();
    paths
}

#[test]
fn every_code_the_reference_wrote_comes_back_from_the_selector() {
    let mut codes_seen = 0;
    let mut parts_seen = 0;
    for path in painted_projects() {
        let Ok(project) = Project::open(&path) else {
            continue;
        };
        for object in &project.objects {
            for part in &object.parts {
                let mut topology = None;
                for kind in PaintKind::ALL {
                    let codes = part.paint_codes(kind);
                    if codes.is_empty() {
                        continue;
                    }
                    parts_seen += 1;
                    let topology = topology.get_or_insert_with(|| Topology::of(&part.mesh));
                    let mut selector = Selector::new(&part.mesh, topology.clone());
                    selector.load_all(codes.iter().map(|(at, code)| (*at, code.as_str())), 255);
                    let back = selector.serialize();
                    assert_eq!(back.len(), codes.len(), "{}: {kind:?}", path.display());
                    for (read, written) in codes.iter().zip(&back) {
                        assert_eq!(read, written, "{}: {kind:?}", path.display());
                    }
                    codes_seen += codes.len();
                }
            }
        }
    }
    // mcbrim_test.3mf alone has 174 374 painted triangles.
    assert!(
        codes_seen > 170_000,
        "only {codes_seen} codes in {parts_seen} parts"
    );
}

#[test]
fn a_part_given_its_own_paint_back_is_written_as_it_was() {
    for path in painted_projects() {
        let Ok(project) = Project::open(&path) else {
            continue;
        };
        if !project
            .objects
            .iter()
            .flat_map(|object| &object.parts)
            .any(|part| part.has_paint(PaintKind::Color))
        {
            continue;
        }
        let mut repainted = project.clone();
        for object in &mut repainted.objects {
            for part in &mut object.parts {
                let codes = part.paint_codes(PaintKind::Color);
                if !codes.is_empty() {
                    *part = part.with_paint(PaintKind::Color, &codes);
                }
            }
        }
        let write = |project: &Project| {
            let mut bytes = std::io::Cursor::new(Vec::new());
            edge_3mf::write_to(
                &Save {
                    project,
                    settings: None,
                    presets: false,
                    filaments: 4,
                },
                &mut bytes,
            )
            .unwrap();
            bytes.into_inner()
        };
        assert!(write(&project) == write(&repainted), "{}", path.display());
        // And the colours read are the same.
        for (a, b) in project.objects.iter().zip(&repainted.objects) {
            for (pa, pb) in a.parts.iter().zip(&b.parts) {
                assert_eq!(
                    pa.paint.as_deref(),
                    pb.paint.as_deref(),
                    "{}",
                    path.display()
                );
            }
        }
    }
}
