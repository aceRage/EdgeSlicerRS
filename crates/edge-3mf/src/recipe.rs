//! A cut that can be made again: `Metadata/cut_recipe.xml` and the uncut
//! meshes it names, `Metadata/cut_recipe/<sha256>.bin` (the reference's
//! "Keep cut editable", CutRecipe.hpp; written by bbs_3mf.cpp
//! `_add_cut_recipe_file_to_archive`, read by
//! `_extract_cut_recipe_from_archive`).
//!
//! Both halves of a cut carry the same recipe: the plane, the settings it
//! was made with, and the object as it was before it, so either half can
//! put the uncut object back and be cut again. The recipes are not in the
//! model's resources, so a slicer that does not know them still reads the
//! halves.

use std::{collections::HashMap, fmt::Write as _, sync::Arc};

use edge_mesh::{Mesh, Transform};
use quick_xml::{Reader, events::Event};

use crate::{
    Error,
    xml::{attributes, malformed},
};

pub(crate) const RECIPES: &str = "Metadata/cut_recipe.xml";
pub(crate) const MESHES: &str = "Metadata/cut_recipe/";

/// The newest recipe the reference writes and reads (CutRecipe.hpp
/// `CutRecipeVersion`); older ones read with what they lack at its
/// default.
pub const RECIPE_VERSION: u32 = 8;

/// Which surface a cut was made with (CutRecipe.hpp `CutRecipeKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipeKind {
    Plane,
    /// A height field over the plane.
    Curved,
    /// A drawn line.
    Drawn,
    /// Tongue and groove.
    Groove,
}

impl RecipeKind {
    fn code(self) -> u32 {
        match self {
            Self::Plane => 0,
            Self::Curved => 1,
            Self::Drawn => 2,
            Self::Groove => 3,
        }
    }

    fn of(code: u32) -> Option<Self> {
        Some(match code {
            0 => Self::Plane,
            1 => Self::Curved,
            2 => Self::Drawn,
            3 => Self::Groove,
            _ => return None,
        })
    }
}

/// A dovetail's shape (CutRecipe.hpp `CutRecipeGroove`): angles in
/// radians, lengths in mm. The `*_init` values are what the tool's reset
/// buttons go back to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecipeGroove {
    pub depth: f64,
    pub width: f64,
    pub flaps_angle: f64,
    pub angle: f64,
    pub depth_init: f64,
    pub width_init: f64,
    pub flaps_angle_init: f64,
    pub angle_init: f64,
    pub depth_tolerance: f64,
    pub width_tolerance: f64,
    pub count: u32,
    pub gap: f64,
    /// The radius the dovetails' spacing was worked out with; 0 when not
    /// recorded.
    pub plane_radius: f64,
}

/// A connector as a cut placed it (CutRecipe.hpp `CutRecipeConnector`),
/// in the coordinates of the recipe's mesh: its point on the plane, its
/// frame (rows x, y, the plane's normal; the move unused), its radius,
/// height and their tolerances in mm, its turn about the normal in
/// radians, and its type (0 plug, 1 dowel, 2 snap, and the fork's own
/// after), style (0 prism, 1 frustum) and shape (0 triangle, 1 square,
/// 2 hexagon, 3 circle).
#[derive(Clone, Debug, PartialEq)]
pub struct RecipeConnector {
    pub pos: [f64; 3],
    pub rotation: Transform,
    pub radius: f64,
    pub height: f64,
    pub radius_tolerance: f64,
    pub height_tolerance: f64,
    pub z_angle: f64,
    pub kind: u32,
    pub style: u32,
    pub shape: u32,
}

/// How a cut was made, and what it was made of.
#[derive(Clone, Debug)]
pub struct CutRecipe {
    /// The recipe's schema (see [`RECIPE_VERSION`]); one this does not
    /// know is kept, but cannot be cut again.
    pub version: u32,
    pub kind: RecipeKind,
    /// The cut plane in `mesh`'s coordinates: rows its x, its y, its
    /// normal (into the upper part), then its middle.
    pub plane: Transform,
    pub thickness: f64,
    /// Where the thickness is taken: 0 about the plane, 1 above it, 2
    /// below (`CutThicknessOffset`).
    pub thickness_offset: u32,
    pub keep_upper: bool,
    pub keep_lower: bool,
    pub keep_as_parts: bool,
    pub place_on_cut_upper: bool,
    pub place_on_cut_lower: bool,
    pub flip_upper: bool,
    pub flip_lower: bool,
    pub groove: Option<RecipeGroove>,
    /// The connectors the cut placed, as they were placed.
    pub connectors: Vec<RecipeConnector>,
    /// The object's printed parts as they were before the cut, as one
    /// mesh, in the coordinates the halves are in.
    pub mesh: Arc<Mesh>,
    /// The blob it was read from and its name, written back as they were.
    pub(crate) blob: Option<(Arc<Vec<u8>>, String)>,
}

