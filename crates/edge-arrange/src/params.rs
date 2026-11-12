//! What an arrangement is asked to do and with what (Arrange.hpp
//! `ArrangePolygon`, `ArrangeParams`), and how the reference sets the
//! spacing, the bed's margin and each item's growth from the print's
//! settings before it arranges (Arrange.cpp `update_arrange_params`,
//! `update_selected_items_inflation`, `update_unselected_items_inflation`,
//! `update_selected_items_axis_align`, `get_shrink_bedpts`).

use edge_geom::{Coord, Point2, Polygon, scaled};

use crate::geom::{Bbox, rotated, translated};

/// The most beds an arrangement opens (libnest2d `MAX_NUM_PLATES`).
pub const MAX_NUM_PLATES: usize = 36;

/// The widest nozzle a by-object print keeps short objects apart by
/// (Print.hpp `MAX_OUTER_NOZZLE_DIAMETER`), in mm.
pub const MAX_OUTER_NOZZLE_DIAMETER: f64 = 4.0;

/// The margin the bed is shrunk by printing by object (Arrange.hpp
/// `BED_SHRINK_SEQ_PRINT`), in mm.
pub const BED_SHRINK_SEQ_PRINT: f64 = 5.0;

/// Something to arrange or to arrange around (`ArrangePolygon`). The
/// outline is in nanometres; `translation` and `rotation` say where it
/// stands, turned about the origin and then moved.
#[derive(Clone, Debug, Default)]
pub struct Piece {
    pub outline: Polygon,
    pub translation: Point2,
    pub rotation: f64,
    /// How far it is grown when arranged, in nanometres.
    pub inflation: Coord,
    /// Which bed it is on (-1: none; for fixed pieces, the bed they stand
    /// on).
    pub bed: i32,
    pub priority: i32,
    pub is_virtual: bool,
    pub is_extrusion_cali: bool,
    pub is_wipe_tower: bool,
    pub has_tree_support: bool,
    /// The filaments it prints with, from 1.
    pub filaments: Vec<i32>,
    /// Print.cpp's `FilamentTempType` of its filaments together.
    pub filament_temp_type: i32,
    pub bed_temp: i32,
    pub first_bed_temp: i32,
    pub height: f64,
    /// What it is grown by for its own sake with the spacing at "auto",
    /// in mm: 1, or 6 with normal supports, 24 with tree supports.
    pub brim_width: f64,
    pub name: String,
    /// The order it was placed in, after arranging.
    pub item_id: i32,
}

impl Piece {
    /// The outline turned and moved (`transformed_poly`).
    pub fn transformed(&self) -> Polygon {
        translated(&rotated(&self.outline, self.rotation), self.translation)
    }
}

/// How to arrange (`ArrangeParams`). Lengths are in mm unless named
/// otherwise.
#[derive(Clone, Debug)]
pub struct Params {
    /// The least distance between any two items, in nanometres; 0 is
    /// "auto": each item keeps its own allowance.
    pub min_obj_distance: Coord,
    pub accuracy: f32,
    pub allow_rotations: bool,
    pub do_final_align: bool,
    pub allow_multi_materials_on_same_plate: bool,
    pub avoid_extrusion_cali_region: bool,
    pub is_seq_print: bool,
    pub align_to_y_axis: bool,
    pub bed_shrink_x: f64,
    pub bed_shrink_y: f64,
    pub brim_skirt_distance: f64,
    pub clearance_height_to_rod: f64,
    pub clearance_height_to_lid: f64,
    pub clearance_radius: f64,
    pub object_skirt_offset: f64,
    pub nozzle_height: f64,
    pub printable_height: f64,
    /// Where on the bed the pile's middle is put, as shares of its width
    /// and depth (`best_object_pos`).
    pub align_center: [f64; 2],
    /// Regions of the bed nothing may stand on.
    pub excluded_regions: Vec<Piece>,
    /// Regions best left free.
    pub nonprefered_regions: Vec<Piece>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            min_obj_distance: 0,
            accuracy: 1.0,
            allow_rotations: false,
            do_final_align: true,
            allow_multi_materials_on_same_plate: true,
            avoid_extrusion_cali_region: true,
            is_seq_print: false,
            align_to_y_axis: false,
            bed_shrink_x: 1.0,
            bed_shrink_y: 1.0,
            brim_skirt_distance: 0.0,
            clearance_height_to_rod: 0.0,
            clearance_height_to_lid: 0.0,
            clearance_radius: 0.0,
            object_skirt_offset: 0.0,
            nozzle_height: 0.0,
            printable_height: 256.0,
            align_center: [0.5, 0.5],
            excluded_regions: Vec::new(),
            nonprefered_regions: Vec::new(),
        }
    }
}

