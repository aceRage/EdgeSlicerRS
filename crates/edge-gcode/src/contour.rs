//! Laying a line contoured in height (`zaa_enabled`, `edge_contour`):
//! every move says the height it ends at, lays as much as the line is
//! tall there, and with `zaa_speed_scaling` goes as much slower as it is
//! thinner (GCode.cpp `_extrude`, its "ZAA" branches).
//!
//! The lines come contoured, with their heights by place
//! ([`edge_contour::Profile`]); which they are is given beside the
//! layers ([`Contours`]), and known here by where each is kept, as arcs
//! fitted ahead are. A contoured line is laid straight from point to
//! point: no arc can say a height on the way, and none is fitted to it;
//! nor is it thinned, its points being where its heights were taken.

use std::{collections::HashMap, sync::Arc};

use edge_contour::{LayerProfiles, Profile, speed};
use edge_extrusion::{Flow, Role};
use edge_geom::Point2;

use crate::{Layer, Writer, address};

/// What the lines contoured in height are laid by: for each of the
/// layers written, the profiles of its loops and paths (or nothing for
/// any layer, where none are); and whether a contoured move is slowed
/// for its height (`zaa_speed_scaling`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Contours<'a> {
    pub layers: &'a [&'a LayerProfiles],
    pub speed_scaling: bool,
}

/// The contoured line being laid.
#[derive(Clone, Debug)]
pub(crate) struct Laying {
    profile: Arc<Profile>,
    /// Whether the move being laid is contoured: a stretch of a wall laid
    /// over air is not, as the reference's overhang walls are paths of
    /// their own and never contoured.
    active: bool,
    /// The height that set the speed in force, in mm; 0 for none yet
    /// (`zaa_speed_h_ref`). And the speed it set, and the line's own
    /// speed it was worked from, in mm/s.
    set_for: f64,
    speed: f64,
    from: f64,
}

/// One contoured move, as [`Writer::lay`] writes it.
pub(crate) struct Contoured {
    /// The height it ends at, in mm above the bed.
    pub(crate) z: f64,
    /// How far the nozzle goes, the climb counted in, in mm.
    pub(crate) length: f64,
    /// What it lays, as a share of what it would at the layer's height.
    pub(crate) flow: f64,
    /// Its speed, in mm/s, where it is slowed for its height.
    pub(crate) speed: Option<f64>,
}

/// Whether a line of this role is laid by its contour.
fn contoured(role: Role) -> bool {
    matches!(
        role,
        Role::TopSolidInfill | Role::Ironing | Role::ExternalPerimeter | Role::Perimeter
    )
}

/// The contoured loops and paths of `parts`, by where they are kept;
/// `profiles` has an entry for each of `parts`, or none at all.
pub(crate) fn kept(
    parts: &[Layer<'_>],
    profiles: &[&LayerProfiles],
) -> HashMap<usize, Arc<Profile>> {
    let mut kept = HashMap::new();
    for (part, profiles) in parts.iter().zip(profiles) {
        for (wall, profile) in part.loops.iter().zip(&profiles.loops) {
            if let Some(profile) = profile {
                kept.insert(address(wall), profile.clone());
            }
        }
        for (path, profile) in part.paths.iter().zip(&profiles.paths) {
            if let Some(profile) = profile {
                kept.insert(address(path), profile.clone());
            }
        }
    }
    kept
}

impl Writer<'_> {
    /// Makes the loop or path kept at `key` the line being laid, as
    /// contoured where it is; nothing is contoured once it is done.
    pub(crate) fn contour(&mut self, key: Option<usize>) {
        self.state.contour = key
            .and_then(|key| self.contoured.get(&key))
            .map(|profile| Laying {
                profile: profile.clone(),
                active: false,
                set_for: 0.0,
                speed: 0.0,
                from: 0.0,
            });
    }

    /// Whether the line being laid is contoured.
    pub(crate) fn contouring(&self) -> bool {
        self.state.contour.is_some()
    }

    /// Readies the move about to be laid in `role`: contoured or not.
    pub(crate) fn contour_role(&mut self, role: Role) {
        if let Some(laying) = &mut self.state.contour {
            laying.active = contoured(role);
        }
    }

    /// The height of the contoured line at `point`, in mm above the bed,
    /// where the move being laid is contoured (the layer's top, plus
    /// the profile there: GCode.cpp `zaa_base_z + z_at`).
    pub(crate) fn contour_height(&self, point: Point2) -> Option<f64> {
        let laying = self.state.contour.as_ref().filter(|laying| laying.active)?;
        Some(self.state.z + laying.profile.at(point))
    }

    /// The height a travel to `to` arrives at before it extrudes: the
    /// contoured line's own there where it starts one, else the layer's.
    pub(crate) fn arrival_height(&self, to: Point2) -> f64 {
        match &self.state.contour {
            Some(laying) => self.state.z + laying.profile.at(to),
            None => self.state.z,
        }
    }

    /// The move from `from` to `to`, `length` mm across the bed, in
    /// `role` with `flow` at `own` mm/s, on the contoured line being
    /// laid; none where it is not contoured. `mm3_per_mm` is what the
    /// line lays a mm at the layer's height, every flow ratio counted in.
    pub(crate) fn contour_on(
        &mut self,
        (from, to): (Point2, Point2),
        role: Role,
        flow: &Flow,
        own: f64,
        length: f64,
        mm3_per_mm: f64,
    ) -> Option<Contoured> {
        let scaling = self.contour_speed;
        let max_volumetric = self.settings.filament_max_volumetric_speed;
        let base = self.state.z;
        let laying = self.state.contour.as_mut().filter(|laying| laying.active)?;
        let (start, end) = (laying.profile.at(from), laying.profile.at(to));
        // "The local layer height varies ALONG the segment. Using the end
        // point's height alone (upstream does) mis-extrudes every Z
        // transition by half the step": the trapezoid mean.
        let mean = 0.5 * (start + end);
        let height = flow.height;
        let ratio = speed::flow_ratio(role == Role::Ironing, height, mean);
        let mut slowed = None;
        if scaling {
            // A new speed of the line's own throws out the one in force
            // ("the base feed rate just changed, so any scaled F in force
            // is stale").
            if laying.from != own {
                laying.from = own;
                laying.set_for = 0.0;
                laying.speed = own;
            }
            let local = height + mean;
            if !speed::keeps(laying.set_for, local) {
                let scaled = speed::speed(own, height, local, max_volumetric, mm3_per_mm * ratio);
                if scaled > 0.0 {
                    laying.speed = scaled;
                }
                laying.set_for = local;
            }
            slowed = Some(laying.speed);
        }
        let z = base + end;
        Some(Contoured {
            z,
            length: length.hypot(end - start),
            flow: ratio,
            speed: slowed,
        })
    }
}
