//! The fork's TAC Joint (libslic3r/TacJoint.cpp, TacJoint.hpp): a
//! connector of a fixed reference shape, 11 mm long along its own z (the
//! cut's normal), scaled as a whole, and the socket it fits, the reference
//! cutter grown or shrunk so that it clears the connector by a side and an
//! end clearance. The connector is printed on its own (laid down) or as a
//! part of one half; each other half takes a socket.
//!
//! Ported from TacJoint.cpp (AGPL-3.0, as this crate); the meshes are its
//! TacJointData.inc ([`crate::tac_data`]).

use std::sync::OnceLock;

use edge_mesh::Mesh;
use edge_mesh::raycast::Tree;

use crate::tac_data;

type V = [f64; 3];

/// Which half the connector is printed as a part of, or neither
/// (`TacJointAttachment`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TacAttachment {
    Separate,
    Upper,
    Lower,
    /// A value a file gave that is none of them: refused by
    /// [`validate`].
    Unknown,
}

impl TacAttachment {
    pub fn code(self) -> i32 {
        match self {
            Self::Separate => 0,
            Self::Upper => 1,
            Self::Lower => 2,
            Self::Unknown => -1,
        }
    }

    pub fn of(code: i32) -> Self {
        match code {
            0 => Self::Separate,
            1 => Self::Upper,
            2 => Self::Lower,
            _ => Self::Unknown,
        }
    }
}

/// The joint's settings (`TacJointParams`): its size as a share of the
/// reference's, the clearance round its sides and at its ends (mm), how
/// far it is moved along its axis (mm), the reference's version, and
/// where it is printed. Single precision as the reference keeps them, so
/// that a file's values and the defaults compare as there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TacParams {
    pub scale: f32,
    pub clearance: f32,
    pub end_clearance: f32,
    pub offset: f32,
    pub version: i32,
    pub attachment: TacAttachment,
}

impl Default for TacParams {
    fn default() -> Self {
        Self {
            scale: 1.0,
            clearance: 0.12,
            end_clearance: 0.30,
            offset: 0.0,
            version: 1,
            attachment: TacAttachment::Separate,
        }
    }
}

/// Nearer than this, in the reference's frame, a point is on a surface.
const CONTAINMENT_EPSILON: f64 = 1e-7;
/// The least engagement either half must have, mm.
const ENGAGEMENT_EPSILON: f64 = 1e-5;
/// How far a socket reaches through its half's face, mm (`mouth_overlap`).
pub const MOUTH_OVERLAP: f64 = 0.02;

/// The two reference meshes and what is asked of them, made once.
struct Reference {
    connector: Mesh,
    cutter: Mesh,
    connector_box: (V, V),
    cutter_box: (V, V),
    connector_edges: Vec<(usize, usize)>,
    cutter_edges: Vec<(usize, usize)>,
    connector_tree: Tree,
    cutter_tree: Tree,
}

fn make_reference(vertices: &[[f32; 3]], triangles: &[[u32; 3]]) -> Mesh {
    let r = tac_data::ROTATION;
    let t = tac_data::TRANSLATION;
    Mesh {
        vertices: vertices
            .iter()
            .map(|v| {
                let v = v.map(f64::from);
                std::array::from_fn(|row| {
                    (r[row][0] * v[0] + r[row][1] * v[1] + r[row][2] * v[2] + t[row]) as f32
                })
            })
            .collect(),
        triangles: triangles.to_vec(),
    }
}

fn bounds(mesh: &Mesh) -> (V, V) {
    mesh.bounds()
        .map(|(lo, hi)| (lo.map(f64::from), hi.map(f64::from)))
        .unwrap_or(([0.0; 3], [0.0; 3]))
}

