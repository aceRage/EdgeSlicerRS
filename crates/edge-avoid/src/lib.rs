//! Travel that keeps to the inside of a part's walls where a straight
//! move would cross them (`reduce_crossing_wall`): the move follows the
//! outline, a little way in from it, from where it would first leave the
//! part to where it would last come back, and a move that passes a hole
//! goes round it.
//!
//! A port of the reference's GCode/AvoidCrossingPerimeters.cpp. What a
//! layer is routed by is made once ([`Layer::new`]), and each move is then
//! asked for ([`Layer::route`]); nothing is kept between moves, so a
//! layer answers the same whenever and however often it is asked.
//!
//! Left out, as the reference has it switched off: whether a move routed
//! this way could go without the wipe (`need_wipe`), which the reference
//! works out and then does not use.

mod grid;
mod route;
mod shape;
mod shelter;

pub use edge_clip::ClipError;
use edge_geom::{ExPolygon, Point2, scaled};
use route::Boundary;
pub use shelter::Shelter;

/// What a layer's moves are routed by.
#[derive(Clone, Copy, Debug)]
pub struct Slices<'a> {
    /// Everything printed on the layer, as outlines with their holes
    /// (the reference's `lslices`).
    pub outline: &'a [ExPolygon],
    /// The layer's top surfaces, which a move goes round.
    pub tops: &'a [ExPolygon],
    /// How far apart the inner walls are laid, in mm.
    pub spacing: f64,
    /// How wide the outer wall is, in mm.
    pub outer_width: f64,
}

/// Which moves a layer has, of those that are not routed the usual way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    /// Moves between supports.
    pub supports: bool,
    /// Moves outside the part: to and round a skirt, a brim or a prime
    /// tower.
    pub outside: bool,
}

/// What a move is between.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Walls and fill of the part. A move that stays within one piece of
    /// the layer, clear of its outer wall, goes straight.
    #[default]
    Part,
    /// Supports, which stand outside the part: every such move is looked
    /// at, and top surfaces are not gone round.
    Support,
    /// Anything outside the part, a skirt, a brim or a prime tower, and
    /// the move to it or from it: only holes are gone round, each in
    /// its own wall (the reference's `use_external_mp`).
    Outside,
}

/// The longest a way round may be than the straight move
/// (`max_travel_detour_distance`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Detour {
    /// In mm.
    Length(f64),
    /// As a percentage of the straight move.
    Percent(f64),
}

/// A layer ready to route moves on.
pub struct Layer {
    /// The spacing of the walls, in scaled units.
    spacing: f64,
    /// The outline a little behind the outer wall's middle: a move
    /// within one piece of it goes straight.
    within: Boundary,
    /// Where a move inside the part runs, top surfaces left out.
    inside: Boundary,
    /// The same with top surfaces not left out, where supports need it
    /// and it differs.
    under_supports: Option<Boundary>,
    /// What a move outside the part goes round, where the layer has
    /// such moves.
    outside: Option<Boundary>,
}

fn rings(regions: &[ExPolygon]) -> Vec<edge_geom::Polygon> {
    regions.iter().flat_map(ExPolygon::rings).cloned().collect()
}

impl Layer {
    /// Works out what the layer's moves are routed by
    /// (`AvoidCrossingPerimeters::init_layer`, and the boundaries
    /// `travel_to` makes when it first needs them).
    ///
    /// Not as the reference, on purpose: it makes the boundary for moves
    /// inside the part on the layer's first such move and keeps it for
    /// the layer, whether that move was between supports (top surfaces
    /// not left out) or in the part (left out). Supports are printed
    /// first, so on a layer that has them its moves in the part cross
    /// top surfaces that the same moves go round on a layer that has
    /// none. Here each kind of move has its own.
    ///
    /// The reference also draws the outline in by the nozzle's diameter
    /// and not by the spacing of the walls when the first move was
    /// between supports, a support layer having no walls to measure
    /// (`get_perimeter_spacing`); here the walls' spacing serves every
    /// kind of move, a hundredth of a millimetre from that.
    pub fn new(slices: &Slices<'_>, needs: Needs) -> Result<Self, ClipError> {
        let spacing = scaled(slices.spacing) as f64;
        let within = shape::within_walls(slices.outline, scaled(slices.outer_width) as f64)?;
        let drawn_in = shape::inside(slices.outline, spacing)?;
        let (inside, under_supports) = if slices.tops.is_empty() {
            (drawn_in, None)
        } else {
            let clear = shape::clear_of_tops(&drawn_in, slices.tops, spacing)?;
            (clear, needs.supports.then_some(drawn_in))
        };
        let outside = if needs.outside {
            Some(Boundary::new(shape::holes(slices.outline, spacing)?))
        } else {
            None
        };
        Ok(Layer {
            spacing,
            within: Boundary::new(rings(&within)),
            inside: Boundary::new(rings(&inside)),
            under_supports: under_supports.map(|regions| Boundary::new(rings(&regions))),
            outside,
        })
    }

    /// The way from `from` to `to`: every point of it after `from`,
    /// ending in `to`. A move that crosses nothing, or whose way round
    /// would be longer than `limit` allows, comes back as `to` alone
    /// (`AvoidCrossingPerimeters::travel_to`).
    pub fn route(
        &self,
        from: Point2,
        to: Point2,
        kind: Kind,
        limit: Option<Detour>,
    ) -> Vec<Point2> {
        let straight = vec![to];
        if from == to {
            return straight;
        }
        let boundary = match kind {
            // Within one piece of the layer and clear of its walls.
            Kind::Part if self.within.is_empty() || self.stays_within(from, to) => return straight,
            Kind::Part => &self.inside,
            Kind::Support => self.under_supports.as_ref().unwrap_or(&self.inside),
            Kind::Outside => match &self.outside {
                Some(outside) => outside,
                None => return straight,
            },
        };
        if boundary.is_empty() {
            return straight;
        }
        let (mut way, _met) = boundary.way(from, to, self.spacing);
        // Whatever the way was worked out from, it starts and ends
        // where the move does.
        way[0] = from;
        *way.last_mut().expect("a way has its two ends") = to;
        if let Some(limit) = limit {
            let direct = route::length(&[from, to]);
            let most = match limit {
                Detour::Length(mm) => scaled(mm) as f64,
                Detour::Percent(percent) => direct * 0.01 * percent,
            };
            if route::length(&way) - direct > most {
                return straight;
            }
        }
        way.remove(0);
        way
    }

    /// Whether the straight move stays within one piece of the layer,
    /// clear of its outer wall (`any_expolygon_contains`).
    fn stays_within(&self, from: Point2, to: Point2) -> bool {
        !self.within.blocks(from, to) && self.within.holds(from)
    }
}

#[cfg(test)]
mod tests;
