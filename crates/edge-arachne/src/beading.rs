//! How many lines to lay across a part of a given thickness, how wide each
//! one is, and where its centre goes.
//!
//! A strategy answers for one thickness at a time. The plain one shares
//! the thickness out among the lines; the others wrap a strategy and
//! change its answer: keep the outer walls at their own width, print parts
//! thinner than a line, move the outer wall inwards, stop at a number of
//! walls. `make_strategy` stacks them the way the reference does.
//!
//! A port of the reference's src/libslic3r/Arachne/BeadingStrategy:
//! BeadingStrategy, DistributedBeadingStrategy, RedistributeBeadingStrategy,
//! WideningBeadingStrategy, OuterWallInsetBeadingStrategy,
//! LimitedBeadingStrategy and BeadingStrategyFactory (.hpp and .cpp of
//! each). The arithmetic keeps the reference's types: where it works in
//! `float` this works in `f32`, and a result that the reference stores in a
//! `coord_t` is cut towards zero the same way.
//!
//! Left out: the reference's log lines (an odd wall limit, too many beads
//! asked for), since this crate has nowhere to log to.

use edge_geom::Coord;

/// `scaled<coord_t>(0.01)`: the reference's stand-in for "next to nothing",
/// 0.01 mm.
const SCALED_0_01: Coord = 10_000;
/// `scaled<coord_t>(1000.)`: what `LimitedBeadingStrategy::getOptimalThickness`
/// answers past its limit.
const SCALED_1000: Coord = 1_000_000_000;
/// `scaled<coord_t>(900.)`: what `LimitedBeadingStrategy::getTransitionThickness`
/// answers past its limit.
const SCALED_900: Coord = 900_000_000;

/// The lines across one thickness (`BeadingStrategy::Beading`,
/// BeadingStrategy.hpp).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beading {
    pub total_thickness: Coord,
    /// The width of each line, from the outside in.
    pub bead_widths: Vec<Coord>,
    /// How far the centre of each line is from the outline.
    pub toolpath_locations: Vec<Coord>,
    /// The distance no line covers.
    pub left_over: Coord,
}

/// What every strategy carries: the fields of the reference's base class
/// (BeadingStrategy.hpp). A wrapper copies its parent's, as the reference's
/// copy constructor does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeadingParams {
    /// The width a line has when nothing is in its way.
    pub optimal_width: Coord,
    /// When a middle line is split in two, as a share of the optimal width.
    pub wall_split_middle_threshold: f64,
    /// When a middle line is added between an even number, as a share of
    /// the optimal width.
    pub wall_add_middle_threshold: f64,
    /// The length over which one number of lines becomes the next.
    pub default_transition_length: Coord,
    /// The largest angle between two edges of the outline at which a
    /// transition is still made.
    pub transitioning_angle: f64,
}

impl BeadingParams {
    /// The base class constructor. The angle comes in as a `float` there and
    /// is kept as a `double`, so it is the same here.
    pub fn new(
        optimal_width: Coord,
        wall_split_middle_threshold: f64,
        wall_add_middle_threshold: f64,
        default_transition_length: Coord,
        transitioning_angle: f32,
    ) -> Self {
        Self {
            optimal_width,
            wall_split_middle_threshold,
            wall_add_middle_threshold,
            default_transition_length,
            transitioning_angle: f64::from(transitioning_angle),
        }
    }
}

/// A way of covering a thickness with lines (the `BeadingStrategy` base
/// class, BeadingStrategy.hpp and .cpp). The methods with a body here are
/// the ones the base class implements; a strategy that does not override
/// one in the reference does not here either.
pub trait BeadingStrategy: Send + Sync {
    /// The fields of the base class.
    fn params(&self) -> &BeadingParams;

    /// The lines that cover `thickness` when there are `bead_count` of
    /// them. The count asked for need not be the best one for the
    /// thickness.
    fn compute(&self, thickness: Coord, bead_count: Coord) -> Beading;

    /// The thickness `bead_count` lines fit best.
    fn get_optimal_thickness(&self, bead_count: Coord) -> Coord {
        self.params().optimal_width * bead_count
    }

    /// The thickness at which the best count goes from `lower_bead_count`
    /// to one more.
    fn get_transition_thickness(&self, lower_bead_count: Coord) -> Coord {
        let lower_ideal_width = self.get_optimal_thickness(lower_bead_count);
        let higher_ideal_width = self.get_optimal_thickness(lower_bead_count + 1);
        let threshold = if lower_bead_count % 2 == 1 {
            self.params().wall_split_middle_threshold
        } else {
            self.params().wall_add_middle_threshold
        };
        (lower_ideal_width as f64 + threshold * (higher_ideal_width - lower_ideal_width) as f64)
            as Coord
    }

    /// The number of lines that suits `thickness` best.
    fn get_optimal_bead_count(&self, thickness: Coord) -> Coord;

    /// The length along the skeleton over which `lower_bead_count` lines
    /// become one more.
    fn get_transitioning_length(&self, lower_bead_count: Coord) -> Coord {
        if lower_bead_count == 0 {
            return SCALED_0_01;
        }
        self.params().default_transition_length
    }

    /// The share of the transition's length that lies before the point
    /// where the count jumps. The reference returns a `float`; the value
    /// here is that `float`, widened.
    fn get_transition_anchor_pos(&self, lower_bead_count: Coord) -> f64 {
        let lower_optimum = self.get_optimal_thickness(lower_bead_count);
        let transition_point = self.get_transition_thickness(lower_bead_count);
        let upper_optimum = self.get_optimal_thickness(lower_bead_count + 1);
        let ratio =
            (transition_point - lower_optimum) as f32 / (upper_optimum - lower_optimum) as f32;
        f64::from((1.0 - f64::from(ratio)) as f32)
    }

    /// The thicknesses, from thin to thick, at which the widths `compute`
    /// gives stop changing in a straight line.
    fn get_nonlinear_thicknesses(&self, _lower_bead_count: Coord) -> Vec<Coord> {
        Vec::new()
    }

    /// The strategy's name with those it wraps (`toString`).
    fn to_string(&self) -> String;

    fn get_split_middle_threshold(&self) -> f64 {
        self.params().wall_split_middle_threshold
    }

    /// The reference has no getter for this one; the field is the base
    /// class's `wall_add_middle_threshold`.
    fn get_add_middle_threshold(&self) -> f64 {
        self.params().wall_add_middle_threshold
    }

    fn get_transitioning_angle(&self) -> f64 {
        self.params().transitioning_angle
    }

