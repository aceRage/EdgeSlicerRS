//! Settings that belong to one object, one part or one region of a print:
//! the reference's region model (PrintApply.cpp `generate_print_object_regions`,
//! PrintObject.cpp `region_config_from_model_volume`).
//!
//! The reference slices each object by itself and gives every set of its
//! volumes that share one region config a `PrintRegion`; each stage reads
//! a region-level option from the region and an object-level one from the
//! object. Here everything on a plate is sliced together, and each layer
//! is shared out among keys, a key standing for one region: a filament
//! number for a region with the plate's settings in that filament, as it
//! always was, and a number from [`BASE`] up for one with settings of its
//! own. Each stage that reads region-level settings reads them from the
//! region's own [`Settings`] (the plate's with the region's laid over
//! them), so a plate without settings of its own anywhere has one region
//! per filament, keyed by its number, and slices as it did before.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

use edge_config::{Reads, Settings, Value};

/// The first key of a region with settings of its own. Below it, a key is
/// a filament number and its region has the plate's settings.
pub(crate) const BASE: usize = 1 << 20;

/// The settings of the reference's `PrintObjectConfig` (PrintConfig.hpp),
/// which an object may set for itself, in order of name.
pub const OBJECT_KEYS: [&str; 168] = [
    "adaptive_layer_height",
    "apply_scarf_seam_on_circles",
    "bridge_acceleration",
    "bridge_no_support",
    "brim_ears_detection_length",
    "brim_ears_max_angle",
    "brim_ears_outer_only",
    "brim_flow_ratio",
    "brim_object_gap",
    "brim_type",
    "brim_use_efc_outline",
    "brim_width",
    "calib_flowrate_topinfill_special_order",
    "circle_compensation_manual_offset",
    "default_acceleration",
    "default_jerk",
    "default_junction_deviation",
    "detect_narrow_internal_solid_infill",
    "dont_filter_internal_bridges",
    "elefant_foot_compensation",
    "elefant_foot_compensation_layers",
    "elefant_foot_layers_density",
    "enable_circle_compensation",
    "enable_extra_bridge_layer",
    "enable_support",
    "enforce_support_layers",
    "first_layer_travel_acceleration",
    "first_layer_travel_jerk",
    "flush_into_infill",
    "flush_into_objects",
    "flush_into_support",
    "gap_fill_target",
    "image_fill_detail",
    "image_fill_params",
    "independent_support_layer_height",
    "infill_jerk",
    "initial_layer_acceleration",
    "initial_layer_jerk",
    "initial_layer_min_bead_width",
    "initial_layer_travel_acceleration",
    "inner_wall_acceleration",
    "inner_wall_jerk",
    "interface_shells",
    "interlocking_beam",
    "interlocking_beam_layer_count",
    "interlocking_beam_width",
    "interlocking_boundary_avoidance",
    "interlocking_depth",
    "interlocking_orientation",
    "internal_bridge_density",
    "internal_solid_infill_acceleration",
    "layer_height",
    "line_width",
    "make_overhang_printable_angle",
    "make_overhang_printable_hole_size",
    "max_bridge_length",
    "min_bead_width",
    "min_feature_size",
    "min_length_factor",
    "mmu_segmented_region_interlocking_depth",
    "mmu_segmented_region_max_width",
    "nozzle_support_line_width",
    "outer_wall_acceleration",
    "outer_wall_jerk",
    "paint_depth_mm",
    "paint_depth_mode",
    "paint_depth_solid_interfaces",
    "paint_depth_walls",
    "paint_infill_override",
    "precise_z_height",
    "raft_contact_distance",
    "raft_expansion",
    "raft_first_layer_density",
    "raft_first_layer_expansion",
    "raft_layers",
    "seam_position",
    "seam_prefer_part_joints",
    "skirt_start_angle",
    "slice_closing_radius",
    "slicing_mode",
    "small_support_perimeter_speed",
    "small_support_perimeter_threshold",
    "sparse_infill_acceleration",
    "stabilizer_brace_max_span",
    "stabilizer_brace_max_unbraced",
    "stabilizer_bracing",
    "stabilizer_column_length",
    "stabilizer_column_min_height",
    "stabilizer_column_shape",
    "stabilizer_column_width",
    "stabilizer_infill_density",
    "stabilizer_infill_pattern",
    "stabilizer_max_island_width",
    "stabilizer_pillar_base_diameter",
    "stabilizer_pillar_diameter",
    "stabilizer_points_per_ring",
    "stabilizer_ring_spacing",
    "stabilizer_supports",
    "stabilizer_tip_diameter",
    "stabilizer_tip_gap",
    "stabilizer_wall_loops",
    "staggered_inner_seams",
    "support_angle",
    "support_base_pattern",
    "support_base_pattern_spacing",
    "support_bottom_interface_spacing",
    "support_bottom_z_distance",
    "support_critical_regions_only",
    "support_expansion",
    "support_filament",
    "support_filament_matching",
    "support_interface_bottom_layers",
    "support_interface_filament",
    "support_interface_interlock",
    "support_interface_interlock_beam_lines",
    "support_interface_interlock_layers",
    "support_interface_loop_pattern",
    "support_interface_not_for_body",
    "support_interface_pattern",
    "support_interface_spacing",
    "support_interface_speed",
    "support_interface_top_layers",
    "support_ironing",
    "support_ironing_direction",
    "support_ironing_flow",
    "support_ironing_inset",
    "support_ironing_pattern",
    "support_ironing_spacing",
    "support_ironing_speed",
    "support_line_width",
    "support_object_first_layer_gap",
    "support_object_xy_distance",
    "support_on_build_plate_only",
    "support_remove_small_overhang",
    "support_speed",
    "support_style",
    "support_threshold_angle",
    "support_threshold_overlap",
    "support_top_z_distance",
    "support_type",
    "thick_bridges",
    "thick_internal_bridges",
    "top_surface_acceleration",
    "top_surface_jerk",
    "travel_acceleration",
    "travel_jerk",
    "tree_support_angle_slow",
    "tree_support_auto_brim",
    "tree_support_branch_angle",
    "tree_support_branch_angle_organic",
    "tree_support_branch_diameter",
    "tree_support_branch_diameter_angle",
    "tree_support_branch_diameter_organic",
    "tree_support_branch_distance",
    "tree_support_branch_distance_organic",
    "tree_support_brim_width",
    "tree_support_tip_diameter",
    "tree_support_top_rate",
    "tree_support_wall_count",
    "wall_distribution_count",
    "wall_generator",
    "wall_maximum_deviation",
    "wall_maximum_resolution",
    "wall_transition_angle",
    "wall_transition_filter_deviation",
    "wall_transition_length",
    "xy_contour_compensation",
    "xy_hole_compensation",
];

