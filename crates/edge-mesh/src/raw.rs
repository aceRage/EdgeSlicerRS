//! For tests: a mesh as raw bytes (corner count, triangle count, the
//! corners' `f32`s, the triangles' `u32`s, little-endian), the form the
//! harnesses that run the reference's own code read and write.

use std::path::Path;

use crate::Mesh;

pub fn to_bytes(mesh: &Mesh) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + mesh.vertices.len() * 12 + mesh.triangles.len() * 12);
    out.extend((mesh.vertices.len() as u32).to_le_bytes());
    out.extend((mesh.triangles.len() as u32).to_le_bytes());
    for v in &mesh.vertices {
        for c in v {
            out.extend(c.to_le_bytes());
        }
    }
    for t in &mesh.triangles {
        for i in t {
            out.extend(i.to_le_bytes());
        }
    }
    out
}

pub fn from_bytes(bytes: &[u8]) -> Mesh {
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
    let (nv, nt) = (word(0) as usize, word(4) as usize);
    let vertices = (0..nv)
        .map(|i| std::array::from_fn(|k| f32::from_bits(word(8 + (i * 3 + k) * 4))))
        .collect();
    let base = 8 + nv * 12;
    let triangles = (0..nt)
        .map(|i| std::array::from_fn(|k| word(base + (i * 3 + k) * 4)))
        .collect();
    Mesh {
        vertices,
        triangles,
    }
}

/// FNV-1a, 64 bits.
pub fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

pub fn load_any(path: &Path) -> Mesh {
    match path.extension().and_then(|e| e.to_str()) {
        Some("stl" | "STL") => crate::stl::load(path).expect("an STL"),
        Some("obj") => crate::obj::load(path).expect("an OBJ"),
        _ => from_bytes(&std::fs::read(path).expect("reads")),
    }
}
