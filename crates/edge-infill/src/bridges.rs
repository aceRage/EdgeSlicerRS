//! What the bridging options of the reference's Quality tab ask of the
//! fill's surfaces: how far a bridge reaches onto what carries it
//! (`bridge_expansion_extra`), and a second layer of bridge over a bridge
//! (`enable_extra_bridge_layer`).

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon, Point2, scaled};
use rayon::prelude::*;

use crate::surfaces::Faces;

/// Gives each bridge a second layer: what of the layer above lies on a
/// bridge, and is neither a top face nor a bottom face of its own, is
/// made a bottom face too, so that it is laid as a bridge and the shell
/// over a bridge starts from it (PrintObject.cpp
/// `PrintObject::detect_surfaces_type`, "Create a SECOND bridge layer
/// above the first bridge layer"). `faces[i]` is layer `i`'s top faces
/// and bottom faces, of which the first layer's are on the bed and no
/// bridge. Patches that do not survive shrinking by `narrow` get no
/// second layer: bridges no wider than their walls, which the walls
/// cover anyway. Nor does anything within `narrow` of a top face on the
/// same layer, where a scrap of bridge would show in the surface.
///
/// Only a first bridge gets a second layer: what is added here does not
/// get one in turn.
///
/// Different from the reference: its clipping grows what it cuts with by
/// a hundredth of a micron (`ApplySafetyOffset::Yes`), which is not done.
pub(crate) fn second_bridge_layers(
    slices: &[&[ExPolygon]],
    mut faces: Vec<Faces>,
    narrow: Coord,
) -> Result<Vec<Faces>, ClipError> {
    let join = Join::MITER;
    let added = (0..faces.len())
        .into_par_iter()
        .map(|layer| {
            if layer < 2 || faces[layer - 1].1.is_empty() {
                return Ok(Vec::new());
            }
            let bridges = &faces[layer - 1].1;
            let (top, bottom) = &faces[layer];
            let internal = if top.is_empty() && bottom.is_empty() {
                slices[layer].to_vec()
            } else {
                let exposed: Vec<ExPolygon> = top.iter().chain(bottom).cloned().collect();
                edge_clip::difference(slices[layer], &exposed)?
            };
            let mut over =
                edge_clip::opening(&edge_clip::intersection(&internal, bridges)?, narrow, join)?;
            if !top.is_empty() && !over.is_empty() {
                over = edge_clip::difference(&over, &edge_clip::offset(top, narrow, join)?)?;
            }
            Ok(over)
        })
        .collect::<Result<Vec<Vec<ExPolygon>>, ClipError>>()?;
    for (own, more) in faces.iter_mut().zip(added) {
        own.1.extend(more);
    }
    Ok(faces)
}

/// How a region is grown into what lies beside it in waves, each cut to
/// the area it grows in before the next sets out, so that growth follows
/// the area round a corner and never jumps a gap
/// (Algorithm/RegionExpansion.cpp `RegionExpansionParameters::build`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Waves {
    /// How far the region is first grown to find where it touches the
    /// area: its outline moved out by this is where the waves start.
    pub tiny: f64,
    /// How far each wave goes, and how many there are.
    pub step: f64,
    pub steps: usize,
}

impl Waves {
    /// The waves that carry a region `full` (scaled, above zero) into an
    /// area: steps of a tenth of a millimetre, but no more than five of
    /// them, and none so short that the first small growth would show.
    pub(crate) fn of(full: Coord) -> Self {
        // `expansion_step` and `max_nr_expansion_steps` of
        // `LayerRegion::process_external_surfaces`.
        let wanted = scaled(0.1) as f64;
        const MOST: usize = 5;
        let full = full as f64;
        let mut tiny = (0.25 * full).min(scaled(0.05) as f64);
        let mut steps = (((full - tiny) / wanted).ceil() as usize).clamp(1, MOST);
        let mut step = (full - tiny) / steps as f64;
        if steps > 1 && 0.25 * step < tiny {
            steps = (((full - tiny) / (4.0 * tiny)).floor() as usize).max(1);
            step = (full - tiny) / steps as f64;
        }
        if 0.25 * step < tiny || steps == 1 {
            tiny = 0.2 * full;
            step = 0.8 * full;
        }
        Self { tiny, step, steps }
    }
}