    /// The base class's `optimal_width`.
    fn get_optimal_width(&self) -> Coord {
        self.params().optimal_width
    }

    /// The base class's `default_transition_length`.
    fn get_default_transition_length(&self) -> Coord {
        self.params().default_transition_length
    }
}

/// A place in a list of beads. The reference indexes with a `coord_t`; a
/// negative one reads outside the list there and stops here.
fn index(i: Coord) -> usize {
    usize::try_from(i).expect("a bead index is not negative")
}

/// Shares a thickness out among the lines, the ones nearest the middle
/// taking most of what differs from the optimal width
/// (DistributedBeadingStrategy.hpp and .cpp).
pub struct DistributedBeadingStrategy {
    params: BeadingParams,
    /// (1 / distribution radius)^2, the radius counted in lines.
    one_over_distribution_radius_squared: f32,
}

impl DistributedBeadingStrategy {
    /// `distribution_radius` is how many lines from the middle take a share
    /// of the difference.
    pub fn new(
        optimal_width: Coord,
        default_transition_length: Coord,
        transitioning_angle: f64,
        wall_split_middle_threshold: f64,
        wall_add_middle_threshold: f64,
        distribution_radius: i32,
    ) -> Self {
        let one_over_distribution_radius_squared = if distribution_radius >= 2 {
            1.0f32 / (distribution_radius - 1) as f32 * 1.0f32 / (distribution_radius - 1) as f32
        } else {
            // `1.0f / 1 * 1.0f / 1` in the reference.
            1.0f32
        };
        Self {
            params: BeadingParams::new(
                optimal_width,
                wall_split_middle_threshold,
                wall_add_middle_threshold,
                default_transition_length,
                // The base class takes a `float`.
                transitioning_angle as f32,
            ),
            one_over_distribution_radius_squared,
        }
    }
}

impl BeadingStrategy for DistributedBeadingStrategy {
    fn params(&self) -> &BeadingParams {
        &self.params
    }

    fn compute(&self, thickness: Coord, bead_count: Coord) -> Beading {
        let optimal_width = self.params.optimal_width;
        let mut ret = Beading {
            total_thickness: thickness,
            ..Beading::default()
        };

        if bead_count > 2 {
            let to_be_divided = thickness - bead_count * optimal_width;
            let middle = (bead_count - 1) as f32 / 2.0;

            let get_weight = |bead_idx: Coord| -> f32 {
                let dev_from_middle = bead_idx as f32 - middle;
                let weight = 1.0f32
                    - self.one_over_distribution_radius_squared * dev_from_middle * dev_from_middle;
                // std::max(0.0f, weight)
                if 0.0f32 < weight { weight } else { 0.0f32 }
            };

            let mut weights = vec![0.0f32; index(bead_count)];
            for bead_idx in 0..bead_count {
                weights[index(bead_idx)] = get_weight(bead_idx);
            }

            let total_weight = weights.iter().fold(0.0f32, |sum, weight| sum + *weight);
            let mut accumulated_width: Coord = 0;
            for bead_idx in 0..bead_count {
                let weight_fraction = weights[index(bead_idx)] / total_weight;
                let splitup_left_over_weight = (to_be_divided as f32 * weight_fraction) as Coord;
                let width = if bead_idx == bead_count - 1 {
                    thickness - accumulated_width
                } else {
                    optimal_width + splitup_left_over_weight
                };

                // Halving the width can leave a location one unit off.
                if bead_idx == 0 {
                    ret.toolpath_locations.push(width / 2);
                } else {
                    let previous_location =
                        ret.toolpath_locations[ret.toolpath_locations.len() - 1];
                    let previous_width = ret.bead_widths[ret.bead_widths.len() - 1];
                    ret.toolpath_locations
                        .push(previous_location + (previous_width + width) / 2);
                }
                ret.bead_widths.push(width);
                accumulated_width += width;
            }
            ret.left_over = 0;
            debug_assert!(accumulated_width + ret.left_over == thickness);
        } else if bead_count == 2 {
            let outer_width = thickness / 2;
            ret.bead_widths.push(outer_width);
            ret.bead_widths.push(outer_width);
            ret.toolpath_locations.push(outer_width / 2);
            ret.toolpath_locations.push(thickness - outer_width / 2);
            ret.left_over = 0;
        } else if bead_count == 1 {
            let outer_width = thickness;
            ret.bead_widths.push(outer_width);
            ret.toolpath_locations.push(outer_width / 2);
            ret.left_over = 0;
        } else {
            ret.left_over = thickness;
        }

        debug_assert!(ret.bead_widths.iter().sum::<Coord>() + ret.left_over == thickness);

        ret
    }

    fn get_optimal_bead_count(&self, thickness: Coord) -> Coord {
        let optimal_width = self.params.optimal_width;
        // How many lines fit for sure, and the room left after them.
        let naive_count = thickness / optimal_width;
        let remainder = thickness - naive_count * optimal_width;
        let threshold = if naive_count % 2 == 1 {
            self.params.wall_split_middle_threshold
        } else {
            self.params.wall_add_middle_threshold
        };
        let minimum_line_width = (optimal_width as f64 * threshold) as Coord;
        // One more where there is room enough for it.
        naive_count + Coord::from(remainder >= minimum_line_width)
    }

    fn to_string(&self) -> String {
        "DistributedBeadingStrategy".to_string()
    }
}

/// Keeps the outer walls at their own width and leaves only the walls
/// between them to the strategy it wraps, so that the surface does not
/// answer to what happens inside (RedistributeBeadingStrategy.hpp and
/// .cpp).
pub struct RedistributeBeadingStrategy {
    params: BeadingParams,
    parent: Box<dyn BeadingStrategy>,
    optimal_width_outer: Coord,
    /// The least a line may be, as a share of the outer width.
    minimum_variable_line_ratio: f64,
}

impl RedistributeBeadingStrategy {
    pub fn new(
        optimal_width_outer: Coord,
        minimum_variable_line_ratio: f64,
        parent: Box<dyn BeadingStrategy>,
    ) -> Self {
        Self {
            params: *parent.params(),
            parent,
            optimal_width_outer,
            minimum_variable_line_ratio,
        }
    }
}

impl BeadingStrategy for RedistributeBeadingStrategy {
    fn params(&self) -> &BeadingParams {
        &self.params
    }

