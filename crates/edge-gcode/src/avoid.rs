//! Travel that goes round the inside of the walls and not across them
//! (`reduce_crossing_wall`): what kind of move each travel is, the
//! layers made ready to route on ahead of writing them, and the way a
//! travel takes. The routing itself is `edge-avoid`'s, a port of the
//! reference's AvoidCrossingPerimeters.cpp; when it is used and when
//! not is GCode.cpp's (`travel_to`, `process_layer`).

use std::{io, ops::Range};

use edge_avoid::{Kind, Needs};
use edge_geom::{Point2, unscaled};
use rayon::prelude::*;

use crate::{Layer, Writer};

/// How the travel about to be made is routed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Routing {
    /// What is being printed, and so what its moves are between.
    pub(crate) kind: Kind,
    /// The next move alone is one outside the part, whatever is being
    /// printed (the reference's `use_external_mp_once`): the move away
    /// from a prime tower.
    pub(crate) outside_once: bool,
    /// The next move alone goes straight (`disable_once`): the first of
    /// the print, and the one from a brim, or from the first layer's
    /// skirt, to what is printed next.
    pub(crate) straight_once: bool,
}

impl Default for Routing {
    fn default() -> Self {
        Routing {
            kind: Kind::Part,
            outside_once: false,
            // "We enable it by default for the first travel move in print."
            straight_once: true,
        }
    }
}

impl Routing {
    /// A travel has been made: what was said of the next move alone is
    /// spent (`reset_once_modifiers`).
    pub(crate) fn passed(&mut self) {
        self.outside_once = false;
        self.straight_once = false;
    }
}

impl Writer<'_> {
    /// Makes some layers ready to route travel on, ahead of writing
    /// them and all at once on as many threads as there are: drawing a
    /// layer's outline in is most of the work, and what comes of it
    /// does not depend on the moves asked of it later. `groups` are the
    /// entries of `layers` that make up each layer, and `first` is which
    /// layer of the print the first of them is.
    pub(crate) fn preroute(
        &mut self,
        layers: &[Layer<'_>],
        groups: &[Range<usize>],
        first: usize,
    ) -> io::Result<()> {
        self.routes.clear();
        self.routes_from = first;
        if !self.settings.reduce_crossing_wall || self.slices.is_empty() {
            return Ok(());
        }
        let slices = self.slices;
        self.routes = groups
            .par_iter()
            .map(|group| {
                let parts = &layers[group.clone()];
                let needs = Needs {
                    supports: parts.iter().any(|part| !part.support.is_empty()),
                    outside: parts.iter().any(|part| {
                        !(part.around.is_empty() && part.tower.is_empty() && part.enter.is_empty())
                            || part.ram.is_some()
                    }),
                };
                edge_avoid::Layer::new(&slices[group.start], needs).map_err(|error| {
                    io::Error::other(format!(
                        "travel could not be routed round the walls at {:.3} mm: {error}",
                        parts[0].z
                    ))
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(())
    }

    /// The way from where the nozzle is to `to`: every point of it
    /// after the nozzle's own, ending in `to`. That is `to` alone where
    /// travel is not routed, where the move crosses nothing, and for a
    /// move that is to go straight (GCode.cpp `travel_to`, which routes
    /// every move whose start is known, long or short).
    pub(crate) fn way(&self, to: Point2) -> Vec<Point2> {
        let routing = self.state.routing;
        let layer = self
            .state
            .layer
            .checked_sub(self.routes_from)
            .and_then(|index| self.routes.get(index));
        let (Some(from), Some(_), Some(layer), false) = (
            self.state.position,
            self.state.height,
            layer,
            routing.straight_once,
        ) else {
            return vec![to];
        };
        let kind = if routing.outside_once {
            Kind::Outside
        } else {
            routing.kind
        };
        let way = layer.route(from, to, kind, self.settings.max_travel_detour);
        // Never off the bed. A way round follows the part's own outline
        // from inside it, so this cannot happen to a print that is on
        // the bed; should it, the straight move is the one known to be.
        if let Some([min_x, min_y, max_x, max_y]) = self.settings.bed
            && way.iter().any(|point| {
                let (x, y) = (unscaled(point.x), unscaled(point.y));
                x < min_x || y < min_y || x > max_x || y > max_y
            })
        {
            return vec![to];
        }
        way
    }
}
