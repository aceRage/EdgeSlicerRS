//! The push-fit "Pin" connector (the reference fork's own: PushFit.cpp,
//! PushFit.hpp): a pin printed on its own, pushed into a hole in each half
//! of a cut. Its body is a solid of revolution with chamfered ends and a
//! lip on each side; it can have a printing flat (one or two) and a slit
//! at each end so that the lip side flexes. Each hole is the pin's outline
//! with a clearance, a flared mouth and room at the bottom.
//!
//! Lengths are in mm. In the connector's frame the pin's axis is z, the
//! cut at z = 0, the upper half where z is greater.

use edge_mesh::{Mesh, Transform, boolean, connector, cut};

/// What is taken out of the pin (`PushFitCutout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinCutout {
    None,
    Flat,
    Slot,
    FlatAndSlot,
}

impl PinCutout {
    pub fn code(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Flat => 1,
            Self::Slot => 2,
            Self::FlatAndSlot => 3,
        }
    }

    pub fn of(code: u32) -> Option<Self> {
        [Self::None, Self::Flat, Self::Slot, Self::FlatAndSlot]
            .get(code as usize)
            .copied()
    }

    pub fn flat(self) -> bool {
        matches!(self, Self::Flat | Self::FlatAndSlot)
    }

    pub fn slot(self) -> bool {
        matches!(self, Self::Slot | Self::FlatAndSlot)
    }
}

/// How the pin is printed (`PushFitPose`): lying on its flat when it can,
/// or upright.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinPose {
    Auto,
    Upright,
}

/// A pin's shape (`PushFitParams`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pin {
    pub diameter: f64,
    /// How far it reaches into each half, from that half's cut face.
    pub length_upper: f64,
    pub length_lower: f64,
    /// The 45° lead-in at each end.
    pub end_chamfer: f64,
    pub lip: bool,
    /// How far the lip stands proud, its axial length, and where its
    /// middle is along each side, of that side's length.
    pub lip_height: f64,
    pub lip_width: f64,
    pub lip_position: f64,
    pub cutout: PinCutout,
    /// The printing flat's depth, and whether there is one each side.
    pub flat_depth: f64,
    pub flat_both_sides: bool,
    /// The slit's width, and how far it runs in from each end.
    pub slot_width: f64,
    pub slot_length: f64,
    /// The holes' radial clearance each side, and their extra depth.
    pub clearance_upper: f64,
    pub clearance_lower: f64,
    pub depth_clearance: f64,
    /// A groove in each hole for the lip, rather than a plain hole.
    pub hole_groove: bool,
    /// The 45° flare at each hole's mouth.
    pub mouth_chamfer: f64,
    /// Each hole ends in a 90° cone rather than a flat bridge.
    pub pointed_end: bool,
    pub pose: PinPose,
    /// The preset it came from (0 S, 1 M, 2 L, 3 its own), kept for the
    /// panel.
    pub preset: u32,
}

impl Default for Pin {
    fn default() -> Self {
        Self {
            diameter: 6.0,
            length_upper: 7.0,
            length_lower: 7.0,
            end_chamfer: 0.5,
            lip: true,
            lip_height: 0.25,
            lip_width: 1.0,
            lip_position: 0.6,
            cutout: PinCutout::FlatAndSlot,
            flat_depth: 0.9,
            flat_both_sides: true,
            slot_width: 1.0,
            slot_length: 4.0,
            clearance_upper: 0.1,
            clearance_lower: 0.1,
            depth_clearance: 0.4,
            hole_groove: false,
            mouth_chamfer: 0.4,
            pointed_end: true,
            pose: PinPose::Auto,
            preset: 1,
        }
    }
}

/// Which half a hole is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinSide {
    Upper,
    Lower,
}

/// Facets round the pin and its holes (`PUSHFIT_SECTORS`).
pub const PIN_SECTORS: usize = 96;
/// How far below its face a hole starts (`PUSHFIT_HOLE_MOUTH_LEAD`).
const MOUTH_LEAD: f64 = 0.2;
/// The least wall left beyond a hole (`PUSHFIT_MIN_WALL`).
const MIN_WALL: f64 = 1.2;