fn edges(mesh: &Mesh) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = mesh
        .triangles
        .iter()
        .flat_map(|t| {
            (0..3).map(move |i| {
                let (a, b) = (t[i] as usize, t[(i + 1) % 3] as usize);
                (a.min(b), a.max(b))
            })
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn reference() -> &'static Reference {
    static MADE: OnceLock<Reference> = OnceLock::new();
    MADE.get_or_init(|| {
        let connector = make_reference(
            &tac_data::CONNECTOR_VERTICES,
            &tac_data::CONNECTOR_TRIANGLES,
        );
        let cutter = make_reference(&tac_data::CUTTER_VERTICES, &tac_data::CUTTER_TRIANGLES);
        Reference {
            connector_box: bounds(&connector),
            cutter_box: bounds(&cutter),
            connector_edges: edges(&connector),
            cutter_edges: edges(&cutter),
            connector_tree: Tree::new(&connector),
            cutter_tree: Tree::new(&cutter),
            connector,
            cutter,
        }
    })
}

fn size(b: (V, V)) -> V {
    std::array::from_fn(|axis| b.1[axis] - b.0[axis])
}

fn centre(b: (V, V)) -> V {
    std::array::from_fn(|axis| 0.5 * (b.0[axis] + b.1[axis]))
}

/// How the reference cutter is grown or shrunk along each axis
/// (`cutter_factors`): at the default clearances it is the original STL.
fn cutter_factors(p: &TacParams) -> V {
    let size = size(reference().cutter_box);
    let nominal = [
        2.0 * f64::from(0.12f32),
        2.0 * f64::from(0.12f32),
        2.0 * f64::from(0.30f32),
    ];
    let desired = [
        2.0 * f64::from(p.clearance),
        2.0 * f64::from(p.clearance),
        2.0 * f64::from(p.end_clearance),
    ];
    std::array::from_fn(|axis| {
        (f64::from(p.scale) * (size[axis] - nominal[axis]) + desired[axis]) / size[axis]
    })
}

/// The settings alone (`validate_scalars`): the reference's words, or
/// none.
fn validate_scalars(p: &TacParams) -> Option<&'static str> {
    if p.version != 1 {
        return Some("Unsupported TAC Joint reference version.");
    }
    if p.attachment == TacAttachment::Unknown {
        return Some("Unsupported TAC Joint attachment.");
    }
    if !p.scale.is_finite()
        || !p.clearance.is_finite()
        || !p.end_clearance.is_finite()
        || !p.offset.is_finite()
    {
        return Some("TAC Joint parameters must be finite.");
    }
    if p.scale <= 0.0 || p.clearance < 0.0 || p.end_clearance < 0.0 {
        return Some("TAC Joint size must be positive and clearances must be nonnegative.");
    }
    let factors = cutter_factors(p);
    let cutter = size(reference().cutter_box);
    let connector = size(reference().connector_box);
    let extent: V = std::array::from_fn(|axis| cutter[axis] * factors[axis]);
    if extent.iter().any(|v| *v <= 1e-5)
        || connector.iter().any(|v| v * f64::from(p.scale) <= 1e-5)
        || extent.iter().any(|v| *v > 1e6)
        || f64::from(p.offset).abs() > 1e6
    {
        return Some("TAC Joint dimensions are outside the supported numerical range.");
    }
    None
}

/// Whether the segment from `a` to `b` crosses the surface between its
/// ends (`edge_crosses_surface`).
fn crosses(a: V, b: V, tree: &Tree) -> bool {
    let delta: V = std::array::from_fn(|axis| b[axis] - a[axis]);
    let length = delta.iter().map(|v| v * v).sum::<f64>().sqrt();
    if length <= CONTAINMENT_EPSILON {
        return false;
    }
    let dir = delta.map(|v| v / length);
    tree.hit(
        a,
        dir,
        (CONTAINMENT_EPSILON, length - CONTAINMENT_EPSILON),
        &|_| true,
    )
    .is_some()
}

/// Whether a point is inside a closed mesh: the first face a ray from it
/// meets is met from inside (`hit_result::is_inside`).
fn inside(point: V, tree: &Tree) -> bool {
    const RAY: V = [0.3713906763541037, 0.529271389135, 0.763618841827];
    let n = RAY.iter().map(|v| v * v).sum::<f64>().sqrt();
    let dir = RAY.map(|v| v / n);
    tree.hit(point, dir, (0.0, f64::INFINITY), &|_| true)
        .is_some_and(|hit| {
            let normal = tree.normal(hit.triangle);
            normal.iter().zip(dir).map(|(a, b)| a * b).sum::<f64>() > 0.0
        })
}

