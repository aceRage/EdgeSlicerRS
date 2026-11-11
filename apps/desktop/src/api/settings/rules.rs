//! Which process options are shown and which can be set, by what the
//! others are set to: a port of the reference's
//! `ConfigManipulation::toggle_print_fff_options`
//! (src/slic3r/GUI/ConfigManipulation.cpp), statement by statement and in
//! its order, so that a later rule for an option replaces an earlier one
//! as it does there. `toggle_line` hides an option's line; `toggle_field`
//! greys the option out.
//!
//! Left out, each where it stands: the values the reference changes by
//! itself while it toggles (arc fitting turned off under extrusion rate
//! smoothing, the smoothing's segment length brought to 1, a fill of
//! several lines brought to one where the pattern has no such form, the
//! reverse threshold set to 0% with "Reverse only internal perimeters").
//! This page changes no value that was not changed by hand.
//! `brim_width` keeps one label; the reference calls it "Brim ear radius"
//! with brim ears.

use std::collections::BTreeMap;

use edge_config::Settings;

/// The settings the rules read, every one of them present: what a slice
/// would be given, over the reference's defaults.
struct Config<'a>(&'a Settings);

impl Config<'_> {
    /// A setting's text; of a list, its first entry.
    fn text(&self, key: &str) -> &str {
        self.0
            .texts(key)
            .and_then(|texts| texts.first())
            .map_or("", |text| text.trim())
    }

    fn on(&self, key: &str) -> bool {
        matches!(self.text(key), "1" | "true")
    }

    /// A number, with or without a `%` after it.
    fn number(&self, key: &str) -> f64 {
        self.text(key)
            .trim_end_matches('%')
            .trim()
            .parse()
            .unwrap_or(0.0)
    }

    fn whole(&self, key: &str) -> i64 {
        self.number(key) as i64
    }

    fn is(&self, key: &str, value: &str) -> bool {
        self.text(key) == value
    }

    fn among(&self, key: &str, values: &[&str]) -> bool {
        values.contains(&self.text(key))
    }
}

/// What the rules say of each option they name; an option they do not
/// name is shown and can be set.
#[derive(Default)]
pub(super) struct Toggles {
    line: BTreeMap<String, bool>,
    field: BTreeMap<String, bool>,
}

impl Toggles {
    pub(super) fn line(&mut self, key: &str, shown: bool) {
        self.line.insert(key.to_owned(), shown);
    }

    pub(super) fn lines(&mut self, keys: &[&str], shown: bool) {
        for key in keys {
            self.line(key, shown);
        }
    }

    pub(super) fn field(&mut self, key: &str, enabled: bool) {
        self.field.insert(key.to_owned(), enabled);
    }

    pub(super) fn fields(&mut self, keys: &[&str], enabled: bool) {
        for key in keys {
            self.field(key, enabled);
        }
    }

    /// Whether an option's line is hidden.
    pub(super) fn hidden(&self, key: &str) -> bool {
        self.line.get(key) == Some(&false)
    }

    /// Whether an option is greyed out.
    pub(super) fn off(&self, key: &str) -> bool {
        self.field.get(key) == Some(&false)
    }
}

