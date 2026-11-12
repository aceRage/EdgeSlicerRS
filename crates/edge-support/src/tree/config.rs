//! The numbers organic trees are built by, worked out once from the
//! settings (TreeSupportCommon.hpp `TreeSupportMeshGroupSettings` and
//! `TreeSupportSettings`). Lengths are in scaled units, as the
//! reference's `coord_t`, and the reference's integer arithmetic is kept
//! where it rounds.

use edge_geom::{Coord, scaled};

use crate::{Job, SupportError};

/// The reference's `EPSILON`.
pub(crate) const EPSILON: f64 = 1e-4;
/// `SCALED_EPSILON`.
pub(crate) const SCALED_EPSILON: Coord = 100;

#[derive(Clone, Debug)]
pub(crate) struct TreeConfig {
    /// Width of a support line, and of a roof's.
    pub line_width: Coord,
    pub roof_line_width: Coord,
    pub layer_height: Coord,
    /// A branch's radius once it has left its tip, and the tip's.
    pub branch_radius: Coord,
    pub min_radius: Coord,
    /// How far an influence area may move outward on one layer: at most,
    /// and as a rule.
    pub max_move: Coord,
    pub max_move_slow: Coord,
    pub tip_layers: usize,
    pub branch_radius_increase_per_layer: f64,
    pub max_to_model_radius_increase: Coord,
    pub min_dtt_to_model: usize,
    pub increase_radius_until_radius: Coord,
    pub increase_radius_until_layer: usize,
    pub rests_on_model: bool,
    pub xy_distance: Coord,
    pub xy_min_distance: Coord,
    /// How much a trunk may gain a layer spreading to 7.5 mm across at the
    /// bed, and the layer from which it starts.
    pub bp_radius_increase_per_layer: f64,
    pub layer_start_bp_radius: i64,
    pub z_distance_top_layers: usize,
    pub z_distance_bottom_layers: usize,
    pub resolution: Coord,
    pub min_feature_size: Coord,
    pub branch_distance: Coord,
    /// `tree_support_top_rate`, in percent.
    pub top_rate: f64,
    pub roof_layers: usize,
    pub floor_layers: usize,
    /// Distance between roof lines, centre to centre.
    pub roof_line_distance: Coord,
    /// Smallest roof worth printing, in scaled units squared.
    pub minimum_roof_area: f64,
    /// `support_line_width` as a spacing, and the roof's.
    pub line_spacing: Coord,
    pub roof_line_spacing: Coord,
}

impl TreeConfig {
    pub fn new(job: &Job<'_>) -> Result<Self, SupportError> {
        let s = job.settings;
        let flow = s.flow(job.layer_height, false)?;
        let layer_height = scaled(job.layer_height);
        let line_width = scaled(flow.width);
        let external_perimeter_width = s.wall_width(job.layer_height)?;
        let top_gap = s.top_z_distance.max(0.0);
        let bottom_gap = s.bottom_z_distance.max(0.0);
        // Slicing.cpp: a gap of zero under an interface is a soluble one.
        let zero_top = top_gap == 0.0 && s.interface_top_layers > 0;
        let mut xy_distance = scaled(s.xy_distance);
        if s.top_z_overrides_xy {
            xy_distance = xy_distance.min(scaled(top_gap).max(scaled(0.2)));
        }
        let xy_overhang = xy_distance.min(scaled(0.5 * external_perimeter_width));
        let mut xy_min_distance = xy_distance.min(xy_overhang);
        if zero_top {
            // TreeSupportSettings: with no gap above, the smaller
            // distance has to be a little more than nothing.
            xy_min_distance = xy_min_distance.max(scaled(0.1));
            xy_distance = xy_distance.max(xy_min_distance);
        }
        let clamp_angle = |degrees: f64, most: f64| (degrees.to_radians()).clamp(0.0, most);
        let angle = clamp_angle(s.tree_branch_angle, 0.5 * std::f64::consts::PI - EPSILON);
        let angle_slow = clamp_angle(s.tree_angle_slow, angle - EPSILON);
        let diameter_angle = clamp_angle(
            s.tree_branch_diameter_angle,
            0.5 * std::f64::consts::PI - EPSILON,
        );
        let branch_diameter = scaled(s.tree_branch_diameter);
        let branch_radius = branch_diameter / 2;
        let min_radius = scaled(s.tree_tip_diameter).clamp(0, branch_diameter) / 2;
        let tip_layers = ((branch_radius - min_radius) / (line_width / 3).max(1))
            .max(branch_radius / layer_height.max(1))
            .max(1) as usize;
        let bp_radius = scaled(7.5) / 2;
        let bp_radius_increase_per_layer =
            (0.7_f64.tan() * layer_height as f64).min(0.5 * line_width as f64);
        let increase_radius_until_radius = branch_diameter / 2;
        let increase_radius_until_layer = if increase_radius_until_radius <= branch_radius {
            tip_layers * (increase_radius_until_radius / branch_radius.max(1)) as usize
        } else {
            0
        };
        let layers_in = |gap: f64| (scaled(gap) as f64 / layer_height as f64).round() as usize;
        let nozzle = s.nozzle_diameter;
        let roof_line_width = line_width;
        Ok(Self {
            line_width,
            roof_line_width,
            layer_height,
            branch_radius,
            min_radius,
            max_move: (angle.tan() * layer_height as f64) as Coord,
            max_move_slow: (angle_slow.tan() * layer_height as f64) as Coord,
            tip_layers,
            branch_radius_increase_per_layer: diameter_angle.tan() * layer_height as f64,
            max_to_model_radius_increase: scaled(1.0) / 2,
            min_dtt_to_model: round_up_divide(scaled(1.0), layer_height) as usize,
            increase_radius_until_radius,
            increase_radius_until_layer,
            rests_on_model: !s.on_build_plate_only,
            xy_distance,
            xy_min_distance,
            bp_radius_increase_per_layer,
            layer_start_bp_radius: ((bp_radius - branch_radius) as f64
                / bp_radius_increase_per_layer) as i64,
            z_distance_top_layers: if zero_top { 0 } else { layers_in(top_gap) },
            z_distance_bottom_layers: layers_in(bottom_gap),
            resolution: scaled(s.resolution.max(0.0)),
            min_feature_size: scaled(s.min_feature_size.resolve(nozzle).max(0.0)),
            branch_distance: scaled(s.tree_branch_distance.max(0.0)),
            top_rate: (s.tree_top_rate * 100.0).max(1.0),
            roof_layers: s.interface_top_layers,
            floor_layers: s.interface_bottom_layers,
            roof_line_distance: scaled(s.interface_spacing) + roof_line_width,
            minimum_roof_area: (scaled(1.0) as f64).powi(2),
            line_spacing: flow.scaled_spacing(),
            roof_line_spacing: flow.scaled_spacing(),
        })
    }

