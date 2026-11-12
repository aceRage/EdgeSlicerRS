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

/// The reference reads no deeper, so a corrupt code cannot run away.
const MAX_DEPTH: usize = 256;
/// The highest filament a triangle can be painted for (`ExtruderMax`).
const MAX_FILAMENT: u32 = 255;

/// The code's hex digits as numbers, last digit first, which is the
/// order they are read in. `None` for anything that is not a hex digit.
fn nibbles(code: &str) -> Option<Vec<u32>> {
    code.chars().rev().map(|digit| digit.to_digit(16)).collect()
}

/// Adds to `out` the triangles `code` makes of `corners`, each with its
/// filament. A code that cannot be read adds nothing and gives `false`:
/// the triangle is then unpainted, as in the reference.
pub(crate) fn decode(code: &str, corners: Triangle, out: &mut Vec<Facet>) -> bool {
    let Some(nibbles) = nibbles(code) else {
        return false;
    };
    let before = out.len();
    let mut at = 0;
    if read(&nibbles, &mut at, corners, 0, out).is_none() {
        out.truncate(before);
        return false;
    }
    true
}

/// One triangle of the tree: a leaf with its filament, or a split with
/// its children, which are stored last child first.
fn read(
    nibbles: &[u32],
    at: &mut usize,
    corners: Triangle,
    depth: usize,
    out: &mut Vec<Facet>,
) -> Option<()> {
    let code = *nibbles.get(*at)?;
    *at += 1;
    let sides = code & 0b11;
    if sides == 0 {
        // A leaf: its filament in the upper two bits, or, when those are
        // both set, three more than what follows in base 15, a digit of
        // 15 meaning that another follows.
        let filament = if code >> 2 != 0b11 {
            code >> 2
        } else {
            let mut more = 0;
            loop {
                if 15 * more + 3 > MAX_FILAMENT {
                    return None;
                }
                let next = *nibbles.get(*at)?;
                *at += 1;
                if next != 0b1111 {
                    break next + 15 * more + 3;
                }
                more += 1;
            }
        };
        if filament > MAX_FILAMENT {
            return None;
        }
        out.push(Facet {
            corners,
            filament: filament as u8,
        });
        return Some(());
    }
    // A split keeps the side it is about in its upper two bits, where 3
    // is no side; with all three sides split there is none to keep.
    let special = if sides == 3 { 0 } else { (code >> 2) as usize };
    if special == 3 || depth >= MAX_DEPTH {
        return None;
    }
    let children = split(corners, sides, special);
    for child in children.into_iter().rev() {
        read(nibbles, at, child, depth + 1, out)?;
    }
    Some(())
}

/// The code of a triangle with each piece's filament changed by `map`
/// (0 is unpainted), the splits as they were: what the reference writes
/// after renumbering a volume's painted states when a filament is deleted
/// (Model.cpp `remap_extruder_ids`, TriangleSelector.cpp `serialize`). None
/// for a code that cannot be read, which is left as it is, and for one
/// that comes to a single unpainted triangle, which the reference does not
/// write.
pub(crate) fn remap(code: &str, map: &dyn Fn(u8) -> u8) -> Option<Option<String>> {
    let nibbles = nibbles(code)?;
    let mut at = 0;
    let mut out = Vec::with_capacity(nibbles.len());
    let mut any = false;
    rewrite(&nibbles, &mut at, 0, map, &mut out, &mut any)?;
    // Nothing painted is left: the triangle is not written. The reference
    // writes back a split whose pieces are all unpainted; it reads the same.
    if !any {
        return Some(None);
    }
    Some(Some(
        out.iter()
            .rev()
            .map(|nibble| {
                char::from_digit(*nibble, 16)
                    .unwrap_or('0')
                    .to_ascii_uppercase()
            })
            .collect(),
    ))
}

fn rewrite(
    nibbles: &[u32],
    at: &mut usize,
    depth: usize,
    map: &dyn Fn(u8) -> u8,
    out: &mut Vec<u32>,
    any: &mut bool,
) -> Option<()> {
    let code = *nibbles.get(*at)?;
    *at += 1;
    let sides = code & 0b11;
    if sides == 0 {
        let filament = if code >> 2 != 0b11 {
            code >> 2
        } else {
            let mut more = 0;
            loop {
                if 15 * more + 3 > MAX_FILAMENT {
                    return None;
                }
                let next = *nibbles.get(*at)?;
                *at += 1;
                if next != 0b1111 {
                    break next + 15 * more + 3;
                }
                more += 1;
            }
        };
        let state = u32::from(map(u8::try_from(filament).ok()?));
        *any |= state != 0;
        if state < 3 {
            out.push(state << 2);
        } else {
            out.push(0b1100);
            let mut rest = state - 3;
            while rest >= 15 {
                out.push(0b1111);
                rest -= 15;
            }
            out.push(rest);
        }
        return Some(());
    }
    if depth >= MAX_DEPTH {
        return None;
    }
    out.push(code);
    for _ in 0..=sides {
        rewrite(nibbles, at, depth + 1, map, out, any)?;
    }
    Some(())
}

/// The triangles a split makes, in the reference's order: new corners
/// are put at the middle of the split sides.
fn split(corners: Triangle, sides: u32, special: usize) -> Vec<Triangle> {
    let middle = |a: [f32; 3], b: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|axis| 0.5 * (a[axis] + b[axis]))
    };
    // The corners, turned to begin at the special side's.
    let [a, b, c]: [[f32; 3]; 3] = std::array::from_fn(|index| corners[(special + index) % 3]);
    match sides {
        1 => {
            let bc = middle(c, b);
            vec![[a, b, bc], [bc, c, a]]
        }
        2 => {
            let ab = middle(b, a);
            let ca = middle(a, c);
            vec![[a, ab, ca], [ab, b, ca], [b, c, ca]]
        }
        _ => {
            let ab = middle(b, a);
            let bc = middle(c, b);
            let ca = middle(a, c);
            vec![[a, ab, ca], [ab, b, bc], [bc, c, ca], [ab, bc, ca]]
        }
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
