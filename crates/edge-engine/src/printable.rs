//! Overhangs made printable (`make_overhang_printable`, with its angle and
//! hole size): what the profile asks, and its place in the pipeline. The
//! work itself is `edge_slice::make_overhangs_printable`.

use edge_clip::ClipError;
use edge_config::{ConfigError, Settings};
use edge_geom::{Coord, ExPolygon};
use edge_slice::{Body, LayerRegions};

use crate::layer_heights::LayerHeights;

/// What the profile says of making overhangs printable.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Printable {
    /// The steepest overhang left, in degrees from upright, and the area
    /// in mm² under which a hole in the underside is kept; `None` with
    /// the option off.
    asked: Option<(f64, f64)>,
    /// The plate's first layer as grown, where it was added to: what
    /// stands on the bed is then no longer what the model has there.
    on_bed: Option<Vec<ExPolygon>>,
}

impl Printable {
    pub(crate) fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        let on = settings.flag("make_overhang_printable")? == Some(true);
        Ok(Self {
            asked: on
                .then(|| -> Result<_, ConfigError> {
                    Ok((
                        settings
                            .float("make_overhang_printable_angle")?
                            .unwrap_or(55.0),
                        settings
                            .float("make_overhang_printable_hole_size")?
                            .unwrap_or(0.0),
                    ))
                })
                .transpose()?,
            on_bed: None,
        })
    }

    /// Grows the layers as cut under their overhangs, before anything
    /// else is done to them, as the reference does
    /// (PrintObjectSlice.cpp `PrintObject::slice_volumes`). `painted`
    /// says whether colour is painted on anything: a note is returned
    /// for what is not done then.
    ///
    /// The reference lets each part and modifier turn the option on or
    /// off for itself; here it is the plate's, as every setting is.
    pub(crate) fn apply(
        &mut self,
        layers: &mut [LayerRegions],
        heights: &LayerHeights,
        painted: bool,
    ) -> Result<Vec<String>, ClipError> {
        let Some((angle, hole)) = self.asked else {
            return Ok(Vec::new());
        };
        let first = layers.first().map(|layer| layer.regions.clone());
        edge_slice::make_overhangs_printable(layers, &heights.all(), angle, hole)?;
        self.on_bed = layers
            .first()
            .map(|layer| layer.regions.clone())
            .filter(|grown| Some(grown) != first.as_ref());
        // Paint is shared out from each painted object's outline as
        // modelled; the reference shares out the layers as grown.
        Ok(if painted {
            vec![
                "make_overhang_printable \"1\": what is added under an overhang is printed in the \
                 filament its part is made of, not in a colour painted on the part"
                    .to_owned(),
            ]
        } else {
            Vec::new()
        })
    }

    /// What one object by itself has on the bed, for its brim: its first
    /// layer as cut, unless the first layer was added to. Then it is the
    /// plate's first layer as grown where the object is `alone` on the
    /// plate with nothing carved out of it, and otherwise the whole
    /// object is cut and grown as the plate was.
    pub(crate) fn first_layer(
        &self,
        body: &[Body<'_>],
        alone: bool,
        planes: &[f64],
        closing_radius: Coord,
        heights: &LayerHeights,
    ) -> Result<Vec<ExPolygon>, ClipError> {
        if let (true, Some(grown)) = (alone, &self.on_bed) {
            return Ok(grown.clone());
        }
        let grown = self.asked.filter(|_| self.on_bed.is_some());
        let cut = if grown.is_some() {
            planes
        } else {
            &planes[..1]
        };
        let mut layers = edge_slice::slice_regions(body, cut, closing_radius)?;
        if let Some((angle, hole)) = grown {
            edge_slice::make_overhangs_printable(&mut layers, &heights.all(), angle, hole)?;
        }
        Ok(layers
            .into_iter()
            .next()
            .map_or_else(Vec::new, |layer| layer.regions))
    }
}

#[cfg(test)]
mod tests {
    use edge_mesh::{Mesh, primitives::cuboid};

    use super::*;