/// The presets S, M, L: diameter, length, lip height and width, flat depth,
/// slot width and length, clearance, depth clearance, mouth chamfer
/// (`PRESET_ROWS`).
const PRESETS: [[f64; 10]; 3] = [
    [4.0, 5.0, 0.20, 0.8, 0.6, 0.8, 3.0, 0.10, 0.4, 0.3],
    [6.0, 7.0, 0.25, 1.0, 0.9, 1.0, 4.0, 0.10, 0.4, 0.4],
    [8.0, 10.0, 0.30, 1.2, 1.2, 1.2, 6.0, 0.12, 0.5, 0.5],
];

impl Pin {
    pub fn length(&self, side: PinSide) -> f64 {
        match side {
            PinSide::Upper => self.length_upper,
            PinSide::Lower => self.length_lower,
        }
    }

    pub fn clearance(&self, side: PinSide) -> f64 {
        match side {
            PinSide::Upper => self.clearance_upper,
            PinSide::Lower => self.clearance_lower,
        }
    }

    /// The pin with a preset's sizes (`pushfit_apply_preset`): 0 S, 1 M,
    /// 2 L; another number leaves it as it is.
    pub fn with_preset(mut self, preset: u32) -> Pin {
        let Some(r) = PRESETS.get(preset as usize) else {
            return self;
        };
        self.diameter = r[0];
        self.length_upper = r[1];
        self.length_lower = r[1];
        self.lip_height = r[2];
        self.lip_width = r[3];
        self.flat_depth = r[4];
        self.slot_width = r[5];
        self.slot_length = r[6];
        self.clearance_upper = r[7];
        self.clearance_lower = r[7];
        self.depth_clearance = r[8];
        self.mouth_chamfer = r[9];
        self.preset = preset;
        self
    }

    /// How deep a hole goes, its pointed end included.
    fn hole_depth(&self, side: PinSide) -> f64 {
        let radius = 0.5 * self.diameter + self.clearance(side);
        self.length(side) + self.depth_clearance + if self.pointed_end { radius } else { 0.0 }
    }

    /// "Auto": the largest preset the cut takes (`pushfit_auto_size`):
    /// its diameter at most half the section's inscribed radius, each hole
    /// leaving a wall in its half (a thickness of 0 is not measured). S,
    /// and false, when none does.
    pub fn auto_size(&self, inscribed: f64, upper: f64, lower: f64) -> (Pin, bool) {
        for preset in (0..3).rev() {
            let pin = self.with_preset(preset);
            if inscribed > 0.0 && pin.diameter > 0.5 * inscribed {
                continue;
            }
            if upper > 0.0 && pin.hole_depth(PinSide::Upper) + MIN_WALL > upper {
                continue;
            }
            if lower > 0.0 && pin.hole_depth(PinSide::Lower) + MIN_WALL > lower {
                continue;
            }
            return (pin, true);
        }
        (self.with_preset(0), false)
    }

    /// The steepest overhang lying on its flat, in degrees
    /// (`pushfit_lying_overhang_deg`); 90 with no flat.
    pub fn lying_overhang(&self) -> f64 {
        if !self.cutout.flat() || self.diameter <= 0.0 {
            return 90.0;
        }
        let f = self.flat_depth.clamp(0.0, 0.5 * self.diameter);
        (1.0 - 2.0 * f / self.diameter)
            .clamp(-1.0, 1.0)
            .asin()
            .to_degrees()
    }

    /// The flat's width (`pushfit_flat_width`).
    pub fn flat_width(&self) -> f64 {
        if !self.cutout.flat() || self.diameter <= 0.0 {
            return 0.0;
        }
        let f = self.flat_depth.clamp(0.0, 0.5 * self.diameter);
        2.0 * (f * (self.diameter - f)).sqrt()
    }

