//! The reference's own end-to-end cases for paint depth
//! (tests/libslic3r/test_paint_depth_clamp.cpp), with its numbers: the
//! same boxes, painted the same way, sliced into layers here and shared
//! out by [`segment`], probed at the same points.

use edge_geom::{ExPolygon, Point2, Polygon};

use crate::depth::{self, Depth, Mode};
use crate::{Facet, PaintSettings, Shell, segment};

/// The reference's test flows: 0.45 mm outer and inner walls.
const LINE: f64 = 0.45;

/// A box's twelve triangles from `low` to `high`, each side painted as
/// `paint` says: 0 and 1 the sides at the least and greatest x, 2 and 3
/// those at the least and greatest y, 4 the bottom and 5 the top.
fn block(high: [f32; 3], paint: impl Fn(usize) -> u8) -> Vec<Facet> {
    let [x0, y0, z0] = [0.0f32; 3];
    let [x1, y1, z1] = high;
    let sides: [[[f32; 3]; 4]; 6] = [
        [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
        [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
        [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
        [[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]],
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
    ];
    sides
        .iter()
        .enumerate()
        .flat_map(|(side, [a, b, c, d])| {
            [[*a, *b, *c], [*a, *c, *d]].map(|corners| Facet {
                corners,
                filament: paint(side),
            })
        })
        .collect()
}

const PLUS_X: fn(usize) -> u8 = |side| u8::from(side == 1) * 2;
const ALL_SIDES: fn(usize) -> u8 = |side| if side < 4 { 2 } else { 0 };
const TOP: fn(usize) -> u8 = |side| u8::from(side == 5) * 2;

/// How a case is sliced.
#[derive(Clone, Copy)]
struct Case {
    mode: Mode,
    walls: i64,
    mm: f64,
    /// `mmu_segmented_region_interlocking_depth`: 0.1, its default, unless
    /// raised.
    notch: f64,
    layer_height: f64,
    classic: bool,
    top: Shell,
    bottom: Shell,
}

impl Case {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            walls: 3,
            mm: 1.5,
            notch: 0.1,
            layer_height: 0.2,
            classic: false,
            top: Shell {
                layers: 0,
                thickness: 0.0,
            },
            bottom: Shell {
                layers: 0,
                thickness: 0.0,
            },
        }
    }

    /// The settings, worked out as the engine works them out from a
    /// profile.
    fn settings(&self) -> PaintSettings {
        let h = self.layer_height;
        let spacing = (LINE - h * (1.0 - std::f64::consts::FRAC_PI_4)) as f32;
        let width = LINE as f32;
        let mut band = depth::band_mm(self.mode, self.walls, self.mm, width, spacing, spacing);
        let stack = width + spacing;
        if self.classic {
            band = depth::band_classic_floor_mm(band, width, spacing);
        }
        let mut notch = if self.mode == Mode::Unlimited {
            0.0
        } else {
            depth::interlocking_mm(self.mode, self.notch, spacing)
        };
        if self.classic {
            notch = depth::classic_notch_cap_mm(notch, band, stack);
        }
        PaintSettings {
            top: self.top,
            bottom: self.bottom,
            wall_width: edge_geom::scaled(LINE),
            wall_spacing: edge_geom::scaled(f64::from(spacing)),
            small: edge_geom::scaled(0.25 * LINE),
            gaps_filled: true,
            classic: self.classic,
            run_scale: 1.0,
            depth: Depth {
                band,
                interlocking: notch,
                min_claim_width: width,
                normal: if self.mode == Mode::Unlimited {
                    0.0
                } else {
                    band
                },
                wall_stack: stack,
                beam: false,
            },
        }
    }

    /// A `x` by `y` by `z` box painted as `paint` says, cut into layers and
    /// shared out: the filament-2 claim of each layer.
    fn slice(&self, size: [f64; 3], paint: fn(usize) -> u8) -> Vec<Vec<ExPolygon>> {
        let h = self.layer_height;
        let count = (size[2] / h).round() as usize;
        let square = vec![ExPolygon {
            contour: Polygon {
                points: [
                    (0.0, 0.0),
                    (size[0], 0.0),
                    (size[0], size[1]),
                    (0.0, size[1]),
                ]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
            },
            holes: Vec::new(),
        }];
        let outlines: Vec<&[ExPolygon]> = vec![square.as_slice(); count];
        let planes: Vec<f64> = (0..count).map(|layer| (layer as f64 + 0.5) * h).collect();
        let z: Vec<[f64; 2]> = (0..count)
            .map(|layer| [layer as f64 * h, (layer + 1) as f64 * h])
            .collect();
        let facets = block(size.map(|v| v as f32), paint);
        segment(&outlines, &planes, &z, &facets, &self.settings())
            .unwrap()
            .into_iter()
            .map(|layer| {
                layer
                    .into_iter()
                    .filter(|(colour, _)| *colour == 2)
                    .flat_map(|(_, regions)| regions)
                    .collect()
            })
            .collect()
    }
}

fn claimed(claim: &[ExPolygon], point: Point2) -> bool {
    claim.iter().any(|region| depth::contains(region, point))
}

/// A point `inset` mm in from the +X side of a box `width` wide, on its
/// middle line.
fn in_from_plus_x(size: [f64; 3], inset: f64) -> Point2 {
    Point2::from_mm(size[0] - inset, size[1] / 2.0)
}

fn centre(size: [f64; 3]) -> Point2 {
    Point2::from_mm(size[0] / 2.0, size[1] / 2.0)
}

/// An even layer near the middle and the odd one over it.
fn parities(layers: usize) -> (usize, usize) {
    let even = layers / 2 - (layers / 2) % 2;
    (even, even + 1)
}

const CUBE: [f64; 3] = [40.0, 40.0, 20.0];

#[test]
fn walls_clamp_a_painted_side_to_the_band_and_unlimited_does_not() {
    let walls = Case::new(Mode::Walls).slice(CUBE, PLUS_X);
    let middle = walls.len() / 2;
    assert!(claimed(&walls[middle], in_from_plus_x(CUBE, 0.3)));
    assert!(!claimed(&walls[middle], in_from_plus_x(CUBE, 10.0)));
    let unlimited = Case::new(Mode::Unlimited).slice(CUBE, PLUS_X);
    assert!(claimed(&unlimited[middle], in_from_plus_x(CUBE, 10.0)));
}

#[test]
fn the_band_is_pinned_and_the_notch_alternates_at_its_inner_edge() {
    let case = Case::new(Mode::Walls);
    let settings = case.settings();
    let (band, notch) = (
        f64::from(settings.depth.band),
        f64::from(settings.depth.interlocking),
    );
    assert!(notch > 0.0 && notch < band);
    let claims = case.slice(CUBE, PLUS_X);
    let (even, odd) = parities(claims.len());
    let at = |layer: usize, inset: f64| claimed(&claims[layer], in_from_plus_x(CUBE, inset));
    let inside_both = band - notch - 0.05;
    assert!(inside_both > 0.05);
    assert!(at(even, inside_both) && at(odd, inside_both));
    let tooth = band - notch / 2.0;
    assert!(at(odd, tooth));
    assert!(!at(even, tooth));
    assert!(!at(even, band + 0.2) && !at(odd, band + 0.2));
}

#[test]
fn a_side_painted_all_round_is_still_clamped_to_the_band() {
    let walls = Case::new(Mode::Walls).slice(CUBE, ALL_SIDES);
    let middle = walls.len() / 2;
    assert!(claimed(&walls[middle], in_from_plus_x(CUBE, 0.3)));
    assert!(!claimed(&walls[middle], centre(CUBE)));
    // Unlimited, the whole layer is the colour's, middle and all.
    let unlimited = Case::new(Mode::Unlimited).slice(CUBE, ALL_SIDES);
    assert!(claimed(&unlimited[middle], centre(CUBE)));
}

#[test]
fn a_fin_too_thin_for_a_painted_skin_is_painted_through() {
    let fin = [1.2, 40.0, 20.0];
    let case = Case {
        mm: 2.0,
        ..Case::new(Mode::Millimeters)
    };
    let claims = case.slice(fin, ALL_SIDES);
    let (even, odd) = parities(claims.len());
    for layer in [even, odd] {
        assert!(claimed(&claims[layer], in_from_plus_x(fin, 0.1)), "{layer}");
        assert!(claimed(&claims[layer], centre(fin)), "{layer}");
    }
}

#[test]
fn a_part_thinner_than_the_band_gets_a_skin_the_same_on_both_parities() {
    let fin = [5.8, 40.0, 6.0];
    let case = Case {
        mm: 6.0,
        notch: 0.4,
        ..Case::new(Mode::Millimeters)
    };
    let claims = case.slice(fin, ALL_SIDES);
    let (even, odd) = parities(claims.len());
    for layer in [even, odd] {
        assert!(claimed(&claims[layer], in_from_plus_x(fin, 0.5)), "{layer}");
        assert!(
            !claimed(&claims[layer], in_from_plus_x(fin, 1.0)),
            "{layer}"
        );
        assert!(!claimed(&claims[layer], centre(fin)), "{layer}");
    }
}

#[test]
fn millimetres_take_the_notch_as_set() {
    let size = [40.0, 40.0, 6.0];
    let case = Case {
        mm: 6.0,
        notch: 0.4,
        ..Case::new(Mode::Millimeters)
    };
    let claims = case.slice(size, PLUS_X);
    let (even, odd) = parities(claims.len());
    let at = |layer: usize, inset: f64| claimed(&claims[layer], in_from_plus_x(size, inset));
    assert!(at(odd, 5.9) && !at(odd, 6.1));
    assert!(at(even, 5.4) && !at(even, 5.7));
}

#[test]
fn a_painted_top_claims_its_shell_by_thickness_in_thin_layers() {
    // A 40 x 40 x 4 mm slab at 0.1 mm layers, its top painted; four shell
    // layers or 0.6 mm, so six layers, full width 2 mm in, and nothing
    // past them.
    let slab = [40.0, 40.0, 4.0];
    let case = Case {
        layer_height: 0.1,
        top: Shell {
            layers: 4,
            thickness: 0.6,
        },
        bottom: Shell {
            layers: 3,
            thickness: 0.0,
        },
        ..Case::new(Mode::Unlimited)
    };
    let claims = case.slice(slab, TOP);
    let top = claims.len() - 1;
    for depth in 0..=5 {
        assert!(
            claimed(&claims[top - depth], in_from_plus_x(slab, 2.0)),
            "{depth}"
        );
    }
    assert!(!claimed(&claims[top - 6], centre(slab)));
    assert!(!claimed(&claims[top - 6], in_from_plus_x(slab, 2.0)));
}

#[test]
fn a_flat_painted_top_stops_at_its_shell_under_a_bounded_depth() {
    // Three walls are 1.435675 mm, fifteen layers of 0.1 mm; a flat top is
    // still claimed only through its six-layer shell.
    let slab = [40.0, 40.0, 4.0];
    let case = Case {
        layer_height: 0.1,
        top: Shell {
            layers: 4,
            thickness: 0.6,
        },
        bottom: Shell {
            layers: 3,
            thickness: 0.0,
        },
        ..Case::new(Mode::Walls)
    };
    let claims = case.slice(slab, TOP);
    let top = claims.len() - 1;
    for depth in 0..=5 {
        assert!(claimed(&claims[top - depth], centre(slab)), "{depth}");
    }
    for depth in 6..=14 {
        assert!(!claimed(&claims[top - depth], centre(slab)), "{depth}");
    }
}