    fn get_optimal_thickness(&self, bead_count: Coord) -> Coord {
        let inner_bead_count = Coord::max(0, bead_count - 2);
        let outer_bead_count = bead_count - inner_bead_count;
        self.parent.get_optimal_thickness(inner_bead_count)
            + self.optimal_width_outer * outer_bead_count
    }

    fn get_transition_thickness(&self, lower_bead_count: Coord) -> Coord {
        match lower_bead_count {
            0 => (self.minimum_variable_line_ratio * self.optimal_width_outer as f64) as Coord,
            1 => {
                ((1.0 + self.parent.get_split_middle_threshold()) * self.optimal_width_outer as f64)
                    as Coord
            }
            _ => {
                self.parent.get_transition_thickness(lower_bead_count - 2)
                    + 2 * self.optimal_width_outer
            }
        }
    }

    fn get_optimal_bead_count(&self, thickness: Coord) -> Coord {
        if (thickness as f64) < self.minimum_variable_line_ratio * self.optimal_width_outer as f64 {
            return 0;
        }
        if thickness <= 2 * self.optimal_width_outer {
            return if thickness as f64
                > (1.0 + self.parent.get_split_middle_threshold()) * self.optimal_width_outer as f64
            {
                2
            } else {
                1
            };
        }
        self.parent
            .get_optimal_bead_count(thickness - 2 * self.optimal_width_outer)
            + 2
    }

    fn get_transitioning_length(&self, lower_bead_count: Coord) -> Coord {
        self.parent.get_transitioning_length(lower_bead_count)
    }

    fn get_transition_anchor_pos(&self, lower_bead_count: Coord) -> f64 {
        self.parent.get_transition_anchor_pos(lower_bead_count)
    }

    fn to_string(&self) -> String {
        format!("RedistributeBeadingStrategy+{}", self.parent.to_string())
    }

    fn compute(&self, thickness: Coord, bead_count: Coord) -> Beading {
        let mut ret = Beading::default();

        // Everything that gives no lines at all.
        if bead_count == 0
            || (thickness as f64)
                < self.minimum_variable_line_ratio * self.optimal_width_outer as f64
        {
            ret.left_over = thickness;
            ret.total_thickness = thickness;
            return ret;
        }

        // The inner walls, if there are any.
        let inner_bead_count = bead_count - 2;
        let inner_thickness = thickness - 2 * self.optimal_width_outer;
        if inner_bead_count > 0 && inner_thickness > 0 {
            ret = self.parent.compute(inner_thickness, inner_bead_count);
            for toolpath_location in &mut ret.toolpath_locations {
                *toolpath_location += self.optimal_width_outer;
            }
        }

        // The outer wall or walls go around them.
        let actual_outer_thickness = if bead_count > 2 {
            Coord::min(thickness / 2, self.optimal_width_outer)
        } else {
            thickness / bead_count
        };
        ret.bead_widths.insert(0, actual_outer_thickness);
        ret.toolpath_locations.insert(0, actual_outer_thickness / 2);
        if bead_count > 1 {
            ret.bead_widths.push(actual_outer_thickness);
            ret.toolpath_locations
                .push(thickness - actual_outer_thickness / 2);
        }

        ret.total_thickness = thickness;
        ret.left_over = thickness - ret.bead_widths.iter().sum::<Coord>();
        ret
    }
}

/// Prints a part thinner than one line as one line of at least a set
/// width, so that small pieces are not lost (WideningBeadingStrategy.hpp
/// and .cpp).
pub struct WideningBeadingStrategy {
    params: BeadingParams,
    parent: Box<dyn BeadingStrategy>,
    /// Parts thinner than this get no line.
    min_input_width: Coord,
    /// The least width of the line a thin part gets.
    min_output_width: Coord,
}

impl WideningBeadingStrategy {
    pub fn new(
        parent: Box<dyn BeadingStrategy>,
        min_input_width: Coord,
        min_output_width: Coord,
    ) -> Self {
        Self {
            params: *parent.params(),
            parent,
            min_input_width,
            min_output_width,
        }
    }
}

impl BeadingStrategy for WideningBeadingStrategy {
    fn params(&self) -> &BeadingParams {
        &self.params
    }

    fn to_string(&self) -> String {
        format!("Widening+{}", self.parent.to_string())
    }

    fn compute(&self, thickness: Coord, bead_count: Coord) -> Beading {
        // A thin wall is one below the thickness where one line becomes
        // two, the same test `get_optimal_bead_count` ends up making. It is
        // made a single line only when no more than one was asked for
        // (Orca #14376): inside the band where one line becomes two, a
        // single line of the whole thickness would bulge.
        if bead_count <= 1 && thickness < self.get_transition_thickness(1) {
            let mut ret = Beading {
                total_thickness: thickness,
                ..Beading::default()
            };
            if thickness >= self.min_input_width {
                ret.bead_widths
                    .push(Coord::max(thickness, self.min_output_width));
                ret.toolpath_locations.push(thickness / 2);
                ret.left_over = 0;
            } else {
                ret.left_over = thickness;
            }
            ret
        } else {
            self.parent.compute(thickness, bead_count)
        }
    }

    fn get_optimal_thickness(&self, bead_count: Coord) -> Coord {
        self.parent.get_optimal_thickness(bead_count)
    }

    fn get_transition_thickness(&self, lower_bead_count: Coord) -> Coord {
        if lower_bead_count == 0 {
            self.min_input_width
        } else {
            self.parent.get_transition_thickness(lower_bead_count)
        }
    }

    fn get_optimal_bead_count(&self, thickness: Coord) -> Coord {
        if thickness < self.min_input_width {
            return 0;
        }
        let ret = self.parent.get_optimal_bead_count(thickness);
        if thickness >= self.min_input_width && ret < 1 {
            return 1;
        }
        ret
    }

    fn get_transitioning_length(&self, lower_bead_count: Coord) -> Coord {
        self.parent.get_transitioning_length(lower_bead_count)
    }

    fn get_transition_anchor_pos(&self, lower_bead_count: Coord) -> f64 {
        self.parent.get_transition_anchor_pos(lower_bead_count)
    }

    fn get_nonlinear_thicknesses(&self, lower_bead_count: Coord) -> Vec<Coord> {
        let mut ret = vec![self.min_output_width];
        ret.extend(self.parent.get_nonlinear_thicknesses(lower_bead_count));
        ret
    }
}

/// Moves the outer wall towards the inside of the part, or outwards for a
/// negative offset (OuterWallInsetBeadingStrategy.hpp and .cpp). As in the
/// reference, the anchor position and the nonlinear thicknesses are the
/// base class's, not the parent's.
pub struct OuterWallInsetBeadingStrategy {
    params: BeadingParams,
    parent: Box<dyn BeadingStrategy>,
    outer_wall_offset: Coord,
}