    /// Whether it can lie on its flat, or why not (`pushfit_can_lie`).
    pub fn can_lie(&self) -> Result<(), &'static str> {
        if !self.cutout.flat() {
            return Err("The pin has no flat to lie on, so it prints upright.");
        }
        if self.lying_overhang() > 45.0 + 1e-3 {
            return Err(
                "The flat is too shallow to lie on without supports (it needs about 0.15 x the diameter), so the pin prints upright.",
            );
        }
        if self.flat_width() < (0.5 * self.diameter).max(1.5) {
            return Err(
                "The flat is too narrow to be a stable footprint, so the pin prints upright.",
            );
        }
        Ok(())
    }

    /// Whether it is printed lying (`pushfit_resolve_pose`).
    pub fn lies(&self) -> bool {
        self.pose == PinPose::Auto && self.can_lie().is_ok()
    }

    /// The pin's outline in (radius, z), from its lower end round to its
    /// upper (`pushfit_pin_profile`), each end reaching `kerf_lo` and
    /// `kerf_hi` further across a cut's thickness.
    pub fn profile(&self, kerf_lo: f64, kerf_hi: f64) -> Vec<[f64; 2]> {
        let r = 0.5 * self.diameter;
        let chamfer = self.end_chamfer.clamp(0.0, 0.5 * r);
        let (bottom, top) = (-(self.length_lower + kerf_lo), self.length_upper + kerf_hi);
        let mut points = vec![[0.0, bottom]];
        if chamfer > 0.0 {
            points.push([r - chamfer, bottom]);
            points.push([r, bottom + chamfer]);
        } else {
            points.push([r, bottom]);
        }
        let lip = |points: &mut Vec<[f64; 2]>, middle: f64| {
            let (h, half) = (self.lip_height, 0.5 * self.lip_width);
            points.push([r, middle - half]);
            points.push([r + h, middle - half + h]);
            points.push([r + h, middle + half - h]);
            points.push([r, middle + half]);
        };
        if self.lip && self.lip_height > 0.0 {
            lip(
                &mut points,
                -(kerf_lo + self.lip_position * self.length_lower),
            );
            lip(&mut points, kerf_hi + self.lip_position * self.length_upper);
        }
        if chamfer > 0.0 {
            points.push([r, top - chamfer]);
            points.push([r - chamfer, top]);
        } else {
            points.push([r, top]);
        }
        points.push([0.0, top]);
        dedupe(points)
    }

    /// A hole's outline in (radius, z) (`pushfit_hole_profile`), its mouth
    /// at `face` from the cut (the lower half's turned to below it).
    pub fn hole_profile(&self, side: PinSide, face: f64) -> Vec<[f64; 2]> {
        let r = 0.5 * self.diameter;
        let c = self.clearance(side).max(0.0);
        let hole = r + c;
        let length = self.length(side);
        let mouth = self.mouth_chamfer.max(0.0);
        let start = face - MOUTH_LEAD;
        let end = face + length + self.depth_clearance.max(0.0);
        let mut points = vec![[0.0, start]];
        if mouth > 0.0 {
            points.push([hole + mouth + MOUTH_LEAD, start]);
            points.push([hole, face + mouth]);
        } else {
            points.push([hole, start]);
        }
        if self.lip && self.hole_groove && self.lip_height > 0.0 {
            let h = self.lip_height;
            let middle = face + self.lip_position * length;
            let half = 0.5 * self.lip_width + c;
            points.push([hole, middle - half]);
            points.push([hole + h, middle - half + h]);
            points.push([hole + h, middle + half - h]);
            points.push([hole, middle + half]);
        }
        points.push([hole, end]);
        points.push([0.0, if self.pointed_end { end + hole } else { end }]);
        if side == PinSide::Lower {
            for point in &mut points {
                point[1] = -point[1];
            }
        }
        dedupe(points)
    }

    /// A hole, in the connector's frame (`pushfit_hole_mesh`).
    pub fn hole_mesh(&self, side: PinSide, face: f64) -> Mesh {
        connector::revolved(&self.hole_profile(side, face), PIN_SECTORS)
    }

    /// The pin as it is made (`pushfit_pin_mesh`): its body, its flats
    /// (cut off by planes, which the reference falls back on when its
    /// boolean fails) and its slits (a boolean). `print_pose`: lying on its
    /// flat along x, or upright, its foot at z = 0 and its middle over the
    /// origin; else in the connector's frame.
    pub fn mesh(&self, kerf_lo: f64, kerf_hi: f64, print_pose: bool) -> PinMesh {
        let r = 0.5 * self.diameter;
        let lip = if self.lip {
            self.lip_height.max(0.0)
        } else {
            0.0
        };
        // Made lying: the axis along x, the flats square to z, the slits
        // through z (`c_to_w`).
        let to_lying = Transform {
            rows: [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0; 3]],
        };
        let mut body = Mesh::default();
        body.append(
            &connector::revolved(&self.profile(kerf_lo, kerf_hi), PIN_SECTORS),
            &to_lying,
        );
        let mut mesh = body.clone();
        if self.cutout.flat() && self.flat_depth > 0.0 && self.flat_depth < r {
            let a = r - self.flat_depth;
            let keep = |mesh: &Mesh, at: f64, upper: bool| {
                let to_plane = Transform::translation([0.0, 0.0, -at]);
                let back = Transform::translation([0.0, 0.0, at]);
                let made = cut::cut(mesh, &to_plane, [0.0, 0.0], &back);
                if upper {
                    made.upper.mesh
                } else {
                    made.lower.mesh
                }
            };
            mesh = keep(&mesh, -a, true);
            if self.flat_both_sides {
                mesh = keep(&mesh, a, false);
            }
            if !sound(&mesh) {
                mesh = body.clone();
            }
        }
        let mut slot_made = false;
        if self.cutout.slot() {
            let (w, length) = (self.slot_width, self.slot_length);
            if w > 1e-3 && length > w {
                let high = self.length_upper + kerf_hi;
                let low = -(self.length_lower + kerf_lo);
                let half = r + lip + 1.0;
                let mut cutter = slit(high - length + 0.5 * w, high + 1.0, w, half);
                cutter.append(
                    &slit(low + length - 0.5 * w, low - 1.0, w, half),
                    &Transform::IDENTITY,
                );
                if let Ok(slit) = boolean::difference(&mesh, &cutter)
                    && sound(&slit)
                    && slit.signed_volume() < mesh.signed_volume() - 1e-3
                {
                    mesh = crate::curved::tidy(slit);
                    slot_made = true;
                }
            }
        }
        let lying = self.lies();
        // Back to the connector's frame, unless it is printed lying.
        let mut to_mesh = Transform::IDENTITY;
        if !print_pose || !lying {
            let back = to_lying.inverse().unwrap_or(Transform::IDENTITY);
            let mut turned = Mesh::default();
            turned.append(&mesh, &back);
            mesh = turned;
        } else {
            to_mesh = to_lying;
        }
        if print_pose && let Some((low, high)) = mesh.bounds() {
            let shift = [
                -0.5 * f64::from(low[0] + high[0]),
                -0.5 * f64::from(low[1] + high[1]),
                -f64::from(low[2]),
            ];
            let mut moved = Mesh::default();
            moved.append(&mesh, &Transform::translation(shift));
            mesh = moved;
            to_mesh = to_mesh.then(&Transform::translation(shift));
        }
        PinMesh {
            mesh,
            lying: print_pose && lying,
            slot_requested: self.cutout.slot(),
            slot_made,
            to_mesh,
        }
    }

    /// Why the pin cannot be made, when it cannot (`pushfit_validate`):
    /// the reference's words.
    pub fn invalid(&self, nozzle: f64) -> Option<&'static str> {
        let values = [
            self.diameter,
            self.length_upper,
            self.length_lower,
            self.end_chamfer,
            self.lip_height,
            self.lip_width,
            self.lip_position,
            self.flat_depth,
            self.slot_width,
            self.slot_length,
            self.clearance_upper,
            self.clearance_lower,
            self.depth_clearance,
            self.mouth_chamfer,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Some("A pin value is not a number.");
        }
        let d = self.diameter;
        if d < 3.0 {
            return Some("Pin diameter must be at least 3 mm.");
        }
        if self.length_upper < 2.0 || self.length_lower < 2.0 {
            return Some("Pin length must be at least 2 mm on each side.");
        }
        if self.end_chamfer < 0.0 || self.end_chamfer > 0.25 * d {
            return Some("End chamfer must be between 0 and a quarter of the diameter.");
        }
        if [self.clearance_upper, self.clearance_lower]
            .iter()
            .any(|c| !(0.0..=1.0).contains(c))
        {
            return Some("Clearance must be between 0 and 1 mm.");
        }
        if !(0.0..=5.0).contains(&self.depth_clearance) {
            return Some("Depth clearance must be between 0 and 5 mm.");
        }
        if self.mouth_chamfer < 0.0 || self.mouth_chamfer > 0.25 * d {
            return Some("Mouth chamfer must be between 0 and a quarter of the diameter.");
        }
        if self.lip {
            if self.lip_height <= 0.0 || self.lip_height >= 0.25 * d {
                return Some("Lip height must be above 0 and below a quarter of the diameter.");
            }
            if self.lip_width <= 2.0 * self.lip_height + 0.05 {
                return Some(
                    "Lip is too narrow for its height: make it wider than twice its height.",
                );
            }
            if self.lip_position <= 0.0 || self.lip_position >= 1.0 {
                return Some("Lip position must be between 0 and 100% of the length.");
            }
            for side in [PinSide::Upper, PinSide::Lower] {
                let length = self.length(side);
                let middle = self.lip_position * length;
                let half = 0.5 * self.lip_width;
                if middle + half > length - self.end_chamfer - 1e-3 {
                    return Some(
                        "The lip runs into the pin's end chamfer: move it towards the cut or shorten it.",
                    );
                }
                if middle - half < 1e-3 {
                    return Some("The lip reaches the cut plane: move it away from the cut.");
                }
                if self.hole_groove
                    && middle - half - self.clearance(side) < self.mouth_chamfer + 1e-3
                {
                    return Some(
                        "The lip groove runs into the hole's mouth chamfer: move the lip away from the cut.",
                    );
                }
            }
        }
        if self.cutout.flat() && (self.flat_depth <= 0.0 || self.flat_depth >= d / 3.0) {
            return Some("Flat depth must be above 0 and below a third of the diameter.");
        }
        if self.cutout.slot() {
            if self.slot_width < 0.6 {
                return Some("Slot width must be at least 0.6 mm.");
            }
            let line = 1.125 * if nozzle > 0.0 { nozzle } else { 0.4 };
            let prong = 0.5 * (d - self.slot_width)
                - if self.cutout.flat() {
                    self.flat_depth
                } else {
                    0.0
                };
            if prong < 2.0 * line - 1e-6 {
                return Some(
                    "The slot leaves the prongs too thin to print (each needs two lines): make the slot narrower, the flat shallower or the pin larger.",
                );
            }
            if self.slot_length < self.slot_width {
                return Some("Slot length must be at least the slot width.");
            }
            let most = self.length_upper.min(self.length_lower)
                - if self.lip { 0.5 * self.lip_width } else { 0.5 };
            if self.slot_length > most + 1e-4 {
                return Some("The slot is too long: it must stop short of the cut plane.");
            }
        }
        None
    }

    /// What may go wrong with a pin that can be made (`pushfit_warnings`);
    /// a thickness of 0 is not measured.
    pub fn warnings(&self, nozzle: f64, upper: f64, lower: f64) -> Vec<&'static str> {
        let mut out = Vec::new();
        let nozzle = if nozzle > 0.0 { nozzle } else { 0.4 };
        if upper > 0.0 && self.hole_depth(PinSide::Upper) + MIN_WALL > upper {
            out.push("The upper hole is deeper than the upper half is thick here: shorten the pin's upper length.");
        }
        if lower > 0.0 && self.hole_depth(PinSide::Lower) + MIN_WALL > lower {
            out.push("The lower hole is deeper than the lower half is thick here: shorten the pin's lower length.");
        }
        if self.clearance_upper.min(self.clearance_lower) < 0.125 * nozzle - 1e-6 {
            out.push(
                "The clearance is very small for this nozzle: the pin will need force to go in.",
            );
        }
        if self.lip && !self.cutout.slot() && !self.hole_groove {
            let interference = self.lip_height - self.clearance_upper.min(self.clearance_lower);
            if interference > 0.15 + 1e-5 {
                out.push("The lip will need force without a slot or a groove: add a slot or a groove, or lower the lip.");
            }
        }
        out
    }

    /// Each value brought within its bounds (`pushfit_sanitize`).
    pub fn sanitized(mut self) -> Pin {
        let start = Pin::default();
        let clamp = |value: f64, low: f64, high: f64, otherwise: f64| {
            if value.is_finite() {
                value.clamp(low, high.max(low))
            } else {
                otherwise
            }
        };
        self.diameter = clamp(self.diameter, 3.0, 50.0, start.diameter);
        let d = self.diameter;
        self.length_upper = clamp(self.length_upper, 2.0, 200.0, start.length_upper);
        self.length_lower = clamp(self.length_lower, 2.0, 200.0, start.length_lower);
        self.end_chamfer = clamp(self.end_chamfer, 0.0, 0.25 * d, start.end_chamfer);
        self.lip_height = clamp(self.lip_height, 0.0, 0.249 * d, start.lip_height);
        self.lip_width = clamp(self.lip_width, 0.0, 50.0, start.lip_width);
        self.lip_position = clamp(self.lip_position, 0.0, 1.0, start.lip_position);
        self.flat_depth = clamp(self.flat_depth, 0.0, 0.333 * d, start.flat_depth);
        self.slot_width = clamp(self.slot_width, 0.0, d, start.slot_width);
        self.slot_length = clamp(self.slot_length, 0.0, 200.0, start.slot_length);
        self.clearance_upper = clamp(self.clearance_upper, 0.0, 1.0, start.clearance_upper);
        self.clearance_lower = clamp(self.clearance_lower, 0.0, 1.0, start.clearance_lower);
        self.depth_clearance = clamp(self.depth_clearance, 0.0, 5.0, start.depth_clearance);
        self.mouth_chamfer = clamp(self.mouth_chamfer, 0.0, 0.25 * d, start.mouth_chamfer);
        if self.preset > 3 {
            self.preset = start.preset;
        }
        self
    }

    /// The pin as the reference writes it on a connector
    /// (`pushfit_attributes`).
    pub fn attributes(&self) -> Vec<(String, f64)> {
        let bit = |value: bool| if value { 1.0 } else { 0.0 };
        vec![
            ("pushfit_diameter".into(), self.diameter),
            ("pushfit_length_upper".into(), self.length_upper),
            ("pushfit_length_lower".into(), self.length_lower),
            ("pushfit_end_chamfer".into(), self.end_chamfer),
            ("pushfit_lip_height".into(), self.lip_height),
            ("pushfit_lip_width".into(), self.lip_width),
            ("pushfit_lip_position".into(), self.lip_position),
            ("pushfit_flat_depth".into(), self.flat_depth),
            ("pushfit_slot_width".into(), self.slot_width),
            ("pushfit_slot_length".into(), self.slot_length),
            ("pushfit_clearance_upper".into(), self.clearance_upper),
            ("pushfit_clearance_lower".into(), self.clearance_lower),
            ("pushfit_depth_clearance".into(), self.depth_clearance),
            ("pushfit_mouth_chamfer".into(), self.mouth_chamfer),
            ("pushfit_lip".into(), bit(self.lip)),
            ("pushfit_flat_both_sides".into(), bit(self.flat_both_sides)),
            ("pushfit_hole_groove".into(), bit(self.hole_groove)),
            ("pushfit_pointed_end".into(), bit(self.pointed_end)),
            ("pushfit_cutout".into(), f64::from(self.cutout.code())),
            (
                "pushfit_pose".into(),
                if self.pose == PinPose::Upright {
                    1.0
                } else {
                    0.0
                },
            ),
            ("pushfit_preset".into(), f64::from(self.preset)),
        ]
    }

    /// A pin from what [`Pin::attributes`] writes, each one missing at its
    /// default (`pushfit_from_attributes`).
    pub fn from_attributes(get: impl Fn(&str) -> Option<f64>) -> Pin {
        let mut pin = Pin::default();
        let float = |name: &str, into: &mut f64| {
            if let Some(value) = get(name) {
                *into = value;
            }
        };
        float("pushfit_diameter", &mut pin.diameter);
        float("pushfit_length_upper", &mut pin.length_upper);
        float("pushfit_length_lower", &mut pin.length_lower);
        float("pushfit_end_chamfer", &mut pin.end_chamfer);
        float("pushfit_lip_height", &mut pin.lip_height);
        float("pushfit_lip_width", &mut pin.lip_width);
        float("pushfit_lip_position", &mut pin.lip_position);
        float("pushfit_flat_depth", &mut pin.flat_depth);
        float("pushfit_slot_width", &mut pin.slot_width);
        float("pushfit_slot_length", &mut pin.slot_length);
        float("pushfit_clearance_upper", &mut pin.clearance_upper);
        float("pushfit_clearance_lower", &mut pin.clearance_lower);
        float("pushfit_depth_clearance", &mut pin.depth_clearance);
        float("pushfit_mouth_chamfer", &mut pin.mouth_chamfer);
        let flag = |name: &str, into: &mut bool| {
            if let Some(value) = get(name).filter(|value| value.is_finite()) {
                *into = value != 0.0;
            }
        };
        flag("pushfit_lip", &mut pin.lip);
        flag("pushfit_flat_both_sides", &mut pin.flat_both_sides);
        flag("pushfit_hole_groove", &mut pin.hole_groove);
        flag("pushfit_pointed_end", &mut pin.pointed_end);
        let whole = |name: &str| {
            get(name)
                .filter(|v| v.is_finite() && v.abs() < 1e6)
                .map(|v| v.round())
        };
        if let Some(cutout) = whole("pushfit_cutout").and_then(|v| PinCutout::of(v.max(0.0) as u32))
        {
            pin.cutout = cutout;
        }
        if let Some(pose) = whole("pushfit_pose") {
            pin.pose = if pose == 1.0 {
                PinPose::Upright
            } else {
                PinPose::Auto
            };
        }
        if let Some(preset) = whole("pushfit_preset") {
            pin.preset = preset.max(0.0) as u32;
        }
        pin.sanitized()
    }
}

