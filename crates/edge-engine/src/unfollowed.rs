//! Settings no stage reads yet: each is said, with what is done instead,
//! so that a profile which asks for one is not followed in silence. An
//! entry goes when the stage that should follow it does.

use edge_config::Settings;

/// A setting, the value that turns it off, and what is done whatever it
/// is set to.
const UNREAD: [(&str, &str, &str); 5] = [
    (
        "elefant_foot_layers_density",
        "100%",
        "solid fill on the first layers is laid at its full density",
    ),
    (
        "zaa_enabled",
        "0",
        "top surfaces are laid flat, not contoured in height",
    ),
    (
        "reduce_crossing_wall",
        "0",
        "travel moves go straight, across walls where those are in the way",
    ),
    (
        "small_area_infill_flow_compensation",
        "0",
        "small solid areas are laid at the usual flow",
    ),
    (
        "unsupported_wall_last",
        "0",
        "a wall loop with nothing under it keeps its place among the walls",
    ),
];

/// What `settings` ask for that nothing follows. `parts` is how many
/// bodies are on the plate: hiding seams where parts meet, on unless
/// turned off, has nothing to act on with one.
pub(crate) fn notes(settings: &Settings, parts: usize) -> Vec<String> {
    let mut notes: Vec<String> = UNREAD
        .iter()
        .filter_map(|(key, off, instead)| {
            let value = settings.text(key)?;
            (value != *off && !(*off == "0" && value == "false"))
                .then(|| format!("{key} \"{value}\": {instead}"))
        })
        .collect();
    if parts > 1
        && !matches!(
            settings.text("seam_prefer_part_joints"),
            Some("0" | "false")
        )
    {
        notes.push(
            "seam_prefer_part_joints: seams are placed part by part, not moved to where parts meet"
                .to_owned(),
        );
    }
    // Vase mode is not here yet. It is said apart from the list, and
    // plainly: the part comes out a different print, with its walls,
    // fill and top as an ordinary one has them.
    if matches!(settings.text("spiral_mode"), Some("1" | "true")) {
        notes.push(
            "spiral_mode \"1\": vase mode is not followed; the part is printed as an ordinary \
             print, with every wall, its fill and its top layers"
                .to_owned(),
        );
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_nothing_reads_are_said_when_they_are_on() {
        // Left alone or turned off, nothing to say.
        assert!(notes(&Settings::default(), 1).is_empty());
        let off = Settings::from_json(
            r#"{"zaa_enabled": "0", "reduce_crossing_wall": "0",
                "elefant_foot_layers_density": "100%", "enable_extra_bridge_layer": "disabled",
                "dont_filter_internal_bridges": "disabled", "seam_prefer_part_joints": "1"}"#,
        )
        .unwrap();
        assert!(notes(&off, 1).is_empty());

        let on = Settings::from_json(
            r#"{"wipe_inward": "1", "zaa_enabled": "1", "reduce_crossing_wall": "1",
                "elefant_foot_layers_density": "70%", "small_area_infill_flow_compensation": "1",
                "dont_filter_internal_bridges": "nofilter"}"#,
        )
        .unwrap();
        // How small bridges over infill are sorted out is followed, and
        // nothing is said of it here; nor of a wipe in over the inner
        // walls.
        let said = notes(&on, 1);
        assert_eq!(said.len(), 4, "{said:?}");
        assert!(said.contains(
            &"zaa_enabled \"1\": top surfaces are laid flat, not contoured in height".to_owned()
        ));
        // Ironing is followed, and nothing is said of it here; nor of a
        // second layer of bridge.
        let ironed = Settings::from_json(
            r#"{"ironing_type": "top", "enable_extra_bridge_layer": "apply_to_all"}"#,
        )
        .unwrap();
        assert!(notes(&ironed, 1).is_empty());
        // Nor of a precise height or printable overhangs, which have
        // their own notes for what they leave undone.
        let shaped =
            Settings::from_json(r#"{"precise_z_height": "1", "make_overhang_printable": "1"}"#)
                .unwrap();
        assert!(notes(&shaped, 1).is_empty());

        // Hiding seams where parts meet is on unless turned off, and
        // only means anything with more than one part.
        assert_eq!(notes(&Settings::default(), 2).len(), 1);
        let hidden_off = Settings::from_json(r#"{"seam_prefer_part_joints": "0"}"#).unwrap();
        assert!(notes(&hidden_off, 2).is_empty());

        // Vase mode is said when it is on, and only then.
        let vase = Settings::from_json(r#"{"spiral_mode": "1"}"#).unwrap();
        let said = notes(&vase, 1);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("spiral_mode \"1\": vase mode is not followed"));
        let plain = Settings::from_json(r#"{"spiral_mode": "0"}"#).unwrap();
        assert!(notes(&plain, 1).is_empty());
    }
}
