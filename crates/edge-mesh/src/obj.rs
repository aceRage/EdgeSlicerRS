//! Wavefront OBJ reading: the geometry only, as the reference reads it
//! for "Import" (libslic3r/Format/OBJ.cpp `load_obj`, objparser.cpp
//! `obj_parseline`).
//!
//! `v x y z` makes a vertex (a fourth number, or the three or four of a
//! vertex colour after it, are read past); `f` makes a face of the
//! vertices it names, by number from 1 or, below 0, counted back from the
//! last vertex read so far; a face's `/texture/normal` numbers are read
//! past. Every other statement (`vt`, `vn`, `o`, `g`, `s`, `usemtl`,
//! `mtllib`, comments) is ignored: colours and materials are not read.
//!
//! A triangle is taken as it is and a quad is cut into two along its
//! first corner, as the reference cuts it. The reference refuses a face of
//! five corners or more ("The file contains polygons with more than 4
//! vertices."); here a convex one is cut into a fan from its first corner,
//! which is the same cut the quad gets, and only a face that is not
//! convex is refused. A mesh that encloses a negative volume (wound
//! inside out) is turned the right way out, as the reference turns it.
//!
//! Where this reader differs from the reference on purpose: a face that
//! names vertex 0 is refused as naming no vertex. The reference makes it
//! index -1, which is its own mark for the end of a face, so the face is
//! silently cut in two and the faces after it are read from the wrong
//! corners.

use std::{fs, io, path::Path};

use crate::Mesh;

#[derive(Debug, thiserror::Error)]
pub enum ObjError {
    #[error("cannot read OBJ: {0}")]
    Io(#[from] io::Error),
    /// The reference's own sentence where it has one.
    #[error("{0}")]
    Malformed(String),
}

fn malformed(line: usize, what: &str) -> ObjError {
    ObjError::Malformed(format!("line {line}: {what}"))
}

pub fn load(path: impl AsRef<Path>) -> Result<Mesh, ObjError> {
    parse(&fs::read(path)?)
}

pub fn parse(bytes: &[u8]) -> Result<Mesh, ObjError> {
    let text = String::from_utf8_lossy(bytes);
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let number = number + 1;
        let mut words = line.split_ascii_whitespace();
        match words.next() {
            Some("v") => {
                let mut corner = [0.0f32; 3];
                for value in &mut corner {
                    *value = words
                        .next()
                        .and_then(|word| word.parse::<f64>().ok())
                        .filter(|value| value.is_finite())
                        .ok_or_else(|| malformed(number, "a vertex without three numbers"))?
                        as f32;
                }
                vertices.push(corner);
            }
            Some("f") => {
                let mut corners: Vec<u32> = Vec::new();
                for word in words {
                    let index = word.split('/').next().unwrap_or_default();
                    let index: i64 = index
                        .parse()
                        .map_err(|_| malformed(number, "a face corner that is not a number"))?;
                    let count = vertices.len() as i64;
                    let at = match index {
                        0 => return Err(malformed(number, "a face names vertex 0")),
                        positive if positive > 0 => positive - 1,
                        negative => count + negative,
                    };
                    if at < 0 || at >= count {
                        return Err(ObjError::Malformed(
                            "The file contains invalid vertex index.".into(),
                        ));
                    }
                    corners.push(at as u32);
                }
                if corners.len() < 3 {
                    return Err(ObjError::Malformed(
                        "The file contains polygons with less than 2 vertices.".into(),
                    ));
                }
                if corners.len() > 4 && !convex(&vertices, &corners) {
                    return Err(ObjError::Malformed(
                        "The file contains polygons with more than 4 vertices.".into(),
                    ));
                }
                // A fan from the first corner: a quad is cut as the
                // reference cuts it, (0, 1, 2) and (0, 2, 3).
                for at in 1..corners.len() - 1 {
                    let triangle = [corners[0], corners[at], corners[at + 1]];
                    if triangle[0] != triangle[1]
                        && triangle[1] != triangle[2]
                        && triangle[2] != triangle[0]
                    {
                        triangles.push(triangle);
                    }
                }
            }
            _ => {}
        }
    }
    if triangles.is_empty() {
        return Err(ObjError::Malformed(
            "This OBJ file couldn't be read because it's empty.".into(),
        ));
    }
    let mut mesh = Mesh {
        vertices,
        triangles,
    };
    if mesh.signed_volume() < 0.0 {
        for triangle in &mut mesh.triangles {
            triangle.swap(1, 2);
        }
    }
    Ok(mesh)
}