impl PartialEq for CutRecipe {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
            && self.kind == other.kind
            && self.plane == other.plane
            && self.thickness == other.thickness
            && self.thickness_offset == other.thickness_offset
            && (
                self.keep_upper,
                self.keep_lower,
                self.keep_as_parts,
                self.place_on_cut_upper,
                self.place_on_cut_lower,
                self.flip_upper,
                self.flip_lower,
            ) == (
                other.keep_upper,
                other.keep_lower,
                other.keep_as_parts,
                other.place_on_cut_upper,
                other.place_on_cut_lower,
                other.flip_upper,
                other.flip_lower,
            )
            && self.groove == other.groove
            && self.connectors == other.connectors
            && Arc::ptr_eq(&self.mesh, &other.mesh)
    }
}

impl CutRecipe {
    /// A recipe made here, of `mesh`.
    pub fn new(kind: RecipeKind, plane: Transform, mesh: Arc<Mesh>) -> Self {
        Self {
            version: RECIPE_VERSION,
            kind,
            plane,
            thickness: 0.0,
            thickness_offset: 0,
            keep_upper: true,
            keep_lower: true,
            keep_as_parts: false,
            place_on_cut_upper: false,
            place_on_cut_lower: false,
            flip_upper: false,
            flip_lower: false,
            groove: None,
            connectors: Vec::new(),
            mesh,
            blob: None,
        }
    }

    /// Whether a recipe of this schema is understood here.
    pub fn known(&self) -> bool {
        (1..=RECIPE_VERSION).contains(&self.version)
    }
}

/// The mesh as the reference stores it (CutRecipe.cpp
/// `cut_recipe_mesh_to_blob`): "ESCUTMSH", the blob's version (1), the
/// counts, then the corners as f32 and the triangles as u32, little-endian.
pub fn mesh_blob(mesh: &Mesh) -> Vec<u8> {
    let mut blob = Vec::with_capacity(20 + 12 * (mesh.vertices.len() + mesh.triangles.len()));
    blob.extend_from_slice(b"ESCUTMSH");
    blob.extend_from_slice(&1u32.to_le_bytes());
    blob.extend_from_slice(&(mesh.vertices.len() as u32).to_le_bytes());
    blob.extend_from_slice(&(mesh.triangles.len() as u32).to_le_bytes());
    for vertex in &mesh.vertices {
        for value in vertex {
            blob.extend_from_slice(&value.to_le_bytes());
        }
    }
    for triangle in &mesh.triangles {
        for corner in triangle {
            blob.extend_from_slice(&corner.to_le_bytes());
        }
    }
    blob
}