/// The settings of the reference's `PrintRegionConfig` (PrintConfig.hpp),
/// which an object, a part, a modifier or a height range may set, in order
/// of name.
pub const REGION_KEYS: [&str; 191] = [
    "adaptive_infill_gradient",
    "adaptive_infill_interior_density",
    "adaptive_infill_mode",
    "align_infill_direction_to_model",
    "alternate_extra_wall",
    "bottom_color_penetration_layers",
    "bottom_layer_direction",
    "bottom_shell_layers",
    "bottom_shell_thickness",
    "bottom_solid_infill_flow_ratio",
    "bottom_surface_density",
    "bottom_surface_fill_order",
    "bottom_surface_pattern",
    "bridge_angle",
    "bridge_density",
    "bridge_expansion_extra",
    "bridge_flow",
    "bridge_line_width",
    "bridge_speed",
    "center_of_surface_pattern",
    "counterbore_hole_bridging",
    "detect_overhang_wall",
    "detect_thin_wall",
    "enable_overhang_speed",
    "ensure_vertical_shell_thickness",
    "extra_perimeters_on_overhangs",
    "extra_solid_infills",
    "filament_ironing_flow",
    "filament_ironing_inset",
    "filament_ironing_spacing",
    "filament_ironing_speed",
    "fill_multiline",
    "filter_out_gap_fill",
    "fuzzy_skin",
    "fuzzy_skin_first_layer",
    "fuzzy_skin_layers_between_ripple_offset",
    "fuzzy_skin_mode",
    "fuzzy_skin_noise_type",
    "fuzzy_skin_octaves",
    "fuzzy_skin_persistence",
    "fuzzy_skin_point_distance",
    "fuzzy_skin_ripple_offset",
    "fuzzy_skin_ripples_per_layer",
    "fuzzy_skin_scale",
    "fuzzy_skin_skip_overhangs",
    "fuzzy_skin_thickness",
    "gap_infill_speed",
    "gyroid_optimized",
    "hole_to_polyhole",
    "hole_to_polyhole_max_edges",
    "hole_to_polyhole_threshold",
    "hole_to_polyhole_twisted",
    "hollow_interior",
    "hollow_shell_thickness",
    "infill_anchor",
    "infill_anchor_max",
    "infill_combination",
    "infill_combination_max_layer_height",
    "infill_complete_top",
    "infill_direction",
    "infill_instead_top_bottom_surfaces",
    "infill_lock_depth",
    "infill_overhang_angle",
    "infill_shift_step",
    "infill_wall_overlap",
    "inner_wall_line_width",
    "inner_wall_speed",
    "internal_bridge_angle",
    "internal_bridge_flow",
    "internal_bridge_speed",
    "internal_solid_infill_line_width",
    "internal_solid_infill_pattern",
    "internal_solid_infill_speed",
    "ironing_angle",
    "ironing_angle_fixed",
    "ironing_direction",
    "ironing_flow",
    "ironing_inset",
    "ironing_pattern",
    "ironing_spacing",
    "ironing_speed",
    "ironing_type",
    "is_infill_first",
    "lateral_lattice_angle_1",
    "lateral_lattice_angle_2",
    "lightning_overhang_angle",
    "lightning_prune_angle",
    "lightning_straightening_angle",
    "locked_skeleton_infill_pattern",
    "locked_skin_infill_pattern",
    "make_overhang_printable",
    "min_layer_time_pause",
    "min_width_top_surface",
    "minimum_sparse_infill_area",
    "nozzle_bridge_line_width",
    "nozzle_inner_wall_line_width",
    "nozzle_internal_solid_infill_line_width",
    "nozzle_outer_wall_line_width",
    "nozzle_region_values",
    "nozzle_skeleton_infill_line_width",
    "nozzle_skin_infill_line_width",
    "nozzle_sparse_infill_line_width",
    "nozzle_top_surface_line_width",
    "offset_layers",
    "only_one_wall_first_layer",
    "only_one_wall_top",
    "outer_wall_filament",
    "outer_wall_line_width",
    "outer_wall_speed",
    "over_support_flow",
    "over_support_speed",
    "over_support_surfaces",
    "overhang_1_4_speed",
    "overhang_2_4_speed",
    "overhang_3_4_speed",
    "overhang_4_4_speed",
    "overhang_reverse",
    "overhang_reverse_internal_only",
    "overhang_reverse_threshold",
    "overhang_shadow_distance",
    "overhang_shadow_inner_wall",
    "overhang_shadow_speed_ratio",
    "override_paint",
    "precise_outer_wall",
    "print_flow_ratio",
    "relative_bridge_angle",
    "role_based_wipe_speed",
    "scarf_angle_threshold",
    "scarf_joint_flow_ratio",
    "scarf_joint_speed",
    "scarf_overhang_threshold",
    "seam_gap",
    "seam_slope_conditional",
    "seam_slope_entire_loop",
    "seam_slope_inner_walls",
    "seam_slope_min_length",
    "seam_slope_start_height",
    "seam_slope_steps",
    "seam_slope_type",
    "separated_infills",
    "skeleton_infill_density",
    "skeleton_infill_line_width",
    "skin_infill_density",
    "skin_infill_depth",
    "skin_infill_line_width",
    "slowdown_for_curled_perimeters",
    "small_area_infill_flow_compensation",
    "small_perimeter_speed",
    "small_perimeter_threshold",
    "solid_infill_direction",
    "solid_infill_filament",
    "solid_infill_rotate_template",
    "sparse_infill_density",
    "sparse_infill_filament",
    "sparse_infill_line_width",
    "sparse_infill_pattern",
    "sparse_infill_rotate_template",
    "sparse_infill_smooth_factor",
    "sparse_infill_speed",
    "symmetric_infill_y_axis",
    "top_bottom_infill_wall_overlap",
    "top_color_penetration_layers",
    "top_layer_direction",
    "top_shell_layers",
    "top_shell_thickness",
    "top_solid_infill_flow_ratio",
    "top_surface_density",
    "top_surface_expansion",
    "top_surface_expansion_direction",
    "top_surface_expansion_margin",
    "top_surface_fill_order",
    "top_surface_line_width",
    "top_surface_pattern",
    "top_surface_speed",
    "undertop_surface_pattern",
    "unsupported_wall_last",
    "wall_direction",
    "wall_filament",
    "wall_infill_order",
    "wall_loops",
    "wall_sequence",
    "wipe_before_external_loop",
    "wipe_inward",
    "wipe_inward_distance",
    "wipe_on_loops",
    "wipe_speed",
    "zaa_dont_alternate_fill_direction",
    "zaa_enabled",
    "zaa_min_z",
    "zaa_minimize_perimeter_height",
    "zaa_speed_scaling",
];

/// Where a setting may be set for less than the whole plate, as the
/// reference's key lists have it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Object-level (`PrintObjectConfig`): on an object only.
    Object,
    /// Region-level (`PrintRegionConfig`): on an object, a part, a
    /// modifier or a height range.
    Region,
    /// Neither: the whole plate's alone. The reference takes no notice of
    /// it set on an object or a part.
    Plate,
}

/// Which of the reference's lists a setting is in.
pub fn scope(key: &str) -> Scope {
    if REGION_KEYS.binary_search(&key).is_ok() {
        Scope::Region
    } else if OBJECT_KEYS.binary_search(&key).is_ok() {
        Scope::Object
    } else {
        Scope::Plate
    }
}

/// What may be set where, and what the engine does with it there: for the
/// desktop app's object and part settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settable {
    /// Whether the reference reads it set on an object, on a part or
    /// modifier, and on a height range.
    pub on_object: bool,
    pub on_part: bool,
    pub on_range: bool,
    /// Whether this engine follows it set on an object, on a part or a
    /// modifier, and on a height range. Where it may be set but is not
    /// followed, a slice says so in a note.
    pub followed_on_object: bool,
    pub followed_on_part: bool,
    pub followed_on_range: bool,
}

/// [`Settable`] for one setting.
pub fn settable(key: &str) -> Settable {
    let scope = scope(key);
    let region = scope == Scope::Region;
    let followed = per_region(key);
    Settable {
        on_object: scope != Scope::Plate,
        on_part: region,
        // A height range is laid over the object's parts as a region's
        // config is (`region_config_from_model_volume`), and its
        // `layer_height` sets the range's layers' height.
        on_range: region || key == "layer_height",
        followed_on_object: scope != Scope::Plate && (followed || OBJECT_FOLLOWED.contains(&key)),
        followed_on_part: region && followed,
        followed_on_range: region && followed,
    }
}

/// The note a slice gives for `key` set on `on` (an object, a part, a
/// modifier or a height range, as the note names it) where it is not
/// followed there, in the words [`Sliced::gaps`](crate::Sliced) uses; none
/// where it is followed, or where the reference takes no notice of it
/// either. For the desktop app's pages of an object's settings.
pub fn unfollowed_note(key: &str, on: &str) -> Option<String> {
    let mut set = BTreeMap::new();
    set.insert(key.to_owned(), BTreeSet::from([on.to_owned()]));
    unfollowed(&set).into_iter().next()
}