impl OuterWallInsetBeadingStrategy {
    pub fn new(outer_wall_offset: Coord, parent: Box<dyn BeadingStrategy>) -> Self {
        Self {
            params: *parent.params(),
            parent,
            outer_wall_offset,
        }
    }
}

impl BeadingStrategy for OuterWallInsetBeadingStrategy {
    fn params(&self) -> &BeadingParams {
        &self.params
    }

    fn get_optimal_thickness(&self, bead_count: Coord) -> Coord {
        self.parent.get_optimal_thickness(bead_count)
    }

    fn get_transition_thickness(&self, lower_bead_count: Coord) -> Coord {
        self.parent.get_transition_thickness(lower_bead_count)
    }

    fn get_optimal_bead_count(&self, thickness: Coord) -> Coord {
        self.parent.get_optimal_bead_count(thickness)
    }

    fn get_transitioning_length(&self, lower_bead_count: Coord) -> Coord {
        self.parent.get_transitioning_length(lower_bead_count)
    }

    fn to_string(&self) -> String {
        // The reference spells it this way.
        format!("OuterWallOfsetBeadingStrategy+{}", self.parent.to_string())
    }

    fn compute(&self, thickness: Coord, bead_count: Coord) -> Beading {
        let mut ret = self.parent.compute(thickness, bead_count);

        // The walls that are really there: a wall of no width only marks a
        // place.
        let bead_count = ret.bead_widths.iter().filter(|width| **width > 0).count();

        // A single wall is not moved.
        if bead_count < 2 {
            return ret;
        }

        // Move the outer wall in, never past the middle.
        ret.toolpath_locations[0] = Coord::min(
            ret.toolpath_locations[0] + self.outer_wall_offset,
            thickness / 2,
        );
        ret
    }
}

/// Stops at a number of walls. Where the part is thicker than they cover,
/// a wall of no width is added on each side to mark where the walls end,
/// for infill to line up with (LimitedBeadingStrategy.hpp and .cpp). As in
/// the reference, the nonlinear thicknesses are the base class's: none.
pub struct LimitedBeadingStrategy {
    params: BeadingParams,
    max_bead_count: Coord,
    parent: Box<dyn BeadingStrategy>,
}

impl LimitedBeadingStrategy {
    /// The reference logs a warning for an odd `max_bead_count` and carries
    /// on; this carries on.
    pub fn new(max_bead_count: Coord, parent: Box<dyn BeadingStrategy>) -> Self {
        Self {
            params: *parent.params(),
            max_bead_count,
            parent,
        }
    }
}

impl BeadingStrategy for LimitedBeadingStrategy {
    fn params(&self) -> &BeadingParams {
        &self.params
    }

    fn to_string(&self) -> String {
        format!("LimitedBeadingStrategy+{}", self.parent.to_string())
    }

    fn get_transitioning_length(&self, lower_bead_count: Coord) -> Coord {
        self.parent.get_transitioning_length(lower_bead_count)
    }

    fn get_transition_anchor_pos(&self, lower_bead_count: Coord) -> f64 {
        self.parent.get_transition_anchor_pos(lower_bead_count)
    }

    fn compute(&self, thickness: Coord, bead_count: Coord) -> Beading {
        let max_bead_count = self.max_bead_count;
        if bead_count <= max_bead_count {
            let mut ret = self.parent.compute(thickness, bead_count);
            let bead_count = ret.toolpath_locations.len() as Coord;

            if bead_count % 2 == 0 && bead_count == max_bead_count {
                let innermost_toolpath_location =
                    ret.toolpath_locations[index(max_bead_count / 2 - 1)];
                let innermost_toolpath_width = ret.bead_widths[index(max_bead_count / 2 - 1)];
                ret.toolpath_locations.insert(
                    index(max_bead_count / 2),
                    innermost_toolpath_location + innermost_toolpath_width / 2,
                );
                ret.bead_widths.insert(index(max_bead_count / 2), 0);
            }
            return ret;
        }
        // The reference also logs "Too many beads!" here and carries on.
        debug_assert!(bead_count == max_bead_count + 1);

        let optimal_thickness = self.parent.get_optimal_thickness(max_bead_count);
        let mut ret = self.parent.compute(optimal_thickness, max_bead_count);
        let bead_count = ret.toolpath_locations.len() as Coord;
        ret.left_over += thickness - ret.total_thickness;
        ret.total_thickness = thickness;

        // Make both sides the same.
        if bead_count % 2 == 1 {
            ret.toolpath_locations[index(bead_count / 2)] = thickness / 2;
            ret.bead_widths[index(bead_count / 2)] = thickness - optimal_thickness;
        }
        for bead_idx in 0..(bead_count + 1) / 2 {
            let mirrored = thickness - ret.toolpath_locations[index(bead_idx)];
            ret.toolpath_locations[index(bead_count - 1 - bead_idx)] = mirrored;
        }

        // A wall of no width at the inner edge of the walls, for whatever
        // fills the area next to them.
        let innermost_toolpath_location = ret.toolpath_locations[index(max_bead_count / 2 - 1)];
        let innermost_toolpath_width = ret.bead_widths[index(max_bead_count / 2 - 1)];
        ret.toolpath_locations.insert(
            index(max_bead_count / 2),
            innermost_toolpath_location + innermost_toolpath_width / 2,
        );
        ret.bead_widths.insert(index(max_bead_count / 2), 0);

        // And the same on the other side.
        let opposite_bead = index(bead_count - (max_bead_count / 2 - 1));
        let innermost_toolpath_location = ret.toolpath_locations[opposite_bead];
        let innermost_toolpath_width = ret.bead_widths[opposite_bead];
        ret.toolpath_locations.insert(
            opposite_bead,
            innermost_toolpath_location - innermost_toolpath_width / 2,
        );
        ret.bead_widths.insert(opposite_bead, 0);

        ret
    }

    fn get_optimal_thickness(&self, bead_count: Coord) -> Coord {
        if bead_count <= self.max_bead_count {
            return self.parent.get_optimal_thickness(bead_count);
        }
        debug_assert!(false, "more beads than the limit");
        SCALED_1000 // 1 metre
    }