/// The mesh of a blob, when it is one (`cut_recipe_mesh_from_blob`: every
/// length checked, every corner in range).
pub fn mesh_of_blob(blob: &[u8]) -> Option<Mesh> {
    let word = |at: usize| -> Option<u32> {
        blob.get(at..at + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    if blob.len() < 20 || &blob[..8] != b"ESCUTMSH" || word(8)? != 1 {
        return None;
    }
    let (vertices, triangles) = (word(12)? as usize, word(16)? as usize);
    if blob.len() as u64 != 20 + 12 * (vertices as u64 + triangles as u64) {
        return None;
    }
    let mut mesh = Mesh::default();
    let mut at = 20;
    for _ in 0..vertices {
        let v: [f32; 3] =
            std::array::from_fn(|axis| f32::from_bits(word(at + 4 * axis).unwrap_or(0)));
        mesh.vertices.push(v);
        at += 12;
    }
    for _ in 0..triangles {
        let t: [u32; 3] = std::array::from_fn(|corner| word(at + 4 * corner).unwrap_or(u32::MAX));
        if t.iter().any(|&corner| corner as usize >= vertices) {
            return None;
        }
        mesh.triangles.push(t);
        at += 12;
    }
    Some(mesh)
}

/// SHA-256 of `bytes` in lowercase hex: what names a mesh blob.
pub fn sha256(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = bytes.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((bytes.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (held, add) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *held = held.wrapping_add(add);
        }
    }
    h.iter().fold(String::with_capacity(64), |mut out, word| {
        let _ = write!(out, "{word:08x}");
        out
    })
}

/// A recipe as read: the recipe, shared by the halves that carry the same
/// one, and this object's own element, to be written back as it was.
pub(crate) struct ReadRecipe {
    pub recipe: Arc<CutRecipe>,
    pub element: String,
}

/// Sixteen numbers, row by row, as a 4×4 matrix.
fn matrix(text: &str) -> Option<[[f64; 4]; 4]> {
    let numbers: Vec<f64> = text
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (numbers.len() == 16)
        .then(|| std::array::from_fn(|row| std::array::from_fn(|col| numbers[row * 4 + col])))
}

/// `Metadata/cut_recipe.xml` read, by the place of the object each recipe
/// is of (from 1), with the meshes it names. A recipe whose mesh is
/// missing or not a mesh is left out, as the reference leaves it out.
pub(crate) fn read(
    text: &str,
    blobs: &HashMap<String, Arc<Vec<u8>>>,
) -> Result<HashMap<usize, ReadRecipe>, Error> {
    let part = RECIPES;
    let failed = |e: quick_xml::Error| malformed(part, e.to_string());
    let mut reader = Reader::from_str(text);
    let mut out = HashMap::new();
    // Recipes alike but for the object they are of are one recipe.
    let mut alike: HashMap<String, Arc<CutRecipe>> = HashMap::new();
    let mut meshes: HashMap<String, Arc<Mesh>> = HashMap::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(failed)?;
        let (element, inner) = match event {
            Event::Start(element) if element.local_name().as_ref() == "recipe" => {
                let name = element.name().as_ref().to_owned();
                let element = element.into_owned();
                let span = reader
                    .read_to_end(quick_xml::name::QName(&name))
                    .map_err(failed)?;
                (element, &text[span.start as usize..span.end as usize])
            }
            Event::Empty(element) if element.local_name().as_ref() == "recipe" => {
                (element.into_owned(), "")
            }
            Event::Eof => break,
            _ => continue,
        };
        let whole = text[start..reader.buffer_position() as usize].to_owned();
        let mut values: HashMap<String, String> = HashMap::new();
        attributes(&element, part, |key, value| {
            values.insert(key.to_owned(), value.to_owned());
            Ok(())
        })?;
        let get = |key: &str| values.get(key).map(String::as_str);
        let float = |key: &str, default: f64| {
            get(key)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(default)
        };
        let flag = |key: &str, default: bool| {
            get(key)
                .and_then(|v| v.trim().parse::<i64>().ok())
                .map_or(default, |v| v != 0)
        };
        let Some(object) = get("object_id")
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|id| *id > 0)
        else {
            continue;
        };
        let version = get("version")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let Some(kind) =
            RecipeKind::of(get("kind").and_then(|v| v.trim().parse().ok()).unwrap_or(0))
        else {
            continue;
        };
        let Some(hash) = get("mesh").map(str::to_owned) else {
            continue;
        };
        let Some(blob) = blobs.get(&hash) else {
            continue;
        };
        // The plane: its middle and its turn (columns its axes).
        let turn = match get("rotation") {
            Some(text) if !text.trim().is_empty() => match matrix(text) {
                Some(turn) => turn,
                None => continue,
            },
            _ => std::array::from_fn(|row| {
                std::array::from_fn(|col| f64::from(u8::from(row == col)))
            }),
        };
        let middle = [
            float("center_x", 0.0),
            float("center_y", 0.0),
            float("center_z", 0.0),
        ];
        // The reference keeps the uncut mesh in the object's coordinates
        // and the plane in those turned and sized as the copy was; here
        // both are in the latter.
        let linear = get("instance_linear").and_then(matrix);
        let plane = Transform {
            rows: std::array::from_fn(|axis| {
                if axis == 3 {
                    middle
                } else {
                    std::array::from_fn(|row| turn[row][axis])
                }
            }),
        };
        let groove = groove_of(inner, part)?;
        let connectors = connectors_of(inner, part)?;
        let thickness_offset = get("thickness_offset")
            .and_then(|v| v.trim().parse().ok())
            .filter(|v| *v <= 2)
            .unwrap_or(0);
        let recipe = CutRecipe {
            version,
            kind,
            plane,
            thickness: float("thickness", 0.0),
            thickness_offset,
            keep_upper: flag("keep_upper", true),
            keep_lower: flag("keep_lower", true),
            keep_as_parts: flag("keep_as_parts", false),
            place_on_cut_upper: flag("place_on_cut_upper", true),
            place_on_cut_lower: flag("place_on_cut_lower", false),
            flip_upper: flag("rotate_upper", false),
            flip_lower: flag("rotate_lower", false),
            groove,
            connectors,
            mesh: Arc::new(Mesh::default()),
            blob: Some((blob.clone(), hash.clone())),
        };
        let key = format!(
            "{}|{}",
            hash,
            without(&whole, &["object_id", "half_part_matrices"])
        );
        let shared = match alike.get(&key) {
            Some(shared) => shared.clone(),
            None => {
                let mesh_key = format!("{hash}|{}", get("instance_linear").unwrap_or(""));
                let mesh = match meshes.get(&mesh_key) {
                    Some(mesh) => mesh.clone(),
                    None => {
                        let Some(mut mesh) = mesh_of_blob(blob) else {
                            continue;
                        };
                        if let Some(linear) = linear {
                            let turn = Transform {
                                rows: std::array::from_fn(|row| {
                                    if row == 3 {
                                        [0.0; 3]
                                    } else {
                                        std::array::from_fn(|col| linear[col][row])
                                    }
                                }),
                            };
                            for vertex in &mut mesh.vertices {
                                let moved = turn.apply(vertex.map(f64::from));
                                *vertex = moved.map(|value| value as f32);
                            }
                            if turn.is_mirrored() {
                                for triangle in &mut mesh.triangles {
                                    triangle.swap(1, 2);
                                }
                            }
                        }
                        let mesh = Arc::new(mesh);
                        meshes.insert(mesh_key, mesh.clone());
                        mesh
                    }
                };
                let shared = Arc::new(CutRecipe { mesh, ..recipe });
                alike.insert(key, shared.clone());
                shared
            }
        };
        out.insert(
            object,
            ReadRecipe {
                recipe: shared,
                element: whole,
            },
        );
    }
    Ok(out)
}