/// The settings every stage that is worked out region by region reads:
/// what a region's own settings change. Found by doing that work's
/// reading with defaults and with each kind of fill, wall and skin, as
/// [`crate::followed`] finds what a slice reads.
fn region_reads() -> &'static BTreeSet<String> {
    static READS: OnceLock<BTreeSet<String>> = OnceLock::new();
    READS.get_or_init(|| {
        let reads = Reads::default();
        let mut variants: Vec<Vec<(&str, &str)>> = vec![
            vec![],
            vec![("ironing_type", "top"), ("fuzzy_skin", "allwalls")],
            vec![("wall_generator", "arachne"), ("ironing_type", "solid")],
            vec![("sparse_infill_pattern", "gyroid")],
            vec![("sparse_infill_pattern", "cubic")],
        ];
        for pattern in [
            "rectilinear",
            "alignedrectilinear",
            "line",
            "honeycomb",
            "lightning",
            "adaptivecubic",
            "supportcubic",
            "concentric",
            "crosshatch",
            "zigzag",
            "crosszag",
            "lockedzag",
            "tpmsd",
            "tpmsfk",
            "3dhoneycomb",
            "hilbertcurve",
            "archimedeanchords",
            "octagramspiral",
            "triangles",
            "tri-hexagon",
            "quartercubic",
        ] {
            variants.push(vec![("sparse_infill_pattern", pattern)]);
        }
        for variant in variants {
            let mut settings = edge_config::reference_defaults().clone();
            for (key, value) in variant {
                settings.insert(key, Value::One(value.to_owned()));
            }
            settings.noted_by(&reads);
            let _ = StageSettings::read(&settings, 1);
        }
        reads.keys()
    })
}

/// Whether a setting set for one object, part, modifier or range is
/// followed there: read by a stage worked out region by region, or one
/// the G-code writer takes from a region's own settings as it lays the
/// region's walls and fill.
pub(crate) fn per_region(key: &str) -> bool {
    (region_reads().contains(key) || WRITTEN_BY_REGION.contains(&key)) && !PLATE_WIDE.contains(&key)
}

/// What the stages worked out region by region read but take from the
/// plate's settings all the same, where the work is done for every
/// region at once: bridges over sparse fill and their second layer, and
/// how many layers elephant foot compensation runs over, and Z contouring,
/// which is laid over the whole plate's walls and fill at once.
const PLATE_WIDE: [&str; 5] = [
    "dont_filter_internal_bridges",
    "enable_extra_bridge_layer",
    "internal_bridge_density",
    "elefant_foot_compensation_layers",
    "zaa_enabled",
];

/// What the G-code writer reads from a region's own settings as it lays
/// its walls and fill (`edge_gcode::Layer::region`): how fast, how hard
/// and with how much plastic each line is laid, and in what order an
/// island's walls and fill come.
const WRITTEN_BY_REGION: [&str; 30] = [
    "outer_wall_speed",
    "inner_wall_speed",
    "small_perimeter_speed",
    "small_perimeter_threshold",
    "sparse_infill_speed",
    "internal_solid_infill_speed",
    "top_surface_speed",
    "gap_infill_speed",
    "bridge_speed",
    "internal_bridge_speed",
    "ironing_speed",
    "enable_overhang_speed",
    "overhang_1_4_speed",
    "overhang_2_4_speed",
    "overhang_3_4_speed",
    "overhang_4_4_speed",
    "default_acceleration",
    "outer_wall_acceleration",
    "inner_wall_acceleration",
    "top_surface_acceleration",
    "sparse_infill_acceleration",
    "internal_solid_infill_acceleration",
    "bridge_acceleration",
    "print_flow_ratio",
    "top_solid_infill_flow_ratio",
    "bottom_solid_infill_flow_ratio",
    "bridge_flow",
    "internal_bridge_flow",
    "wall_sequence",
    "is_infill_first",
];

/// The settings of the stages worked out region by region.
#[derive(Clone)]
pub(crate) struct StageSettings {
    pub(crate) walls: edge_perimeter::WallSettings,
    pub(crate) infill: edge_infill::InfillSettings,
    pub(crate) ironing: edge_infill::IroningSettings,
}

impl StageSettings {
    /// Reads them from a region's settings, as filament `filament` (from
    /// 1) prints that region.
    pub(crate) fn read(settings: &Settings, filament: usize) -> Result<Self, crate::Failure> {
        let walls = edge_perimeter::WallSettings::from_settings(settings)?;
        let infill = edge_infill::InfillSettings::from_settings(settings)?;
        let ironing = edge_infill::IroningSettings::from_settings(settings)?;
        // What is read only once they are worked out at a height.
        for first in [false, true] {
            let _ = walls.params(0.2, first);
            let _ = infill.params(0.2, first);
        }
        let _ = infill.shells(0.2);
        let _ = infill.anchoring(0.2);
        let _ = ironing.for_filament(filament, 0.2);
        Ok(Self {
            walls,
            infill,
            ironing,
        })
    }
}

/// One region with settings of its own.
#[derive(Clone, Debug)]
pub(crate) struct Region {
    /// The filament it prints in, from 1.
    pub(crate) filament: usize,
    /// What it sets over the plate's settings, each differing from them.
    pub(crate) overrides: BTreeMap<String, String>,
    /// The plate's settings with `overrides` laid over them.
    pub(crate) settings: Arc<Settings>,
}

/// Every region of a plate with settings of its own, by key from
/// [`BASE`]; below it a key is a filament number.
#[derive(Clone, Debug, Default)]
pub(crate) struct Regions {
    own: Vec<Region>,
}

/// A value as the plate's settings give it, in the text a file gives it in.
fn plate_text(settings: &Settings, key: &str) -> Option<String> {
    match settings.get(key)? {
        Value::One(text) => Some(text.clone()),
        Value::Many(texts) => Some(texts.join(",")),
    }
}

/// Whether two values of a setting are the same: as text, or as numbers.
fn same_value(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    a == b
        || matches!((a.parse::<f64>(), b.parse::<f64>()), (Ok(x), Ok(y)) if x == y)
        || matches!(
            (a, b),
            ("1" | "true", "1" | "true") | ("0" | "false", "0" | "false")
        )
}

/// Lays `pairs` over `overrides`, keeping only what `allowed` lets through
/// and what differs from the plate's own value: a value that comes back
/// to the plate's takes the setting out again, so a part that sets what
/// its object changed back to what the plate has shares the plate's
/// region, as the reference finds it by comparing configs.
pub(crate) fn lay_over(
    overrides: &mut BTreeMap<String, String>,
    plate: &Settings,
    pairs: &[(String, String)],
    allowed: impl Fn(&str) -> bool,
) {
    for (key, value) in pairs {
        if !allowed(key) || FEATURE_FILAMENTS.contains(&key.as_str()) {
            continue;
        }
        let same = plate_text(plate, key).is_some_and(|own| same_value(&own, value));
        if same {
            overrides.remove(key);
        } else {
            overrides.insert(key.clone(), value.clone());
        }
    }
}

/// The settings of an object, part or range that name the filament a
/// feature is printed in. Not followed: the feature prints in the
/// region's own filament, and a slice says so.
pub(crate) const FEATURE_FILAMENTS: [&str; 4] = [
    "wall_filament",
    "outer_wall_filament",
    "sparse_infill_filament",
    "solid_infill_filament",
];

impl Regions {
    /// The key of the region printed in `filament` with `overrides` over
    /// the plate's settings: the filament itself when there are none.
    pub(crate) fn key(
        &mut self,
        plate: &Settings,
        filament: usize,
        overrides: &BTreeMap<String, String>,
    ) -> usize {
        if overrides.is_empty() {
            return filament;
        }
        if let Some(at) = (self.own.iter())
            .position(|region| region.filament == filament && region.overrides == *overrides)
        {
            return BASE + at;
        }
        let mut settings = plate.clone();
        for (key, value) in overrides {
            let value = match plate.get(key) {
                Some(Value::Many(_)) => Value::Many(
                    value
                        .split(',')
                        .map(|item| item.trim().to_owned())
                        .collect(),
                ),
                _ => Value::One(value.clone()),
            };
            settings.insert(key.clone(), value);
        }
        self.own.push(Region {
            filament,
            overrides: overrides.clone(),
            settings: Arc::new(settings),
        });
        BASE + self.own.len() - 1
    }

