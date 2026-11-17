//! Each copy on the plate labelled in the file so a printer can cancel it
//! while printing, as the reference labels them (GCode.cpp
//! `set_object_info`, `process_layer`, `instance_name`,
//! `_encode_label_ids_to_base64`; GCodeWriter.cpp
//! `add_object_change_labels`).
//!
//! A copy's moves on a layer are put between a start and a stop: on a
//! Bambu Lab printer `M624` and `M625` with comments naming its number,
//! on Klipper `EXCLUDE_OBJECT_START` and `_END` by name, on Marlin and
//! RepRapFirmware `M486 S<n>` and `M486 S-1`. Klipper, Marlin and
//! RepRapFirmware are told every copy before the print starts. The start
//! and the stop are written with the first travel or extrusion after
//! them, so the retraction before a copy's first travel stays outside it,
//! as there.

use edge_extrusion::ExtrusionLoop;
use edge_geom::{Point2, unscaled};
use edge_template::stream_double;

use crate::copy_order;
use crate::settings::{Labelling, ObjectLabel};

/// GCode.cpp `sanitize_instance_name`: each run of the characters
/// `[ !@#$%^&*()=+[]{};:",']` made one underscore, and one underscore
/// at either end taken off.
pub(crate) fn sanitize(name: &str) -> String {
    const ODD: &str = " !@#$%^&*()=+[]{};:\",'";
    let mut out = String::with_capacity(name.len());
    let mut in_run = false;
    for c in name.chars() {
        if ODD.contains(c) {
            if !in_run {
                out.push('_');
            }
            in_run = true;
        } else {
            out.push(c);
            in_run = false;
        }
    }
    if out.starts_with('_') {
        out.remove(0);
    }
    if out.ends_with('_') {
        out.pop();
    }
    out
}

/// GCode.cpp `get_instance_name`: `<object>_id_<n>_copy_<m>`.
pub(crate) fn instance_name(label: &ObjectLabel) -> String {
    sanitize(&format!(
        "{}_id_{}_copy_{}",
        sanitize(&label.name),
        label.object,
        label.copy
    ))
}

