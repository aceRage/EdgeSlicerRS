//! The fuzzy skin settings of one region, and which walls they roughen.

use edge_geom::Coord;

use crate::Error;
use crate::noise::MAX_OCTAVES;

/// Which walls get a fuzzy skin: the reference's `FuzzySkinType`
/// (PrintConfig.hpp), the `fuzzy_skin` setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FuzzyType {
    /// No wall, but painted fuzzy skin still applies. The reference's
    /// default.
    #[default]
    None,
    /// The outer wall of contours.
    External,
    /// The outer wall of holes.
    Hole,
    /// The outer wall of contours and of holes.
    All,
    /// Every wall.
    AllWalls,
    /// Nothing at all, painted or not (`Disabled_fuzzy`).
    Disabled,
}

impl FuzzyType {
    /// The value as the reference's settings spell it
    /// (`s_keys_map_FuzzySkinType`, PrintConfig.cpp).
    pub fn from_text(text: &str) -> Option<Self> {
        Some(match text {
            "none" => Self::None,
            "external" => Self::External,
            "hole" => Self::Hole,
            "all" => Self::All,
            "allwalls" => Self::AllWalls,
            "disabled_fuzzy" => Self::Disabled,
            _ => return None,
        })
    }

    /// The setting text of this value.
    pub fn text(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::External => "external",
            Self::Hole => "hole",
            Self::All => "all",
            Self::AllWalls => "allwalls",
            Self::Disabled => "disabled_fuzzy",
        }
    }
}

/// Where the roughness comes from: the reference's `NoiseType`, the
/// `fuzzy_skin_noise_type` setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NoiseType {
    /// A fresh random number for every point.
    #[default]
    Classic,
    Perlin,
    Billow,
    RidgedMulti,
    Voronoi,
    /// A sine wave along the wall, the same from layer to layer or
    /// stepping round by a set share of a wave.
    Ripple,
}

impl NoiseType {
    /// The value as the reference's settings spell it
    /// (`s_keys_map_NoiseType`).
    pub fn from_text(text: &str) -> Option<Self> {
        Some(match text {
            "classic" => Self::Classic,
            "perlin" => Self::Perlin,
            "billow" => Self::Billow,
            "ridgedmulti" => Self::RidgedMulti,
            "voronoi" => Self::Voronoi,
            "ripple" => Self::Ripple,
            _ => return None,
        })
    }

    /// The setting text of this value.
    pub fn text(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Perlin => "perlin",
            Self::Billow => "billow",
            Self::RidgedMulti => "ridgedmulti",
            Self::Voronoi => "voronoi",
            Self::Ripple => "ripple",
        }
    }
}

/// What the noise changes on a wall of changing width: the reference's
/// `FuzzySkinMode`, the `fuzzy_skin_mode` setting. A wall of fixed width
/// (a plain polygon) can only be displaced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FuzzyMode {
    /// Points move sideways.
    #[default]
    Displacement,
    /// Points stay and the line gets wider and narrower.
    Extrusion,
    /// The width changes and the line moves by half the change, so one
    /// side of it stays where it was.
    Combined,
}

impl FuzzyMode {
    /// The value as the reference's settings spell it
    /// (`s_keys_map_FuzzySkinMode`).
    pub fn from_text(text: &str) -> Option<Self> {
        Some(match text {
            "displacement" => Self::Displacement,
            "extrusion" => Self::Extrusion,
            "combined" => Self::Combined,
            _ => return None,
        })
    }

    /// The setting text of this value.
    pub fn text(self) -> &'static str {
        match self {
            Self::Displacement => "displacement",
            Self::Extrusion => "extrusion",
            Self::Combined => "combined",
        }
    }
}