    /// The region `key` stands for when it has settings of its own.
    pub(crate) fn own(&self, key: usize) -> Option<&Region> {
        key.checked_sub(BASE).and_then(|at| self.own.get(at))
    }

    /// The filament region `key` prints in, from 1.
    pub(crate) fn filament(&self, key: usize) -> usize {
        self.own(key).map_or(key, |region| region.filament)
    }

    /// What region `key` sets over the plate's settings: nothing for a
    /// filament's own region.
    pub(crate) fn overrides(&self, key: usize) -> BTreeMap<String, String> {
        self.own(key)
            .map(|region| region.overrides.clone())
            .unwrap_or_default()
    }

    /// The same region printed in another filament: what a filament
    /// change set at a layer makes of it.
    pub(crate) fn in_filament(&mut self, key: usize, plate: &Settings, filament: usize) -> usize {
        let overrides = self.overrides(key);
        self.key(plate, filament, &overrides)
    }

    /// Whether any region has settings of its own.
    pub(crate) fn any(&self) -> bool {
        !self.own.is_empty()
    }
}

/// What a printed part's region sets over the plate's settings: its
/// object's settings, object-level and region-level, then the part's
/// own, region-level alone, as `region_config_from_model_volume` lays the
/// object's config and then the volume's over the print's. Each setting
/// that differs from the plate's is put in `set`, with what set it, for
/// the notes on what is not followed.
pub(crate) fn part_overrides(
    plate: &Settings,
    object: &crate::OwnObject,
    volume: &crate::OwnVolume,
    set: &mut BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, String> {
    let mut overrides = BTreeMap::new();
    lay_over(&mut overrides, plate, &object.settings, |key| {
        scope(key) != Scope::Plate
    });
    note_set(
        set,
        plate,
        &object.settings,
        |key| scope(key) != Scope::Plate,
        &format!("object \"{}\"", object.name),
    );
    lay_over(&mut overrides, plate, &volume.settings, |key| {
        scope(key) == Scope::Region
    });
    note_set(
        set,
        plate,
        &volume.settings,
        |key| scope(key) == Scope::Region || key == "support_group",
        &format!("part \"{}\" of \"{}\"", volume.name, object.name),
    );
    overrides
}

/// Puts each of `pairs` that differs from the plate's settings in `set`,
/// as set on `what`.
pub(crate) fn note_set(
    set: &mut BTreeMap<String, BTreeSet<String>>,
    plate: &Settings,
    pairs: &[(String, String)],
    applies: impl Fn(&str) -> bool,
    what: &str,
) {
    for (key, value) in pairs.iter().filter(|(key, _)| applies(key)) {
        if !plate_text(plate, key).is_some_and(|own| same_value(&own, value)) {
            set.entry(key.clone()).or_default().insert(what.to_owned());
        }
    }
}

/// What an object may set for itself that is followed for the object as a
/// whole rather than region by region: its brim (Brim.cpp reads
/// `brim_type`, `brim_width` and `brim_object_gap` from each object's
/// config).
const OBJECT_FOLLOWED: [&str; 3] = ["brim_type", "brim_width", "brim_object_gap"];

/// The plate's settings with an object's own brim laid over them, when it
/// sets one that differs.
pub(crate) fn object_brim(plate: &Settings, object: &crate::OwnObject) -> Option<Settings> {
    let mut overrides = BTreeMap::new();
    lay_over(&mut overrides, plate, &object.settings, |key| {
        OBJECT_FOLLOWED.contains(&key)
    });
    if overrides.is_empty() {
        return None;
    }
    let mut theirs = plate.clone();
    for (key, value) in overrides {
        theirs.insert(key, Value::One(value));
    }
    Some(theirs)
}

/// What an object, a part or a modifier sets that is not followed there,
/// each as a note that starts with the setting's name: `set` is each
/// setting with what it was set on, already reduced to what differs from
/// the plate's settings.
pub(crate) fn unfollowed(set: &BTreeMap<String, BTreeSet<String>>) -> Vec<String> {
    set.iter()
        .filter_map(|(key, on)| {
            let on: Vec<&str> = on.iter().map(String::as_str).collect();
            let on = on.join(", ");
            // EdgeSlicer's support groups: a part named into one is
            // supported by the group's own settings.
            if key == "support_group" {
                return Some(format!(
                    "support_group: set on {on}, not followed; the part is supported by its object's settings"
                ));
            }
            if FEATURE_FILAMENTS.contains(&key.as_str()) {
                return Some(format!(
                    "{key}: set on {on}, not followed; walls and fill there print in the part's own filament"
                ));
            }
            match scope(key) {
                // The reference takes no notice of it there either.
                Scope::Plate => None,
                _ if per_region(key) => None,
                // Set on a part, a modifier or a range it is not the
                // reference's either; set on an object it is followed.
                Scope::Object if OBJECT_FOLLOWED.contains(&key.as_str()) => None,
                _ => Some(format!(
                    "{key}: set on {on}, not followed there; the plate's value is used"
                )),
            }
        })
        .collect()
}

/// Whether two regions' lines are written alike: the same in all the
/// G-code writer takes from a region (speeds, accelerations, flow), as
/// the reference fills two regions' surfaces together only where their
/// lines go at the same speed (Fill.cpp `SurfaceFillParams::role_speed`).
pub(crate) fn written_alike(a: &Settings, b: &Settings) -> bool {
    WRITTEN_BY_REGION
        .iter()
        .all(|key| match (plate_text(a, key), plate_text(b, key)) {
            (Some(x), Some(y)) => same_value(&x, &y),
            (x, y) => x == y,
        })
}

/// What decides whether two regions' walls are alike enough to be laid
/// round both at once (Layer.cpp `Layer::is_perimeter_compatible`), with
/// fuzzy skin: the reference merges regions that differ only in fuzzy
/// skin and then fuzzes each piece of the walls by its own region's
/// settings (PerimeterGenerator.cpp); walls here are fuzzed by one set of
/// settings, so regions that differ in it keep walls of their own.
const PERIMETER_KEYS: [&str; 47] = [
    "wall_filament",
    "outer_wall_filament",
    "wall_loops",
    "wall_sequence",
    "is_infill_first",
    "inner_wall_speed",
    "outer_wall_speed",
    "over_support_surfaces",
    "over_support_flow",
    "over_support_speed",
    "small_perimeter_speed",
    "gap_infill_speed",
    "filter_out_gap_fill",
    "detect_overhang_wall",
    "unsupported_wall_last",
    "overhang_reverse",
    "overhang_reverse_threshold",
    "wall_direction",
    "inner_wall_line_width",
    "outer_wall_line_width",
    "detect_thin_wall",
    "infill_wall_overlap",
    "top_bottom_infill_wall_overlap",
    "only_one_wall_first_layer",
    "only_one_wall_top",
    "min_width_top_surface",
    "seam_slope_type",
    "seam_slope_conditional",
    "scarf_angle_threshold",
    "scarf_overhang_threshold",
    "scarf_joint_speed",
    "scarf_joint_flow_ratio",
    "seam_slope_start_height",
    "seam_slope_entire_loop",
    "seam_slope_min_length",
    "seam_slope_steps",
    "seam_slope_inner_walls",
    "fuzzy_skin",
    "fuzzy_skin_thickness",
    "fuzzy_skin_point_distance",
    "fuzzy_skin_first_layer",
    "fuzzy_skin_noise_type",
    "fuzzy_skin_mode",
    "fuzzy_skin_scale",
    "fuzzy_skin_octaves",
    "fuzzy_skin_persistence",
    "fuzzy_skin_ripples_per_layer",
];

/// For each region of `keys` (printed in `used`), the first region whose
/// walls are alike enough to be laid with its own: itself where none
/// before it is. Regions of the plate's settings, one to a filament, are
/// never alike.
pub(crate) fn wall_groups(
    keys: &[usize],
    used: &[usize],
    regions: &Regions,
    plate: &Settings,
) -> Vec<usize> {
    let settings_of = |slot: usize| -> &Settings {
        regions
            .own(keys[slot])
            .map_or(plate, |region| &region.settings)
    };
    let alike = |a: usize, b: usize| {
        used[a] == used[b]
            && PERIMETER_KEYS.iter().all(|key| {
                let (x, y) = (
                    plate_text(settings_of(a), key),
                    plate_text(settings_of(b), key),
                );
                match (x, y) {
                    (Some(x), Some(y)) => same_value(&x, &y),
                    (x, y) => x == y,
                }
            })
    };
    let mut groups: Vec<usize> = Vec::with_capacity(keys.len());
    for slot in 0..keys.len() {
        let leader = (0..slot).find(|other| groups[*other] == *other && alike(*other, slot));
        groups.push(leader.unwrap_or(slot));
    }
    groups
}

/// The surfaces of several regions filled as one: those of a kind and a
/// direction united across them, in the order each first comes; and
/// what is inside all of their walls.
pub(crate) fn merge_surfaces<'a>(
    surfaces: impl Iterator<Item = &'a [edge_infill::Surface]>,
    no_overlap: impl Iterator<Item = &'a [edge_geom::ExPolygon]>,
) -> Result<(Vec<edge_infill::Surface>, Vec<edge_geom::ExPolygon>), edge_clip::ClipError> {
    let mut kinds: Vec<(
        edge_infill::SurfaceKind,
        Option<u64>,
        Vec<edge_geom::ExPolygon>,
    )> = Vec::new();
    for surface in surfaces.flatten() {
        let angle = surface.angle.map(f64::to_bits);
        match (kinds.iter_mut()).find(|(kind, at, _)| *kind == surface.kind && *at == angle) {
            Some((_, _, areas)) => areas.push(surface.area.clone()),
            None => kinds.push((surface.kind, angle, vec![surface.area.clone()])),
        }
    }
    let mut merged = Vec::new();
    for (kind, angle, areas) in kinds {
        let united = if areas.len() > 1 {
            // Pieces that meet along a region's edge are made one, the
            // seam between them closed (Fill.cpp `group_fills`:
            // `union_safety_offset_ex`).
            edge_clip::union_ex(&edge_clip::offset(&areas, 10, edge_clip::Join::MITER)?)
                .and_then(|grown| edge_clip::offset(&grown, -10, edge_clip::Join::MITER))?
        } else {
            areas
        };
        merged.extend(united.into_iter().map(|area| edge_infill::Surface {
            kind,
            area,
            angle: angle.map(f64::from_bits),
        }));
    }
    let all: Vec<edge_geom::ExPolygon> =
        no_overlap.flat_map(|areas| areas.iter().cloned()).collect();
    Ok((merged, edge_clip::union_ex(&all)?))
}

