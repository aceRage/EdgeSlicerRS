//! Colours painted onto a mesh's triangles, as Bambu and Orca projects
//! store them: a `paint_color` attribute on a triangle that says which
//! filament it is painted for, or how it is cut into smaller triangles
//! that each say the same of themselves.
//!
//! Seams painted on or off (`paint_seam`) are stored the same way, a
//! triangle's state then saying whether a seam is wanted there or kept
//! off it (TriangleSelector.hpp `EnforcerBlockerType`).
//!
//! A port of the reference's `FacetsAnnotation::set_triangle_from_string`
//! (Model.cpp) and of `TriangleSelector::deserialize` and `perform_split`
//! (TriangleSelector.cpp).

use edge_mesh::Triangle;

/// A triangle of a painted mesh and the filament it is painted for,
/// counting from 1; 0 where it is left unpainted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Facet {
    pub corners: Triangle,
    pub filament: u8,
}

/// A triangle of a mesh painted to have its seams (an enforcer) or to
/// keep them off (a blocker).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeamFacet {
    pub corners: Triangle,
    pub kind: SeamPaint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeamPaint {
    /// A seam is wanted here (`EnforcerBlockerType::ENFORCER`, 1).
    Enforcer,
    /// A seam is kept off here (`BLOCKER`, 2).
    Blocker,
}

/// Adds to `out` the painted pieces of a triangle whose `paint_seam` is
/// `code`; a code that cannot be read adds nothing, as in the reference.
pub(crate) fn decode_seam(code: &str, corners: Triangle, out: &mut Vec<SeamFacet>) {
    let mut pieces = Vec::new();
    if decode(code, corners, &mut pieces) {
        out.extend(pieces.into_iter().filter_map(|piece| {
            let kind = match piece.filament {
                1 => SeamPaint::Enforcer,
                2 => SeamPaint::Blocker,
                _ => return None,
            };
            Some(SeamFacet {
                corners: piece.corners,
                kind,
            })
        }));
    }
}

/// Adds to `out` the triangles `code` makes of `corners`, each with its
/// filament. A code that cannot be read adds nothing and gives `false`:
/// the triangle is then unpainted, as in the reference. The code is read
/// by the selector's reader (`edge_mesh::selector::code`), the one this
/// workspace has.
pub(crate) fn decode(code: &str, corners: Triangle, out: &mut Vec<Facet>) -> bool {
    match edge_mesh::selector::code::pieces(code, corners) {
        Some(pieces) => {
            out.extend(
                pieces
                    .into_iter()
                    .map(|(corners, filament)| Facet { corners, filament }),
            );
            true
        }
        None => false,
    }
}

/// The code of a triangle with each piece's filament changed by `map`
/// (0 is unpainted), the splits as they were: what the reference writes
/// after renumbering a volume's painted states when a filament is deleted
/// (Model.cpp `remap_extruder_ids`, TriangleSelector.cpp `serialize`). None
/// for a code that cannot be read, which is left as it is, and for one
/// that comes to a single unpainted triangle, which the reference does not
/// write.
pub(crate) fn remap(code: &str, map: &dyn Fn(u8) -> u8) -> Option<Option<String>> {
    use edge_mesh::selector::code::{self as codes, Node};
    let nodes: Vec<Node> = codes::parse(code)?
        .into_iter()
        .map(|node| match node {
            Node::Leaf(state) => Node::Leaf(map(state)),
            split => split,
        })
        .collect();
    // Nothing painted is left: the triangle is not written. The reference
    // writes back a split whose pieces are all unpainted; it reads the same.
    if !nodes
        .iter()
        .any(|node| matches!(node, Node::Leaf(state) if *state != 0))
    {
        return Some(None);
    }
    Some(Some(codes::write(&nodes)))
}

/// What a triangle can be painted with, in the order the reference writes
/// them on a triangle (bbs_3mf.cpp `_add_mesh_to_object_stream`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaintKind {
    /// Supports painted on or off (`supported_facets`).
    Supports,
    /// Seams painted on or off (`seam_facets`).
    Seam,
    /// Filaments (`mmu_segmentation_facets`).
    Color,
    /// Fuzzy skin painted on (`fuzzy_skin_facets`).
    FuzzySkin,
}

impl PaintKind {
    pub const ALL: [PaintKind; 4] = [
        PaintKind::Supports,
        PaintKind::Seam,
        PaintKind::Color,
        PaintKind::FuzzySkin,
    ];