/// The fuzzy skin settings of one region on one layer: the reference's
/// `FuzzySkinConfig` (PerimeterGenerator.hpp), filled as
/// `group_region_by_fuzzify` (FuzzySkin.cpp) fills it.
///
/// Different on purpose: the reference's `operator==` leaves `layer_id`
/// out, and this compares it. Every region of a layer carries the same
/// one, so grouping regions by equal settings comes out the same.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FuzzyConfig {
    /// `fuzzy_skin`.
    pub kind: FuzzyType,
    /// `fuzzy_skin_thickness`, scaled: how far a point may move to either
    /// side.
    pub thickness: Coord,
    /// `fuzzy_skin_point_distance`, scaled: the average distance between
    /// the points put on a wall. Must be more than zero.
    pub point_distance: Coord,
    /// `fuzzy_skin_first_layer`.
    pub first_layer: bool,
    /// `fuzzy_skin_noise_type`.
    pub noise: NoiseType,
    /// `fuzzy_skin_scale`: the size of the noise's features in mm.
    pub scale: f64,
    /// `fuzzy_skin_octaves`.
    pub octaves: i32,
    /// `fuzzy_skin_persistence`.
    pub persistence: f64,
    /// `fuzzy_skin_mode`.
    pub mode: FuzzyMode,
    /// `fuzzy_skin_ripples_per_layer`: waves around one wall.
    pub ripples_per_layer: i32,
    /// `fuzzy_skin_ripple_offset`, in percent of a wave, as the setting
    /// holds it (50 for 50%).
    pub ripple_offset: f64,
    /// `fuzzy_skin_layers_between_ripple_offset`.
    pub layers_between_ripple_offset: i32,
    /// The layer this is for, counted from 0; the ripple's shift depends
    /// on it.
    pub layer_id: i32,
    /// `fuzzy_skin_skip_overhangs`: leave walls smooth where the layer
    /// below does not carry them.
    pub skip_overhangs: bool,
}

impl Default for FuzzyConfig {
    /// The reference's default of every `fuzzy_skin*` setting
    /// (PrintConfig.cpp), on layer 0.
    fn default() -> Self {
        Self {
            kind: FuzzyType::None,
            thickness: 200_000,
            point_distance: 300_000,
            first_layer: false,
            noise: NoiseType::Classic,
            scale: 1.0,
            octaves: 4,
            persistence: 0.5,
            mode: FuzzyMode::Displacement,
            ripples_per_layer: 15,
            ripple_offset: 50.0,
            layers_between_ripple_offset: 1,
            layer_id: 0,
            skip_overhangs: false,
        }
    }
}

impl FuzzyConfig {
    /// Whether these settings can be carried out as the reference would.
    ///
    /// The reference trusts its settings' limits. Outside them it hangs (a
    /// point distance of zero never gets along the wall) or throws
    /// (libnoise refuses an octave count it has no room for). The fuzz
    /// functions here leave a wall alone, or clamp the count, rather than
    /// do either; a caller that wants to tell the user checks first.
    pub fn validate(&self) -> Result<(), Error> {
        if matches!(self.kind, FuzzyType::None | FuzzyType::Disabled) {
            // Painted fuzzy skin under `None` arrives as a region of its
            // own with another kind; these settings fuzz nothing.
            return Ok(());
        }
        if self.point_distance <= 0 {
            return Err(Error::PointDistance(self.point_distance));
        }
        let octaves_from = match self.noise {
            NoiseType::Perlin | NoiseType::Billow => Some(1),
            // `RidgedMulti::SetOctaveCount` checks the top only.
            NoiseType::RidgedMulti => Some(i32::MIN),
            NoiseType::Classic | NoiseType::Voronoi | NoiseType::Ripple => None,
        };
        if let Some(from) = octaves_from
            && (self.octaves < from || self.octaves > MAX_OCTAVES)
        {
            return Err(Error::Octaves(self.octaves));
        }
        let coherent = !matches!(self.noise, NoiseType::Classic | NoiseType::Ripple);
        if coherent && !(self.scale.is_finite() && self.scale != 0.0) {
            return Err(Error::Scale(self.scale));
        }
        Ok(())
    }
}