/// The eight bytes of `value`, lowest first, in base 64 with padding
/// (GCode.cpp `encodeBase64`).
fn base64(value: u64) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = value.to_le_bytes();
    let mut out = String::with_capacity(12);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (at, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if at <= chunk.len() {
                out.push(ALPHABET[((n >> shift) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A copy's outline across the bed from the corners of its mesh
/// (`PrintInstance::get_convex_hull_2d`): their convex hull, thinned by
/// Douglas-Peucker to `tolerance` mm round the closed outline, and each
/// corner rounded to 0.1 mm.
pub fn label_outline(corners: impl IntoIterator<Item = Point2>, tolerance: f64) -> Vec<Point2> {
    let hull = edge_geom::convex_hull(corners).points;
    let Some(&first) = hull.first() else {
        return Vec::new();
    };
    let mut closed = hull;
    closed.push(first);
    let mut thinned = crate::arc::douglas_peucker(&closed, edge_geom::scaled(tolerance) as f64);
    thinned.pop();
    let grid = edge_geom::scaled(0.1) as f64;
    let round = |v: i64| ((v as f64 / grid).round() * grid) as i64;
    thinned
        .into_iter()
        .map(|p| Point2 {
            x: round(p.x),
            y: round(p.y),
        })
        .collect()
}

/// The numbers of the plate's copies a Bambu Lab printer knows them by,
/// in order (GCode.cpp `m_label_objects_ids`).
pub(crate) fn sorted_numbers(labels: &[ObjectLabel]) -> Vec<u32> {
    let mut numbers: Vec<u32> = labels.iter().map(|label| label.identify).collect();
    numbers.sort_unstable();
    numbers
}

/// What goes into the file as a copy starts and as it stops.
pub(crate) fn start_and_stop(
    how: Labelling,
    labels: &[ObjectLabel],
    copy: usize,
) -> (String, String) {
    let label = &labels[copy];
    match how {
        Labelling::Bambu => {
            let numbers = sorted_numbers(labels);
            let bit = numbers
                .iter()
                .position(|number| *number == label.identify)
                .unwrap_or(0);
            (
                format!(
                    "; start printing object, unique label id: {}\nM624 {}\n",
                    label.identify,
                    base64(1u64 << bit)
                ),
                format!(
                    "; stop printing object, unique label id: {}\nM625\n",
                    label.identify
                ),
            )
        }
        Labelling::Klipper => {
            let name = instance_name(label);
            (
                format!("EXCLUDE_OBJECT_START NAME={name}\n"),
                format!("EXCLUDE_OBJECT_END NAME={name}\n"),
            )
        }
        Labelling::Marlin | Labelling::RepRap => {
            (format!("M486 S{}\n", label.unique), "M486 S-1\n".to_owned())
        }
    }
}

/// The comments naming a copy where it starts and stops
/// (`gcode_label_objects`).
pub(crate) fn comments(label: &ObjectLabel) -> (String, String) {
    let named = format!("{} id:{} copy {}", label.name, label.object, label.copy);
    (
        format!("; printing object {named}\n"),
        format!("; stop printing object {named}\n"),
    )
}

/// Every copy as the firmware is told of it before the print starts
/// (GCode.cpp `set_object_info`): Klipper's name, middle and outline,
/// Marlin's and RepRapFirmware's number and name. Nothing for a Bambu
/// Lab printer, whose numbers are in the header.
pub(crate) fn definitions(how: Labelling, labels: &[ObjectLabel]) -> String {
    let mut out = String::new();
    for label in labels {
        let name = instance_name(label);
        match how {
            Labelling::Bambu => {}
            Labelling::Klipper => {
                let point = |p: &Point2| {
                    format!(
                        "[{},{}]",
                        stream_double(unscaled(p.x)),
                        stream_double(unscaled(p.y))
                    )
                };
                let mut corners: Vec<String> = label.hull.iter().map(point).collect();
                if let Some(first) = label.hull.first() {
                    corners.push(point(first));
                }
                out.push_str(&format!(
                    "EXCLUDE_OBJECT_DEFINE NAME={name} CENTER={},{} POLYGON=[{}]\n",
                    stream_double(label.centre[0]),
                    stream_double(label.centre[1]),
                    corners.join(",")
                ));
            }
            Labelling::Marlin => {
                out.push_str(&format!("M486 S{}\nM486 A{name}\nM486 S-1\n", label.unique));
            }
            Labelling::RepRap => {
                out.push_str(&format!("M486 S{} A\"{name}\"\nM486 S-1\n", label.unique));
            }
        }
    }
    out
}

/// Whether `point` is inside or on the convex outline `hull`, which runs
/// counter-clockwise.
fn in_hull(hull: &[Point2], point: Point2) -> bool {
    if hull.len() < 3 {
        return false;
    }
    (0..hull.len()).all(|at| {
        let (a, b) = (hull[at], hull[(at + 1) % hull.len()]);
        i128::from(b.x - a.x) * i128::from(point.y - a.y)
            - i128::from(b.y - a.y) * i128::from(point.x - a.x)
            >= 0
    })
}

fn hull_area(hull: &[Point2]) -> i128 {
    (0..hull.len())
        .map(|at| {
            let (a, b) = (hull[at], hull[(at + 1) % hull.len()]);
            i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y)
        })
        .sum::<i128>()
        .abs()
}

/// The middle of a copy's box, in the toolpaths' units.
pub(crate) fn middle(label: &ObjectLabel) -> Point2 {
    Point2::from_mm(label.centre[0], label.centre[1])
}

/// The copy a point printed belongs to: of the copies whose outline holds
/// it, the smallest, so a copy stood inside another's outline is told
/// from it; failing all, the one whose middle is nearest. Each copy's
/// toolpaths lie over its own outline, so this finds whose they are.
pub(crate) fn owner(labels: &[ObjectLabel], point: Point2) -> usize {
    let holding = (labels.iter().enumerate())
        .filter(|(_, label)| in_hull(&label.hull, point))
        .min_by_key(|(at, label)| (hull_area(&label.hull), *at))
        .map(|(at, _)| at);
    holding.unwrap_or_else(|| {
        (labels.iter().enumerate())
            .min_by_key(|(at, label)| {
                let m = middle(label);
                let (dx, dy) = ((m.x - point.x) as i128, (m.y - point.y) as i128);
                (dx * dx + dy * dy, *at)
            })
            .map_or(0, |(at, _)| at)
    })
}

/// The copy a brim loop belongs to: the one copy it goes round; none for
/// a loop round several (a brim they share, which belongs to none of
/// them, GCode.cpp `generate_object_brim`) or for a skirt. A loop round
/// none (a brim inside a hole) is the copy it lies over.
pub(crate) fn brim_owner(labels: &[ObjectLabel], brim: &ExtrusionLoop) -> Option<usize> {
    let ring = &brim.polygon.points;
    let around: Vec<usize> = (labels.iter().enumerate())
        .filter(|(_, label)| crate::inside::within(&brim.polygon, middle(label)))
        .map(|(at, _)| at)
        .collect();
    match around.as_slice() {
        [one] => Some(*one),
        [] => ring.first().map(|first| owner(labels, *first)),
        _ => None,
    }
}

/// The order the copies in `present` are printed in on a layer (GCode.cpp
/// `process_layer`, `sort_print_object_instances`; ShortestPath.cpp
/// `chain_print_object_instances`): the middles of the copies' boxes
/// chained as the reference chains them ([`copy_order`]). Without a
/// prime tower the chain runs through every copy on the plate, from
/// whichever end the reference's chaining leaves first, once for the
/// whole print (`print_object_instances_ordering`).
///
/// On a layer with a prime tower (`tower`) it runs through every copy of
/// each object printing on the layer and is taken backwards. The
/// reference means to start it from the tower, but makes the point from
/// `wipe_tower_x` and `_y` in millimetres taken as its own units
/// (nanometres; GCode.cpp `process_layer`, `Point wt_pos(...)`), where
/// the copies stand in the project's coordinates with the plate's place
/// among the others added (`get_plate_origin`): it starts from the
/// corner of the first plate, and so ends there. That is followed here,
/// though it is plainly not what was meant, so that the copies come in
/// the reference's order; nothing is printed differently for it but the
/// travel between copies. `origin` is the plate's place in the project,
/// in mm.
pub(crate) fn order(
    labels: &[ObjectLabel],
    present: &[bool],
    tower: Option<Point2>,
    origin: [f64; 2],
) -> Vec<usize> {
    // The tower's chain runs through every copy of each object printing
    // on the layer; the plate's through every copy.
    let chained: Vec<usize> = match tower {
        Some(_) => (0..labels.len())
            .filter(|at| {
                (0..labels.len())
                    .any(|other| present[other] && labels[other].object == labels[*at].object)
            })
            .collect(),
        None => (0..labels.len()).collect(),
    };
    let points: Vec<[f64; 2]> = chained
        .iter()
        .map(|at| {
            let m = middle(&labels[*at]);
            [m.x as f64, m.y as f64]
        })
        .collect();
    let start = tower.map(|at| {
        let corner = Point2::from_mm(-origin[0], -origin[1]);
        [
            (corner.x as f64) + unscaled(at.x).round(),
            (corner.y as f64) + unscaled(at.y).round(),
        ]
    });
    let mut ordered: Vec<usize> = copy_order::order(&points, start)
        .into_iter()
        .map(|at| chained[at])
        .collect();
    if tower.is_some() {
        ordered.reverse();
    }
    ordered.retain(|at| present[*at]);
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(name: &str, object: usize, identify: u32, centre: [f64; 2]) -> ObjectLabel {
        let [x, y] = centre;
        ObjectLabel {
            name: name.to_owned(),
            object,
            copy: 0,
            unique: object,
            identify,
            centre,
            hull: vec![
                Point2::from_mm(x - 5.0, y - 5.0),
                Point2::from_mm(x + 5.0, y - 5.0),
                Point2::from_mm(x + 5.0, y + 5.0),
                Point2::from_mm(x - 5.0, y + 5.0),
            ],
        }
    }

    #[test]
    fn names_and_numbers_are_written_as_the_reference_writes_them() {
        assert_eq!(sanitize("MC Arms"), "MC_Arms");
        assert_eq!(sanitize(" (a) [b]; c"), "a_b_c");
        assert_eq!(
            instance_name(&label("MC Arms", 2, 9, [0.0; 2])),
            "MC_Arms_id_2_copy_0"
        );
        // The reference's file for one copy, numbered 8.
        assert_eq!(base64(1), "AQAAAAAAAAA=");
        assert_eq!(base64(1 << 9), "AAIAAAAAAAA=");
        let labels = [
            label("b", 1, 12, [10.0, 10.0]),
            label("a", 0, 8, [50.0, 10.0]),
        ];
        let (start, stop) = start_and_stop(Labelling::Bambu, &labels, 0);
        assert_eq!(
            start,
            "; start printing object, unique label id: 12\nM624 AgAAAAAAAAA=\n"
        );
        assert_eq!(stop, "; stop printing object, unique label id: 12\nM625\n");
        let (start, _) = start_and_stop(Labelling::Marlin, &labels, 1);
        assert_eq!(start, "M486 S0\n");
        assert_eq!(
            definitions(Labelling::RepRap, &labels[..1]),
            "M486 S1 A\"b_id_1_copy_0\"\nM486 S-1\n"
        );
        assert_eq!(
            definitions(Labelling::Klipper, &labels[..1]),
            "EXCLUDE_OBJECT_DEFINE NAME=b_id_1_copy_0 CENTER=10,10 POLYGON=[[5,5],[15,5],[15,15],[5,15],[5,5]]\n"
        );
    }

    #[test]
    fn each_point_and_brim_finds_its_copy_and_the_copies_their_order() {
        let labels = [
            label("a", 0, 1, [10.0, 10.0]),
            label("b", 1, 2, [40.0, 10.0]),
            label("c", 2, 3, [80.0, 10.0]),
        ];
        assert_eq!(owner(&labels, Point2::from_mm(41.0, 12.0)), 1);
        // Outside every outline: the nearest middle.
        assert_eq!(owner(&labels, Point2::from_mm(70.0, 30.0)), 2);
        // With a tower the chain starts at the first plate's corner,
        // wherever the tower is, and is taken backwards: the copy nearest
        // that corner last (GCode.cpp `process_layer`, `wt_pos`).
        let all = [true; 3];
        assert_eq!(
            order(&labels, &all, Some(Point2::from_mm(100.0, 10.0)), [0.0; 2]),
            [2, 1, 0]
        );
        // On a plate to the right of the first, the corner is to its left.
        assert_eq!(
            order(
                &labels,
                &all,
                Some(Point2::from_mm(5.0, 10.0)),
                [300.0, 0.0]
            ),
            [2, 1, 0]
        );
        // On a plate to the left (none is, but the corner decides): the
        // other way.
        assert_eq!(
            order(
                &labels,
                &all,
                Some(Point2::from_mm(5.0, 10.0)),
                [-300.0, 0.0]
            ),
            [0, 1, 2]
        );
        let free = order(&labels, &[true, false, true], None, [0.0; 2]);
        assert!(free == [0, 2] || free == [2, 0], "{free:?}");
    }
}