/// Whether the socket holds the connector clear of it everywhere
/// (`contains_connector`): every corner of the connector inside the
/// socket and off its surface, no edge of either crossing the other.
fn contains_connector(p: &TacParams) -> bool {
    let r = reference();
    let factors = cutter_factors(p);
    let middle = centre(r.cutter_box);
    let scale = f64::from(p.scale);
    let shifted: V = middle.map(|v| v * scale);
    let mut in_cutter = Vec::with_capacity(r.connector.vertices.len());
    for v in &r.connector.vertices {
        let q: V = std::array::from_fn(|axis| {
            (f64::from(v[axis]) * scale - shifted[axis]) / factors[axis] + middle[axis]
        });
        let near = r.cutter_tree.closest_point(q).is_some_and(|(at, _, _)| {
            (0..3).map(|axis| (at[axis] - q[axis]).powi(2)).sum::<f64>()
                <= CONTAINMENT_EPSILON * CONTAINMENT_EPSILON
        });
        if near || !inside(q, &r.cutter_tree) {
            return false;
        }
        in_cutter.push(q);
    }
    if r.connector_edges
        .iter()
        .any(|&(a, b)| crosses(in_cutter[a], in_cutter[b], &r.cutter_tree))
    {
        return false;
    }
    let in_connector: Vec<V> = r
        .cutter
        .vertices
        .iter()
        .map(|v| {
            std::array::from_fn(|axis| {
                ((f64::from(v[axis]) - middle[axis]) * factors[axis] + shifted[axis]) / scale
            })
        })
        .collect();
    !r.cutter_edges
        .iter()
        .any(|&(a, b)| crosses(in_connector[a], in_connector[b], &r.connector_tree))
}

/// Whether the joint can be made with the cut's faces at `kerf_lo` and
/// `kerf_hi` along its axis (`tac_joint_validate`): the reference's words
/// when it cannot. The gizmo shows them through `_L`.
pub fn validate(p: &TacParams, kerf_lo: f64, kerf_hi: f64) -> Option<&'static str> {
    if let Some(why) = validate_scalars(p) {
        return Some(why);
    }
    if !kerf_lo.is_finite() || !kerf_hi.is_finite() || kerf_lo > kerf_hi {
        return Some("TAC Joint cut gap must have finite, ordered boundaries.");
    }
    let (lo, hi) = reference().connector_box;
    let (scale, offset) = (f64::from(p.scale), f64::from(p.offset));
    if scale * lo[2] + offset >= kerf_lo - ENGAGEMENT_EPSILON
        || scale * hi[2] + offset <= kerf_hi + ENGAGEMENT_EPSILON
    {
        return Some(
            "TAC Joint needs positive engagement in both retained parts. Reduce the offset or cut gap.",
        );
    }
    if !contains_connector(p) {
        return Some(
            "TAC Joint cutter intersects the connector profile. Increase the side or end clearance.",
        );
    }
    None
}

/// The connector, scaled and moved along its axis (`tac_joint_connector_mesh`);
/// none for settings that are not sound.
pub fn connector_mesh(p: &TacParams) -> Option<Mesh> {
    if validate_scalars(p).is_some() {
        return None;
    }
    let mut out = reference().connector.clone();
    for v in &mut out.vertices {
        *v = [v[0] * p.scale, v[1] * p.scale, v[2] * p.scale + p.offset];
    }
    Some(out)
}

/// The socket: the reference cutter grown to the clearances and placed
/// with the connector (`tac_joint_cutter_mesh`).
pub fn cutter_mesh(p: &TacParams) -> Option<Mesh> {
    if validate_scalars(p).is_some() {
        return None;
    }
    let r = reference();
    let factors = cutter_factors(p);
    let middle = centre(r.cutter_box);
    let scale = f64::from(p.scale);
    let shift: V = [
        middle[0] * scale,
        middle[1] * scale,
        middle[2] * scale + f64::from(p.offset),
    ];
    let mut out = r.cutter.clone();
    for v in &mut out.vertices {
        *v = std::array::from_fn(|axis| {
            ((f64::from(v[axis]) - middle[axis]) * factors[axis] + shift[axis]) as f32
        });
    }
    Some(out)
}