/// Whether a wall gets a fuzzy skin under these settings: the reference's
/// `should_fuzzify` (FuzzySkin.cpp). `loop_idx` counts walls from the
/// outside in, from 0; `is_contour` is false for the walls of a hole.
pub fn should_fuzzify(
    config: &FuzzyConfig,
    layer_id: i32,
    loop_idx: usize,
    is_contour: bool,
) -> bool {
    let kind = config.kind;

    if kind == FuzzyType::None || kind == FuzzyType::Disabled {
        return false;
    }
    if !config.first_layer && layer_id <= 0 {
        // Do not fuzzy first layer unless told to.
        return false;
    }

    let fuzzify_contours =
        (loop_idx == 0 && kind != FuzzyType::Hole) || kind == FuzzyType::AllWalls;
    let fuzzify_holes =
        (kind == FuzzyType::Hole || kind == FuzzyType::All || kind == FuzzyType::AllWalls)
            && (loop_idx == 0 || kind == FuzzyType::AllWalls);

    if is_contour {
        fuzzify_contours
    } else {
        fuzzify_holes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_texts_round_trip() {
        let kinds = [
            FuzzyType::None,
            FuzzyType::External,
            FuzzyType::Hole,
            FuzzyType::All,
            FuzzyType::AllWalls,
            FuzzyType::Disabled,
        ];
        for kind in kinds {
            assert_eq!(FuzzyType::from_text(kind.text()), Some(kind));
        }
        let noises = [
            NoiseType::Classic,
            NoiseType::Perlin,
            NoiseType::Billow,
            NoiseType::RidgedMulti,
            NoiseType::Voronoi,
            NoiseType::Ripple,
        ];
        for noise in noises {
            assert_eq!(NoiseType::from_text(noise.text()), Some(noise));
        }
        let modes = [
            FuzzyMode::Displacement,
            FuzzyMode::Extrusion,
            FuzzyMode::Combined,
        ];
        for mode in modes {
            assert_eq!(FuzzyMode::from_text(mode.text()), Some(mode));
        }

        // The exact spellings of PrintConfig.cpp, and nothing near them.
        assert_eq!(
            FuzzyType::from_text("disabled_fuzzy"),
            Some(FuzzyType::Disabled)
        );
        assert_eq!(FuzzyType::from_text("allwalls"), Some(FuzzyType::AllWalls));
        assert_eq!(
            NoiseType::from_text("ridgedmulti"),
            Some(NoiseType::RidgedMulti)
        );
        for wrong in ["", "None", "disabled", "all_walls", "ridged_multi", " all"] {
            assert_eq!(FuzzyType::from_text(wrong), None, "{wrong:?}");
            assert_eq!(NoiseType::from_text(wrong), None, "{wrong:?}");
            assert_eq!(FuzzyMode::from_text(wrong), None, "{wrong:?}");
        }
    }

    #[test]
    fn defaults_are_the_references() {
        // crates/edge-config/src/reference_defaults.txt has the same.
        let config = FuzzyConfig::default();
        assert_eq!(config.kind.text(), "none");
        assert_eq!(config.thickness, edge_geom::scaled(0.2));
        assert_eq!(config.point_distance, edge_geom::scaled(0.3));
        assert!(!config.first_layer);
        assert_eq!(config.noise.text(), "classic");
        assert_eq!(config.scale, 1.0);
        assert_eq!(config.octaves, 4);
        assert_eq!(config.persistence, 0.5);
        assert_eq!(config.mode.text(), "displacement");
        assert_eq!(config.ripples_per_layer, 15);
        assert_eq!(config.ripple_offset, 50.0);
        assert_eq!(config.layers_between_ripple_offset, 1);
        assert!(!config.skip_overhangs);
        assert_eq!(FuzzyType::default(), FuzzyType::None);
        assert_eq!(NoiseType::default(), NoiseType::Classic);
        assert_eq!(FuzzyMode::default(), FuzzyMode::Displacement);
    }

    #[test]
    fn should_fuzzify_table() {
        // (kind, [contour loop 0, contour loop 1, hole loop 0, hole loop 1])
        let table = [
            (FuzzyType::None, [false, false, false, false]),
            (FuzzyType::External, [true, false, false, false]),
            (FuzzyType::Hole, [false, false, true, false]),
            (FuzzyType::All, [true, false, true, false]),
            (FuzzyType::AllWalls, [true, true, true, true]),
            (FuzzyType::Disabled, [false, false, false, false]),
        ];
        for (kind, expected) in table {
            for first_layer in [false, true] {
                let config = FuzzyConfig {
                    kind,
                    first_layer,
                    ..FuzzyConfig::default()
                };
                let on = |layer_id: i32| {
                    [
                        should_fuzzify(&config, layer_id, 0, true),
                        should_fuzzify(&config, layer_id, 1, true),
                        should_fuzzify(&config, layer_id, 0, false),
                        should_fuzzify(&config, layer_id, 1, false),
                    ]
                };
                // Above the first layer the switch makes no difference.
                assert_eq!(on(1), expected, "{kind:?} first_layer {first_layer}");
                assert_eq!(on(37), expected, "{kind:?} first_layer {first_layer}");
                // The first layer (and anything below it) only when asked.
                let on_first = if first_layer { expected } else { [false; 4] };
                assert_eq!(on(0), on_first, "{kind:?} first_layer {first_layer}");
                assert_eq!(on(-1), on_first, "{kind:?} first_layer {first_layer}");
            }
        }
        // Deeper walls go the way of loop 1.
        let all_walls = FuzzyConfig {
            kind: FuzzyType::AllWalls,
            ..FuzzyConfig::default()
        };
        assert!(should_fuzzify(&all_walls, 3, 5, true));
        let external = FuzzyConfig {
            kind: FuzzyType::External,
            ..FuzzyConfig::default()
        };
        assert!(!should_fuzzify(&external, 3, 5, true));
    }

    #[test]
    fn validate_refuses_what_the_reference_cannot_run() {
        let fuzzy = FuzzyConfig {
            kind: FuzzyType::External,
            ..FuzzyConfig::default()
        };
        assert!(fuzzy.validate().is_ok());
        assert!(matches!(
            FuzzyConfig {
                point_distance: 0,
                ..fuzzy
            }
            .validate(),
            Err(Error::PointDistance(0))
        ));
        // Settings that fuzz nothing are not looked at further.
        assert!(
            FuzzyConfig {
                point_distance: 0,
                ..FuzzyConfig::default()
            }
            .validate()
            .is_ok()
        );
        let perlin = FuzzyConfig {
            noise: NoiseType::Perlin,
            ..fuzzy
        };
        assert!(perlin.validate().is_ok());
        assert!(matches!(
            FuzzyConfig {
                octaves: 0,
                ..perlin
            }
            .validate(),
            Err(Error::Octaves(0))
        ));
        assert!(matches!(
            FuzzyConfig {
                octaves: 31,
                ..perlin
            }
            .validate(),
            Err(Error::Octaves(31))
        ));
        assert!(matches!(
            FuzzyConfig {
                scale: 0.0,
                ..perlin
            }
            .validate(),
            Err(Error::Scale(_))
        ));
        let ridged = FuzzyConfig {
            noise: NoiseType::RidgedMulti,
            octaves: 0,
            ..fuzzy
        };
        assert!(ridged.validate().is_ok());
        // The classic noise has no octaves or scale to get wrong.
        assert!(
            FuzzyConfig {
                octaves: 99,
                scale: 0.0,
                ..fuzzy
            }
            .validate()
            .is_ok()
        );
    }
}
