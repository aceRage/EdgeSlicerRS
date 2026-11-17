//! Printing object by object (`print_sequence` "by object"): whether the
//! copies on the plate can be printed one after another without the
//! print head striking one already printed, and the order that allows it
//! (Print.cpp `sequential_print_clearance_valid`), and what else the mode
//! refuses (Print.cpp `validate`).
//!
//! Each copy's outline across the bed, grown by half the head's radius
//! (`extruder_clearance_radius`; a Bambu Lab printer's
//! `extruder_clearance_max_radius`), may not meet another's grown alike,
//! nor its own outline an excluded area of the bed. Each copy printed
//! before another may be no taller than the head reaches below the lid
//! (`extruder_clearance_height_to_lid`), and no taller than its rods
//! (`extruder_clearance_height_to_rod`) where a later copy stands level
//! with what reaches above the rods across the bed; the last is held to
//! the printer's height. The copies are put in the order the project lists
//! them, but a copy that reaches above the rods goes after every copy
//! level with that part of it, and one taller than the lid goes last.

use edge_clip::Join;
use edge_config::Settings;
use edge_geom::{ExPolygon, Point2, Polygon, convex_hull, scaled};
use edge_mesh::Mesh;

use crate::Failure;

/// Print.hpp `MAX_OUTER_NOZZLE_DIAMETER`: how far apart copies all lower
/// than the nozzle are kept, in mm.
const MAX_OUTER_NOZZLE_DIAMETER: f64 = 4.0;

/// One copy on the plate, as the check takes it.
pub(crate) struct Copy<'a> {
    /// Its object's name, for what is said of it.
    pub name: &'a str,
    /// Its printed parts where it stands, in mm.
    pub mesh: &'a Mesh,
}

/// Whether the plate prints object by object.
pub(crate) fn by_object(settings: &Settings) -> bool {
    settings.text("print_sequence") == Some("by object")
}

/// The head's radius (PrintConfig.cpp `sequential_clearance_radius`): a
/// Bambu Lab printer's `extruder_clearance_max_radius` where it has one,
/// else `extruder_clearance_radius`.
fn clearance_radius(settings: &Settings) -> Result<f64, Failure> {
    let max = settings
        .float("extruder_clearance_max_radius")?
        .unwrap_or(0.0);
    let bambu = settings
        .text("printer_model")
        .is_some_and(|model| model.starts_with("Bambu Lab"));
    if bambu && max > 0.0 {
        return Ok(max);
    }
    Ok(settings.float("extruder_clearance_radius")?.unwrap_or(0.0))
}

