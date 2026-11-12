//! The text a project stores for each painted triangle: the tree of how it
//! is split and what each piece is painted with, as hexadecimal digits
//! (Model.cpp `FacetsAnnotation::get_triangle_as_string`,
//! `set_triangle_from_string`; TriangleSelector.cpp `serialize`,
//! `deserialize`, `TriangleSplittingData::read_leaf_state`,
//! `update_used_states`).
//!
//! Each digit is four bits. The digits are read from the last to the
//! first. A node is one digit: its lower two bits are how many sides of
//! the triangle are split (0 for a piece that is not), and for a split of
//! one or two sides the upper two are the side split (one) or kept (two);
//! a split of all three keeps 0 there. A piece's upper two bits are its
//! state when that is 0, 1 or 2; when they are both set, the state is 3
//! more than the digits that follow, in base 15: each digit 15 means 15
//! more and another digit. A split is followed by its children, the last
//! child first (as PrusaSlicer 2.3.1 wrote them).
//!
//! This is the one reader of these codes: the selector loads a mesh's
//! paint through it, and `edge-3mf` reads a project's paint through it.

/// The highest state a piece can have (`EnforcerBlockerType::ExtruderMax`).
pub const MAX_STATE: u8 = 255;
/// The deepest a tree is read: a code deeper than this is not read
/// (TriangleSelector.cpp `kMaxSplitDepth`).
pub const MAX_DEPTH: usize = 256;

/// A node of a code's tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    /// A triangle split on `sides` of its sides (1 to 3); `special` is the
    /// side split (one side) or kept (two sides), 0 for three. Its
    /// `sides + 1` children follow, the last first.
    Split { sides: u8, special: u8 },
    /// A piece that is not split, and its state: 0 is unpainted.
    Leaf(u8),
}

/// The digits of a code as numbers, in the order they are read (the last
/// digit first); none for anything that is not a hexadecimal digit, which
/// the reference does not read at all.
fn digits(code: &str) -> Option<Vec<u8>> {
    code.bytes()
        .rev()
        .map(|digit| char::from(digit).to_digit(16).map(|value| value as u8))
        .collect()
}

/// Reads one tree from `digits` at `at`, adding its nodes to `out`.
fn tree(digits: &[u8], at: &mut usize, out: &mut Vec<Node>) -> Option<()> {
    // How many children each open split still waits for.
    let mut open: Vec<u8> = Vec::new();
    loop {
        let code = *digits.get(*at)?;
        *at += 1;
        let sides = code & 0b11;
        if sides != 0 {
            // A split of one or two sides names a side, 0 to 2.
            if (sides != 3 && code >> 2 == 3) || open.len() >= MAX_DEPTH {
                return None;
            }
            out.push(Node::Split {
                sides,
                special: if sides == 3 { 0 } else { code >> 2 },
            });
            open.push(sides + 1);
            continue;
        }
        let state = if code >> 2 != 0b11 {
            u32::from(code >> 2)
        } else {
            let mut more = 0u32;
            loop {
                if 15 * more + 3 > u32::from(MAX_STATE) {
                    return None;
                }
                let next = *digits.get(*at)?;
                *at += 1;
                if next != 0b1111 {
                    break u32::from(next) + 15 * more + 3;
                }
                more += 1;
            }
        };
        out.push(Node::Leaf(u8::try_from(state).ok()?));
        // The piece closes the splits it was the last child of.
        loop {
            match open.last_mut() {
                None => return Some(()),
                Some(left) => {
                    *left -= 1;
                    if *left > 0 {
                        break;
                    }
                    open.pop();
                }
            }
        }
    }
}

/// The tree a code stores, its nodes in the order stored (a split before
/// its children, the children last first). None for a code the reference
/// drops: not hexadecimal, empty, cut short, a split naming no side, or
/// too deep. What follows the first tree must be whole trees as well, or
/// the reference drops the code (`update_used_states`); only the first is
/// what the triangle is painted with (`deserialize`).
pub fn parse(code: &str) -> Option<Vec<Node>> {
    let digits = digits(code)?;
    if digits.is_empty() {
        return None;
    }
    let mut at = 0;
    let mut first = Vec::new();
    tree(&digits, &mut at, &mut first)?;
    let mut rest = Vec::new();
    while at < digits.len() {
        tree(&digits, &mut at, &mut rest)?;
        rest.clear();
    }
    Some(first)
}

/// Adds a node's digits to `out`, in the order they are read.
pub(crate) fn push_node(node: Node, out: &mut Vec<u8>) {
    match node {
        Node::Split { sides, special } => out.push(sides | (special << 2)),
        Node::Leaf(state) if state < 3 => out.push(state << 2),
        Node::Leaf(state) => {
            out.push(0b1100);
            let mut rest = state - 3;
            while rest >= 15 {
                out.push(0b1111);
                rest -= 15;
            }
            out.push(rest);
        }
    }
}

/// Digits in the order they are read, as the code is written: the last
/// first, in capitals.
pub(crate) fn text(digits: &[u8]) -> String {
    digits
        .iter()
        .rev()
        .map(|digit| char::from(b"0123456789ABCDEF"[usize::from(*digit & 15)]))
        .collect()
}

/// The code of a tree given as nodes in the order stored.
pub fn write(nodes: &[Node]) -> String {
    let mut digits = Vec::with_capacity(nodes.len() + 2);
    for node in nodes {
        push_node(*node, &mut digits);
    }
    text(&digits)
}

/// The triangles a split makes of `corners`, in the reference's order
/// (`perform_split`): new corners are put at the middle of the split
/// sides, each as half of the sum of its two ends.
pub fn split_corners(corners: [[f32; 3]; 3], sides: u8, special: u8) -> Vec<[[f32; 3]; 3]> {
    let middle = |a: [f32; 3], b: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|axis| 0.5 * (a[axis] + b[axis]))
    };
    // The corners, turned to begin at the special side's.
    let special = usize::from(special);
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

/// Each piece a code cuts `corners` into and its state, in the order the
/// code stores them (children last first). None for a code the reference
/// drops.
pub fn pieces(code: &str, corners: [[f32; 3]; 3]) -> Option<Vec<([[f32; 3]; 3], u8)>> {
    let nodes = parse(code)?;
    let mut out = Vec::new();
    let mut at = 0;
    walk(&nodes, &mut at, corners, &mut out);
    Some(out)
}

fn walk(
    nodes: &[Node],
    at: &mut usize,
    corners: [[f32; 3]; 3],
    out: &mut Vec<([[f32; 3]; 3], u8)>,
) {
    let node = nodes[*at];
    *at += 1;
    match node {
        Node::Leaf(state) => out.push((corners, state)),
        Node::Split { sides, special } => {
            for child in split_corners(corners, sides, special).into_iter().rev() {
                walk(nodes, at, child, out);
            }
        }
    }
}