/// One half's socket: the cutter from just inside that half's face on
/// (the upper's above `face_hi`, the lower's below `face_lo`, each
/// [`MOUTH_OVERLAP`] through it), closed.
///
/// DIFFERS FROM THE REFERENCE, which intersects the cutter with a box by
/// its mesh boolean because its plane cut "can triangulate the reference
/// profile's cap with overlapping triangles" (CutUtils.cpp
/// `process_tac_joint_cut`): a workaround for its own cut. The plane cut
/// here closes its caps soundly ([`edge_mesh::cut`]), so it is used.
pub fn socket_mesh(p: &TacParams, upper: bool, face_lo: f64, face_hi: f64) -> Option<Mesh> {
    let cutter = cutter_mesh(p)?;
    let at = if upper {
        face_hi - MOUTH_OVERLAP
    } else {
        face_lo + MOUTH_OVERLAP
    };
    let halves = edge_mesh::cut::cut(
        &cutter,
        &edge_mesh::Transform::IDENTITY,
        [at; 2],
        &edge_mesh::Transform::IDENTITY,
    );
    if halves.open != 0 {
        return None;
    }
    let half = if upper { halves.upper } else { halves.lower };
    (!half.is_empty()).then_some(half.mesh)
}

/// How far the socket reaches from the joint's axis across the cut
/// (`tac_joint_footprint_radius`), 0 for settings that are not sound.
pub fn footprint_radius(p: &TacParams) -> f64 {
    cutter_mesh(p).map_or(0.0, |mesh| {
        mesh.vertices
            .iter()
            .map(|v| f64::from(v[0]).hypot(f64::from(v[1])))
            .fold(0.0, f64::max)
    })
}

impl TacParams {
    /// The reference's attribute names and values (`tac_joint_attributes`).
    pub fn attributes(&self) -> Vec<(String, f64)> {
        vec![
            ("tac_joint_scale".to_owned(), f64::from(self.scale)),
            ("tac_joint_clearance".to_owned(), f64::from(self.clearance)),
            (
                "tac_joint_end_clearance".to_owned(),
                f64::from(self.end_clearance),
            ),
            ("tac_joint_offset".to_owned(), f64::from(self.offset)),
            ("tac_joint_version".to_owned(), f64::from(self.version)),
            (
                "tac_joint_attachment".to_owned(),
                f64::from(self.attachment.code()),
            ),
        ]
    }