/// The rows of a 4×4 matrix written row by row, as a frame: rows its
/// columns (axes), the move its last column.
fn frame_of(matrix: &[[f64; 4]; 4]) -> Transform {
    Transform {
        rows: std::array::from_fn(|axis| std::array::from_fn(|row| matrix[row][axis])),
    }
}

/// A frame written as the reference writes a rotation: sixteen numbers,
/// row by row, its axes the columns.
fn matrix_text(frame: &Transform) -> String {
    let mut values = Vec::with_capacity(16);
    for row in 0..4 {
        for col in 0..4 {
            values.push(match (row, col) {
                (3, 3) => 1.0,
                (3, _) | (_, 3) => 0.0,
                _ => frame.rows[col][row],
            });
        }
    }
    values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `<connector>`s inside a recipe.
fn connectors_of(inner: &str, part: &str) -> Result<Vec<RecipeConnector>, Error> {
    let mut reader = Reader::from_str(inner);
    let mut out = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|e| malformed(part, e.to_string()))?
        {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == "connector" =>
            {
                let mut values: HashMap<String, String> = HashMap::new();
                attributes(&element, part, |key, value| {
                    values.insert(key.to_owned(), value.to_owned());
                    Ok(())
                })?;
                let float = |key: &str, default: f64| {
                    values
                        .get(key)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(default)
                };
                let whole = |key: &str| {
                    values
                        .get(key)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0)
                };
                let rotation = values
                    .get("rotation")
                    .and_then(|text| matrix(text))
                    .map(|m| frame_of(&m))
                    .unwrap_or(Transform::IDENTITY);
                out.push(RecipeConnector {
                    pos: [float("x", 0.0), float("y", 0.0), float("z", 0.0)],
                    rotation: Transform {
                        rows: [
                            rotation.rows[0],
                            rotation.rows[1],
                            rotation.rows[2],
                            [0.0; 3],
                        ],
                    },
                    radius: float("radius", 5.0),
                    height: float("height", 10.0),
                    radius_tolerance: float("r_tolerance", 0.0),
                    height_tolerance: float("h_tolerance", 0.1),
                    z_angle: float("z_angle", 0.0),
                    kind: whole("type"),
                    style: whole("style"),
                    shape: whole("shape"),
                });
            }
            Event::Eof => return Ok(out),
            _ => {}
        }
    }
}