/// What `source` gains of `boundary`, one piece of the area beside it,
/// grown into it in `waves` (RegionExpansion.cpp `wave_seeds` and
/// `propagate_wave_from_boundary`): the source's outline, moved out a
/// little, is kept where it runs inside the piece, thickened by one step,
/// and what that covers of the piece is grown a step at a time, cut to
/// the piece after each.
pub(crate) fn wave(
    source: &ExPolygon,
    boundary: &ExPolygon,
    waves: &Waves,
) -> Result<Vec<ExPolygon>, ClipError> {
    let inside = std::slice::from_ref(boundary);
    let front = edge_clip::offset(
        std::slice::from_ref(source),
        waves.tiny.round() as Coord,
        Join::Square,
    )?;
    let rings: Vec<Vec<Point2>> = front
        .iter()
        .flat_map(ExPolygon::rings)
        .filter(|ring| ring.points.len() >= 2)
        .map(|ring| {
            let mut closed = ring.points.clone();
            closed.push(ring.points[0]);
            closed
        })
        .collect();
    let seeds = edge_clip::clip_lines(&rings, inside)?;
    if seeds.is_empty() {
        return Ok(Vec::new());
    }
    let step = waves.step.round() as Coord;
    // The reference asks its offsetter for arcs within a tenth of a
    // millimetre, and the offsetter never allows more than a quarter of
    // the distance offset (ClipperLib `ClipperOffset::DoOffset`).
    let tolerance = ((0.25 * waves.step) as Coord).min(scaled(0.1));
    let mut reached = edge_clip::intersection(
        &edge_clip::thicken_round(seeds.iter().map(Vec::as_slice), step, tolerance)?,
        inside,
    )?;
    for _ in 1..waves.steps {
        if reached.is_empty() {
            break;
        }
        reached = edge_clip::intersection(
            &edge_clip::offset(&reached, step, Join::Round { tolerance })?,
            inside,
        )?;
    }
    Ok(reached)
}

/// One kind of fill beside a layer's bridges, and how far a bridge
/// reaches into it.
pub(crate) struct Zone<'a> {
    pub area: &'a [ExPolygon],
    pub reach: Coord,
    /// Whether the bridge is grown into it in waves, as the reference
    /// grows it, or by the whole distance at once and cut to the zone, as
    /// this engine grows a surface with no extra reach set.
    pub waves: bool,
}