/// The rules, for the settings a slice would be given (`settled`, the
/// printer's and the filaments' among them) in a project of `filaments`
/// filaments.
pub(super) fn toggles(settled: &Settings, filaments: usize) -> Toggles {
    let mut complete = edge_config::reference_defaults().clone();
    complete.overlay(settled);
    let config = Config(&complete);
    let mut t = Toggles::default();

    let gcf_is_marlin_firmware = config.is("gcode_flavor", "marlin2");
    let gcf_is_klipper = config.is("gcode_flavor", "klipper");
    let semm = config.on("single_extruder_multi_material");
    // The reference asks the vendor of the printer selected
    // (`PresetBundle::is_bbl_vendor`); the printer's model says the same
    // for every profile it ships (WipeTowerEstimate.cpp
    // `resolve_wipe_tower_type`), and a project's own settings have it.
    let is_bbl_printer = config.text("printer_model").starts_with("Bambu Lab");

    let have_volumetric_extrusion_rate_slope =
        config.number("max_volumetric_extrusion_rate_slope") > 0.0;
    t.field("enable_arc_fitting", !have_volumetric_extrusion_rate_slope);
    t.line(
        "max_volumetric_extrusion_rate_slope_segment_length",
        have_volumetric_extrusion_rate_slope,
    );
    t.line(
        "extrusion_rate_smoothing_external_perimeter_only",
        have_volumetric_extrusion_rate_slope,
    );

    let have_perimeters = config.whole("wall_loops") > 0;
    t.fields(
        &[
            "extra_perimeters_on_overhangs",
            "ensure_vertical_shell_thickness",
            "detect_thin_wall",
            "detect_overhang_wall",
            "seam_position",
            "staggered_inner_seams",
            "wall_sequence",
            "outer_wall_line_width",
            "inner_wall_speed",
            "outer_wall_speed",
            "small_perimeter_speed",
            "small_perimeter_threshold",
        ],
        have_perimeters,
    );

    // Part joints only steer the Aligned family.
    t.field(
        "seam_prefer_part_joints",
        have_perimeters
            && config.among(
                "seam_position",
                &["aligned", "aligned_back", "left", "right", "aligned_front"],
            ),
    );

    let have_infill = config.number("sparse_infill_density") > 0.0;
    t.lines(
        &[
            "sparse_infill_pattern",
            "infill_combination",
            "fill_multiline",
            "infill_direction",
            "infill_complete_top",
            "minimum_sparse_infill_area",
            "infill_shift_step",
            "sparse_infill_rotate_template",
            "symmetric_infill_y_axis",
        ],
        have_infill,
    );

    let pattern = config.text("sparse_infill_pattern");
    {
        let have_infill_anchor = have_infill && !matches!(pattern, "concentric" | "spiralinset");
        t.line("infill_anchor", have_infill_anchor);
        t.line("infill_anchor_max", have_infill_anchor);
    }

    let have_combined_infill = config.on("infill_combination") && have_infill;
    t.line("infill_combination_max_layer_height", have_combined_infill);

    // Infill patterns that support multiline infill.
    let have_multiline_infill_pattern = matches!(
        pattern,
        "gyroid"
            | "grid"
            | "rectilinear"
            | "tpmsd"
            | "tpmsfk"
            | "crosshatch"
            | "honeycomb"
            | "lateral-lattice"
            | "lateral-honeycomb"
            | "cubic"
            | "tri-hexagon"
            | "alignedrectilinear"
            | "lightning"
            | "3dhoneycomb"
            | "adaptivecubic"
            | "supportcubic"
    );

    // PrintConfig.hpp `is_adaptive_infill_pattern`.
    let have_adaptive_infill_pattern = have_infill
        && config.number("sparse_infill_density") < 100.0
        && matches!(pattern, "gyroid" | "tpmsd" | "tpmsfk");
    t.line("adaptive_infill_mode", have_adaptive_infill_pattern);
    let have_adaptive_infill =
        have_adaptive_infill_pattern && !config.is("adaptive_infill_mode", "disabled");
    t.line("adaptive_infill_interior_density", have_adaptive_infill);
    t.line("adaptive_infill_gradient", have_adaptive_infill);

    t.line(
        "gyroid_optimized",
        have_infill && pattern == "gyroid" && !have_adaptive_infill,
    );
    // PrintConfig.hpp `is_smoothable_infill_pattern`.
    t.line(
        "sparse_infill_smooth_factor",
        matches!(
            pattern,
            "hilbertcurve"
                | "octagramspiral"
                | "lightning"
                | "honeycomb"
                | "3dhoneycomb"
                | "concentric"
                | "crosshatch"
        ),
    );

    if have_infill {
        t.field("fill_multiline", have_multiline_infill_pattern);
        let infill_anchor = pattern != "line";
        t.field("infill_anchor_max", infill_anchor);
        // Only allow configuration of open anchors if the anchoring is enabled.
        let has_infill_anchors = infill_anchor && config.number("infill_anchor_max") > 0.0;
        t.field("infill_anchor", has_infill_anchors);
    }

    let is_cross_zag = pattern == "crosszag";
    let is_locked_zig = pattern == "lockedzag";
    t.line("infill_shift_step", is_cross_zag || is_locked_zig);
    t.lines(
        &[
            "skeleton_infill_density",
            "skin_infill_density",
            "infill_lock_depth",
            "skin_infill_depth",
            "skin_infill_line_width",
            "skeleton_infill_line_width",
            "locked_skin_infill_pattern",
            "locked_skeleton_infill_pattern",
            "infill_instead_top_bottom_surfaces",
        ],
        is_locked_zig,
    );
    let is_zig_zag = pattern == "zigzag";
    t.line(
        "symmetric_infill_y_axis",
        is_zig_zag || is_cross_zag || is_locked_zig,
    );
    t.line(
        "infill_complete_top",
        have_infill && pattern == "3dhoneycomb",
    );

    let has_spiral_vase = config.on("spiral_mode");
    t.line("spiral_mode_smooth", has_spiral_vase);
    t.line(
        "spiral_mode_max_xy_smoothing",
        has_spiral_vase && config.on("spiral_mode_smooth"),
    );
    t.line("spiral_starting_flow_ratio", has_spiral_vase);
    t.line("spiral_finishing_flow_ratio", has_spiral_vase);
    let has_top_shell_layers = config.whole("top_shell_layers") > 0
        || (has_spiral_vase && config.whole("bottom_shell_layers") > 1);
    let has_top_shell = has_top_shell_layers && config.number("top_surface_density") > 0.0;
    let has_bottom_shell = config.whole("bottom_shell_layers") > 0;
    let has_solid_infill = has_top_shell_layers || has_bottom_shell;
    t.field("top_surface_pattern", has_top_shell);
    t.field("undertop_surface_pattern", has_top_shell_layers);
    t.line(
        "undertop_surface_pattern",
        config.number("top_surface_density") < 100.0 && config.whole("top_shell_layers") > 2,
    );
    t.field("bottom_surface_pattern", has_bottom_shell);
    t.field("top_surface_density", has_top_shell_layers);
    t.field("bottom_surface_density", has_bottom_shell);
    t.field("top_layer_direction", has_top_shell);
    t.field("bottom_layer_direction", has_bottom_shell);

    t.line("top_surface_expansion", has_top_shell);
    t.line("top_surface_expansion_margin", has_top_shell);
    let has_top_surface_expansion = config.number("top_surface_expansion") > 0.0;
    t.field("top_surface_expansion_margin", has_top_surface_expansion);
    t.line("top_surface_expansion_direction", has_top_shell);
    t.field("top_surface_expansion_direction", has_top_surface_expansion);

    // Archimedean Chords and Octagram Spiral are the centred surface
    // patterns the pattern-centring and separated-infill features act on.
    let centred = |key: &str| config.among(key, &["archimedeanchords", "octagramspiral"]);
    let has_centered_surface = (has_top_shell && centred("top_surface_pattern"))
        || (has_bottom_shell && centred("bottom_surface_pattern"));
    t.line("center_of_surface_pattern", has_centered_surface);

    // PrintConfig.hpp `is_separable_infill_pattern`.
    t.line(
        "separated_infills",
        matches!(
            pattern,
            "monotonic"
                | "monotonicline"
                | "rectilinear"
                | "alignedrectilinear"
                | "zigzag"
                | "crosszag"
                | "lockedzag"
                | "line"
                | "grid"
                | "triangles"
                | "tri-hexagon"
                | "cubic"
                | "quartercubic"
                | "honeycomb"
                | "3dhoneycomb"
                | "lateral-honeycomb"
                | "lateral-lattice"
                | "crosshatch"
                | "tpmsd"
                | "tpmsfk"
                | "gyroid"
                | "hilbertcurve"
                | "archimedeanchords"
                | "octagramspiral"
        ),
    );

    // Fill order is only meaningful for the centre-based surface patterns.
    let centred_fill = |key: &str| {
        config.among(
            key,
            &[
                "concentric",
                "spiralinset",
                "archimedeanchords",
                "octagramspiral",
            ],
        )
    };
    t.line(
        "top_surface_fill_order",
        has_top_shell && centred_fill("top_surface_pattern"),
    );
    t.line(
        "bottom_surface_fill_order",
        has_bottom_shell && centred_fill("bottom_surface_pattern"),
    );

    t.fields(
        &[
            "infill_direction",
            "sparse_infill_line_width",
            "gap_fill_target",
            "filter_out_gap_fill",
            "infill_wall_overlap",
            "sparse_infill_speed",
            "bridge_speed",
            "internal_bridge_speed",
            "bridge_angle",
            "internal_bridge_angle",
            "relative_bridge_angle",
            "bridge_expansion_extra",
            "solid_infill_direction",
            "solid_infill_rotate_template",
            "internal_solid_infill_pattern",
            "solid_infill_filament",
        ],
        have_infill || has_solid_infill,
    );

    t.field(
        "top_shell_thickness",
        !has_spiral_vase && has_top_shell_layers,
    );
    t.field(
        "bottom_shell_thickness",
        !has_spiral_vase && has_bottom_shell,
    );

    t.field("wall_direction", !has_spiral_vase);

    // Gap fill is allowed between perimeter lines even with no infill.
    t.field("gap_infill_speed", have_perimeters);

    t.fields(
        &["top_surface_line_width", "top_surface_speed"],
        has_top_shell,
    );

    let have_default_acceleration = config.number("default_acceleration") > 0.0;
    t.fields(
        &[
            "outer_wall_acceleration",
            "inner_wall_acceleration",
            "initial_layer_acceleration",
            "first_layer_travel_acceleration",
            "top_surface_acceleration",
            "travel_acceleration",
            "bridge_acceleration",
            "sparse_infill_acceleration",
            "internal_solid_infill_acceleration",
        ],
        have_default_acceleration,
    );

    // Marlin 2 plans with the junction deviation once the printer's own
    // value is above zero: it replaces the jerk settings, which go away.
    let junction_deviation_enabled =
        gcf_is_marlin_firmware && config.number("machine_max_junction_deviation") > 0.0;
    t.line("default_junction_deviation", gcf_is_marlin_firmware);
    t.field("default_junction_deviation", junction_deviation_enabled);
    t.field("default_jerk", !junction_deviation_enabled);
    const JERK: [&str; 7] = [
        "outer_wall_jerk",
        "inner_wall_jerk",
        "initial_layer_jerk",
        "first_layer_travel_jerk",
        "top_surface_jerk",
        "travel_jerk",
        "infill_jerk",
    ];
    if junction_deviation_enabled {
        t.lines(&JERK, false);
    } else {
        let have_default_jerk = config.number("default_jerk") > 0.0;
        t.lines(&JERK, true);
        t.fields(&JERK, have_default_jerk);
    }

    let have_skirt = config.whole("skirt_loops") > 0;
    let shielded = config.is("draft_shield", "enabled");
    t.field("skirt_height", have_skirt && !shielded);
    t.line("single_loop_draft_shield", have_skirt);
    t.fields(
        &[
            "skirt_type",
            "min_skirt_length",
            "skirt_distance",
            "skirt_start_angle",
            "skirt_speed",
            "draft_shield",
        ],
        have_skirt,
    );

    let brim_type = config.text("brim_type");
    let have_brim = brim_type != "no_brim";
    t.field("brim_object_gap", have_brim);
    t.field("brim_use_efc_outline", have_brim);
    t.field("combine_brims", have_brim);
    let have_brim_width = have_brim && brim_type != "auto_brim" && brim_type != "painted";
    t.field("brim_width", have_brim_width);
    t.field("brim_flow_ratio", have_brim);
    // wall_filament uses the same logic as in Print::extruders()
    t.field("wall_filament", have_perimeters || have_brim);
    t.field("outer_wall_filament", have_perimeters);

    let have_auto_brim_ear = brim_type == "brim_ears";
    let have_painted_brim_ear = brim_type == "painted";
    let brim_width = config.number("brim_width");
    // Automatic brim ear settings require a non-zero brim width.
    t.field("brim_ears_max_angle", brim_width > 0.0);
    t.field("brim_ears_detection_length", brim_width > 0.0);
    // Painted ears carry their own radius and do not depend on brim_width.
    t.field(
        "brim_ears_outer_only",
        have_painted_brim_ear || brim_width > 0.0,
    );
    t.line("brim_ears_max_angle", have_auto_brim_ear);
    t.line("brim_ears_detection_length", have_auto_brim_ear);
    t.line(
        "brim_ears_outer_only",
        have_auto_brim_ear || have_painted_brim_ear,
    );

    // The elephant foot's layers show while its compensation or its
    // layers' density is in use.
    t.line(
        "elefant_foot_compensation_layers",
        config.number("elefant_foot_compensation") > 0.0
            || config.number("elefant_foot_layers_density") / 100.0 < 1.0,
    );

    let have_raft = config.whole("raft_layers") > 0;
    let have_support_material = config.on("enable_support") || have_raft;
    // Side stabilizers do not depend on "Enable support".
    let stab_mode = config.text("stabilizer_supports");
    let stab_on = stab_mode != "off";

    let support_type = config.text("support_type");
    let is_tree = matches!(support_type, "tree(auto)" | "tree(manual)");
    let is_auto = matches!(support_type, "normal(auto)" | "tree(auto)");
    let have_support_interface = config.whole("support_interface_top_layers") > 0
        || config.whole("support_interface_bottom_layers") > 0;
    let have_support_soluble =
        have_support_material && config.number("support_top_z_distance") == 0.0;
    let support_style = config.text("support_style");
    t.fields(
        &[
            "support_style",
            "support_base_pattern",
            "support_base_pattern_spacing",
            "support_expansion",
            "support_angle",
            "support_interface_pattern",
            "support_interface_top_layers",
            "support_interface_bottom_layers",
            "bridge_no_support",
            "max_bridge_length",
            "support_top_z_distance",
            "support_bottom_z_distance",
            "support_type",
            "support_on_build_plate_only",
            "support_critical_regions_only",
            "support_interface_not_for_body",
            "support_object_xy_distance",
            "support_object_first_layer_gap",
            "independent_support_layer_height",
            "top_z_overrides_xy_distance",
        ],
        have_support_material,
    );
    t.line("hollow_shell_thickness", config.on("hollow_interior"));
    t.field("stabilizer_supports", true);
    t.field(
        "support_object_xy_distance",
        have_support_material || stab_on,
    );
    {
        t.lines(
            &[
                "stabilizer_ring_spacing",
                "stabilizer_points_per_ring",
                "stabilizer_tip_diameter",
                "stabilizer_tip_gap",
                "stabilizer_pillar_diameter",
                "stabilizer_max_island_width",
            ],
            stab_on,
        );
        // Manual places struts at the painted points only.
        t.fields(
            &[
                "stabilizer_ring_spacing",
                "stabilizer_points_per_ring",
                "stabilizer_max_island_width",
            ],
            stab_on && stab_mode == "auto",
        );
        let bracing = stab_on && config.on("stabilizer_bracing");
        let shape = config.text("stabilizer_column_shape");
        let columns = stab_on && shape != "round";
        let walls = stab_on && config.whole("stabilizer_wall_loops") > 0;
        t.lines(
            &[
                "stabilizer_pillar_base_diameter",
                "stabilizer_bracing",
                "stabilizer_column_shape",
                "stabilizer_wall_loops",
            ],
            stab_on,
        );
        t.lines(
            &["stabilizer_brace_max_unbraced", "stabilizer_brace_max_span"],
            bracing || (stab_on && shape == "auto"),
        );
        t.lines(
            &["stabilizer_column_width", "stabilizer_column_length"],
            columns,
        );
        t.line("stabilizer_column_min_height", stab_on && shape == "auto");
        t.lines(
            &["stabilizer_infill_density", "stabilizer_infill_pattern"],
            walls,
        );
    }
    t.field("support_threshold_angle", have_support_material && is_auto);
    t.field(
        "support_threshold_overlap",
        config.whole("support_threshold_angle") == 0 && have_support_material && is_auto,
    );

    let support_is_tree = config.on("enable_support") && is_tree;
    // Organic is what "default" means.
    let support_is_normal_tree =
        support_is_tree && support_style != "organic" && support_style != "default";
    let support_is_organic = support_is_tree && !support_is_normal_tree;
    t.lines(
        &[
            "tree_support_branch_angle",
            "tree_support_branch_distance",
            "tree_support_branch_diameter",
        ],
        support_is_normal_tree,
    );
    t.lines(
        &["tree_support_auto_brim", "tree_support_brim_width"],
        support_is_normal_tree,
    );
    t.lines(
        &[
            "tree_support_branch_angle_organic",
            "tree_support_branch_distance_organic",
            "tree_support_branch_diameter_organic",
            "tree_support_angle_slow",
            "tree_support_tip_diameter",
            "tree_support_top_rate",
            "tree_support_branch_diameter_angle",
        ],
        support_is_organic,
    );
    t.field(
        "tree_support_brim_width",
        support_is_tree && !config.on("tree_support_auto_brim"),
    );
    // Tree supports use max_bridge_length instead of bridge_no_support.
    t.line("max_bridge_length", support_is_tree);
    t.line("bridge_no_support", !support_is_tree);
    t.line("support_critical_regions_only", is_auto && support_is_tree);

    t.fields(
        &[
            "support_interface_filament",
            "support_filament_matching",
            "support_interface_loop_pattern",
            "support_bottom_interface_spacing",
        ],
        have_support_material && have_support_interface,
    );

    let can_over_support = have_support_material && config.number("support_top_z_distance") > 0.0;
    t.field("over_support_surfaces", can_over_support);
    t.fields(
        &["over_support_flow", "over_support_speed"],
        can_over_support && config.on("over_support_surfaces"),
    );

    let can_ironing_support =
        have_raft || (have_support_material && config.whole("support_interface_top_layers") > 0);
    t.field("support_ironing", can_ironing_support);
    let has_support_ironing = can_ironing_support && config.on("support_ironing");
    t.lines(
        &[
            "support_ironing_pattern",
            "support_ironing_flow",
            "support_ironing_spacing",
            "support_ironing_inset",
            "support_ironing_speed",
            "support_ironing_direction",
        ],
        has_support_ironing,
    );
    // A solid support interface is forced with support ironing.
    t.field(
        "support_interface_spacing",
        have_support_material && have_support_interface && !has_support_ironing,
    );

    let have_skirt_height = have_skirt && (config.whole("skirt_height") > 1 || !shielded);
    t.line(
        "support_speed",
        have_support_material || have_skirt_height || stab_on,
    );
    t.line(
        "support_interface_speed",
        have_support_material && have_support_interface,
    );
    t.lines(
        &[
            "small_support_perimeter_speed",
            "small_support_perimeter_threshold",
        ],
        have_support_material,
    );

    t.field(
        "inner_wall_line_width",
        have_perimeters || have_skirt || have_brim,
    );
    t.field(
        "support_filament",
        have_support_material || have_skirt || stab_on,
    );

    t.line("raft_contact_distance", have_raft && !have_support_soluble);
    t.line("raft_expansion", have_raft);
    // Slim, Strong and Hybrid trees size their raft by the first layer's
    // expansion only.
    t.field(
        "raft_expansion",
        !(is_tree && matches!(support_style, "tree_slim" | "tree_strong" | "tree_hybrid")),
    );
    t.field("raft_first_layer_density", have_support_material);
    t.field(
        "raft_first_layer_expansion",
        have_support_material
            && ((!support_is_normal_tree || support_style == "tree_hybrid") || have_raft),
    );

    let has_ironing = !config.is("ironing_type", "no ironing");
    t.lines(
        &[
            "ironing_pattern",
            "ironing_flow",
            "ironing_spacing",
            "ironing_angle",
            "ironing_inset",
            "ironing_angle_fixed",
        ],
        has_ironing,
    );
    t.line("ironing_speed", has_ironing || has_support_ironing);

    // The detail settings only matter once Z contouring is on.
    let has_zaa = config.on("zaa_enabled");
    t.lines(
        &[
            "zaa_minimize_perimeter_height",
            "zaa_min_z",
            "zaa_dont_alternate_fill_direction",
            "zaa_speed_scaling",
        ],
        has_zaa,
    );

    let have_sequential_printing = config.is("print_sequence", "by object");
    t.field("print_order", !have_sequential_printing);

    t.field("single_extruder_multi_material", !is_bbl_printer);

    t.field("ooze_prevention", !semm);
    let have_ooze_prevention = config.on("ooze_prevention");
    t.line("standby_temperature_delta", have_ooze_prevention);
    t.line("preheat_time", have_ooze_prevention);
    t.line(
        "preheat_steps",
        have_ooze_prevention && config.whole("preheat_steps") > 0,
    );

    let have_prime_tower = config.on("enable_prime_tower");
    // The options of the second tower generator follow the printer's
    // tower type; a Bambu Lab printer is always on the first.
    let supports_wipe_tower_2 = !is_bbl_printer && !config.is("wipe_tower_type", "type1");
    t.lines(
        &["prime_tower_width", "prime_tower_brim_width"],
        have_prime_tower,
    );

    // Dual-extruder Bambu machines only
    // (`DynamicPrintConfig::support_different_extruders`).
    {
        let nozzles = complete.texts("nozzle_diameter").map_or(0, <[String]>::len);
        let mut variants: Vec<&str> = complete
            .texts("extruder_variant_list")
            .unwrap_or_default()
            .iter()
            .take(nozzles)
            .flat_map(|list| list.split(','))
            .filter(|variant| !variant.is_empty())
            .collect();
        variants.sort_unstable();
        variants.dedup();
        let is_grouping_machine = variants.len() > 1 && nozzles <= 2;
        t.line("preload_all_filaments", is_grouping_machine);
        t.field(
            "preload_all_filaments",
            is_grouping_machine && have_prime_tower,
        );
    }

    // "Filament for Features": shown whenever the project has more than
    // one filament, on every printer.
    let have_multiple_filaments = filaments > 1;
    t.lines(
        &[
            "wall_filament",
            "outer_wall_filament",
            "sparse_infill_filament",
            "solid_infill_filament",
            "wipe_tower_filament",
        ],
        have_multiple_filaments,
    );

    let purge_in_primetower = config.on("purge_in_prime_tower");

    t.lines(
        &[
            "wipe_tower_max_purge_speed",
            "wipe_tower_wall_type",
            "wipe_tower_extra_rib_length",
            "wipe_tower_rib_width",
            "wipe_tower_fillet_wall",
        ],
        have_prime_tower,
    );
    t.lines(
        &[
            "wipe_tower_cone_angle",
            "wipe_tower_extra_spacing",
            "wipe_tower_bridging",
            "wipe_tower_extra_flow",
        ],
        have_prime_tower && supports_wipe_tower_2,
    );
    t.lines(
        &["prime_tower_acceleration", "prime_tower_line_width"],
        have_prime_tower && !supports_wipe_tower_2,
    );
    t.lines(
        &["prime_tower_wipe_speed", "prime_tower_wipe_distance"],
        have_prime_tower,
    );
    t.line("wipe_tower_rotation_angle", have_prime_tower);
    t.field(
        "wipe_tower_rotation_angle",
        have_prime_tower && supports_wipe_tower_2,
    );

    t.line("wipe_tower_no_sparse_layers", have_prime_tower);
    t.line(
        "wipe_tower_sparse_layers_combination",
        have_prime_tower && !config.on("wipe_tower_no_sparse_layers"),
    );

    t.lines(
        &[
            "wipe_tower_interface_temp",
            "wipe_tower_interface_run_in",
            "wipe_tower_interface_extra_prime",
            "wipe_tower_interface_trigger",
        ],
        have_prime_tower,
    );
    t.field(
        "wipe_tower_interface_run_in",
        have_prime_tower && config.on("wipe_tower_wall_gap"),
    );
    let have_tower_interface = config.on("wipe_tower_interface_temp")
        || (config.on("wipe_tower_interface_run_in") && config.on("wipe_tower_wall_gap"))
        || config.on("wipe_tower_interface_extra_prime");
    t.field(
        "wipe_tower_interface_trigger",
        have_prime_tower && have_tower_interface,
    );

    let local_z_dithering_enabled = config.on("dithering_local_z_mode");
    t.line("dithering_local_z_whole_objects", local_z_dithering_enabled);
    t.line("dithering_local_z_infill", local_z_dithering_enabled);
    t.line(
        "dithering_local_z_direct_multicolor",
        local_z_dithering_enabled,
    );

    let wall_type = config.text("wipe_tower_wall_type");
    t.line(
        "wipe_tower_cone_angle",
        have_prime_tower && supports_wipe_tower_2 && wall_type == "cone",
    );
    t.line(
        "wipe_tower_extra_rib_length",
        have_prime_tower && wall_type == "rib",
    );
    t.line(
        "wipe_tower_rib_width",
        have_prime_tower && wall_type == "rib",
    );
    t.line(
        "wipe_tower_fillet_wall",
        have_prime_tower && wall_type == "rib",
    );

    t.field("prime_tower_width", have_prime_tower && wall_type != "rib");

    t.line(
        "single_extruder_multi_material_priming",
        !semm && have_prime_tower && supports_wipe_tower_2,
    );
    t.line(
        "wipe_tower_stagger_toolchange_start",
        have_prime_tower && supports_wipe_tower_2,
    );

    let use_cyclic_ordering = config.is("toolchange_ordering", "cyclic");
    t.line("toolchange_cyclic_order", use_cyclic_ordering);
    t.line("toolchange_cyclic_first_layer", use_cyclic_ordering);

    t.line(
        "prime_volume",
        have_prime_tower && (!purge_in_primetower || !semm),
    );

    t.fields(
        &[
            "flush_into_infill",
            "flush_into_support",
            "flush_into_objects",
        ],
        have_prime_tower,
    );

    let have_avoid_crossing_perimeters = config.on("reduce_crossing_wall");
    t.line("max_travel_detour_distance", have_avoid_crossing_perimeters);

    let has_overhang_speed = config.on("enable_overhang_speed");
    t.lines(
        &[
            "overhang_1_4_speed",
            "overhang_2_4_speed",
            "overhang_3_4_speed",
            "overhang_4_4_speed",
        ],
        has_overhang_speed,
    );
    t.line("slowdown_for_curled_perimeters", has_overhang_speed);
    t.line("overhang_shadow_inner_wall", has_overhang_speed);
    let has_overhang_shadow = has_overhang_speed && config.on("overhang_shadow_inner_wall");
    t.line("overhang_shadow_speed_ratio", has_overhang_shadow);
    t.line("overhang_shadow_distance", has_overhang_shadow);

    // Layer-time speed smoothing: the speed-up fields for the speed-up
    // modes, the slow-down fields for "Slow down".
    {
        let mode = config.text("layer_time_speed_smoothing");
        let speed_up = matches!(mode, "speed_up_exclude_outer" | "speed_up_all");
        let slow_down = mode == "slow_down";
        t.line("layer_time_speed_max_variation", speed_up || slow_down);
        t.line("layer_time_speed_max_speedup", speed_up);
        t.line("layer_time_speed_max_slowdown", slow_down);
        t.line("layer_time_speed_max_time_increase", slow_down);
        t.line("layer_time_speed_slowdown_scope", slow_down);
    }

    // The custom pause position only for Custom X/Y.
    {
        let custom_park = config.is("min_layer_time_pause_park", "custom");
        t.line("min_layer_time_pause_park_x", custom_park);
        t.line("min_layer_time_pause_park_y", custom_park);
    }

    // These are the settings of the whole print (`is_global_config`), so
    // flushing into objects, which is set object by object, is not shown.
    t.line("flush_into_objects", false);

    t.line(
        "support_interface_not_for_body",
        config.whole("support_interface_filament") != 0 && config.whole("support_filament") == 0,
    );

    // The interface interlock needs an interface filament that is not
    // the support's own.
    {
        let iface_filament = config.whole("support_interface_filament");
        let separate_iface =
            iface_filament != 0 && iface_filament != config.whole("support_filament");
        let interlock_visible = separate_iface && have_support_interface;
        t.line("support_interface_interlock", interlock_visible);
        let interlock_on = interlock_visible && config.on("support_interface_interlock");
        t.line("support_interface_interlock_layers", interlock_on);
        t.line("support_interface_interlock_beam_lines", interlock_on);
        let interlock_editable = have_support_material && !config.on("support_filament_matching");
        t.fields(
            &[
                "support_interface_interlock",
                "support_interface_interlock_layers",
                "support_interface_interlock_beam_lines",
            ],
            interlock_editable,
        );
    }

    // The fuzzy skin rows show unless fuzzy skin is "Disabled"; "None"
    // keeps them, for what is painted on.
    let has_fuzzy_skin = !config.is("fuzzy_skin", "disabled_fuzzy");
    t.lines(
        &[
            "fuzzy_skin_mode",
            "fuzzy_skin_noise_type",
            "fuzzy_skin_point_distance",
            "fuzzy_skin_thickness",
            "fuzzy_skin_first_layer",
        ],
        has_fuzzy_skin,
    );
    let noise = config.text("fuzzy_skin_noise_type");
    let is_ripple = noise == "ripple";
    t.line(
        "fuzzy_skin_scale",
        noise != "classic" && !is_ripple && has_fuzzy_skin,
    );
    t.line(
        "fuzzy_skin_octaves",
        noise != "classic" && noise != "voronoi" && !is_ripple && has_fuzzy_skin,
    );
    t.line(
        "fuzzy_skin_persistence",
        matches!(noise, "perlin" | "billow") && !is_ripple && has_fuzzy_skin,
    );
    t.line("fuzzy_skin_ripples_per_layer", is_ripple && has_fuzzy_skin);
    t.line("fuzzy_skin_ripple_offset", is_ripple && has_fuzzy_skin);
    t.line(
        "fuzzy_skin_layers_between_ripple_offset",
        is_ripple && has_fuzzy_skin,
    );
    t.line(
        "fuzzy_skin_skip_overhangs",
        config.on("detect_overhang_wall") && has_fuzzy_skin,
    );

    let have_arachne = config.is("wall_generator", "arachne");
    t.lines(
        &[
            "wall_transition_length",
            "wall_transition_filter_deviation",
            "wall_transition_angle",
            "min_feature_size",
            "min_length_factor",
            "min_bead_width",
            "wall_distribution_count",
            "initial_layer_min_bead_width",
            "wall_maximum_resolution",
            "wall_maximum_deviation",
        ],
        have_arachne,
    );
    t.field("detect_thin_wall", !have_arachne);

    t.field("wipe_speed", !config.on("role_based_wipe_speed"));

    t.line("wipe_inward_distance", config.on("wipe_inward"));

    t.lines(
        &["accel_to_decel_enable", "accel_to_decel_factor"],
        gcf_is_klipper,
    );
    if gcf_is_klipper {
        t.field("accel_to_decel_factor", config.on("accel_to_decel_enable"));
    }

    // Only Klipper's EXCLUDE_OBJECT_DEFINE carries the outline.
    t.line(
        "exclude_object_tolerance",
        gcf_is_klipper && config.on("exclude_object"),
    );

    let have_make_overhang_printable = config.on("make_overhang_printable");
    t.line(
        "make_overhang_printable_angle",
        have_make_overhang_printable,
    );
    t.line(
        "make_overhang_printable_hole_size",
        have_make_overhang_printable,
    );

    // The one-wall options act on top and bottom surfaces, which exist
    // only with a shell.
    t.line("only_one_wall_first_layer", has_bottom_shell);
    t.line("only_one_wall_top", has_top_shell_layers);
    t.line(
        "min_width_top_surface",
        (has_top_shell_layers && config.on("only_one_wall_top"))
            || (config.number("min_length_factor") > 0.5 && have_arachne),
    );

    t.lines(
        &[
            "hole_to_polyhole_threshold",
            "hole_to_polyhole_twisted",
            "hole_to_polyhole_max_edges",
        ],
        config.on("hole_to_polyhole"),
    );

    {
        let circle_compensation = config.on("enable_circle_compensation");
        t.field("xy_hole_compensation", !circle_compensation);
        t.field("xy_contour_compensation", !circle_compensation);
        t.line("circle_compensation_manual_offset", circle_compensation);
        t.line("apply_scarf_seam_on_circles", circle_compensation);
    }

    let has_detect_overhang_wall = config.on("detect_overhang_wall");
    t.line("unsupported_wall_last", has_detect_overhang_wall);
    let has_overhang_reverse = config.on("overhang_reverse");
    let force_wall_direction = !config.is("wall_direction", "auto");
    let allow_overhang_reverse = !has_spiral_vase && !force_wall_direction;
    t.line("overhang_reverse", allow_overhang_reverse);
    t.line(
        "overhang_reverse_internal_only",
        allow_overhang_reverse && has_overhang_reverse,
    );
    let has_overhang_reverse_internal_only = config.on("overhang_reverse_internal_only");
    t.line(
        "overhang_reverse_threshold",
        has_detect_overhang_wall
            && allow_overhang_reverse
            && has_overhang_reverse
            && !has_overhang_reverse_internal_only,
    );
    t.line("timelapse_type", is_bbl_printer);

    t.line(
        "small_area_infill_flow_compensation_model",
        config.on("small_area_infill_flow_compensation"),
    );

    t.field("seam_slope_type", !has_spiral_vase);
    let has_seam_slope = !has_spiral_vase && !config.is("seam_slope_type", "none");
    t.lines(
        &[
            "seam_slope_conditional",
            "seam_slope_start_height",
            "seam_slope_entire_loop",
            "seam_slope_min_length",
            "seam_slope_steps",
            "seam_slope_inner_walls",
            "scarf_joint_speed",
            "scarf_joint_flow_ratio",
        ],
        has_seam_slope,
    );
    t.field(
        "seam_slope_min_length",
        !config.on("seam_slope_entire_loop"),
    );
    let conditional = has_seam_slope && config.on("seam_slope_conditional");
    t.line("scarf_angle_threshold", conditional);
    t.line("scarf_overhang_threshold", conditional);

    // The walls and millimetres of a paint depth are for their own mode.
    let paint_depth_mode = config.text("paint_depth_mode");
    t.line("paint_depth_walls", paint_depth_mode == "walls");
    t.line("paint_depth_mm", paint_depth_mode == "millimeters");
    t.line("paint_infill_override", paint_depth_mode != "unlimited");
    t.line(
        "paint_depth_solid_interfaces",
        paint_depth_mode != "unlimited",
    );

    let use_beam_interlocking = config.on("interlocking_beam");
    t.line(
        "mmu_segmented_region_interlocking_depth",
        !use_beam_interlocking,
    );
    t.lines(
        &[
            "interlocking_beam_width",
            "interlocking_orientation",
            "interlocking_beam_layer_count",
            "interlocking_depth",
            "interlocking_boundary_avoidance",
        ],
        use_beam_interlocking,
    );

    t.lines(
        &["lateral_lattice_angle_1", "lateral_lattice_angle_2"],
        pattern == "lateral-lattice",
    );

    // Adaptive Cubic and Support Cubic do not take an infill rotation.
    let fill_adaptive = matches!(pattern, "adaptivecubic" | "supportcubic");
    t.field("sparse_infill_rotate_template", !fill_adaptive);
    t.field(
        "infill_direction",
        config.text("sparse_infill_rotate_template").is_empty() && !fill_adaptive,
    );
    t.field(
        "solid_infill_direction",
        config.text("solid_infill_rotate_template").is_empty(),
    );

    t.line("infill_overhang_angle", pattern == "lateral-honeycomb");

    t.lines(
        &[
            "lightning_overhang_angle",
            "lightning_prune_angle",
            "lightning_straightening_angle",
        ],
        pattern == "lightning" && have_infill,
    );
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(json: &str) -> Toggles {
        toggles(&Settings::from_json(json).unwrap(), 1)
    }

    #[test]
    fn a_dependent_option_follows_the_one_it_depends_on() {
        // The reference's defaults: no scarf seam, no ironing, Arachne
        // walls, walls and shells there.
        let plain = with("{}");
        for hidden in [
            "seam_slope_conditional",
            "seam_slope_steps",
            "scarf_angle_threshold",
            "ironing_flow",
            "ironing_pattern",
            "zaa_min_z",
            "make_overhang_printable_angle",
            "hole_to_polyhole_threshold",
            "wipe_inward_distance",
            "max_travel_detour_distance",
            "small_area_infill_flow_compensation_model",
            "circle_compensation_manual_offset",
        ] {
            assert!(plain.hidden(hidden), "{hidden} is shown");
        }
        for shown in [
            "wall_transition_angle",
            "min_bead_width",
            "only_one_wall_top",
            "seam_slope_type",
            "layer_height",
        ] {
            assert!(!plain.hidden(shown), "{shown} is hidden");
            assert!(!plain.off(shown), "{shown} is off");
        }
        // Arachne has no use for thin wall detection.
        assert!(plain.off("detect_thin_wall"));

        let scarf = with(
            r#"{"seam_slope_type": "all", "seam_slope_conditional": "1", "seam_slope_entire_loop": "1"}"#,
        );
        for shown in [
            "seam_slope_conditional",
            "seam_slope_steps",
            "scarf_angle_threshold",
            "scarf_overhang_threshold",
            "scarf_joint_speed",
        ] {
            assert!(!scarf.hidden(shown), "{shown} is hidden");
        }
        // A scarf over the whole loop has no least length.
        assert!(scarf.off("seam_slope_min_length"));

        let classic = with(r#"{"wall_generator": "classic", "ironing_type": "top"}"#);
        assert!(classic.hidden("wall_transition_angle") && classic.hidden("min_bead_width"));
        assert!(!classic.off("detect_thin_wall"));
        assert!(!classic.hidden("ironing_flow") && !classic.hidden("ironing_angle_fixed"));
    }

    #[test]
    fn nothing_to_act_on_greys_an_option_out() {
        let no_walls = with(r#"{"wall_loops": "0"}"#);
        for off in [
            "seam_position",
            "wall_sequence",
            "outer_wall_line_width",
            "detect_overhang_wall",
            "seam_prefer_part_joints",
        ] {
            assert!(no_walls.off(off), "{off} can be set");
        }
        let no_shells = with(r#"{"top_shell_layers": "0", "bottom_shell_layers": "0"}"#);
        assert!(no_shells.hidden("only_one_wall_top"));
        assert!(no_shells.hidden("only_one_wall_first_layer"));
        assert!(no_shells.off("top_surface_line_width"));
        // Vase mode holds the wall direction and the scarf seam.
        let vase = with(r#"{"spiral_mode": "1"}"#);
        assert!(vase.off("wall_direction") && vase.off("seam_slope_type"));
        assert!(vase.hidden("overhang_reverse"));
        // The fixed compensation gives way to the one worked out.
        let circles = with(r#"{"enable_circle_compensation": "1"}"#);
        assert!(circles.off("xy_hole_compensation") && circles.off("xy_contour_compensation"));
        assert!(!circles.hidden("apply_scarf_seam_on_circles"));
    }

    #[test]
    fn rules_of_the_other_pages() {
        let plain = with("{}");
        // Supports are off by default: their options are greyed out.
        assert!(plain.off("support_type") && plain.off("support_threshold_angle"));
        assert!(plain.hidden("tree_support_tip_diameter"));
        let tree = with(r#"{"enable_support": "1", "support_type": "tree(auto)"}"#);
        assert!(!tree.off("support_type") && !tree.hidden("tree_support_tip_diameter"));
        assert!(tree.hidden("bridge_no_support") && !tree.hidden("max_bridge_length"));
        // A printer's own flavour decides what its speeds can be given.
        let klipper = with(r#"{"gcode_flavor": "klipper"}"#);
        assert!(!klipper.hidden("accel_to_decel_enable"));
        assert!(plain.hidden("accel_to_decel_enable"));
        // The tower's options need a tower; the second generator's are
        // not a Bambu Lab printer's.
        assert!(plain.hidden("prime_tower_width") && plain.off("flush_into_infill"));
        let tower = with(r#"{"enable_prime_tower": "1"}"#);
        assert!(!tower.hidden("prime_tower_width") && !tower.hidden("wipe_tower_extra_spacing"));
        let bambu = with(r#"{"enable_prime_tower": "1", "printer_model": "Bambu Lab X1 Carbon"}"#);
        assert!(bambu.hidden("wipe_tower_extra_spacing") && !bambu.hidden("timelapse_type"));
        assert!(!bambu.hidden("prime_tower_line_width"));
        assert!(plain.hidden("timelapse_type"));
        // One filament has nothing to share out among features.
        assert!(plain.hidden("wall_filament"));
        let two = toggles(&Settings::default(), 2);
        assert!(!two.hidden("wall_filament") && !two.hidden("sparse_infill_filament"));
        // Brim, skirt, fuzzy skin.
        let brim = with(r#"{"brim_type": "brim_ears", "brim_width": "5", "skirt_loops": "0"}"#);
        assert!(!brim.hidden("brim_ears_max_angle") && !brim.off("brim_width"));
        assert!(brim.off("skirt_distance") && plain.hidden("brim_ears_max_angle"));
        let fuzzy = with(r#"{"fuzzy_skin": "all", "fuzzy_skin_noise_type": "ripple"}"#);
        assert!(!fuzzy.hidden("fuzzy_skin_ripples_per_layer") && fuzzy.hidden("fuzzy_skin_scale"));
    }
}