    /// The attribute a project stores it under.
    pub fn attribute(self) -> &'static str {
        match self {
            PaintKind::Supports => "paint_supports",
            PaintKind::Seam => "paint_seam",
            PaintKind::Color => "paint_color",
            PaintKind::FuzzySkin => "paint_fuzzy_skin",
        }
    }

    /// Whether an attribute of a triangle is this kind of paint: fuzzy
    /// skin was once written `paint_fuzzy`, which the reference still reads.
    fn names(self, key: &str) -> bool {
        key == self.attribute() || (self == PaintKind::FuzzySkin && key == "paint_fuzzy")
    }

    fn of(key: &str) -> Option<PaintKind> {
        PaintKind::ALL.into_iter().find(|kind| kind.names(key))
    }
}

/// A triangle's attributes as written (` key="value"`), each as its key
/// and its value as written.
fn attributes_of(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq].trim();
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else { break };
        out.push((key, &after[..end]));
        rest = &after[end + 1..];
    }
    out
}

impl crate::Part {
    /// The code of `kind` on each triangle of the part's mesh that has one,
    /// by the triangle's place in the mesh, in order. Only what was read or
    /// painted on the mesh the part has now.
    pub fn paint_codes(&self, kind: PaintKind) -> Vec<(u32, String)> {
        let Some(extras) = crate::write::extras_of(self) else {
            return Vec::new();
        };
        extras
            .iter()
            .filter_map(|(triangle, text)| {
                attributes_of(text)
                    .into_iter()
                    .find(|(key, value)| kind.names(key) && !value.is_empty())
                    .map(|(_, value)| (*triangle, value.to_owned()))
            })
            .collect()
    }

    /// Whether any triangle of the part carries `kind`.
    pub fn has_paint(&self, kind: PaintKind) -> bool {
        crate::write::extras_of(self).is_some_and(|extras| {
            extras.iter().any(|(_, text)| {
                attributes_of(text)
                    .iter()
                    .any(|(key, value)| kind.names(key) && !value.is_empty())
            })
        })
    }

