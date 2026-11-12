//! Z contouring (`zaa_enabled`) as the pipeline does it: where each layer
//! is sliced, and every layer's walls and lines contoured once the fill
//! and ironing are made (Print.cpp: "after infill and ironing, before
//! path simplification"). `edge_contour` has the rest.

use edge_3mf::Body;
use edge_contour::{At, LayerProfiles, Surface, ZaaSettings};
use edge_extrusion::ExtrusionPath;
use edge_perimeter::Perimeters;
use rayon::prelude::*;

use crate::{Failure, layer_heights::LayerHeights};

/// Moves every layer's slicing plane but the first's to `zaa_min_z`
/// above its bottom. `floor` is the height the print stands on.
pub(crate) fn plane(
    zaa: &ZaaSettings,
    planes: &mut [f64],
    heights: &LayerHeights,
    floor: f64,
) -> Result<(), Failure> {
    for (index, plane) in planes.iter_mut().enumerate().skip(1) {
        let height = heights.height_of(index);
        *plane = zaa.slice_at(index, floor + heights.top_of(index) - height, height)?;
    }
    Ok(())
}

/// Contours the walls and lines of every layer but the first ("to avoid
/// a build plate collision", PrintObject.cpp `contour_z`), each layer on
/// a thread of its own, and gives what each is laid by: by filament, as
/// `walls_by` and `fills_by` are, then by layer.
pub(crate) fn apply(
    zaa: &ZaaSettings,
    bodies: &[Body],
    planes: &[f64],
    heights: &LayerHeights,
    floor: f64,
    walls_by: &mut [Vec<Perimeters>],
    fills_by: &mut [Vec<Vec<ExtrusionPath>>],
) -> Vec<Vec<LayerProfiles>> {
    let surface = Surface::new(bodies.iter().map(|body| &body.solid));
    walls_by
        .iter_mut()
        .zip(fills_by.iter_mut())
        .map(|(walls, fills)| {
            walls
                .par_iter_mut()
                .zip(fills.par_iter_mut())
                .enumerate()
                .map(|(index, (walls, fills))| {
                    if index == 0 {
                        return LayerProfiles::default();
                    }
                    let at = At {
                        print_z: floor + heights.top_of(index),
                        slice_z: planes[index],
                        height: heights.height_of(index),
                    };
                    edge_contour::contour_layer(&mut walls.loops, fills, &at, zaa, &surface)
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use edge_config::Settings;
    use edge_mesh::{Mesh, primitives};

    use crate::{Chosen, Loaded, Overrides, Unwatched};

    /// A block 12 mm square whose top rises from 2 mm at its left to
    /// 2.84 at its right, 4 degrees, stood in the middle of the bed.
    fn wedge(process: &str) -> String {
        let (min, max) = ([0.0, 0.0, 0.0], [12.0, 12.0, 2.0]);
        let rise = 12.0 * 4f32.to_radians().tan();
        let soup = primitives::cuboid(min, max).into_iter().map(|triangle| {
            triangle.map(|[x, y, z]| [x, y, if z == 2.0 && x == 12.0 { z + rise } else { z }])
        });
        let loaded = Loaded {
            bodies: vec![edge_3mf::Body {
                solid: Mesh::from_triangle_soup(soup),
                negative: None,
                filament: 1,
            }],
            painted: Vec::new(),
            settings: None,
            unread_paint: Vec::new(),
            precise_seams: false,
            unfollowed_parts: Vec::new(),
            objects: Vec::new(),
            negatives: Vec::new(),
            parts: Vec::new(),
            plate: 0,
            first_layer_sequence: Vec::new(),
            plate_settings: Default::default(),
            bounds: (min, [12.0, 12.0, 2.0 + rise]),
            placed: false,
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
        let sliced = crate::slice(job, &Unwatched).unwrap();
        String::from_utf8(sliced.gcode("").unwrap().text).unwrap()
    }

    /// Every extruding move as (layer top, layer height, height said).
    fn heights(text: &str) -> Vec<(f64, f64, Option<f64>)> {
        let (mut top, mut height) = (0.0, 0.0);
        let mut out = Vec::new();
        for line in text.lines() {
            if let Some(z) = line.strip_prefix(";Z:") {
                top = z.parse().unwrap();
            }
            if let Some(h) = line.strip_prefix(";HEIGHT:") {
                height = h.parse().unwrap();
            }
            let word = |w: char| {
                line.split(' ')
                    .find_map(|part| part.strip_prefix(w)?.parse::<f64>().ok())
            };
            if line.starts_with("G1 X") && word('E').is_some_and(|e| e > 0.0) {
                out.push((top, height, word('Z')));
            }
        }
        out
    }

    #[test]
    fn a_shallow_slope_is_contoured_within_its_band_and_not_without() {
        let plain = heights(&wedge(r#"{"layer_height": "0.2"}"#));
        assert!(plain.iter().all(|(_, _, z)| z.is_none()));
        let text = wedge(r#"{"layer_height": "0.2", "zaa_enabled": "1"}"#);
        let contoured = heights(&text);
        let said: Vec<_> = contoured.iter().filter(|(.., z)| z.is_some()).collect();
        assert!(said.len() > 100, "{}", said.len());
        for &&(top, height, z) in &said {
            let z = z.unwrap();
            // Down to the slicing plane, `zaa_min_z` above the layer
            // beneath, and up to as much above its own top.
            assert!(
                z >= top - height + 0.05 - 6e-4 && z <= top + 0.05 + 6e-4,
                "{top} {z}"
            );
        }
        assert!(said.iter().any(|&&(top, _, z)| z.unwrap() < top - 0.05));
        assert!(said.iter().any(|&&(top, _, z)| z.unwrap() > top + 0.01));
        // The same bytes on one thread as on all of them.
        let one = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| wedge(r#"{"layer_height": "0.2", "zaa_enabled": "1"}"#));
        assert_eq!(one, text);
    }

    #[test]
    fn a_min_z_over_the_layer_height_is_refused() {
        let printer = Settings::from_json(r#"{"zaa_enabled": "1", "zaa_min_z": "0.3"}"#).unwrap();
        let zaa = edge_contour::ZaaSettings::read(&printer)
            .unwrap()
            .zaa
            .unwrap();
        let mut planes = vec![0.1, 0.3, 0.5];
        let heights = crate::layer_heights::LayerHeights::uniform(0.2, 0.2, 3);
        assert!(super::plane(&zaa, &mut planes, &heights, 0.0).is_err());
        let fine = edge_contour::ZaaSettings { min_z: 0.05, ..zaa };
        super::plane(&fine, &mut planes, &heights, 0.0).unwrap();
        assert!((planes[1] - 0.25).abs() < 1e-12 && (planes[2] - 0.45).abs() < 1e-12);
        assert_eq!(planes[0], 0.1);
    }
}