/// Whether the face through `corners` is flat enough and convex: every
/// turn along its edges goes the same way about its normal (Newell's).
fn convex(vertices: &[[f32; 3]], corners: &[u32]) -> bool {
    let point = |at: usize| vertices[corners[at % corners.len()] as usize].map(f64::from);
    let count = corners.len();
    let mut normal = [0.0f64; 3];
    for at in 0..count {
        let (a, b) = (point(at), point(at + 1));
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    if normal.iter().all(|c| *c == 0.0) {
        return false;
    }
    (0..count).all(|at| {
        let (a, b, c) = (point(at), point(at + 1), point(at + 2));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - b[0], c[1] - b[1], c[2] - b[2]];
        let turn = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        turn[0] * normal[0] + turn[1] * normal[1] + turn[2] * normal[2] >= 0.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUBE: &str = "# a cube of quads\no cube\nv 0 0 0\nv 10 0 0\nv 10 10 0\nv 0 10 0\nv 0 0 10\nv 10 0 10\nv 10 10 10\nv 0 10 10\nvn 0 0 1\nf 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n";

    #[test]
    fn quads_are_cut_in_two_and_the_solid_is_closed() {
        let mesh = parse(CUBE.as_bytes()).unwrap();
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.triangles.len(), 12);
        assert!((mesh.signed_volume() - 1000.0).abs() < 1e-6);
        // As the reference cuts a quad: (0, 1, 2) and (0, 2, 3).
        assert_eq!(mesh.triangles[0], [0, 3, 2]);
        assert_eq!(mesh.triangles[1], [0, 2, 1]);
        assert_eq!(mesh.bounds(), Some(([0.0; 3], [10.0; 3])));
    }

    #[test]
    fn a_solid_wound_inside_out_is_turned_round() {
        let inside_out = CUBE.replace("f 1 4 3 2", "f 2 3 4 1").replace(
            "f 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8",
            "f 8 7 6 5\nf 5 6 2 1\nf 6 7 3 2\nf 7 8 4 3\nf 8 5 1 4",
        );
        let mesh = parse(inside_out.as_bytes()).unwrap();
        assert!(mesh.signed_volume() > 0.0);
    }

    #[test]
    fn texture_and_normal_numbers_and_negative_indices_are_read() {
        let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvn 0 0 1\nf -3/1/1 -2/1/1 -1/1/1\nv 0 0 1\nf 1//1 2//1 4//1\n";
        let mesh = parse(text.as_bytes()).unwrap();
        assert_eq!(mesh.triangles.len(), 2);
        assert_eq!(mesh.triangles[1], [0, 1, 3]);
    }

    #[test]
    fn a_convex_polygon_is_a_fan_and_a_concave_one_is_refused() {
        let hexagon =
            "v 2 0 0\nv 1 1.7 0\nv -1 1.7 0\nv -2 0 0\nv -1 -1.7 0\nv 1 -1.7 0\nf 1 2 3 4 5 6\n";
        assert_eq!(parse(hexagon.as_bytes()).unwrap().triangles.len(), 4);
        let dart = "v 0 0 0\nv 4 0 0\nv 4 4 0\nv 2 1 0\nv 0 4 0\nf 1 2 3 4 5\n";
        let error = parse(dart.as_bytes()).unwrap_err().to_string();
        assert_eq!(
            error,
            "The file contains polygons with more than 4 vertices."
        );
    }

    #[test]
    fn what_cannot_be_read_says_why() {
        let said = |text: &str| parse(text.as_bytes()).unwrap_err().to_string();
        assert_eq!(
            said("v 0 0 0\n"),
            "This OBJ file couldn't be read because it's empty."
        );
        assert_eq!(
            said("v 0 0 0\nv 1 0 0\nf 1 2\n"),
            "The file contains polygons with less than 2 vertices."
        );
        assert_eq!(
            said("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 9\n"),
            "The file contains invalid vertex index."
        );
        assert_eq!(
            said("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n"),
            "line 4: a face names vertex 0"
        );
        assert_eq!(said("v 0 0\n"), "line 1: a vertex without three numbers");
    }
}