/// One step of what is laid over an object's parts on a layer, in order:
/// a height range, then each modifier.
struct Step {
    /// The layers it applies to by their cutting height, from and below.
    span: (f64, f64),
    /// Its mesh cut at every layer, as regions; none for a range, which
    /// takes the whole of the object on its layers.
    cut: Option<Vec<Vec<edge_geom::ExPolygon>>>,
    /// The key each key it may meet becomes.
    becomes: BTreeMap<usize, usize>,
}

/// Lays an object's height ranges and modifiers over its parts' regions
/// on every layer, as the reference shares an object's layer out among
/// its regions (PrintObjectSlice.cpp `slices_to_regions`): on the layers
/// of a height range each part's area takes the range's settings over its
/// own (`region_config_from_model_volume` with the range's config); then
/// each modifier in the object's order takes what of the object's areas
/// lies inside it, every region it meets becoming one with the modifier's
/// settings over that region's (`modifier_region_config`), a modifier's
/// region among them, so that a later modifier is laid over an earlier.
/// What a modifier sets that changes nothing leaves its parent's region
/// as it is, as the reference gives such a modifier its parent's region.
///
/// Not as the reference: a range's `layer_height` is not followed (every
/// object on a plate is cut at the same heights here), and is said in a
/// note.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lay_over_parts(
    layers: &mut [edge_slice::LayerRegions],
    planes: &[f64],
    closing_radius: i64,
    own: &[crate::OwnObject],
    bodies: &[edge_3mf::Body],
    body_keys: &[usize],
    regions: &mut Regions,
    plate: &Settings,
    set: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), crate::Failure> {
    use crate::PartKind;
    use rayon::prelude::*;
    for (at, object) in own.iter().enumerate() {
        let modifiers: Vec<&crate::OwnVolume> = (object.volumes.iter())
            .filter(|volume| volume.kind == PartKind::Modifier)
            .collect();
        if modifiers.is_empty() && object.ranges.is_empty() {
            continue;
        }
        // The keys the object's parts are under, and what each step
        // makes of every key it may meet.
        let mut met: BTreeSet<usize> = (bodies.iter().zip(body_keys))
            .filter(|(body, _)| body.part.is_some_and(|(of, _)| of == at))
            .map(|(_, key)| *key)
            .collect();
        let mut steps: Vec<Step> = Vec::new();
        let mut range_children = BTreeSet::new();
        for range in &object.ranges {
            let label = format!(
                "height range {}-{} mm of \"{}\"",
                range.min_z, range.max_z, object.name
            );
            note_set(
                set,
                plate,
                &range.settings,
                |key| scope(key) == Scope::Region,
                &label,
            );
            let height = (range.settings.iter()).find(|(key, _)| key == "layer_height");
            if let Some((_, height)) = height
                && !plate_text(plate, "layer_height").is_some_and(|own| same_value(&own, height))
            {
                set.entry("layer_height".to_owned())
                    .or_default()
                    .insert(label.clone());
            }
            let mut becomes = BTreeMap::new();
            for key in &met {
                let mut overrides = regions.overrides(*key);
                lay_over(&mut overrides, plate, &range.settings, |key| {
                    scope(key) == Scope::Region
                });
                let child = regions.key(plate, regions.filament(*key), &overrides);
                becomes.insert(*key, child);
                range_children.insert(child);
            }
            steps.push(Step {
                span: (range.min_z, range.max_z),
                cut: None,
                becomes,
            });
        }
        met.extend(range_children);
        for modifier in &modifiers {
            note_set(
                set,
                plate,
                &modifier.settings,
                |key| scope(key) == Scope::Region,
                &format!("modifier \"{}\" of \"{}\"", modifier.name, object.name),
            );
            let mut becomes = BTreeMap::new();
            for key in &met {
                let mut overrides = regions.overrides(*key);
                lay_over(&mut overrides, plate, &modifier.settings, |key| {
                    scope(key) == Scope::Region
                });
                let filament = modifier.filament.unwrap_or_else(|| regions.filament(*key));
                becomes.insert(*key, regions.key(plate, filament, &overrides));
            }
            met.extend(becomes.values().copied());
            let cut: Vec<Vec<edge_geom::ExPolygon>> = edge_slice::slice(&modifier.mesh, planes)
                .into_par_iter()
                .map(|layer| edge_clip::union(&layer.polygons))
                .collect::<Result<_, _>>()?;
            steps.push(Step {
                span: (f64::NEG_INFINITY, f64::INFINITY),
                cut: Some(cut),
                becomes,
            });
        }
        if steps
            .iter()
            .all(|step| step.becomes.iter().all(|(from, to)| from == to))
        {
            continue;
        }
        // The object's own outline on every layer: what a step may take.
        let parts: Vec<edge_slice::Body<'_>> = (bodies.iter())
            .filter(|body| body.part.is_some_and(|(of, _)| of == at))
            .map(|body| edge_slice::Body {
                solid: &body.solid,
                negative: body.negative.as_ref(),
                filament: 1,
            })
            .collect();
        let outline = edge_slice::slice_regions(&parts, planes, closing_radius)?;
        let single = body_keys.first().copied().unwrap_or(1);
        layers
            .par_iter_mut()
            .zip(outline)
            .enumerate()
            .try_for_each(
                |(index, (layer, outline))| -> Result<(), edge_clip::ClipError> {
                    if outline.regions.is_empty() {
                        return Ok(());
                    }
                    for step in &steps {
                        let z = planes[index];
                        if z < step.span.0 || z >= step.span.1 {
                            continue;
                        }
                        let within = match &step.cut {
                            Some(cut) if cut[index].is_empty() => continue,
                            Some(cut) => edge_clip::intersection(&cut[index], &outline.regions)?,
                            None => outline.regions.clone(),
                        };
                        if within.is_empty() {
                            continue;
                        }
                        let whole = layer.regions.clone();
                        for shares in [&mut layer.by_filament, &mut layer.cut_by_filament] {
                            if shares.is_empty() {
                                *shares = vec![(single, whole.clone())];
                            }
                            let mut moved: Vec<(usize, Vec<edge_geom::ExPolygon>)> = Vec::new();
                            for (key, area) in shares.iter_mut() {
                                let Some(child) =
                                    step.becomes.get(key).filter(|child| *child != key)
                                else {
                                    continue;
                                };
                                let inside = edge_clip::intersection(area, &within)?;
                                if inside.is_empty() {
                                    continue;
                                }
                                *area = edge_clip::difference(area, &within)?;
                                moved.push((*child, inside));
                            }
                            for (child, inside) in moved {
                                match shares.iter_mut().find(|(key, _)| *key == child) {
                                    Some((_, area)) => {
                                        area.extend(inside);
                                        *area = edge_clip::union_ex(area)?;
                                    }
                                    None => shares.push((child, inside)),
                                }
                            }
                            shares.retain(|(_, area)| !area.is_empty());
                            shares.sort_by_key(|(key, _)| *key);
                        }
                    }
                    Ok(())
                },
            )?;
    }
    // A layer left all one key's while others were shared out is set down
    // as that key's, as slicing sets down a layer of several keys.
    if layers.iter().any(|layer| !layer.by_filament.is_empty()) {
        let single = body_keys.first().copied().unwrap_or(1);
        for layer in layers.iter_mut() {
            for shares in [&mut layer.by_filament, &mut layer.cut_by_filament] {
                if shares.is_empty() && !layer.regions.is_empty() {
                    *shares = vec![(single, layer.regions.clone())];
                }
            }
        }
    }
    Ok(())
}