/// The `<groove>` inside a recipe, when there is one.
fn groove_of(inner: &str, part: &str) -> Result<Option<RecipeGroove>, Error> {
    let mut reader = Reader::from_str(inner);
    loop {
        match reader
            .read_event()
            .map_err(|e| malformed(part, e.to_string()))?
        {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == "groove" =>
            {
                let mut values: HashMap<String, f64> = HashMap::new();
                attributes(&element, part, |key, value| {
                    if let Ok(number) = value.trim().parse() {
                        values.insert(key.to_owned(), number);
                    }
                    Ok(())
                })?;
                let get = |key: &str, default: f64| values.get(key).copied().unwrap_or(default);
                return Ok(Some(RecipeGroove {
                    depth: get("depth", 0.0),
                    width: get("width", 0.0),
                    flaps_angle: get("flaps_angle", 0.0),
                    angle: get("angle", 0.0),
                    depth_init: get("depth_init", 0.0),
                    width_init: get("width_init", 0.0),
                    flaps_angle_init: get("flaps_angle_init", 0.0),
                    angle_init: get("angle_init", 0.0),
                    depth_tolerance: get("depth_tolerance", 0.1),
                    width_tolerance: get("width_tolerance", 0.1),
                    count: get("count", 1.0).max(1.0) as u32,
                    gap: get("gap", 10.0),
                    plane_radius: get("plane_radius", 0.0),
                }));
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// An element's text with the named attributes taken out.
fn without(element: &str, names: &[&str]) -> String {
    let mut out = element.to_owned();
    for name in names {
        let wanted = format!(" {name}=\"");
        if let Some(at) = out.find(&wanted)
            && let Some(end) = out[at + wanted.len()..].find('"')
        {
            out.replace_range(at..at + wanted.len() + end + 1, "");
        }
    }
    out
}

/// An element as read, now of the object at `place`.
pub(crate) fn element_for(element: &str, place: usize) -> String {
    let wanted = " object_id=\"";
    match element.find(wanted) {
        Some(at) => {
            let from = at + wanted.len();
            let end = element[from..].find('"').map_or(from, |end| from + end);
            format!("{}{place}{}", &element[..from], &element[end..])
        }
        None => element.to_owned(),
    }
}

/// A recipe's element as the reference writes it, of the object at
/// `place`.
pub(crate) fn element_of(recipe: &CutRecipe, place: usize, hash: &str) -> String {
    let mut out = String::new();
    let p = &recipe.plane;
    let mut rotation = Vec::with_capacity(16);
    for row in 0..4 {
        for col in 0..4 {
            rotation.push(match (row, col) {
                (3, 3) => 1.0,
                (3, _) | (_, 3) => 0.0,
                _ => p.rows[col][row],
            });
        }
    }
    let identity: Vec<f64> = (0..16).map(|i| f64::from(u8::from(i % 5 == 0))).collect();
    let join = |values: &[f64]| {
        values
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let bit = |value: bool| u8::from(value);
    let _ = write!(
        out,
        concat!(
            " <recipe object_id=\"{}\" version=\"{}\" kind=\"{}\" mesh=\"{}\"",
            " center_x=\"{}\" center_y=\"{}\" center_z=\"{}\" rotation=\"{}\" instance_linear=\"{}\"",
            " thickness=\"{}\" thickness_offset=\"{}\" keep_upper=\"{}\" keep_lower=\"{}\" keep_as_parts=\"{}\"",
            " place_on_cut_upper=\"{}\" place_on_cut_lower=\"{}\" rotate_upper=\"{}\" rotate_lower=\"{}\"",
            " upper_visibility=\"0\" lower_visibility=\"0\" draw_direction=\"0\" draw_view_x=\"0\" draw_view_y=\"0\"",
            " draw_view_z=\"-1\" draw_extension=\"5\" draw_angle=\"0\" draw_through_all=\"0\" draw_depth=\"3\">"
        ),
        place,
        recipe.version,
        recipe.kind.code(),
        hash,
        p.rows[3][0],
        p.rows[3][1],
        p.rows[3][2],
        join(&rotation),
        join(&identity),
        recipe.thickness,
        recipe.thickness_offset,
        bit(recipe.keep_upper),
        bit(recipe.keep_lower),
        bit(recipe.keep_as_parts),
        bit(recipe.place_on_cut_upper),
        bit(recipe.place_on_cut_lower),
        bit(recipe.flip_upper),
        bit(recipe.flip_lower),
    );
    if !recipe.connectors.is_empty() {
        out.push_str("\n  <connectors>");
        for c in &recipe.connectors {
            let _ = write!(
                out,
                concat!(
                    "\n   <connector x=\"{}\" y=\"{}\" z=\"{}\" rotation=\"{}\" radius=\"{}\" height=\"{}\"",
                    " r_tolerance=\"{}\" h_tolerance=\"{}\" z_angle=\"{}\" type=\"{}\" style=\"{}\" shape=\"{}\"/>"
                ),
                c.pos[0],
                c.pos[1],
                c.pos[2],
                matrix_text(&c.rotation),
                c.radius as f32,
                c.height as f32,
                c.radius_tolerance as f32,
                c.height_tolerance as f32,
                c.z_angle as f32,
                c.kind,
                c.style,
                c.shape,
            );
        }
        out.push_str("\n  </connectors>");
    }
    if let Some(g) = &recipe.groove {
        let _ = write!(
            out,
            concat!(
                "\n  <groove depth=\"{}\" width=\"{}\" flaps_angle=\"{}\" angle=\"{}\" depth_init=\"{}\"",
                " width_init=\"{}\" flaps_angle_init=\"{}\" angle_init=\"{}\" depth_tolerance=\"{}\"",
                " width_tolerance=\"{}\" count=\"{}\" gap=\"{}\" plane_radius=\"{}\"/>"
            ),
            g.depth as f32,
            g.width as f32,
            g.flaps_angle as f32,
            g.angle as f32,
            g.depth_init as f32,
            g.width_init as f32,
            g.flaps_angle_init as f32,
            g.angle_init as f32,
            g.depth_tolerance as f32,
            g.width_tolerance as f32,
            g.count,
            g.gap as f32,
            g.plane_radius as f32,
        );
    }
    out.push_str("\n </recipe>");
    out
}

/// A file of the package by its name.
pub(crate) type Named = (String, Arc<Vec<u8>>);

/// The recipes of the objects (each with its place, from 1, and the
/// element it was read with while its recipe is the one read), as
/// `Metadata/cut_recipe.xml` and the meshes it names; none when no object
/// has one.
pub(crate) fn write<'a>(
    objects: impl Iterator<Item = (usize, &'a CutRecipe, Option<&'a str>)>,
) -> Option<(String, Vec<Named>)> {
    let mut text = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<cut_recipes>\n");
    let mut blobs: Vec<(String, Arc<Vec<u8>>)> = Vec::new();
    let mut any = false;
    let mut made: Vec<(Arc<Mesh>, Arc<Vec<u8>>, String)> = Vec::new();
    for (place, recipe, element) in objects {
        any = true;
        let (blob, hash) = match &recipe.blob {
            Some((blob, hash)) => (blob.clone(), hash.clone()),
            // The halves of a cut share their mesh: made into a blob once.
            None => match made
                .iter()
                .find(|(mesh, _, _)| Arc::ptr_eq(mesh, &recipe.mesh))
            {
                Some((_, blob, hash)) => (blob.clone(), hash.clone()),
                None => {
                    let blob = Arc::new(mesh_blob(&recipe.mesh));
                    let hash = sha256(&blob);
                    made.push((recipe.mesh.clone(), blob.clone(), hash.clone()));
                    (blob, hash)
                }
            },
        };
        match element {
            Some(element) => {
                text.push(' ');
                text.push_str(element_for(element, place).trim());
            }
            None => text.push_str(&element_of(recipe, place, &hash)),
        }
        text.push('\n');
        if !blobs.iter().any(|(held, _)| *held == hash) {
            blobs.push((hash, blob));
        }
    }
    text.push_str("</cut_recipes>\n");
    any.then(|| {
        let files = blobs
            .into_iter()
            .map(|(hash, blob)| (format!("{MESHES}{hash}.bin"), blob))
            .collect();
        (text, files)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_is_the_standard_one() {
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // Two blocks of padding.
        assert_eq!(
            sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    fn tetrahedron() -> Mesh {
        Mesh {
            vertices: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
        }
    }

    #[test]
    fn a_mesh_blob_reads_back_bit_for_bit_and_a_bad_one_does_not() {
        let mesh = tetrahedron();
        let blob = mesh_blob(&mesh);
        assert_eq!(blob.len(), 20 + 12 * 8);
        let back = mesh_of_blob(&blob).unwrap();
        assert_eq!(back.vertices, mesh.vertices);
        assert_eq!(back.triangles, mesh.triangles);
        assert!(mesh_of_blob(&blob[..blob.len() - 1]).is_none());
        let mut wrong = blob.clone();
        let last = wrong.len() - 4;
        wrong[last..].copy_from_slice(&9u32.to_le_bytes());
        assert!(mesh_of_blob(&wrong).is_none());
    }

    #[test]
    fn recipes_written_read_back_and_the_halves_share_one() {
        let mesh = Arc::new(tetrahedron());
        let mut plane = Transform::IDENTITY;
        plane.rows[0] = [0.0, 1.0, 0.0];
        plane.rows[1] = [-1.0, 0.0, 0.0];
        plane.rows[3] = [0.25, 0.5, 0.75];
        let recipe = CutRecipe {
            groove: Some(RecipeGroove {
                depth: 2.0,
                width: 8.0,
                flaps_angle: 1.0,
                angle: 0.0,
                depth_init: 2.0,
                width_init: 8.0,
                flaps_angle_init: 1.0,
                angle_init: 0.0,
                depth_tolerance: 0.1,
                width_tolerance: 0.1,
                count: 2,
                gap: 10.0,
                plane_radius: 30.0,
            }),
            ..CutRecipe::new(RecipeKind::Groove, plane, mesh)
        };
        let (text, blobs) = write([(1, &recipe, None), (2, &recipe, None)].into_iter()).unwrap();
        assert_eq!(blobs.len(), 1);
        let blobs: HashMap<String, Arc<Vec<u8>>> = blobs
            .into_iter()
            .map(|(name, blob)| {
                let hash = name.trim_start_matches(MESHES).trim_end_matches(".bin");
                (hash.to_owned(), blob)
            })
            .collect();
        let read = read(&text, &blobs).unwrap();
        let (one, two) = (&read[&1].recipe, &read[&2].recipe);
        assert!(Arc::ptr_eq(one, two));
        assert_eq!(one.plane, recipe.plane);
        assert_eq!(one.kind, RecipeKind::Groove);
        assert_eq!(one.groove.unwrap().count, 2);
        assert_eq!(one.mesh.vertices, recipe.mesh.vertices);
        // Written back as read, under another place.
        let again = element_for(&read[&2].element, 5);
        assert!(again.contains("object_id=\"5\""), "{again}");
        // A mesh whose name is not its content is not one.
        assert!(super::read(&text, &HashMap::new()).unwrap().is_empty());
    }

    #[test]
    fn the_references_mesh_is_turned_as_its_copy_was() {
        let blob = mesh_blob(&tetrahedron());
        let hash = sha256(&blob);
        let text = format!(
            "<cut_recipes><recipe object_id=\"1\" version=\"5\" kind=\"0\" mesh=\"{hash}\" \
             center_x=\"1\" center_y=\"2\" center_z=\"3\" \
             rotation=\"1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1\" \
             instance_linear=\"2 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1\"/></cut_recipes>"
        );
        let blobs = HashMap::from([(hash, Arc::new(blob))]);
        let read = read(&text, &blobs).unwrap();
        let recipe = &read[&1].recipe;
        assert_eq!(recipe.mesh.vertices[1], [2.0, 0.0, 0.0]);
        assert_eq!(recipe.plane.rows[3], [1.0, 2.0, 3.0]);
        // What it does not say is the reference's default.
        assert!(recipe.place_on_cut_upper && recipe.keep_upper && recipe.keep_lower);
    }
}