    /// The part with `kind` painted as `codes` say (each triangle's code by
    /// its place in the mesh, in order; a triangle not listed is not
    /// painted with it), what else its triangles carry kept. What the
    /// slice reads of colours and seams is worked out anew.
    pub fn with_paint(&self, kind: PaintKind, codes: &[(u32, String)]) -> crate::Part {
        let old: &[(u32, Box<str>)] =
            crate::write::extras_of(self).map_or(&[], |extras| extras.as_slice());
        let mut extras: crate::Extras = Vec::with_capacity(old.len().max(codes.len()));
        let (mut a, mut b) = (old.iter().peekable(), codes.iter().peekable());
        loop {
            let triangle = match (a.peek(), b.peek()) {
                (None, None) => break,
                (Some((x, _)), None) => *x,
                (None, Some((y, _))) => *y,
                (Some((x, _)), Some((y, _))) => (*x).min(*y),
            };
            let was = a
                .next_if(|(at, _)| *at == triangle)
                .map(|(_, text)| &**text);
            let code = b
                .next_if(|(at, _)| *at == triangle)
                .map(|(_, code)| code.as_str());
            let mut attributes: Vec<(&str, &str)> = was.map(attributes_of).unwrap_or_default();
            attributes.retain(|(key, _)| !kind.names(key));
            if let Some(code) = code.filter(|code| !code.is_empty()) {
                // In the reference's order: after the kinds written before
                // it, before the later kinds and anything else.
                let at = attributes
                    .iter()
                    .position(|(key, _)| {
                        PaintKind::of(key).is_none_or(|other| other as u8 > kind as u8)
                    })
                    .unwrap_or(attributes.len());
                attributes.insert(at, (kind.attribute(), code));
            }
            if attributes.is_empty() {
                continue;
            }
            let mut text = String::new();
            for (key, value) in attributes {
                text.push(' ');
                text.push_str(key);
                text.push_str("=\"");
                text.push_str(value);
                text.push('"');
            }
            extras.push((triangle, text.into_boxed_str()));
        }
        let mut part = self.clone();
        let own = part.kept.get_or_insert_with(Default::default);
        let own = std::sync::Arc::make_mut(own);
        own.extras = (!extras.is_empty()).then(|| std::sync::Arc::new(extras));
        own.painted_on = Some(crate::kept::MeshOf(self.mesh.clone()));
        match kind {
            PaintKind::Color => {
                part.paint = (!codes.is_empty()).then(|| {
                    let mut facets = Vec::with_capacity(self.mesh.triangles.len());
                    let mut listed = codes.iter().peekable();
                    for at in 0..self.mesh.triangles.len() {
                        let corners = self.mesh.triangle(at);
                        let read = listed
                            .next_if(|(triangle, _)| *triangle as usize == at)
                            .is_some_and(|(_, code)| decode(code, corners, &mut facets));
                        if !read {
                            facets.push(Facet {
                                corners,
                                filament: 0,
                            });
                        }
                    }
                    std::sync::Arc::new(facets)
                });
                if part
                    .paint
                    .as_ref()
                    .is_some_and(|facets| facets.iter().all(|facet| facet.filament == 0))
                {
                    part.paint = None;
                }
            }
            PaintKind::Seam => {
                let mut facets = Vec::new();
                for (triangle, code) in codes {
                    if (*triangle as usize) < self.mesh.triangles.len() {
                        decode_seam(code, self.mesh.triangle(*triangle as usize), &mut facets);
                    }
                }
                part.seam = (!facets.is_empty()).then(|| std::sync::Arc::new(facets));
            }
            PaintKind::Supports | PaintKind::FuzzySkin => {}
        }
        part
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHOLE: Triangle = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]];

    fn painted(code: &str) -> Option<Vec<(u8, Triangle)>> {
        let mut out = Vec::new();
        decode(code, WHOLE, &mut out).then(|| {
            out.into_iter()
                .map(|facet| (facet.filament, facet.corners))
                .collect()
        })
    }

    fn area(corners: &Triangle) -> f32 {
        let [a, b, c] = corners;
        0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]))
    }

    #[test]
    fn renumbered_paint_reads_back_renumbered_with_its_splits() {
        let map = |filament: u8| match filament {
            2 => 0,
            3.. => filament - 1,
            other => other,
        };
        for code in ["4", "8", "0C", "1C", "4C", "EC", "0C8483", "8C3", "4848C3"] {
            let Some(was) = painted(code) else { continue };
            let now = match remap(code, &map) {
                Some(Some(now)) => painted(&now).unwrap(),
                Some(None) => vec![(0, WHOLE)],
                None => panic!("{code} not rewritten"),
            };
            let expected: Vec<(u8, Triangle)> = was
                .into_iter()
                .map(|(filament, corners)| (map(filament), corners))
                .collect();
            assert_eq!(now, expected, "{code}");
        }
        // Filament 2 alone, taken away: nothing is painted.
        assert_eq!(remap("8", &map), Some(None));
        // 17 to 16, and 4 to 3.
        assert_eq!(remap("EC", &map).unwrap().unwrap(), "DC");
        assert_eq!(remap("1C", &map).unwrap().unwrap(), "0C");
    }

    #[test]
    fn a_whole_triangle_says_its_filament() {
        // Filaments 1 and 2 fit in the digit; from 3 on another follows,
        // and it is written first because the code is read backwards.
        for (code, filament) in [
            ("4", 1),
            ("8", 2),
            ("0C", 3),
            ("1C", 4),
            ("4C", 7),
            ("EC", 17),
        ] {
            assert_eq!(painted(code), Some(vec![(filament, WHOLE)]), "{code}");
        }
        // A digit of 15 says fifteen more and that another digit follows.
        assert_eq!(painted("0FC"), Some(vec![(18, WHOLE)]));
        assert_eq!(painted("0"), Some(vec![(0, WHOLE)]));
    }

    #[test]
    fn a_split_triangle_is_read_last_child_first() {
        // Split on all three sides (3), children last first: the middle
        // one for filament 2, then 1, unpainted, and 1.
        let found = painted("40483").unwrap();
        let filaments: Vec<u8> = found.iter().map(|(filament, _)| *filament).collect();
        assert_eq!(filaments, [2, 1, 0, 1]);
        // In the reference's order the middle triangle is the last child,
        // so it is the first read.
        assert_eq!(
            found[0].1,
            [[2.0, 0.0, 0.0], [2.0, 2.0, 0.0], [0.0, 2.0, 0.0]]
        );
        // The four make up the whole, each facing the way it did.
        assert!(found.iter().all(|(_, corners)| area(corners) > 0.0));
        let total: f32 = found.iter().map(|(_, corners)| area(corners)).sum();
        assert!((total - area(&WHOLE)).abs() < 1e-6);

        // One side split, the one after corner 1: two triangles.
        let halves = painted("845").unwrap();
        assert_eq!(halves.len(), 2);
        let total: f32 = halves.iter().map(|(_, corners)| area(corners)).sum();
        assert!((total - area(&WHOLE)).abs() < 1e-6);
        assert!(halves.iter().all(|(_, corners)| area(corners) > 0.0));

        // Two sides split: three triangles, and a child may be split again.
        let nested = painted("44483486").unwrap();
        let total: f32 = nested.iter().map(|(_, corners)| area(corners)).sum();
        assert!((total - area(&WHOLE)).abs() < 1e-6);
        assert_eq!(nested.len(), 6);
    }

    #[test]
    fn a_code_that_cannot_be_read_paints_nothing() {
        for code in ["", "x", "3", "43", "C", "F"] {
            assert_eq!(painted(code), None, "{code:?}");
        }
    }
}