/// A pin as made.
#[derive(Clone, Debug)]
pub struct PinMesh {
    pub mesh: Mesh,
    /// Printed lying on its flat.
    pub lying: bool,
    /// A slit was asked for, and whether the boolean made it.
    pub slot_requested: bool,
    pub slot_made: bool,
    /// Takes the connector's frame to the mesh's.
    pub to_mesh: Transform,
}

/// One end's slit: a stadium from `round` (the middle of its round end)
/// out to `out` along x, `width` wide, through z (`slit_prism`).
fn slit(round: f64, out: f64, width: f64, half_height: f64) -> Mesh {
    const ARC: usize = 16;
    let half = 0.5 * width;
    let mut outline = Vec::new();
    let (from, sign) = if out > round {
        outline.push([out, -half]);
        outline.push([out, half]);
        (0.5 * std::f64::consts::PI, 1.0)
    } else {
        outline.push([out, half]);
        outline.push([out, -half]);
        (-0.5 * std::f64::consts::PI, 1.0)
    };
    for k in 0..=ARC {
        let a = from + sign * std::f64::consts::PI * k as f64 / ARC as f64;
        outline.push([round + half * a.cos(), half * a.sin()]);
    }
    let outline = dedupe(outline);
    connector::extruded(&outline, -half_height, half_height)
}

