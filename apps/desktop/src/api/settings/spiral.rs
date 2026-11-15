//! Ticking "Spiral vase" (`spiral_mode`) on the process pages, as the
//! reference's `ConfigManipulation::update_print_fff_config` has it: where
//! the settings do not suit a vase, the person is asked whether to change
//! them (`show_spiral_mode_settings_dialog`); Yes changes them and turns
//! vase mode on, No leaves vase mode off.

use edge_config::Settings;

use super::tr;

/// What Yes sets, each where the process pages have it
/// (`update_print_fff_config`, `wxID_YES`).
pub(super) const FORCED: [(&str, &str); 10] = [
    ("wall_loops", "1"),
    ("top_shell_layers", "0"),
    ("sparse_infill_density", "0%"),
    ("enable_support", "0"),
    ("enforce_support_layers", "0"),
    ("detect_thin_wall", "0"),
    ("overhang_reverse", "0"),
    ("stabilizer_supports", "off"),
    ("wall_direction", "auto"),
    ("timelapse_type", "0"),
];

/// Whether `settled` (the settings with vase mode still off) already suit
/// a vase, so nothing is asked.
pub(super) fn suits(settled: &Settings) -> bool {
    let number = |key: &str| {
        settled
            .text(key)
            .map(|text| text.trim().trim_end_matches('%').trim())
            .and_then(|text| text.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let off = |key: &str| matches!(settled.text(key), None | Some("0" | "false"));
    number("wall_loops") == 1.0
        && number("top_shell_layers") == 0.0
        && number("sparse_infill_density") == 0.0
        && off("enable_support")
        && number("enforce_support_layers") == 0.0
        && off("detect_thin_wall")
        && off("overhang_reverse")
        && matches!(settled.text("stabilizer_supports"), None | Some("off"))
        && matches!(settled.text("wall_direction"), None | Some("auto"))
        && matches!(settled.text("timelapse_type"), None | Some("0"))
}

/// The question, in the reference's words (`show_spiral_mode_settings_dialog`).
pub(super) fn question(settled: &Settings) -> String {
    let mut text = tr(
        "Spiral mode only works when wall loops is 1, support is disabled, top shell layers is 0, sparse infill density is 0 and timelapse type is traditional.",
    );
    text.push(' ');
    text.push_str(&tr("Side stabilizers must be off too."));
    if settled.text("printer_structure") == Some("i3") {
        text.push_str(&tr(
            " But machines with I3 structure will not generate timelapse videos.",
        ));
    }
    text.push_str("\n\n");
    text.push_str(&tr(
        "Change these settings automatically?\nYes - Change these settings and enable spiral mode automatically\nNo  - Give up using spiral mode this time",
    ));
    text
}