/// The bed's excluded areas (PrintConfig.cpp `bed_exclude_area_polygons`):
/// each four points a box where they all make boxes, else one outline.
fn excluded(settings: &Settings) -> Result<Vec<ExPolygon>, Failure> {
    let points: Vec<(f64, f64)> = settings
        .texts("bed_exclude_area")
        .unwrap_or_default()
        .iter()
        .flat_map(|text| text.split(','))
        .filter_map(|point| {
            let (x, y) = point.trim().split_once('x')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect();
    if points.len() < 3 {
        return Ok(Vec::new());
    }
    let rectangle = |group: &[(f64, f64)]| {
        // PrintConfig.cpp `exclude_group_is_rectangle`: four corners,
        // each edge along an axis.
        (0..4).all(|i| {
            let (a, b) = (group[i], group[(i + 1) % 4]);
            a.0 == b.0 || a.1 == b.1
        })
    };
    let ring = |corners: &[(f64, f64)]| Polygon {
        points: corners
            .iter()
            .map(|(x, y)| Point2::from_mm(*x, *y))
            .collect(),
    };
    if points.len() >= 4
        && points
            .as_chunks::<4>()
            .0
            .iter()
            .all(|group| rectangle(group))
    {
        let mut out = Vec::new();
        for group in points.as_chunks::<4>().0 {
            let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
            for (x, y) in group {
                lo = (lo.0.min(*x), lo.1.min(*y));
                hi = (hi.0.max(*x), hi.1.max(*y));
            }
            if hi.0 - lo.0 < 1e-6 || hi.1 - lo.1 < 1e-6 {
                continue;
            }
            out.push(ExPolygon {
                contour: ring(&[lo, (hi.0, lo.1), hi, (lo.0, hi.1)]),
                holes: Vec::new(),
            });
        }
        return Ok(out);
    }
    let mut outline = ring(&points);
    if outline.signed_area_mm2() < 0.0 {
        outline.points.reverse();
    }
    Ok(edge_clip::union_ex(&[ExPolygon {
        contour: outline,
        holes: Vec::new(),
    }])?)
}

/// What the check knows of one copy (`print_instance_info`).
struct Info {
    /// Its outline grown by the clearance, and its box across the bed.
    grown: Vec<ExPolygon>,
    min_y: i64,
    max_y: i64,
    height: f64,
    /// Across the bed, where it reaches above the rods (`above_rod_y_range`).
    above_rod: Option<(i64, i64)>,
}

/// The order the copies are printed in, by their place in `copies`
/// (which is the project's order, object by object and each one's copies
/// in theirs), or the reference's words for why they cannot be printed
/// one after another (Print.cpp `sequential_print_clearance_valid`).
pub(crate) fn order(copies: &[Copy<'_>], settings: &Settings) -> Result<Vec<usize>, Failure> {
    let rod = settings
        .float("extruder_clearance_height_to_rod")?
        .unwrap_or(40.0);
    let lid = settings
        .float("extruder_clearance_height_to_lid")?
        .unwrap_or(120.0);
    let printable_height = settings.float("printable_height")?.unwrap_or(100.0);
    let nozzle_height = settings.float("nozzle_height")?.unwrap_or(2.5);
    let heights: Vec<f64> = copies
        .iter()
        .map(|copy| {
            copy.mesh
                .bounds()
                .map_or(0.0, |(lo, hi)| f64::from(hi[2]) - f64::from(lo[2]))
        })
        .collect();
    // `Print::is_all_objects_are_short`; the skirt round each copy is not
    // made here, so it adds nothing (`object_skirt_offset`).
    let short = heights.iter().all(|height| *height < nozzle_height);
    let distance = if short {
        0.5 * MAX_OUTER_NOZZLE_DIAMETER - 0.1
    } else {
        0.5 * clearance_radius(settings)? - 0.1
    };
    let excluded = excluded(settings)?;
    let mut said: Vec<String> = Vec::new();
    let mut infos: Vec<Info> = Vec::with_capacity(copies.len());
    for (at, copy) in copies.iter().enumerate() {
        let hull = convex_hull(
            copy.mesh
                .vertices
                .iter()
                .map(|v| Point2::from_mm(f64::from(v[0]), f64::from(v[1]))),
        );
        let outline = vec![ExPolygon {
            contour: hull,
            holes: Vec::new(),
        }];
        let grown = edge_clip::offset(
            &outline,
            scaled(distance),
            Join::Round {
                tolerance: scaled(0.1),
            },
        )?;
        if !excluded.is_empty() && !edge_clip::intersection(&excluded, &outline)?.is_empty() {
            said.push(format!(
                "{} is too close to exclusion area, there may be collisions when printing.",
                copy.name
            ));
        }
        for other in &infos {
            if !edge_clip::intersection(&other.grown, &grown)?.is_empty() {
                said.push(format!(
                    "{} is too close to others, and collisions may be caused.",
                    copy.name
                ));
                break;
            }
        }
        let (min_y, max_y) = grown
            .iter()
            .flat_map(|region| &region.contour.points)
            .fold((i64::MAX, i64::MIN), |(lo, hi), p| {
                (lo.min(p.y), hi.max(p.y))
            });
        // Any triangle with a corner above the rods counts whole.
        let mut above: Option<(f64, f64)> = None;
        for triangle in &copy.mesh.triangles {
            let corners = triangle.map(|v| copy.mesh.vertices[v as usize]);
            if corners.iter().all(|c| f64::from(c[2]) <= rod) {
                continue;
            }
            for corner in corners {
                let y = f64::from(corner[1]);
                above = Some(above.map_or((y, y), |(lo, hi)| (lo.min(y), hi.max(y))));
            }
        }
        infos.push(Info {
            grown,
            min_y,
            max_y,
            height: heights[at],
            above_rod: above.map(|(lo, hi)| (scaled(lo), scaled(hi))),
        });
    }

    let n = copies.len();
    let tolerance = scaled(0.1);
    // Whether what of `tall` reaches above the rods is level across the
    // bed with where the head goes printing `other`.
    let overlaps = |tall: usize, other: usize| {
        let Some((lo, hi)) = infos[tall].above_rod else {
            return false;
        };
        let low = (lo + tolerance).max(infos[other].min_y);
        let high = (hi - tolerance).min(infos[other].max_y);
        high - low > 0
    };
    // At most one copy may need the last place, which alone is held to
    // the printer's height; the rest to the lid.
    let last: Vec<usize> = (0..n).filter(|at| infos[*at].height > lid).collect();
    let mut solved: Option<Vec<usize>> = None;
    if last.len() <= 1 {
        let last = last.first().copied();
        // An edge b -> a: b is printed before a.
        let mut after: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut waiting = vec![0usize; n];
        let mut infeasible = false;
        'tall: for a in 0..n {
            if Some(a) == last || infos[a].above_rod.is_none() {
                continue;
            }
            for b in (0..n).filter(|b| *b != a) {
                if !overlaps(a, b) {
                    continue;
                }
                if Some(b) == last {
                    infeasible = true;
                    break 'tall;
                }
                after[b].push(a);
                waiting[a] += 1;
            }
        }
        if !infeasible {
            // Kahn's sort, ties broken by place in the list.
            let mut ready: Vec<usize> = (0..n)
                .filter(|at| Some(*at) != last && waiting[*at] == 0)
                .collect();
            let mut order = Vec::with_capacity(n);
            while !ready.is_empty() {
                let next = ready.remove(0);
                order.push(next);
                for &then in &after[next] {
                    waiting[then] -= 1;
                    if waiting[then] == 0 {
                        let at = ready.partition_point(|x| *x < then);
                        ready.insert(at, then);
                    }
                }
            }
            if let Some(last) = last {
                order.push(last);
            }
            if order.len() == n {
                solved = Some(order);
            }
        }
    }
    let order = solved.unwrap_or_else(|| (0..n).collect());
    // Each copy no taller than allowed where it comes.
    for (k, &at) in order.iter().enumerate() {
        let mut allowed = if k + 1 == n { printable_height } else { lid };
        if let Some((lo, hi)) = infos[at].above_rod {
            let (low, high) = (lo + tolerance, hi - tolerance);
            if order[k + 1..]
                .iter()
                .any(|&later| low.max(infos[later].min_y) < high.min(infos[later].max_y))
            {
                allowed = rod;
            }
        }
        if allowed < infos[at].height {
            said.push(format!(
                "{} is too tall, and collisions will be caused.",
                copies[at].name
            ));
        }
    }
    if said.is_empty() {
        Ok(order)
    } else {
        Err(said.join("\n").into())
    }
}

/// Turns the prime tower off for a plate whose several copies are printed
/// object by object, and says whether it was on: the reference turns the
/// setting itself off (PrintConfig.cpp `normalize_fdm_2`, from
/// PrintApply.cpp). Not as the pinned reference in one thing: it counts
/// objects, so copies of one object keep the tower and are then printed
/// layer by layer (EdgeSlicer #1075); the caller counts copies, as the
/// reference's plater does. Smooth timelapse, which keeps the tower
/// there, is refused before this ([`validate`]).
pub(crate) fn tower_off(settings: &mut Settings) -> Result<bool, Failure> {
    let on = settings.flag("enable_prime_tower")? == Some(true);
    if on {
        settings.insert("enable_prime_tower", edge_config::Value::One("0".into()));
    }
    Ok(on)
}

/// What printing object by object refuses besides the clearances
/// (Print.cpp `validate`), and the order the copies are printed in, by
/// their place in `copies`; none for a plate printed layer by layer.
pub(crate) fn validate(
    settings: &Settings,
    copies: &[Copy<'_>],
    calibration: bool,
) -> Result<Option<Vec<usize>>, Failure> {
    if !by_object(settings) {
        return Ok(None);
    }
    if settings.text("timelapse_type") == Some("1") {
        return Err(
            "Smooth mode of timelapse is not supported when \"by object\" sequence is enabled."
                .into(),
        );
    }
    let order = order(copies, settings)?;
    // The prime tower is not refused: the reference's refusal is switched
    // off (`#if 0`) and it turns the tower off instead where several
    // copies are printed (`normalize_fdm_2`), as `crate::pipeline` does.
    if calibration {
        return Err(
            "a calibration print is printed layer by layer; print_sequence \"by object\" \
             is not done for one; no G-code was written"
                .into(),
        );
    }
    // A skirt round each copy (`skirt_type` "perobject") is not made:
    // one skirt goes round everything, which printing object by object
    // would lay round copies not yet printed.
    if settings.text("skirt_type") == Some("perobject")
        && settings.integer("skirt_loops")?.unwrap_or(0) > 0
        && settings.integer("skirt_height")?.unwrap_or(0) > 0
    {
        return Err(
            "skirt_type \"perobject\" with print_sequence \"by object\": a skirt round each \
             copy is not made yet; no G-code was written"
                .into(),
        );
    }
    Ok(Some(order))
}

#[cfg(test)]
mod tests {
    use edge_mesh::primitives;

    use super::*;

    fn block(x: f32, y: f32, side: f32, height: f32) -> Mesh {
        Mesh::from_triangle_soup(primitives::cuboid(
            [x, y, 0.0],
            [x + side, y + side, height],
        ))
    }

    /// A U1's head: 72.5 mm across, rods 27.5 mm up, lid 140 mm.
    fn u1(extra: &str) -> Settings {
        Settings::from_json(&format!(
            r#"{{"print_sequence": "by object", "extruder_clearance_radius": "72.5",
                "extruder_clearance_height_to_rod": "27.5",
                "extruder_clearance_height_to_lid": "140", "printable_height": "270",
                "nozzle_height": "2.5"{extra}}}"#
        ))
        .unwrap()
    }

    fn said(copies: &[(&str, Mesh)], settings: &Settings) -> Result<Vec<usize>, String> {
        let copies: Vec<Copy<'_>> = copies
            .iter()
            .map(|(name, mesh)| Copy { name, mesh })
            .collect();
        order(&copies, settings).map_err(|e| e.to_string())
    }

    #[test]
    fn copies_too_close_for_the_head_are_refused_by_name() {
        // 20 mm blocks 75 mm apart clear a head 72.5 mm across; 20 mm
        // apart they do not.
        let far = [
            ("A", block(20.0, 120.0, 20.0, 10.0)),
            ("B", block(115.0, 120.0, 20.0, 10.0)),
        ];
        assert_eq!(said(&far, &u1("")), Ok(vec![0, 1]));
        let near = [
            ("Left", block(100.0, 125.0, 20.0, 10.0)),
            ("Right", block(140.0, 125.0, 20.0, 10.0)),
        ];
        assert_eq!(
            said(&near, &u1("")),
            Err("Right is too close to others, and collisions may be caused.".into())
        );
        // All lower than the nozzle, they are kept 4 mm apart only.
        let flat = [
            ("Left", block(100.0, 125.0, 20.0, 2.0)),
            ("Right", block(125.0, 125.0, 20.0, 2.0)),
        ];
        assert_eq!(said(&flat, &u1("")), Ok(vec![0, 1]));
        // An excluded corner of the bed.
        let corner = r#", "bed_exclude_area": ["0x0", "50x0", "50x50", "0x50"]"#;
        assert_eq!(
            said(&[("A", block(40.0, 40.0, 20.0, 10.0))], &u1(corner)),
            Err("A is too close to exclusion area, there may be collisions when printing.".into())
        );
    }

    #[test]
    fn a_copy_above_the_rods_goes_after_those_level_with_it() {
        // The tall one listed first is printed last.
        let copies = [
            ("Tall", block(40.0, 125.0, 15.0, 40.0)),
            ("Short", block(160.0, 122.0, 20.0, 5.0)),
        ];
        assert_eq!(said(&copies, &u1("")), Ok(vec![1, 0]));
        // Two above the rods, level with each other: one must come first.
        let two = [
            ("TallA", block(40.0, 125.0, 15.0, 40.0)),
            ("TallB", block(170.0, 125.0, 15.0, 40.0)),
        ];
        assert_eq!(
            said(&two, &u1("")),
            Err("TallA is too tall, and collisions will be caused.".into())
        );
        // Not level across the bed, both may be.
        let apart = [
            ("TallA", block(40.0, 20.0, 15.0, 40.0)),
            ("TallB", block(170.0, 200.0, 15.0, 40.0)),
        ];
        assert_eq!(said(&apart, &u1("")), Ok(vec![0, 1]));
        // Above the lid only the last may go, and no higher than the printer.
        let lid = [
            ("A", block(40.0, 20.0, 15.0, 150.0)),
            ("B", block(170.0, 200.0, 15.0, 150.0)),
        ];
        assert!(said(&lid, &u1("")).unwrap_err().contains("is too tall"));
        assert_eq!(said(&lid[..1], &u1("")), Ok(vec![0]));
    }

    #[test]
    fn what_printing_by_object_refuses() {
        let one = [("A", block(40.0, 20.0, 15.0, 10.0))];
        let copies: Vec<Copy<'_>> = one.iter().map(|(name, mesh)| Copy { name, mesh }).collect();
        let refused = |extra: &str, calibration: bool| {
            validate(&u1(extra), &copies, calibration)
                .err()
                .map(|e| e.to_string())
        };
        assert_eq!(refused("", false), None);
        assert_eq!(
            refused(r#", "timelapse_type": "1""#, false).as_deref(),
            Some(
                "Smooth mode of timelapse is not supported when \"by object\" sequence is enabled."
            )
        );
        // The prime tower is turned off for it, not refused.
        assert_eq!(refused(r#", "enable_prime_tower": "1""#, false), None);
        let mut with_tower = u1(r#", "enable_prime_tower": "1""#);
        assert!(tower_off(&mut with_tower).unwrap());
        assert_eq!(with_tower.flag("enable_prime_tower").unwrap(), Some(false));
        assert!(!tower_off(&mut with_tower).unwrap());
        assert!(refused("", true).is_some());
        let by_layer = Settings::from_json(r#"{"print_sequence": "by layer"}"#).unwrap();
        assert!(validate(&by_layer, &copies, true).unwrap().is_none());
    }

    /// Two blocks, a tall one at x 20 listed first and a short one at
    /// x 120, sliced and written as `sequence` asks.
    fn two_blocks(sequence: &str) -> Result<String, String> {
        use crate::{Chosen, InstanceLabel, Loaded, Overrides, Unwatched};
        let (short, tall) = (block(120.0, 90.0, 20.0, 4.0), block(20.0, 90.0, 20.0, 8.0));
        let body = |solid: &Mesh| edge_3mf::Body {
            solid: solid.clone(),
            negative: None,
            filament: 1,
            part: None,
        };
        let label = |object: usize, name: &str| InstanceLabel {
            object,
            copy: 0,
            name: name.into(),
            identify: object as u32 + 1,
        };
        let loaded = Loaded {
            bodies: vec![body(&tall), body(&short)],
            painted: Vec::new(),
            settings: None,
            unread_paint: Vec::new(),
            precise_seams: false,
            own: Vec::new(),
            objects: vec![tall.clone(), short.clone()],
            labels: vec![label(0, "Tall"), label(1, "Short")],
            negatives: vec![Mesh::default(), Mesh::default()],
            parts: Vec::new(),
            plate: 0,
            first_layer_sequence: Vec::new(),
            plate_settings: Default::default(),
            bounds: ([20.0, 90.0, 0.0], [140.0, 110.0, 8.0]),
            placed: true,
            custom_gcode: None,
        };
        let printer = Settings::from_json(
            r#"{"machine_start_gcode": "G28", "printable_area": ["0x0", "200x0", "200x200", "0x200"],
                "nozzle_diameter": ["0.4"], "printable_height": "200", "gcode_flavor": "klipper",
                "exclude_object": "1", "extruder_clearance_radius": "60",
                "extruder_clearance_height_to_rod": "6", "extruder_clearance_height_to_lid": "100"}"#,
        )
        .unwrap();
        let process = format!(r#"{{"print_sequence": "{sequence}"}}"#);
        let chosen = Chosen {
            machine: Some(("printer".into(), printer)),
            process: Some(("process".into(), Settings::from_json(&process).unwrap())),
            filaments: Vec::new(),
        };
        let job =
            crate::prepare(loaded, &chosen, &Overrides::default()).map_err(|e| e.to_string())?;
        let sliced = crate::slice(job, &Unwatched).map_err(|e| e.to_string())?;
        let gcode = sliced.gcode("now").map_err(|e| e.to_string())?;
        Ok(String::from_utf8(gcode.text).unwrap())
    }

    #[test]
    fn a_plate_is_printed_copy_by_copy_in_the_order_the_head_allows() {
        let text = two_blocks("by object").unwrap();
        // The tall block, listed first, reaches above the rods level with
        // the short one: the short is printed first, whole, then the tall.
        let starts: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("EXCLUDE_OBJECT_START"))
            .collect();
        let short = starts.iter().filter(|line| line.contains("Short")).count();
        assert_eq!(short, 20);
        assert!(starts[..short].iter().all(|line| line.contains("Short")));
        assert!(starts[short..].iter().all(|line| line.contains("Tall")));
        assert_eq!(text.matches(";LAYER_CHANGE").count(), 20 + 40);
        // Layer by layer, they go together.
        let layered = two_blocks("by layer").unwrap();
        assert_eq!(layered.matches(";LAYER_CHANGE").count(), 40);
    }
}