/// Points closer than a micron to the one before made one, and the last
/// dropped where it meets the first (`dedupe`).
fn dedupe(points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) <= 1e-6;
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for point in points {
        if out.last().is_none_or(|last| !close(*last, point)) {
            out.push(point);
        }
    }
    while out.len() > 1 && close(out[0], out[out.len() - 1]) {
        out.pop();
    }
    out
}

/// Closed, wound outward.
fn sound(mesh: &Mesh) -> bool {
    !mesh.triangles.is_empty() && cut::is_closed(mesh) && mesh.signed_volume() > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(mesh: &Mesh) {
        assert!(sound(mesh), "closed and wound outward");
        assert_eq!(cut::flat_triangles(mesh), 0);
    }

    #[test]
    fn every_pin_and_hole_is_closed_and_the_slit_and_flats_are_made() {
        for preset in 0..3 {
            for cutout in [
                PinCutout::None,
                PinCutout::Flat,
                PinCutout::Slot,
                PinCutout::FlatAndSlot,
            ] {
                for groove in [false, true] {
                    let pin = Pin {
                        cutout,
                        hole_groove: groove,
                        ..Pin::default()
                    }
                    .with_preset(preset);
                    assert_eq!(pin.invalid(0.4), None, "{preset} {cutout:?}");
                    let made = pin.mesh(0.0, 0.0, true);
                    closed(&made.mesh);
                    assert_eq!(made.slot_made, cutout.slot(), "{preset} {cutout:?}");
                    // Lying on its flat, foot at the bed.
                    let (low, high) = made.mesh.bounds().unwrap();
                    assert!(low[2].abs() < 1e-5);
                    if made.lying {
                        assert!(
                            (f64::from(high[2] - low[2]) - (pin.diameter - 2.0 * pin.flat_depth))
                                .abs()
                                < 1e-3
                        );
                    }
                    for side in [PinSide::Upper, PinSide::Lower] {
                        closed(&pin.hole_mesh(side, 0.0));
                    }
                }
            }
        }
        // The slit takes away what it should: two stadiums through the pin.
        let pin = Pin::default();
        let plain = Pin {
            cutout: PinCutout::Flat,
            ..pin
        }
        .mesh(0.0, 0.0, false)
        .mesh;
        let slit = pin.mesh(0.0, 0.0, false).mesh;
        assert!(slit.signed_volume() < plain.signed_volume() - 1.0);
        // The hole: wider than the pin by the clearance, deeper by the depth
        // clearance and its cone.
        let hole = pin.hole_mesh(PinSide::Upper, 0.0);
        let (low, high) = hole.bounds().unwrap();
        assert!((f64::from(high[2]) - (7.0 + 0.4 + 3.1)).abs() < 1e-4);
        assert!((f64::from(low[2]) + 0.2).abs() < 1e-5);
        let lower = pin.hole_mesh(PinSide::Lower, 0.0);
        assert!(lower.bounds().unwrap().0[2] < -10.0);
    }

    #[test]
    fn presets_auto_size_validation_and_attributes() {
        let pin = Pin::default();
        let (fitted, fits) = pin.auto_size(14.0, 0.0, 0.0);
        assert!(fits);
        assert_eq!(fitted.preset, 1);
        let (small, fits) = pin.auto_size(2.0, 0.0, 0.0);
        assert!(!fits);
        assert_eq!(small.preset, 0);
        assert!(pin.can_lie().is_ok() && pin.lies());
        let shallow = Pin {
            flat_depth: 0.3,
            ..pin
        };
        assert!(shallow.can_lie().is_err());
        assert_eq!(
            Pin {
                diameter: 2.0,
                ..pin
            }
            .invalid(0.4),
            Some("Pin diameter must be at least 3 mm.")
        );
        let back = Pin::from_attributes(|name| {
            pin.attributes()
                .into_iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value)
        });
        assert_eq!(back, pin);
    }
}
