//! What the engine does with a set of settings, answered before anything
//! is sliced with them: which of them it reads, what it says it does not
//! do of what they ask, and whether it would refuse them.
//!
//! Nothing here is a list of settings. The answer is found by doing the
//! work: a small block is sliced with the settings and written out as
//! G-code, as any model would be, while every setting asked for by name
//! is noted ([`edge_config::Reads`]). A setting no stage asked for
//! changes nothing in a print, whatever it is set to; one that was asked
//! for is followed, as far as the notes the stages themselves give say.

use std::collections::BTreeSet;

use edge_3mf::Body;
use edge_config::{Reads, Settings};
use edge_mesh::{Mesh, primitives};

use crate::{Chosen, Failure, Loaded, Overrides, Unwatched};

/// The engine's own account of a set of settings.
#[derive(Clone, Debug, Default)]
pub struct Followed {
    /// Every setting a slice with these asked for by name, set or not.
    pub read: BTreeSet<String>,
    /// What the settings ask for that is not done, each with what is done
    /// instead: the notes a slice gives ([`crate::Sliced::notes`]). Each
    /// starts with the name of the setting it is about.
    pub notes: Vec<String>,
    /// Why a slice with these settings stops, when it does: a value the
    /// engine refuses. What was read up to there is still in `read`.
    pub refused: Option<String>,
}

impl Followed {
    /// The notes about one setting: those that start with its name.
    pub fn notes_on<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> {
        self.notes.iter().map(String::as_str).filter(move |note| {
            note.strip_prefix(key)
                .is_some_and(|rest| rest.starts_with([' ', ':']))
        })
    }
}

/// The block that is sliced to find out: 12 mm square and 2.4 mm high, in
/// one filament. Large enough for walls, fill, a top and a bottom, small
/// enough to take a few milliseconds.
fn block(project: Option<&Settings>) -> Loaded {
    let (min, max) = ([0.0; 3], [12.0, 12.0, 2.4]);
    Loaded {
        bodies: vec![Body {
            solid: Mesh::from_triangle_soup(primitives::cuboid(min, max)),
            negative: None,
            filament: 1,
        }],
        painted: Vec::new(),
        settings: project.cloned(),
        unread_paint: Vec::new(),
        objects: Vec::new(),
        plate: 0,
        first_layer_sequence: Vec::new(),
        bounds: (min, max),
        placed: false,
    }
}

/// What the engine does with the settings a model would be sliced with:
/// the presets chosen, what is set over them, and a project's own
/// settings under both, exactly as [`crate::prepare`] takes them.
pub fn followed(chosen: &Chosen, overrides: &Overrides, project: Option<&Settings>) -> Followed {
    let reads = Reads::default();
    let mut notes = Vec::new();
    let sliced = (|| -> Result<(), Failure> {
        let mut job = crate::prepare_noted(block(project), chosen, overrides, &reads)?;
        // The block is put in the middle of the bed, as a bare model is:
        // a project's settings say nothing of where this one would be.
        job.is_project = false;
        let sliced = crate::slice(job, &Unwatched)?;
        notes = sliced.gaps();
        sliced.gcode("")?;
        Ok(())
    })();
    Followed {
        read: reads.keys(),
        notes,
        refused: sliced.err().map(|error| error.to_string()),
    }
}