    /// The settings from those attributes (`tac_joint_from_attributes`):
    /// missing ones take their defaults, values that cannot be are kept
    /// as such for [`validate`] to refuse.
    pub fn from_attributes(get: impl Fn(&str) -> Option<f64>) -> Self {
        let mut p = Self::default();
        let float = |key: &str, value: &mut f32| {
            if let Some(v) = get(key) {
                *value = if v.is_finite() && v.abs() <= f64::from(f32::MAX) {
                    v as f32
                } else {
                    f32::NAN
                };
            }
        };
        float("tac_joint_scale", &mut p.scale);
        float("tac_joint_clearance", &mut p.clearance);
        float("tac_joint_end_clearance", &mut p.end_clearance);
        float("tac_joint_offset", &mut p.offset);
        if let Some(v) = get("tac_joint_version") {
            p.version = if v.is_finite()
                && v.floor() == v
                && v >= f64::from(i32::MIN)
                && v <= f64::from(i32::MAX)
            {
                v as i32
            } else {
                -1
            };
        }
        if let Some(v) = get("tac_joint_attachment") {
            p.attachment = if v.is_finite() && v.floor() == v && (0.0..=2.0).contains(&v) {
                TacAttachment::of(v as i32)
            } else {
                TacAttachment::Unknown
            };
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(mesh: &Mesh) {
        assert!(edge_mesh::cut::is_closed(mesh));
        assert!(mesh.signed_volume() > 0.0, "{}", mesh.signed_volume());
        assert_eq!(edge_mesh::cut::flat_triangles(mesh), 0);
    }

    #[test]
    fn the_reference_meshes_are_closed_and_eleven_millimetres_long() {
        let p = TacParams::default();
        let connector = connector_mesh(&p).unwrap();
        let cutter = cutter_mesh(&p).unwrap();
        closed(&connector);
        closed(&cutter);
        let (lo, hi) = bounds(&connector);
        assert!((hi[2] - lo[2] - 11.0).abs() < 0.1, "{lo:?} {hi:?}");
        // The connector straddles the plane.
        assert!(lo[2] < 0.0 && hi[2] > 0.0);
        assert!(cutter.signed_volume() > connector.signed_volume());
    }

    #[test]
    fn the_default_joint_fits_and_too_little_clearance_does_not() {
        let p = TacParams::default();
        assert_eq!(validate(&p, 0.0, 0.0), None);
        let tight = TacParams {
            clearance: 0.0,
            end_clearance: 0.0,
            ..p
        };
        assert_eq!(
            validate(&tight, 0.0, 0.0),
            Some(
                "TAC Joint cutter intersects the connector profile. Increase the side or end clearance."
            )
        );
        // Scaled up with its clearances, it still fits.
        let big = TacParams { scale: 2.0, ..p };
        assert_eq!(validate(&big, 0.0, 0.0), None);
    }

    #[test]
    fn the_joint_must_reach_into_both_halves() {
        let p = TacParams::default();
        let (lo, hi) = reference().connector_box;
        assert!(validate(&p, lo[2] + 0.5, hi[2] - 0.5).is_none());
        assert_eq!(
            validate(&p, lo[2] - 0.1, 0.0),
            Some(
                "TAC Joint needs positive engagement in both retained parts. Reduce the offset or cut gap."
            )
        );
        let moved = TacParams {
            offset: (hi[2] + 1.0) as f32,
            ..p
        };
        assert!(validate(&moved, 0.0, 0.0).is_some());
        assert_eq!(
            validate(&p, 1.0, 0.0),
            Some("TAC Joint cut gap must have finite, ordered boundaries.")
        );
    }

    #[test]
    fn unsound_settings_are_refused_in_the_references_words() {
        let p = TacParams::default();
        let cases = [
            (
                TacParams { version: 2, ..p },
                "Unsupported TAC Joint reference version.",
            ),
            (
                TacParams {
                    attachment: TacAttachment::Unknown,
                    ..p
                },
                "Unsupported TAC Joint attachment.",
            ),
            (
                TacParams {
                    scale: f32::NAN,
                    ..p
                },
                "TAC Joint parameters must be finite.",
            ),
            (
                TacParams { scale: 0.0, ..p },
                "TAC Joint size must be positive and clearances must be nonnegative.",
            ),
        ];
        for (params, words) in cases {
            assert_eq!(validate(&params, 0.0, 0.0), Some(words));
            assert!(connector_mesh(&params).is_none());
        }
    }

    #[test]
    fn each_socket_is_closed_and_through_its_face() {
        let p = TacParams::default();
        for (upper, lo, hi) in [(true, -0.5, 0.5), (false, -0.5, 0.5), (true, 0.0, 0.0)] {
            let socket = socket_mesh(&p, upper, lo, hi).unwrap();
            closed(&socket);
            let (low, high) = bounds(&socket);
            if upper {
                assert!((low[2] - (hi - MOUTH_OVERLAP)).abs() < 1e-5);
            } else {
                assert!((high[2] - (lo + MOUTH_OVERLAP)).abs() < 1e-5);
            }
        }
        assert!(footprint_radius(&p) > 1.0);
    }

    #[test]
    fn the_settings_round_trip_through_their_attributes() {
        let p = TacParams {
            scale: 1.5,
            clearance: 0.2,
            end_clearance: 0.4,
            offset: -0.75,
            version: 1,
            attachment: TacAttachment::Lower,
        };
        let attributes = p.attributes();
        let back = TacParams::from_attributes(|name| {
            attributes
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, v)| *v)
        });
        assert_eq!(back, p);
        assert_eq!(TacParams::from_attributes(|_| None), TacParams::default());
        let odd = TacParams::from_attributes(|name| match name {
            "tac_joint_version" => Some(1.5),
            "tac_joint_attachment" => Some(7.0),
            _ => None,
        });
        assert_eq!(odd.version, -1);
        assert_eq!(odd.attachment, TacAttachment::Unknown);
    }
}
