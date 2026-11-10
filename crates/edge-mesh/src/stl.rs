//! STL reading (binary and ASCII) and binary writing.

use std::{fs, io, path::Path};

use crate::{Mesh, Triangle};

#[derive(Debug, thiserror::Error)]
pub enum StlError {
    #[error("cannot read STL: {0}")]
    Io(#[from] io::Error),
    #[error("not a valid STL: {0}")]
    Malformed(&'static str),
}

const HEADER_LEN: usize = 84;
const RECORD_LEN: usize = 50;

pub fn load(path: impl AsRef<Path>) -> Result<Mesh, StlError> {
    parse(&fs::read(path)?)
}

pub fn parse(bytes: &[u8]) -> Result<Mesh, StlError> {
    // Binary files may begin with the word "solid" too, so the declared
    // triangle count matching the file size is the only reliable test.
    if is_binary(bytes) {
        Ok(Mesh::from_triangle_soup(
            bytes[HEADER_LEN..]
                .as_chunks::<RECORD_LEN>()
                .0
                .iter()
                .map(binary_record),
        ))
    } else {
        Ok(Mesh::from_triangle_soup(parse_ascii(bytes)?))
    }
}

pub fn to_binary(mesh: &Mesh) -> Vec<u8> {
    let mut out = vec![0u8; 80];
    let count = u32::try_from(mesh.triangles.len()).expect("STL holds at most u32::MAX triangles");
    out.extend_from_slice(&count.to_le_bytes());
    for index in 0..mesh.triangles.len() {
        let [a, b, c] = mesh.triangle(index);
        let (u, v): ([f32; 3], [f32; 3]) = (
            std::array::from_fn(|i| b[i] - a[i]),
            std::array::from_fn(|i| c[i] - a[i]),
        );
        let normal = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let length = normal.iter().map(|c| c * c).sum::<f32>().sqrt();
        for component in normal.map(|c| if length > 0.0 { c / length } else { 0.0 }) {
            out.extend_from_slice(&component.to_le_bytes());
        }
        for component in [a, b, c].as_flattened() {
            out.extend_from_slice(&component.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

fn is_binary(bytes: &[u8]) -> bool {
    let Some(count) = bytes.get(80..HEADER_LEN) else {
        return false;
    };
    let count = u32::from_le_bytes(count.try_into().unwrap()) as u64;
    HEADER_LEN as u64 + count * RECORD_LEN as u64 == bytes.len() as u64
}

fn binary_record(record: &[u8; RECORD_LEN]) -> Triangle {
    // Bytes 0..12 are the normal, which is recomputed from winding when needed.
    std::array::from_fn(|corner| {
        std::array::from_fn(|axis| {
            let at = 12 + corner * 12 + axis * 4;
            f32::from_le_bytes(record[at..at + 4].try_into().unwrap())
        })
    })
}

fn parse_ascii(bytes: &[u8]) -> Result<Vec<Triangle>, StlError> {
    let text = String::from_utf8_lossy(bytes);
    let mut corners: Vec<[f32; 3]> = Vec::new();
    for line in text.lines() {
        let mut words = line.split_ascii_whitespace();
        if !words
            .next()
            .is_some_and(|w| w.eq_ignore_ascii_case("vertex"))
        {
            continue;
        }
        let mut corner = [0.0f32; 3];
        for value in &mut corner {
            *value = words
                .next()
                .and_then(|w| w.parse().ok())
                .ok_or(StlError::Malformed("vertex line without three numbers"))?;
        }
        corners.push(corner);
    }
    if corners.is_empty() {
        return Err(StlError::Malformed(
            "neither a sized binary STL nor ASCII with vertices",
        ));
    }
    if !corners.len().is_multiple_of(3) {
        return Err(StlError::Malformed(
            "ASCII vertex count is not a multiple of three",
        ));
    }
    Ok(corners.as_chunks::<3>().0.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::cuboid;

    #[test]
    fn binary_round_trip() {
        let mesh = Mesh::from_triangle_soup(cuboid([0.0; 3], [10.0, 20.0, 30.0]));
        let parsed = parse(&to_binary(&mesh)).unwrap();
        assert_eq!(parsed.vertices, mesh.vertices);
        assert_eq!(parsed.triangles, mesh.triangles);
    }

    #[test]
    fn ascii_parses_any_case_and_indent() {
        let text = "solid t\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   VERTEX 1e1 0 0\n\tvertex 0 10.5 0\n  endloop\n endfacet\nendsolid t\n";
        let mesh = parse(text.as_bytes()).unwrap();
        assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
        assert_eq!(mesh.vertices[1], [10.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices[2], [0.0, 10.5, 0.0]);
    }

    #[test]
    fn rejects_garbage_and_truncated_files() {
        assert!(matches!(parse(b"not an stl"), Err(StlError::Malformed(_))));
        let mut bytes = to_binary(&Mesh::from_triangle_soup(cuboid([0.0; 3], [1.0; 3])));
        bytes.truncate(bytes.len() - 7);
        assert!(matches!(parse(&bytes), Err(StlError::Malformed(_))));
        assert!(matches!(
            parse(b"solid\nvertex 1 2\n"),
            Err(StlError::Malformed(_))
        ));
    }
}