/// The settings a model would be sliced with, complete: a project's own
/// under the presets chosen, under what is set over them, with the
/// reference's defaults for whatever a printer's profile leaves unsaid.
/// What [`crate::prepare`] settles, without a model.
pub fn settled(
    chosen: &Chosen,
    overrides: &Overrides,
    project: Option<&Settings>,
) -> Result<Settings, Failure> {
    crate::prepare(block(project), chosen, overrides).map(|job| job.settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A printer's settings in a few lines: what makes `prepare` fill in
    /// the reference's defaults, and a bed to stand the block on.
    fn printer() -> Settings {
        Settings::from_json(
            r#"{"machine_start_gcode": "G28", "printable_area": ["0x0", "200x0", "200x200", "0x200"],
                "nozzle_diameter": ["0.4"], "printable_height": "200"}"#,
        )
        .unwrap()
    }

    fn asked(process: &str) -> Followed {
        let chosen = Chosen {
            machine: Some(("printer".into(), printer())),
            process: Some(("process".into(), Settings::from_json(process).unwrap())),
            filaments: Vec::new(),
        };
        followed(&chosen, &Overrides::default(), None)
    }

    #[test]
    fn what_a_slice_reads_is_told_from_what_it_never_asks_for() {
        let plain = asked("{}");
        assert_eq!(plain.refused, None);
        // Read by the stages that follow them, wherever they are.
        for key in [
            "layer_height",
            "wall_loops",
            "wall_generator",
            "seam_slope_type",
            "enable_arc_fitting",
            "top_shell_layers",
            "sparse_infill_density",
            "enable_support",
            "brim_type",
        ] {
            assert!(plain.read.contains(key), "{key} was not read");
        }
        // Known to the reference, written into the file's list of
        // settings with every other, and asked for by nothing.
        for key in ["hollow_interior", "interlocking_beam", "timelapse_type"] {
            assert!(!plain.read.contains(key), "{key} was read");
        }
        // Nothing the block's settings ask for is left undone.
        assert!(
            plain.notes.iter().all(|note| !note.starts_with("zaa")),
            "{:?}",
            plain.notes
        );
    }

    #[test]
    fn a_setting_read_and_not_followed_has_its_note() {
        let contoured = asked(r#"{"zaa_enabled": "1", "spiral_mode": "1"}"#);
        assert_eq!(contoured.refused, None);
        let said: Vec<&str> = contoured.notes_on("zaa_enabled").collect();
        assert_eq!(
            said,
            ["zaa_enabled \"1\": top surfaces are laid flat, not contoured in height"]
        );
        assert_eq!(contoured.notes_on("spiral_mode").count(), 1);
        // A note about another setting whose name starts the same way is
        // not this one's.
        assert_eq!(contoured.notes_on("zaa").count(), 0);
        assert_eq!(contoured.notes_on("layer_height").count(), 0);
    }

    #[test]
    fn a_value_the_engine_refuses_is_said_with_what_was_read() {
        let thin = asked(r#"{"elefant_foot_layers_density": "20%"}"#);
        let refused = thin.refused.expect("20% is outside 50% to 100%");
        assert!(refused.contains("elefant_foot_layers_density"), "{refused}");
        assert!(thin.read.contains("layer_height"));
    }

    #[test]
    fn what_is_set_by_hand_comes_over_the_preset_and_is_read() {
        let chosen = Chosen {
            machine: Some(("printer".into(), printer())),
            process: Some((
                "process".into(),
                Settings::from_json(r#"{"layer_height": "0.2", "wall_loops": "2"}"#).unwrap(),
            )),
            filaments: Vec::new(),
        };
        let overrides = Overrides {
            settings: Settings::from_json(r#"{"layer_height": "0.3", "wall_loops": "4"}"#).unwrap(),
            ..Overrides::default()
        };
        let job = crate::prepare(block(None), &chosen, &overrides).unwrap();
        assert_eq!(job.layer_height, (0.3, "set by hand"));
        assert_eq!(job.settings.text("layer_height"), Some("0.3"));
        assert_eq!(job.settings.text("wall_loops"), Some("4"));
        // A height given outright still comes first.
        let outright = Overrides {
            layer_height: Some(0.1),
            ..overrides.clone()
        };
        let job = crate::prepare(block(None), &chosen, &outright).unwrap();
        assert_eq!(job.layer_height, (0.1, "command line"));
        let answer = followed(&chosen, &overrides, None);
        assert_eq!(answer.refused, None);
        assert!(answer.read.contains("wall_loops"));
    }
}