/// The pieces of `triangles` that face downward, each cut to the slab
/// of every layer it passes through and laid flat: what is painted under
/// a part, projected up onto the layers it hangs over (PrintObject.cpp
/// `project_and_append_custom_facets`: "Project downward facing painted
/// areas upwards to their respective slicing plane", by
/// `slice_mesh_slabs`). A layer's slab runs from the plane the layer
/// below is cut at to its own. Not as the reference: painted triangles
/// that stand upright, which it hands tree supports as points to grow a
/// tip from, are not followed.
pub(crate) fn project_down(
    triangles: &[edge_mesh::Triangle],
    planes: &[f64],
) -> Result<Vec<Vec<edge_geom::ExPolygon>>, edge_clip::ClipError> {
    use edge_geom::{Point2, Polygon};
    let mut by_layer: Vec<Vec<Polygon>> = vec![Vec::new(); planes.len()];
    for triangle in triangles {
        let [a, b, c] = triangle.map(|corner| corner.map(f64::from));
        let normal_z = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if normal_z >= 0.0 {
            continue;
        }
        let low = a[2].min(b[2]).min(c[2]);
        let high = a[2].max(b[2]).max(c[2]);
        let first = planes.partition_point(|plane| *plane < low);
        for index in first..planes.len() {
            let floor = if index == 0 {
                f64::NEG_INFINITY
            } else {
                planes[index - 1]
            };
            if floor > high {
                break;
            }
            // The triangle cut to floor < z <= plane.
            let mut piece = vec![a, b, c];
            for (level, keep_above) in [(floor, true), (planes[index], false)] {
                if !level.is_finite() {
                    continue;
                }
                let inside = |p: &[f64; 3]| {
                    if keep_above {
                        p[2] >= level
                    } else {
                        p[2] <= level
                    }
                };
                let mut cut = Vec::with_capacity(piece.len() + 1);
                for at in 0..piece.len() {
                    let (p, q) = (piece[at], piece[(at + 1) % piece.len()]);
                    if inside(&p) {
                        cut.push(p);
                    }
                    if inside(&p) != inside(&q) {
                        let t = (level - p[2]) / (q[2] - p[2]);
                        cut.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1]), level]);
                    }
                }
                piece = cut;
                if piece.len() < 3 {
                    break;
                }
            }
            if piece.len() < 3 {
                continue;
            }
            // Facing down, it runs clockwise seen from above.
            let mut points: Vec<Point2> =
                piece.iter().map(|p| Point2::from_mm(p[0], p[1])).collect();
            points.reverse();
            by_layer[index].push(Polygon { points });
        }
    }
    by_layer
        .into_iter()
        .map(|polygons| {
            if polygons.is_empty() {
                Ok(Vec::new())
            } else {
                edge_clip::union(&polygons)
            }
        })
        .collect()
}