    fn get_transition_thickness(&self, lower_bead_count: Coord) -> Coord {
        if lower_bead_count < self.max_bead_count {
            return self.parent.get_transition_thickness(lower_bead_count);
        }

        if lower_bead_count == self.max_bead_count {
            return self.parent.get_optimal_thickness(lower_bead_count + 1) - SCALED_0_01;
        }

        debug_assert!(false, "more beads than the limit");
        SCALED_900 // 0.9 metre
    }

    fn get_optimal_bead_count(&self, thickness: Coord) -> Coord {
        let max_bead_count = self.max_bead_count;
        let parent_bead_count = self.parent.get_optimal_bead_count(thickness);
        if parent_bead_count <= max_bead_count {
            self.parent.get_optimal_bead_count(thickness)
        } else if parent_bead_count == max_bead_count + 1 {
            if thickness < self.parent.get_optimal_thickness(max_bead_count + 1) - SCALED_0_01 {
                max_bead_count
            } else {
                max_bead_count + 1
            }
        } else {
            max_bead_count + 1
        }
    }
}

/// Stacks the strategies for one set of wall settings
/// (`BeadingStrategyFactory::makeStrategy`, BeadingStrategyFactory.hpp and
/// .cpp): the distributed one, the outer walls kept at their width, thin
/// walls if asked for, the outer wall's offset if there is one, and the
/// limit on the number of walls last, since nothing else may touch the
/// walls of no width it adds.
///
/// The reference gives every parameter a default; its one caller
/// (WallToolPaths.cpp) leaves out only the last, which is 0.5 there.
#[allow(clippy::too_many_arguments)]
pub fn make_strategy(
    preferred_bead_width_outer: Coord,
    preferred_bead_width_inner: Coord,
    preferred_transition_length: Coord,
    transitioning_angle: f32,
    print_thin_walls: bool,
    min_bead_width: Coord,
    min_feature_size: Coord,
    wall_split_middle_threshold: f64,
    wall_add_middle_threshold: f64,
    max_bead_count: Coord,
    outer_wall_offset: Coord,
    inward_distributed_center_wall_count: i32,
    minimum_variable_line_ratio: f64,
) -> Box<dyn BeadingStrategy> {
    // With a single outer wall on each side the outer width is used
    // throughout: a large difference between the outer and inner widths
    // does not suit the strategies.
    let optimal_width = if max_bead_count <= 2 {
        preferred_bead_width_outer
    } else {
        preferred_bead_width_inner
    };
    let mut ret: Box<dyn BeadingStrategy> = Box::new(DistributedBeadingStrategy::new(
        optimal_width,
        preferred_transition_length,
        f64::from(transitioning_angle),
        wall_split_middle_threshold,
        wall_add_middle_threshold,
        inward_distributed_center_wall_count,
    ));

    ret = Box::new(RedistributeBeadingStrategy::new(
        preferred_bead_width_outer,
        minimum_variable_line_ratio,
        ret,
    ));

    if print_thin_walls {
        ret = Box::new(WideningBeadingStrategy::new(
            ret,
            min_feature_size,
            min_bead_width,
        ));
    }
    // A negative offset is allowed.
    if outer_wall_offset != 0 {
        ret = Box::new(OuterWallInsetBeadingStrategy::new(outer_wall_offset, ret));
    }

    Box::new(LimitedBeadingStrategy::new(max_bead_count, ret))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn angle() -> f32 {
        10.0f32.to_radians()
    }

    fn distributed(optimal_width: Coord, radius: i32) -> DistributedBeadingStrategy {
        DistributedBeadingStrategy::new(
            optimal_width,
            400_000,
            f64::from(angle()),
            0.5,
            0.5,
            radius,
        )
    }

    /// Outer walls of 420_000 around inner ones of 450_000.
    fn redistribute() -> RedistributeBeadingStrategy {
        RedistributeBeadingStrategy::new(420_000, 0.5, Box::new(distributed(450_000, 1)))
    }

    fn typical(
        print_thin_walls: bool,
        max_bead_count: Coord,
        outer_wall_offset: Coord,
    ) -> Box<dyn BeadingStrategy> {
        make_strategy(
            420_000,
            450_000,
            400_000,
            angle(),
            print_thin_walls,
            340_000,
            100_000,
            0.5,
            0.5,
            max_bead_count,
            outer_wall_offset,
            1,
            0.5,
        )
    }

    fn beading(
        total_thickness: Coord,
        bead_widths: &[Coord],
        toolpath_locations: &[Coord],
        left_over: Coord,
    ) -> Beading {
        Beading {
            total_thickness,
            bead_widths: bead_widths.to_vec(),
            toolpath_locations: toolpath_locations.to_vec(),
            left_over,
        }
    }

    #[test]
    fn distributed_two_beads_halve_the_thickness() {
        let b = distributed(400_000, 1).compute(1_000_000, 2);
        assert_eq!(
            b,
            beading(1_000_000, &[500_000, 500_000], &[250_000, 750_000], 0)
        );
        assert_eq!(b.bead_widths.iter().sum::<Coord>(), 1_000_000);
    }

    #[test]
    fn distributed_three_beads_put_the_difference_in_the_middle() {
        // 200_000 too little for three of 400_000; with a radius of one
        // bead only the middle one has any weight.
        let b = distributed(400_000, 1).compute(1_000_000, 3);
        assert_eq!(
            b,
            beading(
                1_000_000,
                &[400_000, 200_000, 400_000],
                &[200_000, 500_000, 800_000],
                0
            )
        );
        assert_eq!(b.bead_widths.iter().sum::<Coord>(), 1_000_000);
        // A radius of two is one over (2 - 1) squared: the same.
        assert_eq!(distributed(400_000, 2).compute(1_000_000, 3), b);
    }

    #[test]
    fn distributed_wider_radius_shares_the_difference() {
        // Radius 3: weights 0.75, 1, 0.75, so shares of 0.3, 0.4, 0.3 of
        // -200_000, worked in f32 and cut towards zero.
        let b = distributed(400_000, 3).compute(1_000_000, 3);
        assert_eq!(
            b,
            beading(
                1_000_000,
                &[340_000, 320_000, 340_000],
                &[170_000, 500_000, 830_000],
                0
            )
        );
    }

    #[test]
    fn distributed_one_and_no_beads() {
        let s = distributed(400_000, 1);
        assert_eq!(
            s.compute(300_001, 1),
            beading(300_001, &[300_001], &[150_000], 0)
        );
        assert_eq!(s.compute(300_000, 0), beading(300_000, &[], &[], 300_000));
        assert_eq!(s.compute(300_000, -1), beading(300_000, &[], &[], 300_000));
    }

    #[test]
    fn distributed_counts_and_thicknesses() {
        let s = distributed(400_000, 1);
        assert_eq!(s.get_optimal_thickness(0), 0);
        assert_eq!(s.get_optimal_thickness(3), 1_200_000);
        assert_eq!(s.get_transition_thickness(0), 200_000);
        assert_eq!(s.get_transition_thickness(2), 1_000_000);
        assert_eq!(s.get_optimal_bead_count(999_999), 2);
        assert_eq!(s.get_optimal_bead_count(1_000_000), 3);
        // The count changes exactly at the transition thickness.
        for n in 0..6 {
            let t = s.get_transition_thickness(n);
            assert_eq!(s.get_optimal_bead_count(t), n + 1);
            assert_eq!(s.get_optimal_bead_count(t - 1), n);
            assert_eq!(s.get_optimal_bead_count(s.get_optimal_thickness(n)), n);
        }
    }

    #[test]
    fn distributed_thresholds_differ_for_odd_and_even() {
        // Split a middle line at a quarter, add one at three quarters.
        let s = DistributedBeadingStrategy::new(400_000, 400_000, 0.5, 0.25, 0.75, 1);
        assert_eq!(s.get_transition_thickness(0), 300_000);
        assert_eq!(s.get_transition_thickness(1), 500_000);
        assert_eq!(s.get_transition_thickness(2), 1_100_000);
        assert_eq!(s.get_optimal_bead_count(499_999), 1);
        assert_eq!(s.get_optimal_bead_count(500_000), 2);
        assert_eq!(s.get_optimal_bead_count(1_099_999), 2);
        assert_eq!(s.get_optimal_bead_count(1_100_000), 3);
        assert_eq!(s.get_transition_anchor_pos(0), 0.25);
        assert_eq!(s.get_transition_anchor_pos(1), 0.75);
        assert_eq!(s.get_split_middle_threshold(), 0.25);
        assert_eq!(s.get_add_middle_threshold(), 0.75);
    }

    #[test]
    fn base_class_answers() {
        let s = distributed(400_000, 1);
        assert_eq!(s.get_transitioning_length(0), 10_000);
        assert_eq!(s.get_transitioning_length(1), 400_000);
        assert_eq!(s.get_transition_anchor_pos(0), 0.5);
        assert_eq!(s.get_transition_anchor_pos(3), 0.5);
        assert!(s.get_nonlinear_thicknesses(1).is_empty());
        assert_eq!(s.get_optimal_width(), 400_000);
        assert_eq!(s.get_default_transition_length(), 400_000);
        // The angle goes through a float on its way in.
        assert_eq!(s.get_transitioning_angle(), f64::from(angle()));
        let narrowed = DistributedBeadingStrategy::new(400_000, 400_000, 0.1, 0.5, 0.5, 1);
        assert_eq!(narrowed.get_transitioning_angle(), f64::from(0.1f32));
        assert_eq!(s.to_string(), "DistributedBeadingStrategy");
    }

    #[test]
    fn redistribute_outer_walls_keep_their_width() {
        let s = redistribute();
        assert_eq!(
            s.compute(1_500_000, 3),
            beading(
                1_500_000,
                &[420_000, 660_000, 420_000],
                &[210_000, 750_000, 1_290_000],
                0
            )
        );
        assert_eq!(
            s.compute(2_000_000, 4),
            beading(
                2_000_000,
                &[420_000, 580_000, 580_000, 420_000],
                &[210_000, 710_000, 1_290_000, 1_790_000],
                0
            )
        );
    }

    #[test]
    fn redistribute_thin_parts() {
        let s = redistribute();
        // Two walls and one wall share the thickness evenly.
        assert_eq!(
            s.compute(1_000_000, 2),
            beading(1_000_000, &[500_000, 500_000], &[250_000, 750_000], 0)
        );
        assert_eq!(
            s.compute(500_000, 1),
            beading(500_000, &[500_000], &[250_000], 0)
        );
        // Three asked for, but no room between two outer walls: two come
        // back, each half the thickness.
        assert_eq!(
            s.compute(800_000, 3),
            beading(800_000, &[400_000, 400_000], &[200_000, 600_000], 0)
        );
        // Below half an outer wall, and for no beads, nothing.
        assert_eq!(s.compute(209_999, 1), beading(209_999, &[], &[], 209_999));
        assert_eq!(s.compute(210_000, 1).bead_widths, vec![210_000]);
        assert_eq!(
            s.compute(1_000_000, 0),
            beading(1_000_000, &[], &[], 1_000_000)
        );
    }

    #[test]
    fn redistribute_counts_and_thicknesses() {
        let s = redistribute();
        assert_eq!(s.get_optimal_thickness(0), 0);
        assert_eq!(s.get_optimal_thickness(1), 420_000);
        assert_eq!(s.get_optimal_thickness(2), 840_000);
        assert_eq!(s.get_optimal_thickness(3), 1_290_000);
        assert_eq!(s.get_optimal_thickness(4), 1_740_000);
        assert_eq!(s.get_transition_thickness(0), 210_000);
        assert_eq!(s.get_transition_thickness(1), 630_000);
        assert_eq!(s.get_transition_thickness(2), 1_065_000);
        assert_eq!(s.get_transition_thickness(3), 1_515_000);

        assert_eq!(s.get_optimal_bead_count(209_999), 0);
        assert_eq!(s.get_optimal_bead_count(210_000), 1);
        // One becomes two just past the transition thickness, not at it.
        assert_eq!(s.get_optimal_bead_count(630_000), 1);
        assert_eq!(s.get_optimal_bead_count(630_001), 2);
        assert_eq!(s.get_optimal_bead_count(840_000), 2);
        assert_eq!(s.get_optimal_bead_count(840_001), 2);
        assert_eq!(s.get_optimal_bead_count(1_064_999), 2);
        assert_eq!(s.get_optimal_bead_count(1_065_000), 3);
        assert_eq!(s.get_optimal_bead_count(1_514_999), 3);
        assert_eq!(s.get_optimal_bead_count(1_515_000), 4);

        // Passed on to the parent, whose own thicknesses these come from.
        assert_eq!(s.get_transitioning_length(0), 10_000);
        assert_eq!(s.get_transitioning_length(2), 400_000);
        assert_eq!(s.get_transition_anchor_pos(1), 0.5);
        assert!(s.get_nonlinear_thicknesses(2).is_empty());
        // The base fields are the parent's.
        assert_eq!(s.get_optimal_width(), 450_000);
        assert_eq!(
            s.to_string(),
            "RedistributeBeadingStrategy+DistributedBeadingStrategy"
        );
    }

    fn widening() -> WideningBeadingStrategy {
        WideningBeadingStrategy::new(Box::new(redistribute()), 100_000, 340_000)
    }

    #[test]
    fn widening_thin_part_gets_one_bead() {
        let s = widening();
        // Thinner than a line, but a feature: one bead of the least width,
        // centred on the part.
        assert_eq!(
            s.compute(200_000, 1),
            beading(200_000, &[340_000], &[100_000], 0)
        );
        assert_eq!(s.compute(100_000, 1).bead_widths, vec![340_000]);
        // Wider than the least width: its own thickness.
        assert_eq!(
            s.compute(500_000, 1),
            beading(500_000, &[500_000], &[250_000], 0)
        );
        // Too thin to be a feature.
        assert_eq!(s.compute(99_999, 1), beading(99_999, &[], &[], 99_999));
        // A count of none still gives the bead: only the thickness is asked.
        assert_eq!(
            s.compute(200_000, 0),
            beading(200_000, &[340_000], &[100_000], 0)
        );
    }

    #[test]
    fn widening_leaves_the_rest_to_its_parent() {
        let s = widening();
        let parent = redistribute();
        // Two asked for in the thin band: two come back.
        assert_eq!(
            s.compute(500_000, 2),
            beading(500_000, &[250_000, 250_000], &[125_000, 375_000], 0)
        );
        // At the thickness where one becomes two, no longer thin.
        assert_eq!(s.compute(630_000, 1), parent.compute(630_000, 1));
        assert_eq!(s.compute(2_000_000, 4), parent.compute(2_000_000, 4));
        assert_eq!(s.get_optimal_thickness(3), 1_290_000);
        assert_eq!(s.get_transition_thickness(1), 630_000);
        assert_eq!(s.get_transitioning_length(0), 10_000);
        assert_eq!(s.get_transition_anchor_pos(2), 0.5);
    }

    #[test]
    fn widening_counts_and_thicknesses() {
        let s = widening();
        assert_eq!(s.get_transition_thickness(0), 100_000);
        assert_eq!(s.get_optimal_bead_count(99_999), 0);
        assert_eq!(s.get_optimal_bead_count(100_000), 1);
        assert_eq!(s.get_optimal_bead_count(300_000), 1);
        assert_eq!(s.get_optimal_bead_count(630_001), 2);
        assert_eq!(s.get_nonlinear_thicknesses(0), vec![340_000]);
        assert_eq!(
            s.to_string(),
            "Widening+RedistributeBeadingStrategy+DistributedBeadingStrategy"
        );
        // Two of them: the outer one's width first.
        let twice = WideningBeadingStrategy::new(Box::new(widening()), 50_000, 200_000);
        assert_eq!(twice.get_nonlinear_thicknesses(0), vec![200_000, 340_000]);
    }

    #[test]
    fn outer_wall_inset_moves_the_first_location() {
        let s = OuterWallInsetBeadingStrategy::new(50_000, Box::new(redistribute()));
        assert_eq!(
            s.compute(2_000_000, 4),
            beading(
                2_000_000,
                &[420_000, 580_000, 580_000, 420_000],
                &[260_000, 710_000, 1_290_000, 1_790_000],
                0
            )
        );
        // A single wall stays where it is.
        assert_eq!(
            s.compute(500_000, 1),
            beading(500_000, &[500_000], &[250_000], 0)
        );
        // Never past the middle.
        let far = OuterWallInsetBeadingStrategy::new(200_000, Box::new(redistribute()));
        assert_eq!(
            far.compute(500_000, 2).toolpath_locations,
            vec![250_000, 375_000]
        );
        // A negative offset moves it out.
        let out = OuterWallInsetBeadingStrategy::new(-50_000, Box::new(redistribute()));
        assert_eq!(
            out.compute(2_000_000, 4).toolpath_locations,
            vec![160_000, 710_000, 1_290_000, 1_790_000]
        );
    }

    #[test]
    fn outer_wall_inset_passes_on_and_falls_back() {
        let parent = redistribute();
        let s = OuterWallInsetBeadingStrategy::new(50_000, Box::new(widening()));
        assert_eq!(s.get_optimal_thickness(3), parent.get_optimal_thickness(3));
        assert_eq!(s.get_transition_thickness(0), 100_000);
        assert_eq!(s.get_transition_thickness(2), 1_065_000);
        assert_eq!(s.get_optimal_bead_count(150_000), 1);
        assert_eq!(s.get_transitioning_length(0), 10_000);
        // Not passed on in the reference: the widening parent's are lost.
        assert!(s.get_nonlinear_thicknesses(0).is_empty());
        assert_eq!(
            s.to_string(),
            "OuterWallOfsetBeadingStrategy+Widening+RedistributeBeadingStrategy+DistributedBeadingStrategy"
        );

        // Nor is the anchor position: it is worked out from this strategy's
        // own thicknesses, where the parent asks the distributed one.
        let quarter =
            || RedistributeBeadingStrategy::new(420_000, 0.25, Box::new(distributed(450_000, 1)));
        assert_eq!(quarter().get_transition_thickness(0), 105_000);
        assert_eq!(quarter().get_transition_anchor_pos(0), 0.5);
        let inset = OuterWallInsetBeadingStrategy::new(50_000, Box::new(quarter()));
        assert_eq!(inset.get_transition_anchor_pos(0), 0.75);
    }

    fn limited() -> LimitedBeadingStrategy {
        LimitedBeadingStrategy::new(4, Box::new(redistribute()))
    }

    #[test]
    fn limited_marks_the_middle_at_the_limit() {
        let s = limited();
        // As many beads as the limit: one of no width between the two
        // halves, at the inner edge of the second wall.
        assert_eq!(
            s.compute(2_000_000, 4),
            beading(
                2_000_000,
                &[420_000, 580_000, 0, 580_000, 420_000],
                &[210_000, 710_000, 1_000_000, 1_290_000, 1_790_000],
                0
            )
        );
        // Fewer: the parent's answer.
        assert_eq!(
            s.compute(1_500_000, 3),
            redistribute().compute(1_500_000, 3)
        );
        assert_eq!(
            s.compute(1_000_000, 2),
            redistribute().compute(1_000_000, 2)
        );
        assert_eq!(s.compute(100_000, 0), beading(100_000, &[], &[], 100_000));
    }

    #[test]
    fn limited_adds_zero_width_beads_beyond_the_limit() {
        // Four walls at their best widths against each side, and a bead of
        // no width at the inner edge of each pair.
        assert_eq!(
            limited().compute(2_500_000, 5),
            beading(
                2_500_000,
                &[420_000, 450_000, 0, 0, 450_000, 420_000],
                &[210_000, 645_000, 870_000, 1_630_000, 1_855_000, 2_290_000],
                760_000
            )
        );
    }

    #[test]
    fn limited_counts_and_thicknesses() {
        let s = limited();
        assert_eq!(s.get_optimal_thickness(4), 1_740_000);
        assert_eq!(s.get_transition_thickness(3), 1_515_000);
        // To one more than the limit: 0.01 mm short of what the parent
        // would like for five.
        assert_eq!(s.get_transition_thickness(4), 2_180_000);

        assert_eq!(s.get_optimal_bead_count(1_514_999), 3);
        assert_eq!(s.get_optimal_bead_count(1_515_000), 4);
        // The parent says five from 1_965_000; held at four until the
        // transition thickness.
        assert_eq!(redistribute().get_optimal_bead_count(1_965_000), 5);
        assert_eq!(s.get_optimal_bead_count(1_965_000), 4);
        assert_eq!(s.get_optimal_bead_count(2_179_999), 4);
        assert_eq!(s.get_optimal_bead_count(2_180_000), 5);
        // Never more than one past the limit.
        assert_eq!(redistribute().get_optimal_bead_count(3_000_000), 7);
        assert_eq!(s.get_optimal_bead_count(3_000_000), 5);

        for n in [0, 2, 3, 4] {
            let t = s.get_transition_thickness(n);
            assert_eq!(s.get_optimal_bead_count(t), n + 1);
            assert_eq!(s.get_optimal_bead_count(t - 1), n);
        }

        assert_eq!(s.get_transitioning_length(0), 10_000);
        assert_eq!(s.get_transitioning_length(4), 400_000);
        assert_eq!(s.get_transition_anchor_pos(4), 0.5);
        assert_eq!(
            s.to_string(),
            "LimitedBeadingStrategy+RedistributeBeadingStrategy+DistributedBeadingStrategy"
        );
    }

    #[test]
    fn limited_drops_the_nonlinear_thicknesses() {
        // Not passed on in the reference.
        let s = LimitedBeadingStrategy::new(4, Box::new(widening()));
        assert!(s.get_nonlinear_thicknesses(0).is_empty());
    }

    #[test]
    fn factory_wrapping_order() {
        assert_eq!(
            typical(true, 4, 0).to_string(),
            "LimitedBeadingStrategy+Widening+RedistributeBeadingStrategy+DistributedBeadingStrategy"
        );
        assert_eq!(
            typical(false, 4, 0).to_string(),
            "LimitedBeadingStrategy+RedistributeBeadingStrategy+DistributedBeadingStrategy"
        );
        assert_eq!(
            typical(true, 4, 30_000).to_string(),
            "LimitedBeadingStrategy+OuterWallOfsetBeadingStrategy+Widening+RedistributeBeadingStrategy+DistributedBeadingStrategy"
        );
        assert_eq!(
            typical(false, 4, -30_000).to_string(),
            "LimitedBeadingStrategy+OuterWallOfsetBeadingStrategy+RedistributeBeadingStrategy+DistributedBeadingStrategy"
        );
    }

    #[test]
    fn factory_picks_the_optimal_width() {
        // More than one wall a side: the inner width. One: the outer.
        assert_eq!(typical(true, 4, 0).get_optimal_width(), 450_000);
        assert_eq!(typical(true, 3, 0).get_optimal_width(), 450_000);
        assert_eq!(typical(true, 2, 0).get_optimal_width(), 420_000);
    }

    #[test]
    fn factory_typical_settings() {
        let s = typical(true, 4, 0);
        assert_eq!(s.get_default_transition_length(), 400_000);
        assert_eq!(s.get_split_middle_threshold(), 0.5);
        assert_eq!(s.get_add_middle_threshold(), 0.5);
        assert_eq!(s.get_transitioning_angle(), f64::from(angle()));

        assert_eq!(s.get_transition_thickness(0), 100_000);
        assert_eq!(s.get_transition_thickness(1), 630_000);
        assert_eq!(s.get_transition_thickness(4), 2_180_000);
        assert_eq!(s.get_optimal_bead_count(50_000), 0);
        assert_eq!(s.get_optimal_bead_count(150_000), 1);
        assert_eq!(s.get_optimal_bead_count(700_000), 2);
        assert_eq!(s.get_optimal_bead_count(2_000_000), 4);
        assert_eq!(s.get_optimal_bead_count(5_000_000), 5);
        assert_eq!(s.get_transitioning_length(0), 10_000);
        assert_eq!(s.get_transitioning_length(1), 400_000);
        assert_eq!(s.get_transition_anchor_pos(1), 0.5);
        assert!(s.get_nonlinear_thicknesses(0).is_empty());

        // A thin wall, widened.
        assert_eq!(
            s.compute(150_000, 1),
            beading(150_000, &[340_000], &[75_000], 0)
        );
        // Four walls with the marker between the halves.
        assert_eq!(
            s.compute(2_000_000, 4),
            beading(
                2_000_000,
                &[420_000, 580_000, 0, 580_000, 420_000],
                &[210_000, 710_000, 1_000_000, 1_290_000, 1_790_000],
                0
            )
        );
        // Thicker than four walls cover.
        assert_eq!(
            s.compute(5_000_000, 5),
            beading(
                5_000_000,
                &[420_000, 450_000, 0, 0, 450_000, 420_000],
                &[210_000, 645_000, 870_000, 4_130_000, 4_355_000, 4_790_000],
                3_260_000
            )
        );
    }

    #[test]
    fn factory_outer_wall_offset() {
        let s = typical(true, 4, 30_000);
        // The marker is added after the outer wall has moved.
        assert_eq!(
            s.compute(2_000_000, 4).toolpath_locations,
            vec![240_000, 710_000, 1_000_000, 1_290_000, 1_790_000]
        );
        // Beyond the limit the far side mirrors the moved wall.
        assert_eq!(
            s.compute(2_500_000, 5).toolpath_locations,
            vec![240_000, 645_000, 870_000, 1_630_000, 1_855_000, 2_260_000]
        );
    }

    #[test]
    fn strategies_are_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Box<dyn BeadingStrategy>>();
    }
}