/// A layer's bridges grown into the `zones` beside them, and those
/// whose gains run into one another made one bridge, laid the way the
/// first of them is (LayerRegion.cpp `expand_bridges_detect_orientations`:
/// `expand_expolygons`, `get_grouped_bridges`, `merge_bridges`).
/// `angles[i]` is the direction of `bridges[i]`. Gaps narrower than twice
/// `closing` left in a grown bridge are closed. Each bridge returned is
/// its area and its direction; two of them do not overlap.
pub(crate) fn expand(
    bridges: &[ExPolygon],
    angles: &[f64],
    zones: &[Zone<'_>],
    closing: Coord,
) -> Result<Vec<(Vec<ExPolygon>, f64)>, ClipError> {
    assert_eq!(bridges.len(), angles.len(), "one direction per bridge");
    let join = Join::MITER;
    let mut gains: Vec<Vec<ExPolygon>> = Vec::with_capacity(bridges.len());
    for bridge in bridges {
        let mut gain = Vec::new();
        for zone in zones {
            if zone.area.is_empty() || zone.reach <= 0 {
                continue;
            }
            if zone.waves {
                let waves = Waves::of(zone.reach);
                for boundary in zone.area {
                    gain.extend(wave(bridge, boundary, &waves)?);
                }
            } else {
                gain.extend(edge_clip::intersection(
                    &edge_clip::offset(std::slice::from_ref(bridge), zone.reach, join)?,
                    zone.area,
                )?);
            }
        }
        gains.push(gain);
    }
    // Bridges whose gains overlap share the lowest number among them.
    let mut group: Vec<usize> = (0..bridges.len()).collect();
    fn head(group: &mut [usize], mut of: usize) -> usize {
        while group[of] != of {
            group[of] = group[group[of]];
            of = group[of];
        }
        of
    }
    for first in 0..bridges.len() {
        for second in first + 1..bridges.len() {
            if gains[first].is_empty()
                || gains[second].is_empty()
                || edge_clip::intersection(&gains[first], &gains[second])?.is_empty()
            {
                continue;
            }
            let (a, b) = (head(&mut group, first), head(&mut group, second));
            group[a.max(b)] = a.min(b);
        }
    }
    // A grown bridge stays within what it started from and may grow into.
    let mut allowed: Vec<ExPolygon> = bridges.to_vec();
    for zone in zones.iter().filter(|zone| zone.reach > 0) {
        allowed.extend(zone.area.iter().cloned());
    }
    let allowed = edge_clip::union_ex(&allowed)?;
    let mut merged = Vec::new();
    let mut taken: Vec<ExPolygon> = Vec::new();
    for (first, &angle) in angles.iter().enumerate() {
        if head(&mut group, first) != first {
            continue;
        }
        let mut all = Vec::new();
        for member in first..bridges.len() {
            if head(&mut group, member) == first {
                all.push(bridges[member].clone());
                all.extend(gains[member].iter().cloned());
            }
        }
        let mut area =
            edge_clip::intersection(&edge_clip::closing(&all, closing, join)?, &allowed)?;
        if !taken.is_empty() {
            area = edge_clip::difference(&area, &taken)?;
        }
        taken.extend(area.iter().cloned());
        merged.push((area, angle));
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use edge_geom::{Polygon, unscaled};

    use super::*;

    fn block(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    #[test]
    fn what_lies_on_a_bridge_becomes_a_second_layer_of_it() {
        // Four layers 40 x 10 mm. The first is on the bed; the second
        // has a bridge 20 mm long in its middle; the fourth is the top.
        let whole = vec![block(0.0, 0.0, 40.0, 10.0)];
        let slices: Vec<&[ExPolygon]> = vec![&whole; 4];
        let gap = vec![block(10.0, 0.0, 30.0, 10.0)];
        let faces: Vec<Faces> = vec![
            (Vec::new(), whole.clone()),
            (Vec::new(), gap.clone()),
            (Vec::new(), Vec::new()),
            (whole.clone(), Vec::new()),
        ];
        let found = second_bridge_layers(&slices, faces.clone(), scaled(0.87)).unwrap();
        // The third layer gets the bridge's own area as a bottom face.
        assert!((area(&found[2].1) - 200.0).abs() < 1e-6);
        // The bed's layer gives the second nothing: it is no bridge. And
        // the second layer of bridge gives the fourth nothing.
        assert_eq!(found[1], faces[1]);
        assert_eq!(found[3], faces[3]);
        assert_eq!(found[0], faces[0]);

        // A bridge no wider than twice the walls gets none.
        let narrow: Vec<Faces> = vec![
            (Vec::new(), whole.clone()),
            (Vec::new(), vec![block(10.0, 0.0, 11.7, 10.0)]),
            (Vec::new(), Vec::new()),
            (whole.clone(), Vec::new()),
        ];
        let found = second_bridge_layers(&slices, narrow.clone(), scaled(0.87)).unwrap();
        assert_eq!(found, narrow);

        // What is a face of its own on the layer above stays that, and
        // within the walls' width of a top face nothing is added: here
        // the layer above is top from x = 25 on.
        let top = vec![block(25.0, 0.0, 40.0, 10.0)];
        let stepped: Vec<Faces> = vec![
            (Vec::new(), whole.clone()),
            (Vec::new(), gap),
            (top.clone(), Vec::new()),
            (whole.clone(), Vec::new()),
        ];
        let found = second_bridge_layers(&slices, stepped, scaled(0.87)).unwrap();
        assert_eq!(found[2].0, top);
        // From x = 10 to 25 less 0.87 mm.
        assert!(
            (area(&found[2].1) - 141.3).abs() < 1e-3,
            "{}",
            area(&found[2].1)
        );
    }

    #[test]
    fn waves_are_the_reference_s() {
        // The usual reach into solid fill with two walls, 1.406 mm: five
        // waves from a twentieth of a millimetre out.
        let usual = Waves::of(1_406_000);
        assert_eq!((usual.tiny, usual.steps), (50_000.0, 5));
        assert!((usual.step - 271_200.0).abs() < 1e-6);
        // One wall's spacing into sparse fill: the waves would be too
        // short beside the first growth, so it is one wave of four
        // fifths from a fifth out.
        let short = Waves::of(407_000);
        assert_eq!(short.steps, 1);
        assert!((short.tiny - 81_400.0).abs() < 1e-6 && (short.step - 325_600.0).abs() < 1e-6);
        // However far, no more than five waves.
        let far = Waves::of(10_000_000);
        assert_eq!((far.tiny, far.steps, far.step), (50_000.0, 5, 1_990_000.0));
        // Very short: a quarter of it is the first growth, then one wave
        // takes over.
        let least = Waves::of(100);
        assert_eq!((least.tiny, least.step, least.steps), (20.0, 80.0, 1));
        // In every case the waves end at the full distance.
        for full in [100, 407_000, 600_000, 1_406_000, 4_406_000, 10_000_000] {
            let waves = Waves::of(full);
            let end = waves.tiny + waves.step * waves.steps as f64;
            assert!((end - full as f64).abs() < 1e-6, "{full}: {waves:?}");
        }
    }

    #[test]
    fn a_wave_follows_the_area_and_does_not_jump_a_gap() {
        // A bridge with solid fill against its right side, and more solid
        // fill 1 mm away from its top that it does not touch.
        let bridge = block(0.0, 0.0, 10.0, 10.0);
        let touching = block(10.0, 0.0, 20.0, 10.0);
        let apart = block(0.0, 11.0, 10.0, 21.0);
        let waves = Waves::of(scaled(3.0));
        let gained = wave(&bridge, &touching, &waves).unwrap();
        // 3 mm deep along the whole shared side.
        assert!((area(&gained) - 30.0).abs() < 0.05, "{}", area(&gained));
        let reach = gained
            .iter()
            .flat_map(|region| &region.contour.points)
            .map(|point| unscaled(point.x))
            .fold(f64::MIN, f64::max);
        assert!((reach - 13.0).abs() < 1e-3, "{reach}");
        // Nothing of the fill it does not touch, however near.
        assert!(wave(&bridge, &apart, &waves).unwrap().is_empty());
    }

    #[test]
    fn a_wave_goes_round_a_corner_no_further_than_its_way_there() {
        // Solid fill in an L: against the bridge's right side for the
        // lower 2 mm only, then up behind a 1 mm slot that is not fill.
        let bridge = block(0.0, 0.0, 10.0, 10.0);
        let mut ell = block(10.0, 0.0, 20.0, 10.0);
        ell.contour.points = [
            (10.0, 0.0),
            (20.0, 0.0),
            (20.0, 10.0),
            (11.0, 10.0),
            (11.0, 2.0),
            (10.0, 2.0),
        ]
        .map(|(x, y)| Point2::from_mm(x, y))
        .to_vec();
        let gained = wave(&bridge, &ell, &Waves::of(scaled(4.0))).unwrap();
        // Straight across the slot the fill at (11.5, 9) is 1.5 mm from
        // the bridge; by way of the fill it is over 7 mm, out of reach.
        let highest = gained
            .iter()
            .flat_map(|region| &region.contour.points)
            .map(|point| unscaled(point.y))
            .fold(f64::MIN, f64::max);
        assert!(highest < 6.5, "{highest}");
        // Along the bottom it reaches the full 4 mm.
        let furthest = gained
            .iter()
            .flat_map(|region| &region.contour.points)
            .map(|point| unscaled(point.x))
            .fold(f64::MIN, f64::max);
        assert!((furthest - 14.0).abs() < 1e-3, "{furthest}");
    }

    #[test]
    fn bridges_that_grow_into_each_other_become_one() {
        // Two bridges 4 mm apart with solid fill between them, and a
        // third far away with solid fill of its own beside it.
        let bridges = [
            block(0.0, 0.0, 10.0, 10.0),
            block(14.0, 0.0, 24.0, 10.0),
            block(60.0, 0.0, 70.0, 10.0),
        ];
        let solid = [block(10.0, 0.0, 14.0, 10.0), block(70.0, 0.0, 80.0, 10.0)];
        let angles = [0.25, 1.0, 1.5];
        let grow = |reach: f64| {
            expand(
                &bridges,
                &angles,
                &[Zone {
                    area: &solid,
                    reach: scaled(reach),
                    waves: true,
                }],
                scaled(0.15),
            )
            .unwrap()
        };
        // Reaching 1.5 mm each, the first two stay 1 mm apart.
        let apart = grow(1.5);
        assert_eq!(apart.len(), 3);
        assert!((area(&apart[0].0) - 115.0).abs() < 0.05);
        assert_eq!(
            apart.iter().map(|found| found.1).collect::<Vec<_>>(),
            angles
        );
        // Reaching 3 mm they meet: one bridge over all 24 mm, laid the
        // first one's way. The third is as it was, 3 mm longer.
        let met = grow(3.0);
        assert_eq!(met.len(), 2);
        assert!(
            (area(&met[0].0) - 240.0).abs() < 0.05,
            "{}",
            area(&met[0].0)
        );
        assert_eq!((met[0].1, met[1].1), (0.25, 1.5));
        assert!((area(&met[1].0) - 130.0).abs() < 0.05);
    }

    #[test]
    fn a_zone_not_grown_in_waves_is_reached_at_once() {
        let bridges = [block(0.0, 0.0, 10.0, 10.0)];
        let sparse = [block(10.0, 0.0, 20.0, 10.0)];
        let found = expand(
            &bridges,
            &[0.0],
            &[Zone {
                area: &sparse,
                reach: scaled(0.4),
                waves: false,
            }],
            scaled(0.15),
        )
        .unwrap();
        assert!((area(&found[0].0) - 104.0).abs() < 1e-3);
    }
}