    /// The radius of a branch `distance_to_top` layers under its tip,
    /// grown `elephant_foot_increases` times towards its foot
    /// (TreeSupportSettings::getRadius).
    pub fn radius(&self, distance_to_top: usize, elephant_foot_increases: f64) -> Coord {
        let base = if distance_to_top <= self.tip_layers {
            (self.min_radius
                + (self.branch_radius - self.min_radius) * distance_to_top as Coord
                    / self.tip_layers as Coord) as f64
        } else {
            self.branch_radius as f64
                + (distance_to_top - self.tip_layers) as f64 * self.branch_radius_increase_per_layer
        };
        (base
            + elephant_foot_increases
                * (self.bp_radius_increase_per_layer - self.branch_radius_increase_per_layer)
                    .max(0.0)) as Coord
    }

    /// The radius every branch tries to reach on a layer near the bed
    /// (TreeSupportSettings::recommendedMinRadius).
    pub fn recommended_min_radius(&self, layer: i64) -> Coord {
        let widened = (self.layer_start_bp_radius - layer) as f64;
        if widened > 0.0 {
            (self.branch_radius as f64 + widened * self.bp_radius_increase_per_layer) as Coord
        } else {
            0
        }
    }

    /// Tips are this far apart along the lines they are set on
    /// (TreeSupport3D.cpp `generate_initial_areas`).
    pub fn connect_length(&self) -> Coord {
        (self.line_width as f64 * 100.0 / self.top_rate
            + (2.0 * self.min_radius as f64 - self.line_width as f64).max(0.0)) as Coord
    }

    /// The step offsets of influence areas are taken in, so that they
    /// cannot jump a wall (`safe_movement_distance`).
    pub fn safe_movement_distance(&self, use_min_xy_dist: bool) -> Coord {
        let distance = if use_min_xy_dist {
            self.xy_min_distance
        } else {
            self.xy_distance
        } + if self
            .z_distance_top_layers
            .min(self.z_distance_bottom_layers)
            > 0
        {
            self.min_feature_size
        } else {
            0
        };
        // Orca: never zero, as it divides and steps.
        if distance == 0 { scaled(0.1) } else { distance }
    }
}

pub(crate) fn round_up_divide(a: Coord, b: Coord) -> Coord {
    if b <= 0 { a } else { (a + b - 1) / b }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SupportSettings;
    use edge_geom::unscaled;

    #[test]
    fn a_branch_grows_as_the_reference_has_it() {
        let settings = SupportSettings {
            line_width: edge_config::FloatOrPercent::Value(0.42),
            ..SupportSettings::default()
        };
        let job = Job {
            slices: &[],
            first_layer_height: 0.2,
            layer_height: 0.2,
            settings: &settings,
            gap_above: 1,
            gap_below: 1,
            bed: None,
        };
        let config = TreeConfig::new(&job).unwrap();
        // The reference's worked example at 0.2 mm layers, 0.42 mm lines.
        assert_eq!(config.tip_layers, 5);
        assert_eq!(config.increase_radius_until_layer, 5);
        assert_eq!(config.min_dtt_to_model, 5);
        assert_eq!(
            config.max_move_slow,
            (25f64.to_radians().tan() * 2e5) as Coord
        );
        assert_eq!(config.radius(0, 0.0), scaled(0.4));
        assert_eq!(config.radius(5, 0.0), scaled(1.0));
        let grown = 1.0 + 100.0 * 5f64.to_radians().tan() * 0.2;
        assert!((unscaled(config.radius(105, 0.0)) - grown).abs() < 1e-5);
        // Near the bed a trunk aims for 7.5 mm across, gaining no more
        // than half a line a layer.
        assert_eq!(
            config.recommended_min_radius(config.layer_start_bp_radius),
            0
        );
        assert!(config.recommended_min_radius(0) <= scaled(3.75));
        assert!(config.recommended_min_radius(0) > scaled(3.5));
        // Tips are 1.4 mm apart on a line, and a bit more for the tip's width.
        assert!((unscaled(config.connect_length()) - (1.4 + 0.38)).abs() < 1e-5);
        // Beside the overhang the smaller distance holds: half the outer
        // wall (0.4 mm by default).
        assert_eq!(config.xy_min_distance, scaled(0.2));
    }
}