    fn read(json: &str) -> Printable {
        Printable::from_settings(&Settings::from_json(json).unwrap()).unwrap()
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    /// A 10 mm stem `stem` mm tall under a 30 mm cap 2 mm thick.
    fn mushroom(stem: f32) -> Mesh {
        let mut soup = cuboid([10.0, 10.0, 0.0], [20.0, 20.0, stem]);
        soup.extend(cuboid([0.0, 0.0, stem], [30.0, 30.0, stem + 2.0]));
        Mesh::from_triangle_soup(soup)
    }

    fn cut(mesh: &Mesh, count: usize) -> (Vec<LayerRegions>, Vec<f64>, LayerHeights) {
        let planes = edge_slice::layer_planes(0.0, count as f64 * 0.2, 0.2, 0.2);
        assert_eq!(planes.len(), count);
        let body = Body {
            solid: mesh,
            negative: None,
            filament: 1,
        };
        let layers = edge_slice::slice_regions(&[body], &planes, 0).unwrap();
        (layers, planes, LayerHeights::uniform(0.2, 0.2, count))
    }

    #[test]
    fn the_option_is_off_unless_asked_for() {
        assert_eq!(read("{}"), Printable::default());
        assert_eq!(
            read(r#"{"make_overhang_printable": "0", "make_overhang_printable_angle": "30"}"#),
            Printable::default()
        );
        assert_eq!(
            read(r#"{"make_overhang_printable": "1"}"#).asked,
            Some((55.0, 0.0))
        );
        assert_eq!(
            read(
                r#"{"make_overhang_printable": "1", "make_overhang_printable_angle": "40",
                    "make_overhang_printable_hole_size": "25"}"#
            )
            .asked,
            Some((40.0, 25.0))
        );
    }

    #[test]
    fn layers_are_left_alone_with_the_option_off() {
        let mesh = mushroom(10.0);
        let (mut layers, _, heights) = cut(&mesh, 60);
        let as_cut: Vec<Vec<ExPolygon>> = layers.iter().map(|l| l.regions.clone()).collect();
        let mut off = read("{}");
        assert!(off.apply(&mut layers, &heights, true).unwrap().is_empty());
        assert!(layers.iter().zip(&as_cut).all(|(l, cut)| l.regions == *cut));
    }

    #[test]
    fn a_slope_that_stops_short_of_the_bed_leaves_the_first_layer_as_modelled() {
        // 10 mm out at 55 degrees is 35 layers down; the stem has 50.
        let mesh = mushroom(10.0);
        let (mut layers, planes, heights) = cut(&mesh, 60);
        let mut on = read(r#"{"make_overhang_printable": "1"}"#);
        let notes = on.apply(&mut layers, &heights, false).unwrap();
        assert!(notes.is_empty());
        assert!(area(&layers[49].regions) > 800.0);
        assert!((area(&layers[0].regions) - 100.0).abs() < 1e-6);
        assert!(on.on_bed.is_none());
        let body = [Body {
            solid: &mesh,
            negative: None,
            filament: 1,
        }];
        let first = on.first_layer(&body, true, &planes, 0, &heights).unwrap();
        assert_eq!(first, layers[0].regions);
    }

    #[test]
    fn a_slope_that_reaches_the_bed_is_what_the_brim_goes_round() {
        // The cap 2 mm up: the slope is still 7 mm out at the bed.
        let mesh = mushroom(2.0);
        let (mut layers, planes, heights) = cut(&mesh, 20);
        let mut on = read(r#"{"make_overhang_printable": "1"}"#);
        on.apply(&mut layers, &heights, false).unwrap();
        assert_eq!(on.on_bed.as_ref(), Some(&layers[0].regions));
        let side = 30.0 - 2.0 * 10.0 * 55f64.to_radians().tan() * 0.2;
        assert!((area(&layers[0].regions) - side * side).abs() < 1e-3);
        let body = [Body {
            solid: &mesh,
            negative: None,
            filament: 1,
        }];
        // One object of several is cut and grown by itself, and comes
        // to the same; alone on the plate, the plate's layer is its own.
        for alone in [false, true] {
            let first = on.first_layer(&body, alone, &planes, 0, &heights).unwrap();
            assert_eq!(first, layers[0].regions);
        }
        // Off, the object's own first layer is all that is cut.
        let first = read("{}")
            .first_layer(&body, true, &planes, 0, &heights)
            .unwrap();
        assert!((area(&first) - 100.0).abs() < 1e-6);
    }

    #[test]
    fn paint_on_the_plate_is_noted() {
        let mesh = mushroom(10.0);
        let (mut layers, _, heights) = cut(&mesh, 60);
        let mut on = read(r#"{"make_overhang_printable": "1"}"#);
        let notes = on.apply(&mut layers, &heights, true).unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].starts_with("make_overhang_printable"));
    }
}
