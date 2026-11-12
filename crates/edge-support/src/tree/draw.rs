//! Each branch drawn on its layers: what the support's body is on every
//! layer, and where it stands on the part.

use std::f64::consts::TAU;

use edge_geom::{Coord, ExPolygon, Point2, Polygon, scaled};
use rayon::prelude::*;

use super::config::TreeConfig;
use super::element::Element;
use super::geometry::{Polys, diff_safe, intersection, remove_small, tiny_area, union};
use super::volumes::Volumes;
use crate::SupportError;

/// The support's body on each layer, and the places it stands on the part.
pub(crate) struct Drawn {
    pub base: Vec<Polys>,
    pub bottom_contacts: Vec<Polys>,
}

/// A circle with corners enough to stay within `eps` of round.
pub(crate) fn round_circle(centre: Point2, radius: Coord, eps: f64) -> ExPolygon {
    let r = radius as f64;
    let corners = if r <= eps {
        8
    } else {
        ((TAU / (2.0 * (1.0 - eps / r).acos())).ceil() as usize).clamp(8, 256)
    };
    ExPolygon {
        contour: Polygon {
            points: (0..corners)
                .map(|corner| {
                    let a = TAU * corner as f64 / corners as f64;
                    Point2 {
                        x: centre.x + (r * a.cos()).round() as Coord,
                        y: centre.y + (r * a.sin()).round() as Coord,
                    }
                })
                .collect(),
        },
        holes: Vec::new(),
    }
}

/// Every branch as a circle of its radius round its centre, cut clear of
/// the part and kept on the bed.
pub(crate) fn draw_circles(
    volumes: &Volumes,
    config: &TreeConfig,
    layers: &[Vec<Element>],
    bed: Option<&ExPolygon>,
) -> Result<Drawn, SupportError> {
    let eps = scaled(0.015) as f64;
    let base = layers
        .par_iter()
        .enumerate()
        .map(|(layer, elements)| -> Result<Polys, SupportError> {
            let circles: Polys = elements
                .iter()
                .filter_map(|element| {
                    element
                        .state
                        .result
                        .map(|at| round_circle(at, element.state.radius(config), eps))
                })
                .collect();
            if circles.is_empty() {
                return Ok(Vec::new());
            }
            let mut out = diff_safe(&union(&circles)?, volumes.collision(0, layer, true))?;
            if let Some(bed) = bed {
                out = intersection(&out, std::slice::from_ref(bed))?;
            }
            remove_small(&mut out, tiny_area());
            Ok(out)
        })
        .collect::<Result<Vec<Polys>, SupportError>>()?;
    let bottom_contacts = vec![Vec::new(); base.len()];
    Ok(Drawn {
        base,
        bottom_contacts,
    })
}