/// The distance a by-object print needs between objects: a nozzle's
/// width between short ones, the extruder's clearance radius otherwise;
/// the person's own distance may only raise it
/// (`seq_print_min_obj_distance`).
pub fn seq_print_min_obj_distance(params: &Params, selected: &[Piece]) -> Coord {
    let all_short = selected.iter().all(|p| p.height < params.nozzle_height);
    let needed = if all_short {
        MAX_OUTER_NOZZLE_DIAMETER.max(params.object_skirt_offset * 2.0) + 0.001
    } else {
        params.clearance_radius + 0.001
    };
    params.min_obj_distance.max(scaled(needed))
}

/// The bed's margin: the skirt's distance from the objects added to it,
/// and printing by object, the bed grown by half the distance between
/// objects, which each item's growth may take past its edge
/// (`update_arrange_params`).
pub fn update_arrange_params(params: &mut Params, skirt_distance: f64, selected: &[Piece]) {
    params.brim_skirt_distance = skirt_distance;
    params.bed_shrink_x += params.brim_skirt_distance;
    params.bed_shrink_y += params.brim_skirt_distance;
    if params.is_seq_print {
        let grow = seq_print_min_obj_distance(params, selected) as f64 / 1e6 / 2.0;
        params.bed_shrink_x -= grow;
        params.bed_shrink_y -= grow;
    }
}

/// The bed `[min_x, min_y, max_x, max_y]` (mm) shrunk by the margin, in
/// nanometres (`get_shrink_bedpts` on a rectangular bed).
pub fn shrunk_bed(bed: [f64; 4], params: &Params) -> [Coord; 4] {
    let (sx, sy) = (scaled(params.bed_shrink_x), scaled(params.bed_shrink_y));
    [
        scaled(bed[0]) + sx,
        scaled(bed[1]) + sy,
        scaled(bed[2]) - sx,
        scaled(bed[3]) - sy,
    ]
}

/// How much each item to arrange is grown: half the distance between
/// objects, or with the spacing at "auto" its own allowance (with tree
/// supports anywhere, half the widest), never so much that it would not
/// fit the bed (`update_selected_items_inflation`).
pub fn update_selected_items_inflation(
    selected: &mut [Piece],
    bed: [Coord; 4],
    params: &mut Params,
) {
    let bed = Bbox::new(
        Point2 {
            x: bed[0],
            y: bed[1],
        },
        Point2 {
            x: bed[2],
            y: bed[3],
        },
    );
    if params.is_seq_print {
        params.min_obj_distance = seq_print_min_obj_distance(params, selected);
    }
    let brim_max = selected.iter().map(|p| p.brim_width).fold(0.0, f64::max);
    let tree = selected.iter().any(|p| p.has_tree_support);
    for piece in selected.iter_mut() {
        piece.inflation = if params.min_obj_distance != 0 {
            params.min_obj_distance / 2
        } else if tree {
            scaled(brim_max / 2.0)
        } else {
            scaled(piece.brim_width)
        };
        let own = Bbox::of(&piece.outline.points);
        let diffx = bed.width() - own.width() - 5;
        let diffy = bed.height() - own.height() - 5;
        if diffx > 0 && diffy > 0 {
            piece.inflation = (diffx.min(diffy) / 2).min(piece.inflation);
        }
    }
}

/// How much what is not arranged is grown: objects as the arranged ones
/// are; excluded regions by a millimetre (by object, by the margin too,
/// and moved with the grown bed); the extrusion calibration region not at
/// all (`update_unselected_items_inflation`).
pub fn update_unselected_items_inflation(unselected: &mut [Piece], params: &Params) {
    let mut exclusion_gap = 1.0;
    if params.is_seq_print {
        exclusion_gap = f64::max(
            exclusion_gap,
            params.min_obj_distance as f64 / 1e6 / 2.0 + params.bed_shrink_x + 1.0,
        );
        for region in unselected.iter_mut().filter(|p| p.is_virtual) {
            let by = Point2 {
                x: scaled(params.bed_shrink_x.min(0.0)),
                y: scaled(params.bed_shrink_y.min(0.0)),
            };
            region.outline = translated(&region.outline, by);
        }
    }
    for piece in unselected.iter_mut() {
        piece.inflation = if !piece.is_virtual {
            if params.min_obj_distance == 0 {
                scaled(piece.brim_width)
            } else {
                params.min_obj_distance / 2
            }
        } else if piece.is_extrusion_cali {
            0
        } else {
            scaled(exclusion_gap)
        };
    }
}

