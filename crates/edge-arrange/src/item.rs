//! One thing to arrange, as libnest2d's `_Item` holds it (nester.hpp): an
//! outline with how far it is grown, turned and moved, and what the
//! arranger weighs besides its shape.

use edge_clip::Join;
use edge_geom::{Coord, ExPolygon, Point2, Polygon};

use crate::geom::{Bbox, rotated, signed_area, translated};

/// libnest2d's bin id for an item not (yet) on any bed.
pub const UNSET: i32 = -1;
/// The bin id the selection gives an item no empty bed holds.
pub const UNFIT: i32 = -2;

#[derive(Clone, Debug)]
pub struct Item {
    /// The outline as given, counter-clockwise.
    shape: Polygon,
    translation: Point2,
    rotation: f64,
    inflation: Coord,
    /// The outline grown by `inflation`.
    inflated: Polygon,
    /// That, turned by `rotation` (not moved).
    turned: Polygon,
    /// The box round `turned`.
    turned_box: Bbox,
    area: f64,

    pub bin: i32,
    pub priority: i32,
    pub fixed: bool,
    pub item_id: i32,
    pub filaments: Vec<i32>,
    pub filament_temp_type: i32,
    pub height: f64,
    pub bed_temp: f64,
    pub name: String,
    pub is_virtual: bool,
    pub is_wipe_tower: bool,
    pub tried_with_excluded: bool,
}

/// Grows an outline by `delta` with square corners (the reference's
/// `sl::offset`, `jtSquare`), keeping the first region.
fn grown(shape: &Polygon, delta: Coord) -> Polygon {
    if delta == 0 {
        return shape.clone();
    }
    let region = ExPolygon {
        contour: shape.clone(),
        holes: Vec::new(),
    };
    edge_clip::offset(&[region], delta, Join::Square)
        .ok()
        .and_then(|regions| regions.into_iter().next())
        .map(|region| region.contour)
        .unwrap_or_default()
}

impl Item {
    pub fn new(shape: Polygon) -> Self {
        let mut shape = shape;
        if signed_area(&shape) < 0.0 {
            shape.points.reverse();
        }
        let mut item = Self {
            inflated: shape.clone(),
            turned: shape.clone(),
            turned_box: Bbox::of(&shape.points),
            area: signed_area(&shape).abs(),
            shape,
            translation: Point2::default(),
            rotation: 0.0,
            inflation: 0,
            bin: UNSET,
            priority: 0,
            fixed: false,
            item_id: 0,
            filaments: Vec::new(),
            filament_temp_type: -1,
            height: 0.0,
            bed_temp: 0.0,
            name: String::new(),
            is_virtual: false,
            is_wipe_tower: false,
            tried_with_excluded: false,
        };
        item.refresh();
        item
    }

    fn refresh(&mut self) {
        self.inflated = grown(&self.shape, self.inflation);
        self.area = signed_area(&self.inflated).abs();
        self.turn();
    }

    fn turn(&mut self) {
        self.turned = if self.rotation == 0.0 {
            self.inflated.clone()
        } else {
            rotated(&self.inflated, self.rotation)
        };
        self.turned_box = Bbox::of(&self.turned.points);
    }

    pub fn set_inflation(&mut self, inflation: Coord) {
        if inflation != self.inflation {
            self.inflation = inflation;
            self.refresh();
        }
    }

    pub fn inflate(&mut self, by: Coord) {
        self.set_inflation(self.inflation + by);
    }

    pub fn rotation(&self) -> f64 {
        self.rotation
    }

    pub fn set_rotation(&mut self, rotation: f64) {
        if rotation != self.rotation {
            self.rotation = rotation;
            self.turn();
        }
    }

    pub fn translation(&self) -> Point2 {
        self.translation
    }

    pub fn set_translation(&mut self, translation: Point2) {
        self.translation = translation;
    }

    pub fn translate(&mut self, by: Point2) {
        self.translation.x += by.x;
        self.translation.y += by.y;
    }

    /// The area of the grown outline, in square nanometres.
    pub fn area(&self) -> f64 {
        self.area
    }

    /// The outline grown, turned and moved.
    pub fn transformed(&self) -> Polygon {
        translated(&self.turned, self.translation)
    }

    pub fn bounding_box(&self) -> Bbox {
        self.turned_box.translated(self.translation)
    }

    /// The top corner, the rightmost of them, where it stands
    /// (`rightmostTopVertex`, libnest2d's reference vertex).
    pub fn reference_vertex(&self) -> Point2 {
        let p = self
            .turned
            .points
            .iter()
            .copied()
            .max_by_key(|p| (p.y, p.x))
            .unwrap_or_default();
        Point2 {
            x: p.x + self.translation.x,
            y: p.y + self.translation.y,
        }
    }

    pub fn mark_fixed(&mut self, bin: i32) {
        self.fixed = bin >= 0;
        self.bin = bin;
    }
}
