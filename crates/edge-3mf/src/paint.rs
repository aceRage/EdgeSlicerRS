//! Colours painted onto a mesh's triangles, as Bambu and Orca projects
//! store them: a `paint_color` attribute on a triangle that says which
//! filament it is painted for, or how it is cut into smaller triangles
//! that each say the same of themselves.
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