/// Where supports are asked for and kept off on every layer: each
/// object's support enforcers and blockers cut at the layer
/// (PrintObjectSlice.cpp `slice_support_volumes`), an enforcer only where
/// it lies over its own object, and what is painted under its parts.
pub(crate) fn support_annotations(
    own: &[crate::OwnObject],
    bodies: &[edge_3mf::Body],
    planes: &[f64],
    closing_radius: i64,
) -> Result<edge_support::Annotations, crate::Failure> {
    use crate::PartKind;
    use rayon::prelude::*;
    let mut annotations = edge_support::Annotations {
        enforcers: vec![Vec::new(); planes.len()],
        blockers: vec![Vec::new(); planes.len()],
        blocking_volumes: vec![Vec::new(); planes.len()],
    };
    for (at, object) in own.iter().enumerate() {
        let volumes: Vec<&crate::OwnVolume> = (object.volumes.iter())
            .filter(|volume| {
                matches!(
                    volume.kind,
                    PartKind::SupportEnforcer | PartKind::SupportBlocker
                )
            })
            .collect();
        if !volumes.is_empty() {
            let parts: Vec<edge_slice::Body<'_>> = (bodies.iter())
                .filter(|body| body.part.is_some_and(|(of, _)| of == at))
                .map(|body| edge_slice::Body {
                    solid: &body.solid,
                    negative: body.negative.as_ref(),
                    filament: 1,
                })
                .collect();
            let outline = edge_slice::slice_regions(&parts, planes, closing_radius)?;
            for volume in volumes {
                let cut: Vec<Vec<edge_geom::ExPolygon>> = edge_slice::slice(&volume.mesh, planes)
                    .into_par_iter()
                    .zip(&outline)
                    .map(|(layer, outline)| {
                        if layer.polygons.is_empty() {
                            return Ok(Vec::new());
                        }
                        let cut = edge_clip::union(&layer.polygons)?;
                        // A blocker is taken whole: tree branches keep out
                        // of all of it. An enforcer asks for supports under
                        // its own object alone (`intersection(layer.lslices,
                        // enforcers)`).
                        if volume.kind == PartKind::SupportBlocker {
                            Ok(cut)
                        } else {
                            edge_clip::intersection(&cut, &outline.regions)
                        }
                    })
                    .collect::<Result<_, edge_clip::ClipError>>()?;
                if volume.kind == PartKind::SupportEnforcer {
                    for (layer, areas) in annotations.enforcers.iter_mut().zip(cut) {
                        layer.extend(areas);
                    }
                } else {
                    for ((layer, volumes), areas) in (annotations.blockers.iter_mut())
                        .zip(annotations.blocking_volumes.iter_mut())
                        .zip(cut)
                    {
                        volumes.extend(areas.iter().cloned());
                        layer.extend(areas);
                    }
                }
            }
        }
        for (kind, into) in [
            (edge_3mf::SeamPaint::Enforcer, &mut annotations.enforcers),
            (edge_3mf::SeamPaint::Blocker, &mut annotations.blockers),
        ] {
            let painted: Vec<edge_mesh::Triangle> = (object.support_paint.iter())
                .filter(|(_, of)| *of == kind)
                .map(|(triangle, _)| *triangle)
                .collect();
            if painted.is_empty() {
                continue;
            }
            for (layer, areas) in into.iter_mut().zip(project_down(&painted, planes)?) {
                layer.extend(areas);
            }
        }
    }
    Ok(annotations)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use edge_mesh::{Mesh, primitives};

    use crate::{Chosen, Loaded, Overrides, OwnObject, OwnVolume, PartKind, Sliced, Unwatched};

    /// Two 10 mm cubes 5 mm apart, each an object; the second sets
    /// `object` for itself and its part `part`, and `extra` are its
    /// modifiers and the like, already in world coordinates.
    pub(crate) fn two_cubes(
        object: &[(&str, &str)],
        part: &[(&str, &str)],
        extra: Vec<OwnVolume>,
        ranges: Vec<edge_3mf::LayerRange>,
        process: &str,
    ) -> Sliced {
        let cube = |x: f32| {
            Mesh::from_triangle_soup(primitives::cuboid([x, 0.0, 0.0], [x + 10.0, 10.0, 10.0]))
        };
        let pairs = |list: &[(&str, &str)]| -> Vec<(String, String)> {
            list.iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        let mut volumes = vec![OwnVolume {
            kind: PartKind::Normal,
            name: "cube".into(),
            filament: Some(1),
            settings: pairs(part),
            mesh: Mesh::default(),
        }];
        volumes.extend(extra);
        let loaded = Loaded {
            bodies: vec![
                edge_3mf::Body {
                    solid: cube(0.0),
                    negative: None,
                    filament: 1,
                    part: None,
                },
                edge_3mf::Body {
                    solid: cube(15.0),
                    negative: None,
                    filament: 1,
                    part: Some((0, 0)),
                },
            ],
            painted: Vec::new(),
            settings: None,
            unread_paint: Vec::new(),
            precise_seams: false,
            own: vec![OwnObject {
                object: 1,
                name: "second".into(),
                settings: pairs(object),
                volumes,
                ranges,
                support_paint: Vec::new(),
            }],
            objects: vec![cube(0.0), cube(15.0)],
            negatives: vec![Mesh::default(), Mesh::default()],
            parts: Vec::new(),
            plate: 0,
            first_layer_sequence: Vec::new(),
            plate_settings: Default::default(),
            bounds: ([0.0; 3], [25.0, 10.0, 10.0]),
            placed: true,
            custom_gcode: None,
        };
        let printer = Settings::from_json(
            r#"{"machine_start_gcode": "G28", "printable_area": ["0x0", "200x0", "200x200", "0x200"],
                "nozzle_diameter": ["0.4"], "printable_height": "200"}"#,
        )
        .unwrap();
        let chosen = Chosen {
            machine: Some(("printer".into(), printer)),
            process: Some(("process".into(), Settings::from_json(process).unwrap())),
            filaments: Vec::new(),
        };
        let job = crate::prepare(loaded, &chosen, &Overrides::default()).unwrap();
        crate::slice(job, &Unwatched).unwrap()
    }

    /// The walls of the layer `index` that lie in x from `low` to `high`.
    pub(crate) fn loops_within(sliced: &Sliced, index: usize, low: f64, high: f64) -> usize {
        (0..sliced.slots.len())
            .flat_map(|slot| &sliced.walls_by[slot][index].loops)
            .filter(|wall| {
                wall.polygon.points.iter().all(|point| {
                    let x = edge_geom::unscaled(point.x);
                    x >= low && x <= high
                })
            })
            .count()
    }

    #[test]
    fn an_object_and_a_part_set_their_own_walls() {
        let plain = two_cubes(&[], &[], Vec::new(), Vec::new(), r#"{"wall_loops": "2"}"#);
        assert_eq!(plain.slots.len(), 1);
        assert_eq!(loops_within(&plain, 20, -1.0, 11.0), 2);
        assert_eq!(loops_within(&plain, 20, 14.0, 26.0), 2);
        // The second object's own count, and the part's over it.
        let own = two_cubes(
            &[("wall_loops", "4")],
            &[],
            Vec::new(),
            Vec::new(),
            r#"{"wall_loops": "2"}"#,
        );
        assert_eq!(own.slots, vec![1, 1]);
        assert_eq!(loops_within(&own, 20, -1.0, 11.0), 2);
        assert_eq!(loops_within(&own, 20, 14.0, 26.0), 4);
        let part = two_cubes(
            &[("wall_loops", "4")],
            &[("wall_loops", "3")],
            Vec::new(),
            Vec::new(),
            r#"{"wall_loops": "2"}"#,
        );
        assert_eq!(loops_within(&part, 20, 14.0, 26.0), 3);
        // A part that sets back what its object changed is the plate's.
        let back = two_cubes(
            &[("wall_loops", "4")],
            &[("wall_loops", "2")],
            Vec::new(),
            Vec::new(),
            r#"{"wall_loops": "2"}"#,
        );
        assert_eq!(back.slots.len(), 1);
        // What is set and not followed is said; what a part may not set
        // is not, as the reference takes no notice of it either.
        let noted = two_cubes(
            &[("layer_height", "0.1"), ("travel_speed", "50")],
            &[("brim_width", "9")],
            Vec::new(),
            Vec::new(),
            "{}",
        );
        let gaps = noted.gaps();
        assert!(
            gaps.iter()
                .any(|gap| gap.starts_with("layer_height: set on object \"second\"")),
            "{gaps:?}"
        );
        assert!(!gaps.iter().any(|gap| gap.starts_with("travel_speed")));
        assert!(!gaps.iter().any(|gap| gap.starts_with("brim_width")));
    }

    #[test]
    fn the_key_lists_are_sorted_and_apart() {
        assert!(OBJECT_KEYS.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(REGION_KEYS.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(OBJECT_KEYS.iter().all(|key| !REGION_KEYS.contains(key)));
        assert_eq!(scope("wall_loops"), Scope::Region);
        assert_eq!(scope("layer_height"), Scope::Object);
        assert_eq!(scope("enable_support"), Scope::Object);
        assert_eq!(scope("travel_speed"), Scope::Plate);
    }

    #[test]
    fn what_is_settable_where_and_followed() {
        let walls = settable("wall_loops");
        assert!(walls.on_object && walls.on_part && walls.on_range);
        assert!(walls.followed_on_object && walls.followed_on_part);
        let density = settable("sparse_infill_density");
        assert!(density.followed_on_part);
        let layer = settable("layer_height");
        assert!(layer.on_object && !layer.on_part && layer.on_range);
        assert!(!layer.followed_on_object);
        let travel = settable("travel_speed");
        assert!(!travel.on_object && !travel.on_part);
    }

    /// A box from `low` to `high` as a modifier setting `settings`.
    fn modifier(low: [f32; 3], high: [f32; 3], settings: &[(&str, &str)]) -> OwnVolume {
        OwnVolume {
            kind: PartKind::Modifier,
            name: "box".into(),
            filament: None,
            settings: (settings.iter())
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            mesh: Mesh::from_triangle_soup(primitives::cuboid(low, high)),
        }
    }

    #[test]
    fn a_modifier_takes_what_it_overlaps_of_its_object_alone() {
        // A box through the right half of the second cube and on into the
        // first's place is not: it reaches over to x = -5, but only its
        // own object is modified.
        let walls = modifier(
            [20.0, -1.0, -1.0],
            [30.0, 11.0, 11.0],
            &[("wall_loops", "4")],
        );
        let sliced = two_cubes(&[], &[], vec![walls], Vec::new(), r#"{"wall_loops": "2"}"#);
        assert_eq!(sliced.slots.len(), 2);
        // The first cube keeps its two walls; the right half of the second
        // has four of its own, the left half two.
        assert_eq!(loops_within(&sliced, 20, -1.0, 11.0), 2);
        assert_eq!(loops_within(&sliced, 20, 14.0, 20.5), 2);
        assert_eq!(loops_within(&sliced, 20, 19.5, 26.0), 4);
        // A modifier that changes only the fill leaves the walls laid round
        // the whole of the object (Layer.cpp `make_perimeters`).
        let density = modifier(
            [20.0, -1.0, -1.0],
            [30.0, 11.0, 11.0],
            &[("sparse_infill_density", "50%")],
        );
        let sliced = two_cubes(
            &[],
            &[],
            vec![density],
            Vec::new(),
            r#"{"wall_loops": "2"}"#,
        );
        assert_eq!(loops_within(&sliced, 20, 14.0, 26.0), 2);
        // Two modifiers: the later is laid over the earlier where they
        // overlap, so the overlap has both.
        let first = modifier(
            [20.0, -1.0, -1.0],
            [30.0, 11.0, 11.0],
            &[("wall_loops", "4")],
        );
        let second = modifier(
            [10.0, -1.0, -1.0],
            [30.0, 5.0, 11.0],
            &[("sparse_infill_density", "50%")],
        );
        let sliced = two_cubes(&[], &[], vec![first, second], Vec::new(), "{}");
        let both = (0..sliced.slots.len()).find(|slot| {
            let own = sliced.regions.overrides(sliced.keys[*slot]);
            own.len() == 2
        });
        assert!(both.is_some(), "{:?}", sliced.keys);
        // A modifier with a filament of its own prints what it takes in
        // it, and the plate in two tools.
        let mut own_filament = modifier([20.0, -1.0, -1.0], [30.0, 11.0, 11.0], &[]);
        own_filament.filament = Some(2);
        let sliced = two_cubes(&[], &[], vec![own_filament], Vec::new(), "{}");
        assert_eq!(sliced.used, vec![1, 2]);
        assert_eq!(sliced.slots, vec![1, 2]);
    }

    #[test]
    fn a_height_range_sets_its_own_layers() {
        let range = edge_3mf::LayerRange {
            min_z: 4.0,
            max_z: 6.0,
            settings: vec![
                ("wall_loops".into(), "3".into()),
                ("layer_height".into(), "0.1".into()),
            ],
        };
        let sliced = two_cubes(&[], &[], Vec::new(), vec![range], r#"{"wall_loops": "2"}"#);
        // Below, within and above the range.
        assert_eq!(loops_within(&sliced, 10, 14.0, 26.0), 2);
        assert_eq!(loops_within(&sliced, 25, 14.0, 26.0), 3);
        assert_eq!(loops_within(&sliced, 40, 14.0, 26.0), 2);
        assert_eq!(loops_within(&sliced, 25, -1.0, 11.0), 2);
        // Its layer height is said not to be followed.
        let gaps = sliced.gaps();
        assert!(
            gaps.iter()
                .any(|gap| gap.starts_with("layer_height: set on height range 4-6 mm")),
            "{gaps:?}"
        );
    }

    #[test]
    fn what_is_painted_underneath_is_laid_on_the_layers_it_hangs_over() {
        // A square face 1 mm up, facing down; and one facing up.
        let down: Vec<edge_mesh::Triangle> = vec![
            [[0.0, 0.0, 1.0], [0.0, 2.0, 1.0], [2.0, 2.0, 1.0]],
            [[0.0, 0.0, 1.0], [2.0, 2.0, 1.0], [2.0, 0.0, 1.0]],
        ];
        let up: Vec<edge_mesh::Triangle> = down.iter().map(|[a, b, c]| [*a, *c, *b]).collect();
        let planes = [0.1, 0.3, 0.5, 0.7, 0.9, 1.1, 1.3];
        let laid = project_down(&down, &planes).unwrap();
        let area = |layer: &[edge_geom::ExPolygon]| -> f64 {
            layer.iter().map(edge_geom::ExPolygon::area_mm2).sum()
        };
        for (index, layer) in laid.iter().enumerate() {
            let expected = if index == 5 { 4.0 } else { 0.0 };
            assert!(
                (area(layer) - expected).abs() < 1e-6,
                "{index}: {}",
                area(layer)
            );
        }
        assert!(
            project_down(&up, &planes)
                .unwrap()
                .iter()
                .all(Vec::is_empty)
        );
        // A face leaning out as it goes down is laid out in strips, one
        // a layer, each as wide as the face is in the layer's slab.
        let leaning: Vec<edge_mesh::Triangle> = vec![
            [[0.0, 0.0, 0.0], [1.0, 2.0, 1.0], [1.0, 0.0, 1.0]],
            [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [1.0, 2.0, 1.0]],
        ];
        let laid = project_down(&leaning, &planes).unwrap();
        assert!((area(&laid[1]) - 0.4).abs() < 1e-6, "{}", area(&laid[1]));
        let all: f64 = laid.iter().map(|layer| area(layer)).sum();
        assert!((all - 2.0).abs() < 1e-6, "{all}");
    }

    #[test]
    fn an_object_sets_its_own_brim() {
        let first_layer_brim = |sliced: &Sliced| -> usize {
            (sliced.around[0].iter())
                .filter(|line| line.role == edge_extrusion::Role::Brim)
                .count()
        };
        let none = two_cubes(
            &[],
            &[],
            Vec::new(),
            Vec::new(),
            r#"{"brim_type": "no_brim"}"#,
        );
        assert_eq!(first_layer_brim(&none), 0);
        let own = two_cubes(
            &[("brim_type", "outer_only"), ("brim_width", "3")],
            &[],
            Vec::new(),
            Vec::new(),
            r#"{"brim_type": "no_brim"}"#,
        );
        // Round the second cube alone, and said nowhere as not followed.
        assert!(first_layer_brim(&own) > 0);
        assert!(
            (own.around[0].iter())
                .filter(|line| line.role == edge_extrusion::Role::Brim)
                .flat_map(|line| &line.polygon.points)
                .all(|point| edge_geom::unscaled(point.x) > 11.0)
        );
        assert!(!own.gaps().iter().any(|gap| gap.starts_with("brim")));
        assert!(settable("brim_width").followed_on_object);
    }

    #[test]
    fn regions_are_found_again_by_what_they_set() {
        let plate =
            Settings::from_json(r#"{"wall_loops": "2", "sparse_infill_density": "15%"}"#).unwrap();
        let mut regions = Regions::default();
        let mut overrides = BTreeMap::new();
        // Nothing of its own: the filament's region.
        assert_eq!(regions.key(&plate, 2, &overrides), 2);
        lay_over(
            &mut overrides,
            &plate,
            &[("wall_loops".into(), "5".into())],
            |_| true,
        );
        let five = regions.key(&plate, 1, &overrides);
        assert_eq!(five, BASE);
        assert_eq!(regions.key(&plate, 1, &overrides), BASE);
        assert_eq!(regions.key(&plate, 2, &overrides), BASE + 1);
        assert_eq!(regions.filament(BASE + 1), 2);
        assert_eq!(
            regions
                .own(five)
                .unwrap()
                .settings
                .integer("wall_loops")
                .unwrap(),
            Some(5)
        );
        // A part that sets it back to the plate's value is the plate's.
        lay_over(
            &mut overrides,
            &plate,
            &[("wall_loops".into(), "2".into())],
            |_| true,
        );
        assert!(overrides.is_empty());
    }
}