/// "Align to Y axis": each item turned so the long axis of its outline's
/// second moments lies along y, unless it has none to speak of
/// (`update_selected_items_axis_align`).
pub fn update_selected_items_axis_align(selected: &mut [Piece], params: &Params) {
    if !params.align_to_y_axis {
        return;
    }
    for piece in selected.iter_mut() {
        let pts = piece.transformed().points;
        if pts.is_empty() {
            continue;
        }
        let (mut a00, mut a10, mut a01, mut a20, mut a11, mut a02) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        let last = pts[pts.len() - 1];
        let (mut xi_1, mut yi_1) = (last.x as f64, last.y as f64);
        for p in &pts {
            let (xi, yi) = (p.x as f64, p.y as f64);
            let (xi2, yi2) = (xi * xi, yi * yi);
            let dxy = xi_1 * yi - xi * yi_1;
            let xii_1 = xi_1 + xi;
            let yii_1 = yi_1 + yi;
            a00 += dxy;
            a10 += dxy * xii_1;
            a01 += dxy * yii_1;
            a20 += dxy * (xi_1 * xii_1 + xi2);
            a11 += dxy * (xi_1 * (yii_1 + yi_1) + xi * (yii_1 + yi));
            a02 += dxy * (yi_1 * yii_1 + yi2);
            xi_1 = xi;
            yi_1 = yi;
        }
        if a00.abs() <= 1e-4 {
            continue;
        }
        let sign = if a00 > 0.0 { 1.0 } else { -1.0 };
        let m00 = a00 * 0.5 * sign;
        let m10 = a10 / 6.0 * sign;
        let m01 = a01 / 6.0 * sign;
        let m20 = a20 / 12.0 * sign;
        let m11 = a11 / 24.0 * sign;
        let m02 = a02 / 12.0 * sign;
        let cx = m10 / m00;
        let cy = m01 / m00;
        let a = m20 / m00 - cx * cx;
        let b = m11 / m00 - cx * cy;
        let c = m02 / m00 - cy * cy;
        // Close second moments: no axis to align.
        let ratio = if a.abs() > c.abs() {
            (c / a).abs()
        } else if c.abs() > 0.0 {
            (a / c).abs()
        } else {
            0.0
        };
        if ratio > 0.66 {
            continue;
        }
        let mut angle = (2.0 * b).atan2(a - c) / 2.0;
        angle = std::f64::consts::FRAC_PI_2 - angle;
        if (angle.abs() - std::f64::consts::PI).abs() < 0.01 {
            angle = 0.0;
        }
        piece.rotation += angle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f64, h: f64) -> Polygon {
        Polygon {
            points: [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    #[test]
    fn auto_spacing_grows_each_item_by_its_own_allowance() {
        let mut params = Params::default();
        let mut pieces = vec![
            Piece {
                outline: rect(20.0, 20.0),
                brim_width: 1.0,
                ..Piece::default()
            },
            Piece {
                outline: rect(20.0, 20.0),
                brim_width: 6.0,
                ..Piece::default()
            },
        ];
        update_arrange_params(&mut params, 0.0, &pieces);
        let bed = shrunk_bed([0.0, 0.0, 200.0, 200.0], &params);
        assert_eq!(
            bed,
            [scaled(1.0), scaled(1.0), scaled(199.0), scaled(199.0)]
        );
        update_selected_items_inflation(&mut pieces, bed, &mut params);
        assert_eq!(pieces[0].inflation, scaled(1.0));
        assert_eq!(pieces[1].inflation, scaled(6.0));
        // A spacing given is halved onto each.
        params.min_obj_distance = scaled(10.0);
        update_selected_items_inflation(&mut pieces, bed, &mut params);
        assert_eq!(pieces[1].inflation, scaled(5.0));
    }

    #[test]
    fn by_object_the_distance_is_the_clearance_radius() {
        let mut params = Params {
            is_seq_print: true,
            clearance_radius: 40.0,
            nozzle_height: 4.0,
            bed_shrink_x: BED_SHRINK_SEQ_PRINT,
            bed_shrink_y: BED_SHRINK_SEQ_PRINT,
            ..Params::default()
        };
        let tall = vec![Piece {
            outline: rect(10.0, 10.0),
            height: 30.0,
            ..Piece::default()
        }];
        assert_eq!(seq_print_min_obj_distance(&params, &tall), scaled(40.001));
        update_arrange_params(&mut params, 0.0, &tall);
        assert!((params.bed_shrink_x - (5.0 - 20.0005)).abs() < 1e-9);
    }

    #[test]
    fn a_long_part_is_aligned_to_y() {
        let params = Params {
            align_to_y_axis: true,
            ..Params::default()
        };
        let mut pieces = vec![Piece {
            outline: rect(100.0, 10.0),
            ..Piece::default()
        }];
        update_selected_items_axis_align(&mut pieces, &params);
        let b = Bbox::of(&pieces[0].transformed().points);
        assert!(b.height() > b.width() * 5, "{b:?}");
    }
}
