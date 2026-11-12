//! The Flexi joint (the reference fork's own: FlexiJoint.cpp,
//! FlexiJoint.hpp): a "print in place" joint made at a planar cut. Six
//! kinds: a double ring (an annular dovetail lip in a channel), a ball and
//! socket, a chain link (two interlocked loops), a pin hinge, a screw
//! thread and a bayonet lock.
//!
//! Everything here is in the CUT FRAME: the cut plane is z = 0, the
//! joint's axis is +z, the lower ("male") half is below the plane and the
//! upper ("female") half above it. The two halves' cut faces are set back
//! from the plane by half the gap each, so the lower half ends at
//! z = -gap/2 and the upper one starts at z = +gap/2; the joint's bodies
//! bridge that gap. A cut puts each half's bodies in ([`lower_bodies`],
//! [`upper_bodies`]) and then takes each half's reliefs out
//! ([`lower_reliefs`], [`upper_reliefs`]). Every mesh is a closed solid,
//! wound outward, in that frame. Lengths are in mm, angles in degrees.
//!
//! The clearance is exact by construction, as in the reference: the
//! revolved kinds' cavity is the revolution of the male section offset by
//! the clearance; the chain link's relief is the other loop's tube grown
//! by it; the hinge's and the bayonet's are the same cylinders and
//! sectors with bigger numbers; the thread's groove is the same helix
//! swept with the profile grown by it. Male and female use the same
//! facet counts and phases, so their facets stay parallel.
//!
//! The validation messages are the reference's own English sentences; it
//! returns them untranslated, and its gizmo shows them as they are
//! (GLGizmoCut.cpp, `flexi_validate` is not wrapped in `_L`).

use edge_clip::Join;
use edge_geom::{ExPolygon, Point2, Polygon, scaled, unscaled};
use edge_mesh::{Mesh, connector};
use std::f64::consts::{PI, TAU};

/// libslic3r's `EPSILON`.
const EPSILON: f64 = 1e-4;
/// Facets round every revolved body and cylinder (`FLEXI_SECTORS`); also
/// the thread's steps per turn.
pub const SECTORS: usize = 120;
/// Steps along the ball's spherical arc (`FLEXI_ARC_STEPS`).
const ARC_STEPS: usize = 32;
/// Facets round the chain link's wire (`FLEXI_WIRE_SECTORS`), and steps
/// per half circle of its centreline's ends (`FLEXI_CAP_STEPS`).
const WIRE_SECTORS: usize = 32;
const CAP_STEPS: usize = 24;

/// The joint's kind (`FlexiJointKind`), with the reference's codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexiKind {
    DoubleRing,
    BallSocket,
    ChainLink,
    Hinge,
    Thread,
    Bayonet,
}

impl FlexiKind {
    pub const ALL: [FlexiKind; 6] = [
        Self::DoubleRing,
        Self::BallSocket,
        Self::ChainLink,
        Self::Hinge,
        Self::Thread,
        Self::Bayonet,
    ];

    pub fn code(self) -> i32 {
        self as i32
    }

    pub fn of(code: i32) -> Option<Self> {
        usize::try_from(code)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }

    /// A twist lock: a lid screwed or turned onto a body about the cut's
    /// normal (`flexi_kind_is_twist`).
    pub fn is_twist(self) -> bool {
        matches!(self, Self::Thread | Self::Bayonet)
    }
}

/// A joint's shape (`FlexiJointParams`). Which fields a kind reads is
/// said on each; the defaults are the reference's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlexiParams {
    pub kind: FlexiKind,
    /// Double ring: the lip ring's outer radius. Ball and socket: the
    /// ball's radius.
    pub outer_radius: f64,
    /// Double ring: the lip's radial width at its widest, and how far it
    /// stands above the face.
    pub ring_width: f64,
    pub ring_height: f64,
    /// Every kind: the gap between each male face and its female one.
    pub clearance: f64,
    /// Every kind: how far apart the two cut faces end up; 0 is the
    /// kind's default ([`effective_gap`]).
    pub gap: f64,
    /// Double ring: the solid hub's radius; 0.2 or less, none.
    pub hub_radius: f64,
    /// Revolved kinds: headroom in the cavity's ceiling, to rock.
    pub tilt: f64,
    /// Double ring: the stem's width as a fraction of `ring_width`.
    pub neck_ratio: f64,
    /// Ball and socket: the socket mouth's half angle.
    pub open_angle: f64,
    /// Chain link: each loop's centreline length and width, the wire's
    /// radius, the upper ring's tilt in its own plane and how deep each
    /// loop's far end sits in its own half.
    pub link_length: f64,
    pub link_width: f64,
    pub wire: f64,
    pub tilt_angle: f64,
    pub stem: f64,
    /// Hinge: knuckles (1 to 9), the pin's and the barrel's diameters,
    /// the run's length, how far out along -y the barrel sits, and
    /// whether the pin and the even knuckles are the upper half's.
    pub hinge_knuckles: i32,
    pub hinge_pin_dia: f64,
    pub hinge_barrel_dia: f64,
    pub hinge_length: f64,
    pub hinge_edge_offset: f64,
    pub hinge_fold_upper: bool,
    /// Thread and bayonet: the thread's major diameter (the lug circle's,
    /// for a bayonet), and whether the upper half is the lid.
    pub thread_major_dia: f64,
    pub thread_lid_upper: bool,
    /// Thread: rise per turn, starts (1 to 4), turns of engagement, hand,
    /// and the lead-in's length in turns at each end. A bayonet reads the
    /// hand for its turning sense.
    pub thread_pitch: f64,
    pub thread_starts: i32,
    pub thread_turns: f64,
    pub thread_left_hand: bool,
    pub thread_lead_turns: f64,
    /// Bayonet: lugs (2 to 4), a lug's radial height, axial thickness and
    /// width in degrees, the turn to lock, the entry channel's depth and
    /// the detent's height.
    pub bayonet_lugs: i32,
    pub bayonet_lug_height: f64,
    pub bayonet_lug_thickness: f64,
    pub bayonet_lug_arc: f64,
    pub bayonet_lock_angle: f64,
    pub bayonet_entry_depth: f64,
    pub bayonet_detent: f64,
    /// The joint turned about the cut's normal, in degrees; the revolved
    /// kinds ignore it. For a thread it is the start angle.
    pub rotation: f64,
}

impl Default for FlexiParams {
    fn default() -> Self {
        Self {
            kind: FlexiKind::DoubleRing,
            outer_radius: 4.0,
            ring_width: 2.0,
            ring_height: 1.5,
            clearance: 0.35,
            gap: 0.0,
            hub_radius: 0.0,
            tilt: 0.20,
            neck_ratio: 0.45,
            open_angle: 40.0,
            link_length: 7.0,
            link_width: 5.5,
            wire: 1.0,
            tilt_angle: 7.0,
            stem: 1.5,
            hinge_knuckles: 3,
            hinge_pin_dia: 2.0,
            hinge_barrel_dia: 4.0,
            hinge_length: 12.0,
            hinge_edge_offset: 0.0,
            hinge_fold_upper: false,
            thread_major_dia: 12.0,
            thread_lid_upper: true,
            thread_pitch: 6.0,
            thread_starts: 2,
            thread_turns: 1.25,
            thread_left_hand: false,
            thread_lead_turns: 0.5,
            bayonet_lugs: 3,
            bayonet_lug_height: 1.6,
            bayonet_lug_thickness: 2.4,
            bayonet_lug_arc: 30.0,
            bayonet_lock_angle: 75.0,
            bayonet_entry_depth: 4.0,
            bayonet_detent: 0.35,
            rotation: 0.0,
        }
    }
}

// ------------------------------------------------------------- 3MF attributes

/// The joint as the reference writes it on a connector of
/// `Metadata/cut_information.xml` (bbs_3mf.cpp, the `flexi_*` attributes),
/// in its order. Counts and flags are whole numbers.
pub fn attributes(p: &FlexiParams) -> Vec<(String, f64)> {
    let bit = |value: bool| if value { 1.0 } else { 0.0 };
    [
        ("flexi_kind", f64::from(p.kind.code())),
        ("flexi_outer_radius", p.outer_radius),
        ("flexi_ring_width", p.ring_width),
        ("flexi_ring_height", p.ring_height),
        ("flexi_clearance", p.clearance),
        ("flexi_gap", p.gap),
        ("flexi_hub_radius", p.hub_radius),
        ("flexi_tilt", p.tilt),
        ("flexi_neck_ratio", p.neck_ratio),
        ("flexi_open_angle", p.open_angle),
        ("flexi_link_length", p.link_length),
        ("flexi_link_width", p.link_width),
        ("flexi_wire", p.wire),
        ("flexi_tilt_angle", p.tilt_angle),
        ("flexi_stem", p.stem),
        ("flexi_rotation", p.rotation),
        ("flexi_hinge_knuckles", f64::from(p.hinge_knuckles)),
        ("flexi_hinge_pin_dia", p.hinge_pin_dia),
        ("flexi_hinge_barrel_dia", p.hinge_barrel_dia),
        ("flexi_hinge_length", p.hinge_length),
        ("flexi_hinge_edge_offset", p.hinge_edge_offset),
        ("flexi_hinge_fold_upper", bit(p.hinge_fold_upper)),
        ("flexi_thread_major_dia", p.thread_major_dia),
        ("flexi_thread_lid_upper", bit(p.thread_lid_upper)),
        ("flexi_thread_pitch", p.thread_pitch),
        ("flexi_thread_starts", f64::from(p.thread_starts)),
        ("flexi_thread_turns", p.thread_turns),
        ("flexi_thread_left_hand", bit(p.thread_left_hand)),
        ("flexi_thread_lead_turns", p.thread_lead_turns),
        ("flexi_bayonet_lugs", f64::from(p.bayonet_lugs)),
        ("flexi_bayonet_lug_height", p.bayonet_lug_height),
        ("flexi_bayonet_lug_thickness", p.bayonet_lug_thickness),
        ("flexi_bayonet_lug_arc", p.bayonet_lug_arc),
        ("flexi_bayonet_lock_angle", p.bayonet_lock_angle),
        ("flexi_bayonet_entry_depth", p.bayonet_entry_depth),
        ("flexi_bayonet_detent", p.bayonet_detent),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value))
    .collect()
}

/// A joint from what [`attributes`] writes, as the reference's reader
/// takes it (bbs_3mf.cpp, the connector's `flexi_*` attributes): each one
/// missing, or not a number of the right sort, at its default; a kind out
/// of range at the default kind; knuckles clamped to 1..9, starts to 1..4
/// and lugs to 2..4. The reader only reads a connector's flexi
/// attributes when it has `flexi_kind`; that test is the caller's.
///
/// The reference holds these values in single precision, so each is
/// taken at the single-precision number's shortest decimal: what it
/// writes as 0.349999994 reads back as 0.35. A value that is not finite
/// is taken as missing (the reference would parse it and build nothing
/// sensible from it).
pub fn from_attributes(get: impl Fn(&str) -> Option<f64>) -> FlexiParams {
    let def = FlexiParams::default();
    let float = |name: &str, default: f64| {
        get(name)
            .filter(|v| v.is_finite())
            .map(single)
            .unwrap_or(default)
    };
    // `get<int>`: a value that is not a whole number is not read.
    let whole = |name: &str, default: i32| {
        get(name)
            .filter(|v| v.is_finite() && v.fract() == 0.0 && v.abs() <= f64::from(i32::MAX))
            .map(|v| v as i32)
            .unwrap_or(default)
    };
    let flag = |name: &str, default: bool| whole(name, i32::from(default)) != 0;
    FlexiParams {
        kind: FlexiKind::of(whole("flexi_kind", def.kind.code())).unwrap_or(def.kind),
        outer_radius: float("flexi_outer_radius", def.outer_radius),
        ring_width: float("flexi_ring_width", def.ring_width),
        ring_height: float("flexi_ring_height", def.ring_height),
        clearance: float("flexi_clearance", def.clearance),
        gap: float("flexi_gap", def.gap),
        hub_radius: float("flexi_hub_radius", def.hub_radius),
        tilt: float("flexi_tilt", def.tilt),
        neck_ratio: float("flexi_neck_ratio", def.neck_ratio),
        open_angle: float("flexi_open_angle", def.open_angle),
        link_length: float("flexi_link_length", def.link_length),
        link_width: float("flexi_link_width", def.link_width),
        wire: float("flexi_wire", def.wire),
        tilt_angle: float("flexi_tilt_angle", def.tilt_angle),
        stem: float("flexi_stem", def.stem),
        rotation: float("flexi_rotation", def.rotation),
        hinge_knuckles: whole("flexi_hinge_knuckles", def.hinge_knuckles).clamp(1, 9),
        hinge_pin_dia: float("flexi_hinge_pin_dia", def.hinge_pin_dia),
        hinge_barrel_dia: float("flexi_hinge_barrel_dia", def.hinge_barrel_dia),
        hinge_length: float("flexi_hinge_length", def.hinge_length),
        hinge_edge_offset: float("flexi_hinge_edge_offset", def.hinge_edge_offset),
        hinge_fold_upper: flag("flexi_hinge_fold_upper", def.hinge_fold_upper),
        thread_major_dia: float("flexi_thread_major_dia", def.thread_major_dia),
        thread_lid_upper: flag("flexi_thread_lid_upper", def.thread_lid_upper),
        thread_pitch: float("flexi_thread_pitch", def.thread_pitch),
        thread_starts: whole("flexi_thread_starts", def.thread_starts).clamp(1, 4),
        thread_turns: float("flexi_thread_turns", def.thread_turns),
        thread_left_hand: flag("flexi_thread_left_hand", def.thread_left_hand),
        thread_lead_turns: float("flexi_thread_lead_turns", def.thread_lead_turns),
        bayonet_lugs: whole("flexi_bayonet_lugs", def.bayonet_lugs).clamp(2, 4),
        bayonet_lug_height: float("flexi_bayonet_lug_height", def.bayonet_lug_height),
        bayonet_lug_thickness: float("flexi_bayonet_lug_thickness", def.bayonet_lug_thickness),
        bayonet_lug_arc: float("flexi_bayonet_lug_arc", def.bayonet_lug_arc),
        bayonet_lock_angle: float("flexi_bayonet_lock_angle", def.bayonet_lock_angle),
        bayonet_entry_depth: float("flexi_bayonet_entry_depth", def.bayonet_entry_depth),
        bayonet_detent: float("flexi_bayonet_detent", def.bayonet_detent),
    }
}

/// A value at its single-precision number's shortest decimal.
fn single(value: f64) -> f64 {
    (value as f32).to_string().parse().unwrap_or(value)
}

// ------------------------------------------------------------------ defaults

/// The hub's default radius (`flexi_default_hub_radius`): what is left
/// inside the lip and its channel, none when that is 0.2 or less.
pub fn default_hub_radius(p: &FlexiParams) -> f64 {
    let r = p.outer_radius - p.ring_width - p.clearance - p.ring_width;
    if r > 0.2 { r } else { 0.0 }
}

/// The kind's gap (`flexi_default_gap`): 0.4 for a twist lock (the faces
/// meet when it is done up), 1.5 for the chain link (its loops swing
/// through each other), 0.6 for the rest.
pub fn default_gap(kind: FlexiKind) -> f64 {
    if kind.is_twist() {
        0.4
    } else if kind == FlexiKind::ChainLink {
        1.5
    } else {
        0.6
    }
}

/// The gap used (`flexi_effective_gap`): the set one or the kind's, never
/// below the clearance.
pub fn effective_gap(p: &FlexiParams) -> f64 {
    let g = if p.gap > 0.0 {
        p.gap
    } else {
        default_gap(p.kind)
    };
    g.max(p.clearance)
}

/// The least clearance a nozzle leaves open (`flexi_clearance_floor`):
/// 0.75 of its diameter, 0.2 for none.
pub fn clearance_floor(nozzle_diameter: f64) -> f64 {
    if nozzle_diameter <= 0.0 {
        0.2
    } else {
        0.75 * nozzle_diameter
    }
}

/// The joint sized to a cut section whose inscribed radius is given
/// (`flexi_auto_size`); a radius of 0 or less leaves the sizes alone.
pub fn auto_size(mut p: FlexiParams, inscribed_radius: f64) -> FlexiParams {
    if inscribed_radius > 0.0 {
        p.outer_radius = (0.4 * inscribed_radius).max(1.2);
        let max_r = inscribed_radius - p.clearance - 0.4;
        if max_r > 1.0 && p.outer_radius > max_r {
            p.outer_radius = max_r;
        }
    }
    if p.kind == FlexiKind::DoubleRing {
        p.ring_width = p.ring_width.min(0.8 * p.outer_radius);
        p.hub_radius = default_hub_radius(&p);
    } else if p.kind == FlexiKind::Hinge && inscribed_radius > 0.0 {
        p.hinge_length = (1.6 * inscribed_radius).clamp(2.0, 60.0);
        let mut barrel = (0.35 * inscribed_radius).clamp(2.0, 10.0);
        let pin = (0.45 * barrel).max(1.0);
        let wall = 0.8;
        if barrel < pin + 2.0 * p.clearance + 2.0 * wall {
            barrel = pin + 2.0 * p.clearance + 2.0 * wall;
        }
        p.hinge_barrel_dia = barrel;
        p.hinge_pin_dia = pin;
    } else if p.kind.is_twist() && inscribed_radius > 0.0 {
        let room = 2.0 * (inscribed_radius - p.clearance - thread_bore_wall() - 0.4);
        let major = (1.4 * inscribed_radius).min(room.max(3.0));
        p.thread_major_dia = major.max(4.0);
        if p.kind == FlexiKind::Thread {
            // Sized by the crest spacing, so the pitch carries the starts.
            let starts = f64::from(thread_start_count(&p));
            let spacing = (0.25 * p.thread_major_dia).clamp(2.0, 6.0);
            p.thread_pitch = spacing * starts;
        } else {
            p.bayonet_lug_height = (0.14 * p.thread_major_dia).clamp(1.0, 3.0);
            p.bayonet_lug_thickness = (0.20 * p.thread_major_dia).clamp(1.6, 4.0);
        }
    } else if p.kind == FlexiKind::ChainLink && inscribed_radius > 0.0 {
        // The loops scaled together, so the pair fits with a wall to spare.
        let reference = FlexiParams::default();
        let want = (inscribed_radius - p.clearance - 0.4).max(1.0);
        let probe = FlexiParams {
            link_length: reference.link_length,
            link_width: reference.link_width,
            wire: reference.wire,
            stem: reference.stem,
            ..p
        };
        let have = outer_extent(&probe);
        let s = if have > EPSILON { want / have } else { 1.0 };
        let s = s.clamp(0.45, 2.5);
        p.link_length = s * reference.link_length;
        p.link_width = s * reference.link_width;
        p.wire = s * reference.wire;
        p.stem = s * reference.stem;
        let need = min_link_size(&p);
        p.link_length = p.link_length.max(need);
        p.link_width = p.link_width.max(need);
    }
    p
}

// -------------------------------------------------------------- key measures

/// How far the male body reaches down into its half (`flexi_anchor_depth`).
fn anchor_depth(p: &FlexiParams) -> f64 {
    let h = if p.kind == FlexiKind::DoubleRing {
        p.ring_height
    } else {
        ball_centre_z(p) + p.outer_radius
    };
    (0.4 * h).max(0.6)
}

/// How far the revolved kinds' profiles are lifted, so the stem starts at
/// the upper face (`flexi_revolved_z_shift`).
fn revolved_z_shift(p: &FlexiParams) -> f64 {
    0.5 * effective_gap(p)
}

/// How far the joint stands above the cut plane (`flexi_protrusion_height`).
pub fn protrusion_height(p: &FlexiParams) -> f64 {
    if p.kind.is_twist() {
        let plug = if p.kind == FlexiKind::Thread {
            thread_plug_length(p)
        } else {
            bayonet_plug_length(p)
        };
        return plug + 0.5 * effective_gap(p);
    }
    match p.kind {
        FlexiKind::Hinge => 0.5 * p.hinge_barrel_dia,
        FlexiKind::ChainLink => {
            let f = chain_frame(p);
            let upper_top = f.ucz + 0.5 * f.l * f.tilt.cos() + 0.5 * f.w * f.tilt.sin();
            f.lower_top.max(upper_top) + p.wire
        }
        FlexiKind::DoubleRing => p.ring_height + revolved_z_shift(p),
        _ => ball_centre_z(p) + p.outer_radius + revolved_z_shift(p),
    }
}

/// The least loop length and width that passes the other loop's wire with
/// the clearance each side (`flexi_min_link_size`).
pub fn min_link_size(p: &FlexiParams) -> f64 {
    4.0 * p.wire + 2.0 * p.clearance
}

/// The furthest the joint reaches from its axis, clearance included
/// (`flexi_outer_extent`).
pub fn outer_extent(p: &FlexiParams) -> f64 {
    if p.kind.is_twist() {
        return thread_bore_radius(p) + thread_bore_wall();
    }
    match p.kind {
        FlexiKind::Hinge => hinge_footprint_corners(p, p.clearance)
            .iter()
            .map(|c| c[0].hypot(c[1]))
            .fold(0.0, f64::max),
        FlexiKind::ChainLink => {
            let far = chain_lower_path(p)
                .iter()
                .chain(&chain_upper_path(p))
                .map(|v| v[0].hypot(v[1]))
                .fold(0.0, f64::max);
            far + p.wire + p.clearance
        }
        _ => p.outer_radius + p.clearance,
    }
}

/// The female mouth's half width (`flexi_mouth_half_width`): the ring's
/// channel at the face, the socket's mouth, the hole a loop's wire passes,
/// the hinge's bore, a twist lock's minor radius.
pub fn mouth_half_width(p: &FlexiParams) -> f64 {
    match p.kind {
        FlexiKind::Thread | FlexiKind::Bayonet => thread_minor_radius(p),
        FlexiKind::Hinge => hinge_bore_radius(p),
        FlexiKind::ChainLink => 2.0 * p.wire + 2.0 * p.clearance,
        FlexiKind::DoubleRing => 0.5 * p.ring_width * p.neck_ratio + p.clearance,
        FlexiKind::BallSocket => ball_neck_radius(p) + p.clearance,
    }
}

/// The male lip's half width (`flexi_lip_half_width`).
pub fn lip_half_width(p: &FlexiParams) -> f64 {
    match p.kind {
        FlexiKind::Thread | FlexiKind::Bayonet => thread_major_radius(p),
        FlexiKind::Hinge => 0.5 * p.hinge_barrel_dia,
        FlexiKind::ChainLink => p.link_length.min(p.link_width) - 2.0 * p.wire,
        FlexiKind::DoubleRing => 0.5 * p.ring_width,
        FlexiKind::BallSocket => p.outer_radius,
    }
}

/// Whether the mouth is narrower than the lip (`flexi_is_captive`).
pub fn is_captive(p: &FlexiParams) -> bool {
    mouth_half_width(p) < lip_half_width(p)
}

/// Whether both halves get a body of their own rather than a body and a
/// cavity (`flexi_is_two_sided`).
pub fn is_two_sided(p: &FlexiParams) -> bool {
    matches!(p.kind, FlexiKind::ChainLink | FlexiKind::Hinge) || p.kind.is_twist()
}

/// The ball and socket's neck radius (`flexi_ball_neck_radius`).
fn ball_neck_radius(p: &FlexiParams) -> f64 {
    let (c, rb) = (p.clearance, p.outer_radius);
    ((rb + c) * p.open_angle.to_radians().sin() - c)
        .max(0.25 * rb)
        .min(0.85 * rb)
}

/// The ball's centre above the female face (`flexi_ball_centre_z`).
fn ball_centre_z(p: &FlexiParams) -> f64 {
    let (c, rb) = (p.clearance, p.outer_radius);
    c + (rb + c) * p.open_angle.to_radians().cos() + 0.2
}

// ---------------------------------------------------------------- chain link

/// The chain link's numbers (`ChainLinkFrame`): each ring's centre `off`
/// from the plane on its own side, clamped between clearing its own face
/// (and embedding `stem` deep) and still threading the other ring.
#[derive(Clone, Copy, Debug)]
struct ChainFrame {
    l: f64,
    w: f64,
    off_min: f64,
    off_max: f64,
    ucz: f64,
    lcz: f64,
    /// The upper ring's tilt, radians.
    tilt: f64,
    lower_top: f64,
    /// How far the upper ring's free end reaches below the top of the
    /// lower ring's hole; a link only while positive.
    interlock: f64,
}

fn chain_frame(p: &FlexiParams) -> ChainFrame {
    let (l, w, t) = (p.link_length, p.link_width, p.wire);
    let gap = effective_gap(p);
    let off_min = (0.5 * gap + t).max(p.stem + 0.5 * gap - 0.5 * l);
    let off_max = 0.5 * (l - t) - p.clearance;
    let off = (0.25 * l).max(off_min).min(off_min.max(off_max));
    let (lcz, ucz) = (-off, off);
    let lower_top = lcz + 0.5 * l;
    let upper_bottom = ucz - 0.5 * l;
    ChainFrame {
        l,
        w,
        off_min,
        off_max,
        ucz,
        lcz,
        tilt: p.tilt_angle.to_radians(),
        lower_top,
        interlock: (lower_top - t) - upper_bottom,
    }
}

/// A closed stadium centreline of length `l` and width `w` in (a, b), `a`
/// the long axis (`stadium_path`).
fn stadium_path(l: f64, w: f64, cap_steps: usize) -> Vec<[f64; 2]> {
    let r = 0.5 * w;
    let straight = (l - w).max(EPSILON);
    let (a0, a1) = (-0.5 * straight, 0.5 * straight);
    let mut pts = vec![[a0, -r], [a1, -r]];
    for i in 1..cap_steps {
        let th = PI * (-0.5 + i as f64 / cap_steps as f64);
        pts.push([a1 + r * th.cos(), r * th.sin()]);
    }
    pts.push([a1, r]);
    pts.push([a0, r]);
    for i in 1..cap_steps {
        let th = PI * (0.5 + i as f64 / cap_steps as f64);
        pts.push([a0 + r * th.cos(), r * th.sin()]);
    }
    pts
}

/// Points turned by the joint's rotation about +z (`chain_apply_rotation`).
fn apply_rotation(path: &mut [[f64; 3]], p: &FlexiParams) {
    let a = p.rotation.to_radians();
    if a.abs() <= EPSILON {
        return;
    }
    let (s, c) = a.sin_cos();
    for v in path {
        *v = [c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]];
    }
}

/// The lower ring's centreline, in the x-z plane (`chain_lower_path`).
fn chain_lower_path(p: &FlexiParams) -> Vec<[f64; 3]> {
    let f = chain_frame(p);
    let mut out: Vec<[f64; 3]> = stadium_path(f.l, f.w, CAP_STEPS)
        .into_iter()
        .map(|ab| [ab[1], 0.0, f.lcz + ab[0]])
        .collect();
    apply_rotation(&mut out, p);
    out
}

/// The upper ring's centreline, in the y-z plane, tipped by the tilt
/// about +x inside that plane (`chain_upper_path`).
fn chain_upper_path(p: &FlexiParams) -> Vec<[f64; 3]> {
    let f = chain_frame(p);
    let (sa, ca) = f.tilt.sin_cos();
    let mut out: Vec<[f64; 3]> = stadium_path(f.l, f.w, CAP_STEPS)
        .into_iter()
        .map(|ab| {
            let (y, z) = (ab[1], ab[0]);
            [0.0, y * ca - z * sa, f.ucz + y * sa + z * ca]
        })
        .collect();
    apply_rotation(&mut out, p);
    out
}

/// The chain link's lower ring centreline, after the rotation
/// (`flexi_chain_lower_centreline`); empty for other kinds.
pub fn chain_lower_centreline(p: &FlexiParams) -> Vec<[f64; 3]> {
    if p.kind == FlexiKind::ChainLink {
        chain_lower_path(p)
    } else {
        Vec::new()
    }
}

/// The upper ring's (`flexi_chain_upper_centreline`).
pub fn chain_upper_centreline(p: &FlexiParams) -> Vec<[f64; 3]> {
    if p.kind == FlexiKind::ChainLink {
        chain_upper_path(p)
    } else {
        Vec::new()
    }
}

// --------------------------------------------------------------------- hinge

/// Knuckles used: 1..9 (`hinge_knuckle_count`).
pub fn hinge_knuckle_count(p: &FlexiParams) -> i32 {
    p.hinge_knuckles.clamp(1, 9)
}

/// One knuckle's slot along the axis (`hinge_slot_length`).
pub fn hinge_slot_length(p: &FlexiParams) -> f64 {
    (p.hinge_length / f64::from(hinge_knuckle_count(p))).max(EPSILON)
}

/// One knuckle's solid length: the slot less the gap, at least a tenth of
/// the slot (`hinge_knuckle_length`).
pub fn hinge_knuckle_length(p: &FlexiParams) -> f64 {
    let s = hinge_slot_length(p);
    (s - effective_gap(p)).max(0.1 * s)
}

/// Slot `i`'s centre along +x, the run centred on 0 (`hinge_slot_centre`).
pub fn hinge_slot_centre(p: &FlexiParams, i: i32) -> f64 {
    let n = hinge_knuckle_count(p);
    (f64::from(i) - 0.5 * f64::from(n - 1)) * hinge_slot_length(p)
}

/// Whether knuckle `i` is the lower half's (`hinge_knuckle_is_lower`):
/// the even ones, or the odd ones when the fold side is upper.
pub fn hinge_knuckle_is_lower(p: &FlexiParams, i: i32) -> bool {
    let even = i % 2 == 0;
    even != p.hinge_fold_upper
}

/// The bore through the knuckles that do not carry the pin
/// (`hinge_bore_radius`).
pub fn hinge_bore_radius(p: &FlexiParams) -> f64 {
    0.5 * p.hinge_pin_dia + p.clearance
}

/// The barrel centreline's y (`hinge_axis_y`).
pub fn hinge_axis_y(p: &FlexiParams) -> f64 {
    -p.hinge_edge_offset
}

/// The knuckle run's rectangle on the cut plane, grown by `pad` and turned
/// by the rotation (`hinge_footprint_corners`).
pub fn hinge_footprint_corners(p: &FlexiParams, pad: f64) -> Vec<[f64; 2]> {
    let hx = 0.5 * p.hinge_length + pad;
    let hy = 0.5 * p.hinge_barrel_dia + pad;
    let cy = hinge_axis_y(p);
    let mut out = vec![[-hx, cy - hy], [hx, cy - hy], [hx, cy + hy], [-hx, cy + hy]];
    let a = p.rotation.to_radians();
    if a.abs() > EPSILON {
        let (s, c) = a.sin_cos();
        for v in &mut out {
            *v = [c * v[0] - s * v[1], s * v[0] + c * v[1]];
        }
    }
    out
}

/// Whether a hinge axis (in world coordinates) is more than about 6° off
/// horizontal, which the reference warns of (`hinge_axis_needs_care`).
pub fn hinge_axis_needs_care(axis_world: [f64; 3]) -> bool {
    let n = norm(axis_world);
    n > EPSILON && axis_world[2].abs() / n > 0.1
}

/// One barrel: a cylinder of radius `r` and length `len` along +x centred
/// at `cx` on the barrel centreline, the whole turned by the rotation
/// (`hinge_cylinder`).
fn hinge_cylinder(p: &FlexiParams, cx: f64, r: f64, len: f64) -> Mesh {
    let (s, c) = p.rotation.to_radians().sin_cos();
    let y = hinge_axis_y(p);
    cylinder(r, len, |v| {
        // Along +z turned to along +x, then placed, then turned about z.
        let q = [v[2] + cx - 0.5 * len, v[1] + y, -v[0]];
        [c * q[0] - s * q[1], s * q[0] + c * q[1], q[2]]
    })
}

/// The continuous pin, flush with the outermost knuckles' ends
/// (`hinge_pin`).
fn hinge_pin(p: &FlexiParams, r: f64) -> Mesh {
    let n = hinge_knuckle_count(p);
    let x0 = hinge_slot_centre(p, 0) - 0.5 * hinge_knuckle_length(p);
    let x1 = hinge_slot_centre(p, n - 1) + 0.5 * hinge_knuckle_length(p);
    hinge_cylinder(p, 0.5 * (x0 + x1), r, (x1 - x0).max(EPSILON))
}

/// One half's knuckles, grown by `grow` all round and at each end
/// (`hinge_knuckles`, `hinge_inflated_knuckles`).
fn hinge_knuckles(p: &FlexiParams, lower: bool, grow: f64) -> Vec<Mesh> {
    let r = 0.5 * p.hinge_barrel_dia + grow;
    let len = hinge_knuckle_length(p) + 2.0 * grow;
    (0..hinge_knuckle_count(p))
        .filter(|&i| hinge_knuckle_is_lower(p, i) == lower)
        .map(|i| hinge_cylinder(p, hinge_slot_centre(p, i), r, len))
        .collect()
}

// --------------------------------------------------------- thread and bayonet

/// The thread's major radius, or the lug circle's (`thread_major_radius`).
pub fn thread_major_radius(p: &FlexiParams) -> f64 {
    0.5 * p.thread_major_dia
}

/// Starts used: 1..4 (`thread_start_count`).
pub fn thread_start_count(p: &FlexiParams) -> i32 {
    p.thread_starts.clamp(1, 4)
}

/// One crest to the next along the axis: pitch / starts
/// (`thread_crest_spacing`).
pub fn thread_crest_spacing(p: &FlexiParams) -> f64 {
    (p.thread_pitch / f64::from(thread_start_count(p))).max(EPSILON)
}

/// The trapezoid's axial height at a depth (`thread_profile_height`).
pub fn thread_profile_height(p: &FlexiParams, depth: f64) -> f64 {
    0.25 * thread_crest_spacing(p) + 2.0 * depth * 30f64.to_radians().tan()
}

/// How much taller the groove is than its thread (`thread_groove_growth`).
pub fn thread_groove_growth(p: &FlexiParams) -> f64 {
    let c = p.clearance;
    2.0 * c / 30f64.to_radians().cos() + 0.6 * c
}

/// The thread's radial depth (`thread_depth`): 0.35 of the pitch, capped
/// so the thread and its groove fit between two crests with a tenth of
/// the spacing to spare, and the core keeps a third of the major radius.
pub fn thread_depth(p: &FlexiParams) -> f64 {
    let mut d = 0.35 * p.thread_pitch.max(EPSILON);
    let s = thread_crest_spacing(p);
    let room = 0.9 * s - thread_groove_growth(p);
    if thread_profile_height(p, d) > room {
        d = ((room - 0.25 * s) / (2.0 * 30f64.to_radians().tan())).max(0.05);
    }
    d.min(0.66 * thread_major_radius(p)).max(0.05)
}

/// The core's radius (`thread_minor_radius`).
pub fn thread_minor_radius(p: &FlexiParams) -> f64 {
    thread_major_radius(p) - thread_depth(p)
}

/// The threaded band's axial length (`thread_axial_length`).
pub fn thread_axial_length(p: &FlexiParams) -> f64 {
    let pitch = p.thread_pitch;
    let n = f64::from(thread_start_count(p));
    pitch * p.thread_turns + pitch * (n - 1.0) / n + thread_profile_height(p, thread_depth(p))
}

/// The female bore's radius: major plus clearance (`thread_bore_radius`).
pub fn thread_bore_radius(p: &FlexiParams) -> f64 {
    thread_major_radius(p) + p.clearance
}

/// The wall left round the bore (`thread_bore_wall`).
pub fn thread_bore_wall() -> f64 {
    1.2
}

/// The thread's trapezoid in (radial, axial) offsets from the helix's
/// centreline, counter-clockwise, shifted out by `radial_offset`
/// (`thread_profile`).
pub fn thread_profile(p: &FlexiParams, radial_offset: f64) -> Vec<[f64; 2]> {
    thread_profile_grown(p, radial_offset, 0.0)
}

/// The trapezoid grown by `grow` along each edge's own normal, its root
/// flat a little longer still so its sharp corners clear too
/// (`thread_profile_grown`).
pub fn thread_profile_grown(p: &FlexiParams, radial_offset: f64, grow: f64) -> Vec<[f64; 2]> {
    let d = thread_depth(p);
    let hr = 0.5 * d;
    let crest = 0.25 * thread_crest_spacing(p);
    let flank = d * 30f64.to_radians().tan();
    let hc = 0.5 * crest;
    let hf = hc + flank;
    let o = radial_offset;
    let g = grow.max(0.0);
    let hrg = hr + g;
    let ext = g / 30f64.to_radians().cos();
    let hcg = hc + ext;
    let hfg = hf + ext + 0.6 * g;
    vec![
        [o - hrg, -hfg],
        [o - hrg, hfg],
        [o + hrg, hcg],
        [o + hrg, -hcg],
    ]
}

/// The helix's angle from vertical at the major radius, in degrees
/// (`thread_helix_angle_deg`).
pub fn thread_helix_angle_deg(p: &FlexiParams) -> f64 {
    let r = thread_major_radius(p);
    if r <= EPSILON {
        return 90.0;
    }
    let lead = p.thread_pitch * f64::from(thread_start_count(p));
    lead.atan2(TAU * r).to_degrees()
}

/// The male thread's length below the lid face: the band and a collar
/// (`thread_plug_length`).
fn thread_plug_length(p: &FlexiParams) -> f64 {
    thread_axial_length(p) + 0.6
}

/// How deep the bore goes (`thread_bore_depth`).
fn thread_bore_depth(p: &FlexiParams) -> f64 {
    thread_plug_length(p) + p.clearance + 0.4 + 0.5 * p.thread_pitch
}

/// Whether a twist lock's axis (the cut normal, in world coordinates) is
/// more than about 6° off vertical, which the reference warns of
/// (`twist_axis_needs_care`).
pub fn twist_axis_needs_care(axis_world: [f64; 3]) -> bool {
    let n = norm(axis_world);
    !(n > EPSILON && axis_world[2].abs() / n > 0.995)
}

/// The thread's strands: the profile grown by `offset` swept along the
/// helix, hung from the lid face; the groove (`extra_turns` above 0) runs
/// that much further below and right out of the mouth above, at full
/// depth end to end (`thread_strands`).
fn thread_strands(p: &FlexiParams, offset: f64, extra_turns: f64) -> Vec<Mesh> {
    let pitch = p.thread_pitch;
    let n = thread_start_count(p);
    let rc = thread_major_radius(p) - 0.5 * thread_depth(p);
    let groove = extra_turns > 0.0;
    let run_out = extra_turns;
    let run_up = if groove {
        p.thread_turns + extra_turns
    } else {
        0.0
    };
    let turns = (p.thread_turns + run_out + run_up).max(0.05);
    let sign = if p.thread_left_hand { -1.0 } else { 1.0 };
    let phase = p.rotation - if groove { sign * 360.0 * run_out } else { 0.0 };
    let lead = if groove { 0.0 } else { p.thread_lead_turns };
    let top = 0.5 * effective_gap(p);
    let male_band = pitch * p.thread_turns + pitch * f64::from(n - 1) / f64::from(n);
    let dz = top - 0.3 - male_band - if groove { pitch * run_out } else { 0.0 };
    helical_sweep_at(
        &thread_profile_grown(p, 0.0, offset),
        rc,
        pitch,
        n,
        turns,
        SECTORS,
        p.thread_left_hand,
        phase,
        lead,
        dz,
    )
}

/// Lugs used: 2..4 (`bayonet_lug_count`).
pub fn bayonet_lug_count(p: &FlexiParams) -> i32 {
    p.bayonet_lugs.clamp(2, 4)
}

/// The plug's radius: the lug circle's less the lug height, at least 0.5
/// (`bayonet_plug_radius`).
pub fn bayonet_plug_radius(p: &FlexiParams) -> f64 {
    (thread_major_radius(p) - p.bayonet_lug_height).max(0.5)
}

/// The lock angle used: 30..120, and short of the next lug's entry
/// channel, at least 20 (`bayonet_effective_lock_angle`).
pub fn bayonet_effective_lock_angle(p: &FlexiParams) -> f64 {
    let span = 360.0 / f64::from(bayonet_lug_count(p));
    let room = span - p.bayonet_lug_arc - 8.0;
    p.bayonet_lock_angle.clamp(30.0, 120.0).min(room).max(20.0)
}

/// The plug's length below the lid face (`bayonet_plug_length`).
pub fn bayonet_plug_length(p: &FlexiParams) -> f64 {
    p.bayonet_entry_depth + p.bayonet_lug_thickness + 1.0
}

/// Lug `i`'s centre angle, the rotation included (`bayonet_lug_angle_deg`).
pub fn bayonet_lug_angle_deg(p: &FlexiParams, i: i32) -> f64 {
    p.rotation + 360.0 * f64::from(i) / f64::from(bayonet_lug_count(p))
}

/// The track's height: the lug's centre, fully in (`bayonet_track_z`).
fn bayonet_track_z(p: &FlexiParams) -> f64 {
    0.5 * effective_gap(p) - p.bayonet_entry_depth
}

/// An annular sector from `r0` to `r1`, `arc_deg` wide about `centre_deg`
/// and `thickness` tall about `cz` (`bayonet_sector`).
fn bayonet_sector(
    r0: f64,
    r1: f64,
    centre_deg: f64,
    arc_deg: f64,
    cz: f64,
    thickness: f64,
) -> Mesh {
    let r0 = r0.max(EPSILON);
    let r1 = r1.max(r0 + EPSILON);
    let arc_deg = arc_deg.clamp(1.0, 350.0);
    let a0 = (centre_deg - 0.5 * arc_deg).to_radians();
    let a1 = (centre_deg + 0.5 * arc_deg).to_radians();
    let n = ((arc_deg / 3.0).ceil() as usize).max(2);
    let (z0, z1) = (cz - 0.5 * thickness, cz + 0.5 * thickness);
    let at = |i: usize| a0 + (a1 - a0) * i as f64 / n as f64;
    // The outer arc, then back along the inner one.
    let mut ring: Vec<[f64; 2]> = (0..=n)
        .map(|i| [r1 * at(i).cos(), r1 * at(i).sin()])
        .collect();
    ring.extend((0..=n).rev().map(|i| [r0 * at(i).cos(), r0 * at(i).sin()]));
    let m = ring.len() as u32;
    let mut mesh = Mesh::default();
    for z in [z0, z1] {
        for q in &ring {
            mesh.vertices.push([q[0] as f32, q[1] as f32, z as f32]);
        }
    }
    for i in 0..m {
        let j = (i + 1) % m;
        mesh.triangles.push([i, j, m + j]);
        mesh.triangles.push([i, m + j, m + i]);
    }
    // The caps: point i of the outer arc faces point m-1-i of the inner.
    let n = n as u32;
    for i in 0..n {
        let (a, b, c, d) = (i, i + 1, m - 1 - i, m - 2 - i);
        mesh.triangles.push([a, c, b]);
        mesh.triangles.push([b, c, d]);
        mesh.triangles.push([m + a, m + b, m + c]);
        mesh.triangles.push([m + b, m + d, m + c]);
    }
    outward(mesh)
}

/// The lugs on the plug, grown by `pad` (`bayonet_lugs_bodies`).
fn bayonet_lugs(p: &FlexiParams, pad: f64) -> Vec<Mesh> {
    let r0 = bayonet_plug_radius(p) - 0.2;
    let r1 = thread_major_radius(p) + pad;
    let cz = bayonet_track_z(p);
    (0..bayonet_lug_count(p))
        .map(|i| {
            bayonet_sector(
                r0,
                r1,
                bayonet_lug_angle_deg(p, i),
                p.bayonet_lug_arc + (2.0 * pad / r1.max(1.0)).to_degrees(),
                cz,
                p.bayonet_lug_thickness + 2.0 * pad,
            )
        })
        .collect()
}

/// Lug `i`'s L-shaped slot, to take out of the body half (`bayonet_slot`):
/// the entry channel down from the mouth a lock angle back from the lug,
/// and the track round to the lug, cut in two around the detent's sliver
/// with only the inboard part of the sliver cut.
fn bayonet_slot(p: &FlexiParams, i: i32) -> Vec<Mesh> {
    let c = p.clearance;
    let r0 = bayonet_plug_radius(p) - 0.3;
    let r1 = thread_major_radius(p) + c;
    let cz = bayonet_track_z(p);
    let th = p.bayonet_lug_thickness + 2.0 * c;
    let apad = (c / r1.max(1.0)).to_degrees();
    let arc = p.bayonet_lug_arc + 2.0 * apad;
    let lock = bayonet_effective_lock_angle(p);
    let sign = if p.thread_left_hand { 1.0 } else { -1.0 };
    let a0 = bayonet_lug_angle_deg(p, i);
    let mouth = 0.5 * effective_gap(p) + c;
    let mut out = Vec::new();
    let a_entry = a0 - sign * lock;
    let z0 = cz - 0.5 * th;
    out.push(bayonet_sector(
        r0,
        r1,
        a_entry,
        arc,
        0.5 * (z0 + mouth),
        mouth - z0,
    ));
    let det = p.bayonet_detent.min(0.8 * p.bayonet_lug_height).max(0.0);
    if det <= EPSILON {
        out.push(bayonet_sector(
            r0,
            r1,
            a_entry + sign * 0.5 * lock,
            arc + lock,
            cz,
            th,
        ));
        return out;
    }
    let bump_arc = (0.12 * lock).clamp(3.0, 6.0);
    let a_bump = a0 - sign * (0.5 * arc + 0.5 * bump_arc);
    let a_lo = a_entry - sign * 0.5 * arc;
    let a_hi = a0 + sign * 0.5 * arc;
    let b_lo = a_bump - sign * 0.5 * bump_arc;
    let b_hi = a_bump + sign * 0.5 * bump_arc;
    let mut between = |r_out: f64, from: f64, to: f64| {
        let span = (to - from).abs();
        if span < 0.5 || r_out <= r0 + EPSILON {
            return;
        }
        out.push(bayonet_sector(r0, r_out, 0.5 * (from + to), span, cz, th));
    };
    between(r1, a_lo, b_lo);
    between(r1, b_hi, a_hi);
    between(r1 - det, b_lo, b_hi);
    out
}

/// Whether the lid is the upper half (`twist_lid_is_upper`).
fn lid_is_upper(p: &FlexiParams) -> bool {
    p.thread_lid_upper
}

/// Built lid-on-top; mirrored through the plane when the lid is the lower
/// half, which also turns the hand back (`twist_orient`).
fn twist_orient(p: &FlexiParams, meshes: Vec<Mesh>) -> Vec<Mesh> {
    if lid_is_upper(p) {
        return meshes;
    }
    meshes
        .into_iter()
        .map(|mut mesh| {
            for v in &mut mesh.vertices {
                v[2] = -v[2] + 0.0;
            }
            for t in &mut mesh.triangles {
                t.swap(1, 2);
            }
            mesh
        })
        .collect()
}

/// A cylinder of radius `r` from `z0` to `z1` (`twist_cylinder`).
fn twist_cylinder(r: f64, z0: f64, z1: f64) -> Mesh {
    cylinder(r.max(EPSILON), (z1 - z0).max(EPSILON), |v| {
        [v[0], v[1], v[2] + z0]
    })
}

/// The lid's bodies: the core and the strands, or the plug and the lugs
/// (`twist_lid_bodies`).
fn twist_lid_bodies(p: &FlexiParams) -> Vec<Mesh> {
    let top = 0.5 * effective_gap(p);
    let mut out = Vec::new();
    if p.kind == FlexiKind::Thread {
        let len = thread_plug_length(p);
        out.push(twist_cylinder(thread_minor_radius(p), top - len, top + 0.6));
        out.extend(thread_strands(p, 0.0, 0.0));
    } else {
        let len = bayonet_plug_length(p);
        out.push(twist_cylinder(bayonet_plug_radius(p), top - len, top + 0.6));
        out.extend(bayonet_lugs(p, 0.0));
    }
    twist_orient(p, out)
}

/// The body half's reliefs: the bore and the groove, or the bore and the
/// slots (`twist_body_reliefs`).
fn twist_body_reliefs(p: &FlexiParams) -> Vec<Mesh> {
    let c = p.clearance;
    let top = 0.5 * effective_gap(p);
    let mut out = Vec::new();
    if p.kind == FlexiKind::Thread {
        // The minor radius plus the clearance: a bore of the major radius
        // would swallow the groove.
        out.push(twist_cylinder(
            thread_minor_radius(p) + c,
            top - thread_bore_depth(p),
            top + 0.6,
        ));
        out.extend(thread_strands(p, c, 0.5));
    } else {
        out.push(twist_cylinder(
            bayonet_plug_radius(p) + c,
            top - bayonet_plug_length(p) - c - 0.4,
            top + 0.6,
        ));
        for i in 0..bayonet_lug_count(p) {
            out.extend(bayonet_slot(p, i));
        }
    }
    twist_orient(p, out)
}

// ------------------------------------------------------------------ profiles

/// The dovetail lip's section in (r, z), relative to the upper face, its
/// ceiling at `top_z` (`ring_profile`).
fn ring_profile(p: &FlexiParams, top_z: f64) -> Vec<[f64; 2]> {
    let rm = p.outer_radius - 0.5 * p.ring_width;
    let w = p.ring_width;
    let wn = w * p.neck_ratio;
    let h = p.ring_height;
    let zn = h - 0.6 * h;
    let a = anchor_depth(p) + effective_gap(p);
    let mut prof = vec![
        [rm - 0.5 * wn, -a],
        [rm + 0.5 * wn, -a],
        [rm + 0.5 * wn, zn],
        [rm + 0.5 * w, h],
    ];
    if top_z > h + EPSILON {
        prof.push([rm + 0.5 * w, top_z]);
        prof.push([rm - 0.5 * w, top_z]);
    }
    prof.push([rm - 0.5 * w, h]);
    prof.push([rm - 0.5 * wn, zn]);
    prof
}

/// The hub's section (`hub_profile`).
fn hub_profile(p: &FlexiParams, top_z: f64) -> Vec<[f64; 2]> {
    let rh = p.hub_radius;
    let a = anchor_depth(p) + effective_gap(p);
    vec![[0.0, -a], [rh, -a], [rh, top_z], [0.0, top_z]]
}

/// The ball on its neck (`ball_profile`).
fn ball_profile(p: &FlexiParams) -> Vec<[f64; 2]> {
    let rb = p.outer_radius;
    let rn = ball_neck_radius(p);
    let zb = ball_centre_z(p);
    let a = anchor_depth(p) + effective_gap(p);
    let zj = zb - (rb * rb - rn * rn).max(0.0).sqrt();
    let mut prof = vec![[0.0, -a], [rn, -a], [rn, zj]];
    // The arc from the junction over the pole, beta from +z. The
    // reference starts it at i = 0, which is the junction (rn, zj) again:
    // a point twice over, and a band of zero-area triangles round the
    // revolved ball. It is started one step on here.
    let beta_j = PI - (rn / rb).min(1.0).asin();
    for i in 1..=ARC_STEPS {
        let beta = beta_j * (1.0 - i as f64 / ARC_STEPS as f64);
        prof.push([rb * beta.sin(), zb + rb * beta.cos()]);
    }
    prof
}

/// A section offset by `delta` in the (r, z) half plane, mitred (limit 3)
/// as the reference's Clipper offset is; one that touches the axis is
/// mirrored across it first and cut back to r >= 0 after
/// (`dilate_profile`).
fn dilate_profile(prof: &[[f64; 2]], delta: f64, touches_axis: bool) -> Vec<[f64; 2]> {
    let mut points: Vec<Point2> = prof.iter().map(|v| Point2::from_mm(v[0], v[1])).collect();
    if touches_axis {
        for v in prof.iter().skip(1).rev() {
            if v[0] > EPSILON {
                points.push(Point2::from_mm(-v[0], v[1]));
            }
        }
    }
    let mut src = Polygon { points };
    if src.signed_area_mm2() < 0.0 {
        src.points.reverse();
    }
    let biggest = |regions: Vec<ExPolygon>| {
        regions.into_iter().map(|r| r.contour).max_by(|a, b| {
            a.signed_area_mm2()
                .abs()
                .total_cmp(&b.signed_area_mm2().abs())
        })
    };
    let region = ExPolygon {
        contour: src,
        holes: Vec::new(),
    };
    let Some(mut res) = edge_clip::offset(&[region], scaled(delta), Join::MITER)
        .ok()
        .and_then(biggest)
    else {
        return prof.to_vec();
    };
    if touches_axis {
        let big = scaled(1000.0);
        let half = ExPolygon {
            contour: Polygon {
                points: vec![
                    Point2 { x: 0, y: -big },
                    Point2 { x: big, y: -big },
                    Point2 { x: big, y: big },
                    Point2 { x: 0, y: big },
                ],
            },
            holes: Vec::new(),
        };
        let whole = ExPolygon {
            contour: res.clone(),
            holes: Vec::new(),
        };
        if let Some(clipped) = edge_clip::intersection(&[whole], &[half])
            .ok()
            .and_then(biggest)
        {
            res = clipped;
        }
    }
    if res.signed_area_mm2() < 0.0 {
        res.points.reverse();
    }
    res.points
        .iter()
        .map(|q| {
            let r = unscaled(q.x);
            [if r < EPSILON { 0.0 } else { r }, unscaled(q.y)]
        })
        .collect()
}

fn shift_profiles(profiles: &mut [Vec<[f64; 2]>], dz: f64) {
    if dz.abs() <= EPSILON {
        return;
    }
    for v in profiles.iter_mut().flatten() {
        v[1] += dz;
    }
}

/// The revolved kinds' male sections in the cut frame
/// (`flexi_male_profiles`): the lip and the hub, or the ball.
pub fn male_profiles(p: &FlexiParams) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    if p.kind == FlexiKind::DoubleRing {
        out.push(ring_profile(p, p.ring_height));
        if p.hub_radius > 0.2 {
            out.push(hub_profile(p, p.ring_height));
        }
    } else {
        out.push(ball_profile(p));
    }
    shift_profiles(&mut out, revolved_z_shift(p));
    out
}

/// Their cavities: the sections, the ceiling raised by the tilt, offset
/// by the clearance (`flexi_female_profiles`).
pub fn female_profiles(p: &FlexiParams) -> Vec<Vec<[f64; 2]>> {
    let c = p.clearance;
    let top = p.ring_height + p.tilt;
    let mut out = Vec::new();
    if p.kind == FlexiKind::DoubleRing {
        out.push(dilate_profile(&ring_profile(p, top), c, false));
        if p.hub_radius > 0.2 {
            out.push(dilate_profile(&hub_profile(p, top), c, true));
        }
    } else {
        out.push(dilate_profile(&ball_profile(p), c, true));
    }
    shift_profiles(&mut out, revolved_z_shift(p));
    out
}

fn revolve_all(profiles: &[Vec<[f64; 2]>]) -> Vec<Mesh> {
    profiles
        .iter()
        .map(|prof| connector::revolved(&dedupe(prof), SECTORS))
        .collect()
}

// -------------------------------------------------------------------- bodies

/// The lower half's own bodies (`flexi_male_bodies`): the lip and hub or
/// the ball; the lower ring; the lower knuckles and the pin when it is
/// down here; a twist lock's lid bodies when the lid is below.
pub fn male_bodies(p: &FlexiParams) -> Vec<Mesh> {
    match p.kind {
        FlexiKind::Thread | FlexiKind::Bayonet => {
            if lid_is_upper(p) {
                Vec::new()
            } else {
                twist_lid_bodies(p)
            }
        }
        FlexiKind::Hinge => {
            let mut out = hinge_knuckles(p, true, 0.0);
            if hinge_knuckle_is_lower(p, 0) {
                out.push(hinge_pin(p, 0.5 * p.hinge_pin_dia));
            }
            out
        }
        FlexiKind::ChainLink => vec![swept_loop(&chain_lower_path(p), p.wire, WIRE_SECTORS)],
        _ => revolve_all(&male_profiles(p)),
    }
}

/// The upper half's (`flexi_female_cavities`): for the revolved kinds the
/// cavity, otherwise the upper half's solid bodies.
pub fn female_cavities(p: &FlexiParams) -> Vec<Mesh> {
    match p.kind {
        FlexiKind::Thread | FlexiKind::Bayonet => {
            if lid_is_upper(p) {
                twist_lid_bodies(p)
            } else {
                Vec::new()
            }
        }
        FlexiKind::Hinge => {
            let mut out = hinge_knuckles(p, false, 0.0);
            if !hinge_knuckle_is_lower(p, 0) {
                out.push(hinge_pin(p, 0.5 * p.hinge_pin_dia));
            }
            out
        }
        FlexiKind::ChainLink => vec![swept_loop(&chain_upper_path(p), p.wire, WIRE_SECTORS)],
        _ => revolve_all(&female_profiles(p)),
    }
}

/// What is added to the lower half (`flexi_lower_bodies`).
pub fn lower_bodies(p: &FlexiParams) -> Vec<Mesh> {
    male_bodies(p)
}

/// What is added to the upper half (`flexi_upper_bodies`): nothing for
/// the revolved kinds, whose upper half gets a cavity.
pub fn upper_bodies(p: &FlexiParams) -> Vec<Mesh> {
    if is_two_sided(p) {
        female_cavities(p)
    } else {
        Vec::new()
    }
}

/// What is taken out of the lower half (`flexi_lower_reliefs`): the upper
/// half's bodies grown by the clearance, or a twist lock's bore when the
/// body is below.
pub fn lower_reliefs(p: &FlexiParams) -> Vec<Mesh> {
    match p.kind {
        FlexiKind::Thread | FlexiKind::Bayonet => {
            if lid_is_upper(p) {
                twist_body_reliefs(p)
            } else {
                Vec::new()
            }
        }
        FlexiKind::Hinge => {
            let mut out = hinge_knuckles(p, false, p.clearance);
            if !hinge_knuckle_is_lower(p, 0) {
                out.push(hinge_pin(p, 0.5 * p.hinge_pin_dia + p.clearance));
            }
            out
        }
        FlexiKind::ChainLink => vec![swept_loop(
            &chain_upper_path(p),
            p.wire + p.clearance,
            WIRE_SECTORS,
        )],
        _ => Vec::new(),
    }
}

/// What is taken out of the upper half (`flexi_upper_reliefs`): the
/// lower half's bodies grown by the clearance (the pin's is the bore), a
/// twist lock's bore when the body is above, the revolved kinds' cavity.
pub fn upper_reliefs(p: &FlexiParams) -> Vec<Mesh> {
    match p.kind {
        FlexiKind::Thread | FlexiKind::Bayonet => {
            if lid_is_upper(p) {
                Vec::new()
            } else {
                twist_body_reliefs(p)
            }
        }
        FlexiKind::Hinge => {
            let mut out = hinge_knuckles(p, true, p.clearance);
            if hinge_knuckle_is_lower(p, 0) {
                out.push(hinge_pin(p, 0.5 * p.hinge_pin_dia + p.clearance));
            }
            out
        }
        FlexiKind::ChainLink => vec![swept_loop(
            &chain_lower_path(p),
            p.wire + p.clearance,
            WIRE_SECTORS,
        )],
        _ => female_cavities(p),
    }
}

/// One mesh to show where the joint goes (`flexi_preview_body`): a twist
/// lock's lid bodies; every knuckle and the pin; both loops grown by the
/// clearance; the revolved kinds' cavity. Separate closed shells.
pub fn preview_body(p: &FlexiParams) -> Mesh {
    let parts = match p.kind {
        FlexiKind::Thread | FlexiKind::Bayonet => twist_lid_bodies(p),
        FlexiKind::Hinge => {
            let mut all = male_bodies(p);
            all.extend(female_cavities(p));
            all
        }
        FlexiKind::ChainLink => {
            let mut all = lower_reliefs(p);
            all.extend(upper_reliefs(p));
            all
        }
        _ => female_cavities(p),
    };
    let mut out = Mesh::default();
    for part in &parts {
        out.append(part, &edge_mesh::Transform::IDENTITY);
    }
    out
}

// ---------------------------------------------------------------- footprints

/// The rectangle the two loops sweep through the cut plane, grown by the
/// wire, the clearance and `pad` (`chain_footprint_corners`).
pub fn chain_footprint_corners(p: &FlexiParams, pad: f64) -> Vec<[f64; 2]> {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for v in chain_lower_path(p).iter().chain(&chain_upper_path(p)) {
        for k in 0..2 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    let m = p.wire + p.clearance + pad;
    let (xlo, ylo, xhi, yhi) = (lo[0] - m, lo[1] - m, hi[0] + m, hi[1] + m);
    vec![[xlo, ylo], [xhi, ylo], [xhi, yhi], [xlo, yhi]]
}

/// What has to fit inside the cut's contour (`flexi_footprint_corners`):
/// the hinge's and the chain link's rectangles, a 60-gon of the outer
/// extent for the rest.
pub fn footprint_corners(p: &FlexiParams, pad: f64) -> Vec<[f64; 2]> {
    match p.kind {
        FlexiKind::Hinge => hinge_footprint_corners(p, pad),
        FlexiKind::ChainLink => chain_footprint_corners(p, pad),
        _ => {
            let r = outer_extent(p) + pad;
            (0..60)
                .map(|i| {
                    let a = TAU * f64::from(i) / 60.0;
                    [r * a.cos(), r * a.sin()]
                })
                .collect()
        }
    }
}

// -------------------------------------------------------------------- guards

/// Whether the slicer's gap closing radius would weld the clearance shut
/// (`flexi_gap_closing_conflict`).
pub fn gap_closing_conflict(p: &FlexiParams, slice_gap_closing_radius: f64) -> bool {
    slice_gap_closing_radius >= 0.5 * p.clearance
}

/// The largest safe gap closing radius (`flexi_max_safe_gap_closing_radius`).
pub fn max_safe_gap_closing_radius(p: &FlexiParams) -> f64 {
    0.5 * p.clearance
}

/// Why the joint cannot be made, when it cannot (`flexi_validate`): the
/// reference's words, which it shows untranslated.
pub fn validate(p: &FlexiParams) -> Option<&'static str> {
    let revolved = matches!(p.kind, FlexiKind::DoubleRing | FlexiKind::BallSocket);
    if revolved && p.outer_radius <= 0.5 {
        return Some("Joint radius is too small.");
    }
    if p.clearance <= 0.0 {
        return Some(
            "Clearance must be greater than zero, otherwise the joint prints as one solid.",
        );
    }
    if p.gap > 0.0 && p.gap < p.clearance {
        return Some("Gap cannot be smaller than the clearance.");
    }
    if p.kind.is_twist() {
        return validate_twist(p);
    }
    match p.kind {
        FlexiKind::Hinge => {
            if !(1..=9).contains(&p.hinge_knuckles) {
                return Some("A hinge needs between 1 and 9 knuckles.");
            }
            if p.hinge_pin_dia <= 0.0 {
                return Some("Pin diameter must be greater than zero.");
            }
            if p.hinge_barrel_dia <= 2.0 * hinge_bore_radius(p) + 0.4 {
                return Some(
                    "Barrel diameter is too small for this pin and clearance: it has to clear the pin, twice the clearance and a wall on each side.",
                );
            }
            if p.hinge_length <= 0.0 {
                return Some("Hinge length must be greater than zero.");
            }
            if hinge_slot_length(p) <= effective_gap(p) + 0.2 {
                return Some(
                    "Too many knuckles for this hinge length and gap: each knuckle needs more than the gap plus 0.2 mm of length.",
                );
            }
            if !(0.0..=180.0).contains(&p.rotation) {
                return Some("Rotation must be between 0 and 180 degrees.");
            }
            None
        }
        FlexiKind::ChainLink => {
            if p.wire <= 0.0 {
                return Some("Wire thickness must be greater than zero.");
            }
            let need = min_link_size(p);
            if p.link_length < need || p.link_width < need {
                return Some(
                    "The loops are too small for this wire and clearance: each loop's opening has to pass the other loop's wire, which needs a length and a width of at least 4 x wire + 2 x clearance.",
                );
            }
            if p.link_length < p.link_width {
                return Some("Link length must be at least the link width.");
            }
            if !(0.0..=30.0).contains(&p.tilt_angle) {
                return Some("The upper ring's tilt must be between 0 and 30 degrees.");
            }
            if p.stem <= 0.0 {
                return Some("Stem depth must be greater than zero, or the ring is not attached.");
            }
            if !(0.0..=180.0).contains(&p.rotation) {
                return Some("Rotation must be between 0 and 180 degrees.");
            }
            let f = chain_frame(p);
            if f.off_max < f.off_min || f.interlock <= 0.0 {
                return Some(
                    "The rings cannot interlock with these proportions: each has to sit clear of its own cut face and still reach through the other ring's hole. Lengthen the link, or reduce the gap, the stem or the wire.",
                );
            }
            None
        }
        _ => {
            if p.kind == FlexiKind::DoubleRing {
                if p.ring_width <= 2.0 * p.clearance {
                    return Some("Ring width is too small for this clearance.");
                }
                if p.ring_height <= 2.0 * p.clearance {
                    return Some("Ring height is too small for this clearance.");
                }
                if p.ring_width >= p.outer_radius {
                    return Some("Ring width must be smaller than the outer radius.");
                }
                if p.neck_ratio <= 0.05 || p.neck_ratio >= 0.95 {
                    return Some("Neck ratio must be between 0.05 and 0.95.");
                }
                if p.hub_radius > 0.0 && p.hub_radius + p.clearance >= p.outer_radius - p.ring_width
                {
                    return Some("Hub radius leaves no room between the hub and the ring.");
                }
            } else if !(10.0..=75.0).contains(&p.open_angle) {
                return Some("Socket opening angle must be between 10 and 75 degrees.");
            }
            if !is_captive(p) {
                return Some(
                    "These parameters do not capture the joint: the mouth is as wide as the lip.",
                );
            }
            None
        }
    }
}

fn validate_twist(p: &FlexiParams) -> Option<&'static str> {
    if p.thread_major_dia < 4.0 {
        return Some(
            "Major diameter is too small: a twist lock under 4 mm across has no wall left once the clearance is taken out.",
        );
    }
    if !(0.0..=360.0).contains(&p.rotation) {
        return Some("Rotation must be between 0 and 360 degrees.");
    }
    if p.kind == FlexiKind::Thread {
        if !(1..=4).contains(&p.thread_starts) {
            return Some("A thread needs between 1 and 4 starts.");
        }
        if p.thread_turns <= 0.0 {
            return Some("Turns must be greater than zero.");
        }
        if p.thread_pitch < 1.0 {
            return Some(
                "Pitch is too fine to print: keep it at 1 mm or more (2 mm or more is the recommended floor on a 0.4 mm nozzle).",
            );
        }
        let s = thread_crest_spacing(p);
        if 0.25 * s + thread_groove_growth(p) >= 0.9 * s {
            return Some(
                "This pitch, start count and clearance leave no wall between one thread and the next: the groove's turns would run into each other and the lid would pull straight out. Raise the pitch, reduce the starts, or reduce the clearance.",
            );
        }
        if thread_minor_radius(p) < 1.2 {
            return Some(
                "Major diameter is too small for this pitch: the thread eats the whole core. Raise the diameter, or lower the pitch.",
            );
        }
        if p.thread_lead_turns < 0.0 || p.thread_lead_turns > 0.5 * p.thread_turns {
            return Some(
                "The lead-in cannot be longer than half the thread: it is tapered at both ends.",
            );
        }
        return None;
    }
    if !(2..=4).contains(&p.bayonet_lugs) {
        return Some("A bayonet needs between 2 and 4 lugs.");
    }
    if p.bayonet_lug_height <= 2.0 * p.clearance {
        return Some(
            "Lug height is too small for this clearance: the lug would vanish into its own slot.",
        );
    }
    if p.bayonet_lug_thickness <= 2.0 * p.clearance {
        return Some("Lug thickness is too small for this clearance.");
    }
    if bayonet_plug_radius(p) <= 1.0 {
        return Some(
            "The lugs are taller than the plug is wide: reduce the lug height, or raise the major diameter.",
        );
    }
    // The lock angle asked for, not the one clamped to fit.
    let span = 360.0 / f64::from(bayonet_lug_count(p));
    let want = p.bayonet_lock_angle.clamp(30.0, 120.0);
    if p.bayonet_lug_arc + want + 8.0 > span {
        return Some(
            "These lugs are too wide, or the lock angle is too large, for this lug count: each lug's slot and track have to fit in its own share of the bore with wall to spare.",
        );
    }
    if p.bayonet_entry_depth < p.bayonet_lug_thickness {
        return Some(
            "The entry channel has to be at least as deep as the lug is thick, or the lug cannot drop in far enough to turn.",
        );
    }
    if p.bayonet_detent < 0.0 {
        return Some("The detent height cannot be negative.");
    }
    if p.bayonet_detent >= p.bayonet_lug_height {
        return Some(
            "The detent is as tall as the lug: it would block the track instead of clicking.",
        );
    }
    None
}

// ------------------------------------------------------------ mesh builders

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = norm(a);
    if n > 0.0 { mul(a, 1.0 / n) } else { a }
}

/// `v` turned by `angle` about the unit `axis` (Rodrigues).
fn rotated(v: [f64; 3], axis: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = angle.sin_cos();
    add(
        add(mul(v, c), mul(cross(axis, v), s)),
        mul(axis, dot(axis, v) * (1.0 - c)),
    )
}

fn to_f32(p: [f64; 3]) -> [f32; 3] {
    p.map(|c| c as f32)
}

fn outward(mut mesh: Mesh) -> Mesh {
    if mesh.signed_volume() < 0.0 {
        for t in &mut mesh.triangles {
            t.swap(1, 2);
        }
    }
    mesh
}

/// Points closer than a micron to the one before made one, and the last
/// dropped where it meets the first.
fn dedupe(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) <= 1e-6;
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for &point in points {
        if out.last().is_none_or(|last| !close(*last, point)) {
            out.push(point);
        }
    }
    while out.len() > 1 && close(out[0], out[out.len() - 1]) {
        out.pop();
    }
    out
}

/// A cylinder of radius `r` from z = 0 to `h`, [`SECTORS`] sides, the
/// first corner on +y as `its_make_cylinder` puts it, each corner placed
/// by `place`; closed and wound outward.
fn cylinder(r: f64, h: f64, place: impl Fn([f64; 3]) -> [f64; 3]) -> Mesh {
    let n = SECTORS as u32;
    let mut mesh = Mesh::default();
    mesh.vertices.push(to_f32(place([0.0, 0.0, 0.0])));
    mesh.vertices.push(to_f32(place([0.0, 0.0, h])));
    for z in [0.0, h] {
        for i in 0..n {
            let a = TAU * f64::from(i) / f64::from(n);
            mesh.vertices
                .push(to_f32(place([-r * a.sin(), r * a.cos(), z])));
        }
    }
    let (low, high) = (2, 2 + n);
    for i in 0..n {
        let j = (i + 1) % n;
        mesh.triangles.push([0, low + j, low + i]);
        mesh.triangles.push([1, high + i, high + j]);
        mesh.triangles.push([low + i, low + j, high + j]);
        mesh.triangles.push([low + i, high + j, high + i]);
    }
    outward(mesh)
}

/// A closed tube of radius `tube_r` along the closed path `path`, its
/// section carried round by parallel transport and the leftover twist
/// spread over the loop so the seam closes (TriangleMesh.cpp
/// `its_make_swept_loop`). Empty for fewer than three distinct points or
/// sectors.
pub fn swept_loop(path_in: &[[f64; 3]], tube_r: f64, sectors: usize) -> Mesh {
    let mut mesh = Mesh::default();
    if sectors < 3 || tube_r <= 0.0 {
        return mesh;
    }
    let mut path: Vec<[f64; 3]> = Vec::with_capacity(path_in.len());
    for &p in path_in {
        if path.last().is_none_or(|&last| norm(sub(p, last)) > EPSILON) {
            path.push(p);
        }
    }
    while path.len() > 1 && norm(sub(path[0], path[path.len() - 1])) <= EPSILON {
        path.pop();
    }
    let n = path.len();
    if n < 3 {
        return mesh;
    }
    let tangent: Vec<[f64; 3]> = (0..n)
        .map(|i| {
            let prev = unit(sub(path[i], path[(i + n - 1) % n]));
            let next = unit(sub(path[(i + 1) % n], path[i]));
            let t = add(prev, next);
            unit(if norm(t) < EPSILON { next } else { t })
        })
        .collect();
    let any_perp = |t: [f64; 3]| {
        let a = if t[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        unit(cross(t, a))
    };
    // `u` carried from one tangent onto the next.
    let transport = |u: [f64; 3], t0: [f64; 3], t1: [f64; 3]| {
        let ax = cross(t0, t1);
        let s = norm(ax);
        let mut up = u;
        if s > EPSILON {
            up = rotated(up, mul(ax, 1.0 / s), s.atan2(dot(t0, t1)));
        }
        sub(up, mul(t1, dot(t1, up)))
    };
    let mut u = vec![any_perp(tangent[0]); n];
    for i in 1..n {
        let up = transport(u[i - 1], tangent[i - 1], tangent[i]);
        u[i] = if norm(up) < EPSILON {
            any_perp(tangent[i])
        } else {
            unit(up)
        };
    }
    let mut residual = 0.0;
    let up = transport(u[n - 1], tangent[n - 1], tangent[0]);
    if norm(up) > EPSILON {
        let up = unit(up);
        let w0 = cross(tangent[0], u[0]);
        residual = dot(w0, up).atan2(dot(u[0], up));
    }
    let mut v = vec![[0.0; 3]; n];
    for i in 0..n {
        let phi = -residual * i as f64 / n as f64;
        let w = cross(tangent[i], u[i]);
        u[i] = unit(add(mul(u[i], phi.cos()), mul(w, phi.sin())));
        v[i] = unit(cross(tangent[i], u[i]));
    }
    for i in 0..n {
        for j in 0..sectors {
            let a = TAU * j as f64 / sectors as f64;
            let off = add(mul(u[i], a.cos()), mul(v[i], a.sin()));
            mesh.vertices.push(to_f32(add(path[i], mul(off, tube_r))));
        }
    }
    let s = sectors as u32;
    for i in 0..n as u32 {
        let i2 = (i + 1) % n as u32;
        for j in 0..s {
            let j2 = (j + 1) % s;
            let (a, b, c, d) = (i * s + j, i * s + j2, i2 * s + j2, i2 * s + j);
            mesh.triangles.push([a, b, c]);
            mesh.triangles.push([a, c, d]);
        }
    }
    outward(mesh)
}

/// Helical ribbons, one per start (TriangleMesh.cpp
/// `its_make_helical_sweep`): the closed `profile` in (radial, axial)
/// offsets swept along a helix of centreline radius `radius` about +z,
/// rising `pitch` per turn for `turns` turns from z = 0, in
/// `segments_per_turn` steps per turn. Start s is the same helix lifted by
/// pitch × s / starts; `phase_deg` turns them all, `left_hand` reverses
/// the sense. The profile fades to nothing over `lead_frac` of a turn at
/// each end, shrunk about the middle of its innermost edge (so a convex
/// profile stays inside its full self; the reference shrinks only its
/// radial extent, towards the centreline), which closes the ends; at 0
/// they are capped flat. Empty for fewer than three profile points, a radius,
/// pitch or turns not above 0, fewer than three steps per turn or starts
/// outside 1..8.
#[allow(clippy::too_many_arguments)]
pub fn helical_sweep(
    profile: &[[f64; 2]],
    radius: f64,
    pitch: f64,
    starts: i32,
    turns: f64,
    segments_per_turn: usize,
    left_hand: bool,
    phase_deg: f64,
    lead_frac: f64,
) -> Vec<Mesh> {
    helical_sweep_at(
        profile,
        radius,
        pitch,
        starts,
        turns,
        segments_per_turn,
        left_hand,
        phase_deg,
        lead_frac,
        0.0,
    )
}

/// [`helical_sweep`] lifted by `dz`.
#[allow(clippy::too_many_arguments)]
fn helical_sweep_at(
    profile: &[[f64; 2]],
    radius: f64,
    pitch: f64,
    starts: i32,
    turns: f64,
    segments_per_turn: usize,
    left_hand: bool,
    phase_deg: f64,
    lead_frac: f64,
    dz: f64,
) -> Vec<Mesh> {
    let np = profile.len();
    if np < 3
        || radius <= 0.0
        || pitch <= 0.0
        || turns <= 0.0
        || segments_per_turn < 3
        || !(1..=8).contains(&starts)
    {
        return Vec::new();
    }
    let mut prof = profile.to_vec();
    let twice: f64 = (0..np)
        .map(|i| {
            let (a, b) = (prof[i], prof[(i + 1) % np]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if twice < 0.0 {
        prof.reverse();
    }
    let nsteps = ((turns * segments_per_turn as f64).ceil() as usize).max(2);
    let nrings = nsteps + 1;
    let dir = if left_hand { -1.0 } else { 1.0 };
    let lead_turns = lead_frac.min(0.5 * turns).max(0.0);
    let taper = lead_turns > 0.0;
    let depth_scale = |t: f64| {
        if !taper {
            return 1.0;
        }
        (t.min(turns - t) / lead_turns).clamp(0.0, 1.0)
    };
    // THE LEAD-IN FADES TOWARDS THE ROOT. The reference scales only the
    // radial offsets, towards the centreline (TriangleMesh.cpp
    // `its_make_helical_sweep`, `dr = sc * prof[j].x()`). That keeps the
    // root flat's full axial width while pulling it outward, so the faded
    // profile is not inside the full one: its root corners stand past the
    // flanks by up to depth/2 x tan 30°, more than the clearance the
    // groove leaves, and the cut halves of a thread overlap at the
    // strands' ends. It also lifts the faded strand off the core it should
    // grow from. Here the profile is scaled as a whole about the middle of
    // its innermost edge: a convex profile shrunk about a point of its own
    // stays inside itself, flanks at the same angle, and the end closes on
    // that point, on the core.
    let ax = prof.iter().map(|q| q[0]).fold(f64::INFINITY, f64::min);
    let (lo, hi) = prof
        .iter()
        .filter(|q| q[0] <= ax + 1e-9)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| {
            (lo.min(q[1]), hi.max(q[1]))
        });
    let anchor = [ax, 0.5 * (lo + hi)];
    let phase0 = phase_deg.to_radians();
    (0..starts)
        .map(|s| {
            let mut mesh = Mesh::default();
            let z0 = pitch * f64::from(s) / f64::from(starts) + dz;
            let mut base = Vec::with_capacity(nrings);
            let mut point = Vec::with_capacity(nrings);
            for i in 0..nrings {
                let t = (turns * i as f64 / nsteps as f64).min(turns);
                let ang = phase0 + dir * TAU * t;
                let zc = z0 + pitch * t;
                let sc = depth_scale(t);
                let (sa, ca) = ang.sin_cos();
                base.push(mesh.vertices.len() as u32);
                let collapsed = taper && sc <= 0.0;
                point.push(collapsed);
                if collapsed {
                    let r = radius + anchor[0];
                    mesh.vertices.push(to_f32([r * ca, r * sa, zc + anchor[1]]));
                    continue;
                }
                for q in &prof {
                    let r = radius + anchor[0] + sc * (q[0] - anchor[0]);
                    let z = zc + anchor[1] + sc * (q[1] - anchor[1]);
                    mesh.vertices.push(to_f32([r * ca, r * sa, z]));
                }
            }
            let id = |i: usize, j: usize| {
                if point[i] {
                    base[i]
                } else {
                    base[i] + (j % np) as u32
                }
            };
            for i in 0..nrings - 1 {
                if point[i] && point[i + 1] {
                    continue;
                }
                for j in 0..np {
                    let (a, b, c, d) = (id(i, j), id(i, j + 1), id(i + 1, j + 1), id(i + 1, j));
                    if !point[i] {
                        mesh.triangles.push([a, b, c]);
                    }
                    if !point[i + 1] {
                        mesh.triangles.push([a, c, d]);
                    }
                }
            }
            if !taper {
                let (first, last) = (base[0], base[nrings - 1]);
                for j in 1..np as u32 - 1 {
                    mesh.triangles.push([first, first + j + 1, first + j]);
                    mesh.triangles.push([last, last + j, last + j + 1]);
                }
            }
            outward(mesh)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use edge_mesh::boolean::{self, check};

    fn sound(mesh: &Mesh, what: &str) {
        assert!(!mesh.triangles.is_empty(), "{what}: empty");
        let d = check::defects(mesh);
        assert!(d.is_clean(), "{what}: {d:?}");
        assert!(mesh.signed_volume() > 0.0, "{what}: wound inward");
    }

    fn ring_params() -> FlexiParams {
        let mut p = FlexiParams::default();
        p.hub_radius = default_hub_radius(&p);
        p
    }

    fn ball_params() -> FlexiParams {
        FlexiParams {
            kind: FlexiKind::BallSocket,
            ..FlexiParams::default()
        }
    }

    fn chain_params() -> FlexiParams {
        FlexiParams {
            kind: FlexiKind::ChainLink,
            gap: 1.5,
            ..FlexiParams::default()
        }
    }

    /// Parked at the edge of a 20 mm round face, as the test article is.
    fn hinge_params() -> FlexiParams {
        let mut p = FlexiParams {
            kind: FlexiKind::Hinge,
            gap: 0.6,
            ..FlexiParams::default()
        };
        p.hinge_edge_offset =
            (100.0f64 - (0.5 * p.hinge_length).powi(2)).sqrt() - 0.5 * p.hinge_barrel_dia - 0.1;
        p
    }

    fn thread_params() -> FlexiParams {
        FlexiParams {
            kind: FlexiKind::Thread,
            clearance: 0.25,
            gap: 0.4,
            ..FlexiParams::default()
        }
    }

    fn bayonet_params() -> FlexiParams {
        FlexiParams {
            kind: FlexiKind::Bayonet,
            clearance: 0.25,
            gap: 0.4,
            ..FlexiParams::default()
        }
    }

    fn every_mesh(p: &FlexiParams) -> Vec<(String, Mesh)> {
        let mut out = Vec::new();
        for (name, meshes) in [
            ("lower body", lower_bodies(p)),
            ("upper body", upper_bodies(p)),
            ("lower relief", lower_reliefs(p)),
            ("upper relief", upper_reliefs(p)),
            ("male", male_bodies(p)),
            ("female", female_cavities(p)),
        ] {
            for (i, mesh) in meshes.into_iter().enumerate() {
                out.push((format!("{:?} {name} {i}", p.kind), mesh));
            }
        }
        out
    }

    /// The closest point of triangle abc to q (Ericson, Real-Time Collision
    /// Detection 5.1.5).
    fn closest_on_triangle(q: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
        let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(q, a));
        let (d1, d2) = (dot(ab, ap), dot(ac, ap));
        if d1 <= 0.0 && d2 <= 0.0 {
            return a;
        }
        let bp = sub(q, b);
        let (d3, d4) = (dot(ab, bp), dot(ac, bp));
        if d3 >= 0.0 && d4 <= d3 {
            return b;
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            return add(a, mul(ab, d1 / (d1 - d3)));
        }
        let cp = sub(q, c);
        let (d5, d6) = (dot(ab, cp), dot(ac, cp));
        if d6 >= 0.0 && d5 <= d6 {
            return c;
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            return add(a, mul(ac, d2 / (d2 - d6)));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            return add(b, mul(sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))));
        }
        let denom = 1.0 / (va + vb + vc);
        add(a, add(mul(ab, vb * denom), mul(ac, vc * denom)))
    }

    fn vertex_to_surface(a: &Mesh, b: &Mesh) -> f64 {
        let tris: Vec<[[f64; 3]; 3]> = (0..b.triangles.len())
            .map(|i| b.triangle(i).map(|p| p.map(f64::from)))
            .collect();
        a.vertices
            .iter()
            .map(|v| {
                let q = v.map(f64::from);
                tris.iter()
                    .map(|t| norm(sub(q, closest_on_triangle(q, t[0], t[1], t[2]))))
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(f64::INFINITY, f64::min)
    }

    /// The least distance between two surfaces, sampled at the vertices of
    /// each (as the reference's `min_surface_distance`).
    fn min_surface_distance(a: &Mesh, b: &Mesh) -> f64 {
        vertex_to_surface(a, b).min(vertex_to_surface(b, a))
    }

    fn intersection_volume(a: &Mesh, b: &Mesh) -> f64 {
        boolean::intersection(a, b)
            .expect("intersection")
            .signed_volume()
            .abs()
    }

    fn centroid(mesh: &Mesh) -> [f64; 3] {
        let n = mesh.vertices.len() as f64;
        mesh.vertices
            .iter()
            .fold([0.0; 3], |acc, v| add(acc, v.map(f64::from)))
            .map(|c| c / n)
    }

    /// How far the vertices stray from the plane through `centre` with
    /// `normal`.
    fn off_plane(mesh: &Mesh, centre: [f64; 3], normal: [f64; 3]) -> f64 {
        mesh.vertices
            .iter()
            .map(|v| dot(sub(v.map(f64::from), centre), normal).abs())
            .fold(0.0, f64::max)
    }

    fn bounds(meshes: &[Mesh]) -> ([f64; 3], [f64; 3]) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for v in meshes.iter().flat_map(|m| &m.vertices) {
            for k in 0..3 {
                lo[k] = lo[k].min(f64::from(v[k]));
                hi[k] = hi[k].max(f64::from(v[k]));
            }
        }
        (lo, hi)
    }

    fn components(mesh: &Mesh) -> usize {
        edge_mesh::split::split(mesh).len()
    }

    #[test]
    fn every_kind_makes_closed_outward_solids_at_several_sizes() {
        let mut cases = Vec::new();
        for kind in FlexiKind::ALL {
            let base = match kind {
                FlexiKind::DoubleRing => ring_params(),
                FlexiKind::BallSocket => ball_params(),
                FlexiKind::ChainLink => chain_params(),
                FlexiKind::Hinge => hinge_params(),
                FlexiKind::Thread => thread_params(),
                FlexiKind::Bayonet => bayonet_params(),
            };
            // The defaults as they stand, and sized to two sections.
            cases.push(FlexiParams {
                kind,
                ..FlexiParams::default()
            });
            cases.push(base);
            cases.push(auto_size(base, 6.0));
            cases.push(auto_size(base, 20.0));
        }
        // The other side, the other hand, turned.
        cases.push(FlexiParams {
            hinge_fold_upper: true,
            rotation: 37.5,
            ..hinge_params()
        });
        cases.push(FlexiParams {
            rotation: 62.5,
            ..chain_params()
        });
        for twist in [thread_params(), bayonet_params()] {
            cases.push(FlexiParams {
                thread_lid_upper: false,
                thread_left_hand: true,
                rotation: 42.5,
                ..twist
            });
        }
        cases.push(FlexiParams {
            bayonet_detent: 0.0,
            ..bayonet_params()
        });
        cases.push(FlexiParams {
            thread_lead_turns: 0.0,
            thread_starts: 1,
            thread_pitch: 3.0,
            ..thread_params()
        });
        for p in &cases {
            assert_eq!(validate(p), None, "{p:?}");
            for (what, mesh) in every_mesh(p) {
                sound(&mesh, &what);
            }
            let preview = preview_body(p);
            assert!(edge_mesh::cut::is_closed(&preview));
            assert_eq!(edge_mesh::cut::flat_triangles(&preview), 0);
        }
    }

    #[test]
    fn revolve_and_sweep_generators() {
        // its_make_revolved: a square ring by Pappus, a sphere.
        let ring = connector::revolved(&[[4.0, -1.0], [6.0, -1.0], [6.0, 1.0], [4.0, 1.0]], 64);
        sound(&ring, "ring");
        assert!((ring.signed_volume() / (4.0 * TAU * 5.0) - 1.0).abs() < 0.01);
        // its_make_swept_loop: a torus, a wobbly loop, and nothing from
        // too little.
        let circle: Vec<[f64; 3]> = (0..96)
            .map(|i| {
                let a = TAU * f64::from(i) / 96.0;
                [5.0 * a.cos(), 5.0 * a.sin(), 0.0]
            })
            .collect();
        let torus = swept_loop(&circle, 1.0, 48);
        sound(&torus, "torus");
        assert!((torus.signed_volume() / (2.0 * PI * PI * 5.0) - 1.0).abs() < 0.02);
        let wobbly: Vec<[f64; 3]> = (0..96)
            .map(|i| {
                let a = TAU * f64::from(i) / 96.0;
                [6.0 * a.cos(), 4.0 * a.sin(), 1.5 * (2.0 * a).sin()]
            })
            .collect();
        sound(&swept_loop(&wobbly, 0.8, 32), "wobbly");
        assert!(
            swept_loop(&[[0.0; 3], [1.0, 0.0, 0.0]], 1.0, 16)
                .triangles
                .is_empty()
        );
        assert!(swept_loop(&circle, 1.0, 2).triangles.is_empty());
        // its_make_helical_sweep.
        let prof = [[-0.5, -0.6], [-0.5, 0.6], [0.5, 0.3], [0.5, -0.3]];
        for starts in [1, 2, 4] {
            let strands = helical_sweep(&prof, 5.0, 3.0, starts, 1.25, 120, false, 0.0, 0.5);
            assert_eq!(strands.len(), starts as usize);
            for s in &strands {
                sound(s, "strand");
                assert_eq!(components(s), 1);
                let ratio = s.signed_volume() / strands[0].signed_volume();
                assert!((ratio - 1.0).abs() < 0.02);
            }
        }
        let flat = helical_sweep(&prof, 5.0, 3.0, 1, 1.0, 120, false, 0.0, 0.0);
        sound(&flat[0], "flat strand");
        assert!(helical_sweep(&prof, 5.0, 3.0, 0, 1.0, 120, false, 0.0, 0.5).is_empty());
        assert!(helical_sweep(&prof, 0.0, 3.0, 1, 1.0, 120, false, 0.0, 0.5).is_empty());
        assert!(helical_sweep(&prof, 5.0, 0.0, 1, 1.0, 120, false, 0.0, 0.5).is_empty());
        assert!(helical_sweep(&prof, 5.0, 3.0, 1, -1.0, 120, false, 0.0, 0.5).is_empty());
        assert!(helical_sweep(&prof, 5.0, 3.0, 1, 1.0, 2, false, 0.0, 0.5).is_empty());
        // A right-hand helix turns anticlockwise as it rises.
        let prof = [[-0.4, -0.5], [-0.4, 0.5], [0.4, 0.25], [0.4, -0.25]];
        let quarter = |left: bool| {
            let s = helical_sweep(&prof, 5.0, 4.0, 1, 1.0, 120, left, 0.0, 0.0);
            *s[0]
                .vertices
                .iter()
                .min_by(|a, b| (a[2] - 1.0).abs().total_cmp(&(b[2] - 1.0).abs()))
                .unwrap()
        };
        assert!(quarter(false)[1] > 3.0);
        assert!(quarter(true)[1] < -3.0);
    }

    #[test]
    fn revolved_kinds_are_captive_and_their_cavity_clears_by_the_clearance() {
        // A ring big enough for a hub too.
        let hub = auto_size(ring_params(), 20.0);
        assert!(hub.hub_radius > 0.2);
        assert_eq!(male_profiles(&hub).len(), 2);
        for p in [ring_params(), ball_params(), hub] {
            assert_eq!(validate(&p), None);
            assert!(is_captive(&p));
            let male = male_bodies(&p);
            let female = female_cavities(&p);
            assert_eq!(male.len(), female.len());
            assert_eq!(upper_bodies(&p).len(), 0);
            assert_eq!(lower_reliefs(&p).len(), 0);
            for (m, f) in male.iter().zip(&female) {
                let gap = min_surface_distance(m, f);
                assert!((gap - p.clearance).abs() < 0.02, "{:?}: {gap}", p.kind);
                assert!(f.signed_volume() > m.signed_volume());
            }
            // The rotation changes nothing for them.
            let turned = male_bodies(&FlexiParams {
                rotation: 90.0,
                ..p
            });
            for (a, b) in male.iter().zip(&turned) {
                assert_eq!(a.signed_volume(), b.signed_volume());
            }
            // The lip's stem starts at the upper face and reaches down past
            // the lower one.
            let (lo, hi) = bounds(&male);
            assert!(lo[2] < -0.5 * effective_gap(&p));
            assert!((hi[2] - protrusion_height(&p)).abs() < 1e-4, "{:?}", p.kind);
        }
        // The ball's neck meets the sphere once: no zero-area band.
        let ball = &male_bodies(&ball_params())[0];
        assert_eq!(edge_mesh::cut::flat_triangles(ball), 0);
    }

    #[test]
    fn guards() {
        let mut p = ring_params();
        assert!(!gap_closing_conflict(&p, 0.049));
        assert!(gap_closing_conflict(&p, 0.2));
        assert!((max_safe_gap_closing_radius(&p) - 0.175).abs() < 1e-9);
        assert!((clearance_floor(0.4) - 0.30).abs() < 1e-9);
        assert!((clearance_floor(0.6) - 0.45).abs() < 1e-9);
        let a = auto_size(p, 10.0);
        assert!((a.outer_radius - 4.0).abs() < 1e-9);
        assert_eq!(a.hub_radius, default_hub_radius(&a));
        p.clearance = 0.0;
        assert_eq!(
            validate(&p),
            Some("Clearance must be greater than zero, otherwise the joint prints as one solid.")
        );
        p = ring_params();
        p.neck_ratio = 0.94;
        assert!(!is_captive(&p));
        assert!(validate(&p).is_some());
        // The gap.
        assert_eq!(default_gap(FlexiKind::ChainLink), 1.5);
        assert_eq!(default_gap(FlexiKind::DoubleRing), 0.6);
        assert_eq!(default_gap(FlexiKind::BallSocket), 0.6);
        assert_eq!(default_gap(FlexiKind::Thread), 0.4);
        let mut p = ring_params();
        assert_eq!(effective_gap(&p), 0.6);
        p.gap = 2.0;
        assert_eq!(effective_gap(&p), 2.0);
        p.gap = 0.1;
        assert_eq!(effective_gap(&p), 0.35);
        assert_eq!(
            validate(&p),
            Some("Gap cannot be smaller than the clearance.")
        );
        // Hinge guards.
        let mut p = hinge_params();
        assert_eq!(validate(&p), None);
        p.hinge_barrel_dia = p.hinge_pin_dia + 2.0 * p.clearance;
        assert!(validate(&p).is_some());
        p = hinge_params();
        p.hinge_knuckles = 9;
        p.hinge_length = 4.0;
        assert!(validate(&p).is_some());
        for n in [0, 10] {
            p = hinge_params();
            p.hinge_knuckles = n;
            assert_eq!(
                validate(&p),
                Some("A hinge needs between 1 and 9 knuckles.")
            );
        }
        p = hinge_params();
        assert!(is_captive(&p));
        assert_eq!(mouth_half_width(&p), hinge_bore_radius(&p));
        assert_eq!(lip_half_width(&p), 0.5 * p.hinge_barrel_dia);
        for inscribed in [5.0, 10.0, 20.0] {
            let a = auto_size(hinge_params(), inscribed);
            assert_eq!(validate(&a), None);
            assert!(a.hinge_barrel_dia > a.hinge_pin_dia + 2.0 * a.clearance);
        }
        assert!(!hinge_axis_needs_care([1.0, 0.0, 0.0]));
        assert!(!hinge_axis_needs_care([1.0, 0.0, 0.05]));
        assert!(hinge_axis_needs_care([0.0, 0.0, 1.0]));
        assert!(hinge_axis_needs_care([1.0, 0.0, -0.5]));
        assert!(!hinge_axis_needs_care([0.0; 3]));
        assert!(!twist_axis_needs_care([0.0, 0.0, -1.0]));
        assert!(!twist_axis_needs_care([0.05, 0.0, 1.0]));
        assert!(twist_axis_needs_care([0.2, 0.0, 1.0]));
        assert!(twist_axis_needs_care([0.0; 3]));
        // Twist guards.
        let t = thread_params();
        assert_eq!(validate(&t), None);
        let refused = |q: FlexiParams| assert!(validate(&q).is_some(), "{q:?}");
        refused(FlexiParams {
            thread_major_dia: 3.0,
            ..t
        });
        refused(FlexiParams {
            thread_starts: 5,
            ..t
        });
        refused(FlexiParams {
            thread_starts: 0,
            ..t
        });
        refused(FlexiParams {
            thread_turns: 0.0,
            ..t
        });
        refused(FlexiParams {
            thread_pitch: 0.5,
            ..t
        });
        refused(FlexiParams {
            thread_lead_turns: 1.0,
            ..t
        });
        refused(FlexiParams {
            thread_starts: 4,
            thread_pitch: 2.0,
            clearance: 0.4,
            ..t
        });
        refused(FlexiParams {
            thread_major_dia: 4.2,
            thread_pitch: 12.0,
            ..t
        });
        let s = thread_crest_spacing(&t);
        let h = thread_profile_height(&t, thread_depth(&t)) + thread_groove_growth(&t);
        assert!(h < s && s - h > 0.15);
        assert!((max_safe_gap_closing_radius(&t) - 0.125).abs() < 1e-9);
        let b = bayonet_params();
        assert_eq!(validate(&b), None);
        for q in [
            FlexiParams {
                bayonet_lugs: 5,
                ..b
            },
            FlexiParams {
                bayonet_lugs: 1,
                ..b
            },
            FlexiParams {
                bayonet_lug_height: 0.2,
                ..b
            },
            FlexiParams {
                bayonet_lug_thickness: 0.3,
                ..b
            },
            FlexiParams {
                bayonet_lug_height: 7.0,
                ..b
            },
            FlexiParams {
                bayonet_lug_arc: 85.0,
                ..b
            },
            FlexiParams {
                bayonet_entry_depth: 1.0,
                ..b
            },
            FlexiParams {
                bayonet_detent: 2.0,
                ..b
            },
            FlexiParams {
                bayonet_detent: -0.1,
                ..b
            },
        ] {
            refused(q);
        }
        // The defaults make a valid joint of every kind.
        for kind in FlexiKind::ALL {
            let p = FlexiParams {
                kind,
                ..FlexiParams::default()
            };
            assert_eq!(validate(&p), None, "{kind:?}");
        }
    }

    #[test]
    fn chain_link_rings_stand_perpendicular_interlocked_and_clear() {
        let p = chain_params();
        let min = min_link_size(&p);
        assert!((min - (4.0 * p.wire + 2.0 * p.clearance)).abs() < 1e-12);
        assert!(
            validate(&FlexiParams {
                link_width: 0.9 * min,
                ..p
            })
            .is_some()
        );
        for deg in [0.0, 17.0, 45.0, 90.0, 133.0, 180.0] {
            let q = FlexiParams { rotation: deg, ..p };
            assert_eq!(validate(&q), None);
            let lower = &lower_bodies(&q)[0];
            let upper = &upper_bodies(&q)[0];
            let a = f64::to_radians(deg);
            // Both planes contain the normal and are square to each other:
            // the lower ring's normal is the turned +y, the upper's the
            // turned +x.
            let (ca, cb) = (centroid(lower), centroid(upper));
            assert!(off_plane(lower, ca, [-a.sin(), a.cos(), 0.0]) < p.wire + 1e-3);
            assert!(off_plane(upper, cb, [a.cos(), a.sin(), 0.0]) < p.wire + 1e-3);
            assert!(ca[2] < 0.0 && cb[2] > 0.0);
            assert!((ca[2] + cb[2]).abs() < 1e-4);
            assert!(ca[0].hypot(ca[1]) < 1e-4);
            assert!((outer_extent(&q) - outer_extent(&p)).abs() < 1e-4);
            assert!(intersection_volume(lower, upper) < 1e-3);
            if deg == 0.0 {
                assert!(min_surface_distance(lower, upper) >= p.clearance - 0.01);
            }
        }
        for gap in [0.35, 1.5, 2.5] {
            let q = FlexiParams { gap, ..p };
            if validate(&q).is_none() {
                assert!(intersection_volume(&lower_bodies(&q)[0], &upper_bodies(&q)[0]) < 1e-3);
            }
        }
        // The relief is the other ring grown by the clearance.
        let relief = &lower_reliefs(&p)[0];
        let ring = &upper_bodies(&p)[0];
        assert!((min_surface_distance(relief, ring) - p.clearance).abs() < 0.01);
        // The centrelines: empty for other kinds.
        assert!(chain_lower_centreline(&ring_params()).is_empty());
        assert_eq!(chain_upper_centreline(&p).len(), 2 * (2 + CAP_STEPS - 1));
        for inscribed in [5.0, 10.0, 20.0] {
            let q = auto_size(chain_params(), inscribed);
            assert_eq!(validate(&q), None);
            assert!(outer_extent(&q) <= inscribed);
            assert!(q.link_length >= min_link_size(&q) && q.link_width >= min_link_size(&q));
        }
    }

    #[test]
    fn hinge_knuckles_alternate_and_the_pin_runs_through() {
        for n in [1, 2, 3, 4, 5, 9] {
            let p = FlexiParams {
                hinge_knuckles: n,
                ..hinge_params()
            };
            assert_eq!(hinge_knuckle_count(&p), n);
            assert_eq!(validate(&p), None);
            for i in 0..n - 1 {
                assert_ne!(
                    hinge_knuckle_is_lower(&p, i),
                    hinge_knuckle_is_lower(&p, i + 1)
                );
            }
            assert!(hinge_knuckle_is_lower(&p, 0));
            let f = FlexiParams {
                hinge_fold_upper: true,
                ..p
            };
            for i in 0..n {
                assert_eq!(
                    hinge_knuckle_is_lower(&f, i),
                    !hinge_knuckle_is_lower(&p, i)
                );
            }
            assert_eq!(male_bodies(&p).len(), (n as usize).div_ceil(2) + 1);
            assert_eq!(female_cavities(&p).len(), n as usize / 2);
            assert_eq!(lower_bodies(&f).len(), n as usize / 2);
            assert_eq!(upper_bodies(&f).len(), (n as usize).div_ceil(2) + 1);
            let mid = 0.5 * (hinge_slot_centre(&p, 0) + hinge_slot_centre(&p, n - 1));
            assert!(mid.abs() < 1e-9);
        }
        let p = hinge_params();
        let bodies = male_bodies(&p);
        assert_eq!(bodies.len(), 3);
        let pin = &bodies[2];
        assert_eq!(components(pin), 1);
        let x0 = hinge_slot_centre(&p, 0) - 0.5 * hinge_knuckle_length(&p);
        let x1 = hinge_slot_centre(&p, 2) + 0.5 * hinge_knuckle_length(&p);
        let (lo, hi) = bounds(std::slice::from_ref(pin));
        assert!((lo[0] - x0).abs() < 1e-5 && (hi[0] - x1).abs() < 1e-5);
        let r = 0.5 * p.hinge_pin_dia;
        let area = 0.5 * SECTORS as f64 * (TAU / SECTORS as f64).sin() * r * r;
        assert!((pin.signed_volume() / (area * (x1 - x0)) - 1.0).abs() < 1e-4);
        let axis_y = 0.5 * (lo[1] + hi[1]);
        assert!((axis_y - hinge_axis_y(&p)).abs() < 1e-4);
        // The other half's knuckle has the pin through it, with its bore.
        assert!(hinge_slot_centre(&p, 1) > x0 && hinge_slot_centre(&p, 1) < x1);
        let bore = upper_reliefs(&p).pop().unwrap();
        // As long as the pin, and the clearance wider all round.
        let rb = r + p.clearance;
        let bore_area = 0.5 * SECTORS as f64 * (TAU / SECTORS as f64).sin() * rb * rb;
        assert!((bore.signed_volume() / (bore_area * (x1 - x0)) - 1.0).abs() < 1e-4);
        let (blo, bhi) = bounds(std::slice::from_ref(&bore));
        assert!((blo[0] - lo[0]).abs() < 1e-5 && (bhi[0] - hi[0]).abs() < 1e-5);
        assert!((0.5 * (bhi[1] - blo[1]) - rb).abs() < 1e-4);
        // The end faces are a gap apart.
        for gap in [0.4, 0.6, 1.2] {
            let q = FlexiParams { gap, ..p };
            let len = hinge_knuckle_length(&q);
            assert!((hinge_slot_length(&q) - len - gap).abs() < 1e-9);
            for i in 0..2 {
                let apart = (hinge_slot_centre(&q, i + 1) - 0.5 * len)
                    - (hinge_slot_centre(&q, i) + 0.5 * len);
                assert!((apart - gap).abs() < 1e-9);
            }
        }
        // The footprint: inside the 20 mm face, its far corners on the rim.
        for c in hinge_footprint_corners(&p, 0.0) {
            assert!(c[0].hypot(c[1]) <= 10.0 + 1e-6);
        }
        let far = (0.5 * p.hinge_length).hypot(-hinge_axis_y(&p) + 0.5 * p.hinge_barrel_dia);
        assert!(far < 10.0 && far > 9.85);
        let too_far = FlexiParams {
            hinge_edge_offset: p.hinge_edge_offset + 0.5,
            ..p
        };
        assert!(
            hinge_footprint_corners(&too_far, 0.0)
                .iter()
                .any(|c| c[0].hypot(c[1]) > 10.0)
        );
        // Turned a quarter, the rectangle and the run turn with it.
        let span = |corners: &[[f64; 2]]| {
            let xs = corners.iter().map(|c| c[0]);
            let ys = corners.iter().map(|c| c[1]);
            (
                xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min),
                ys.clone().fold(f64::MIN, f64::max) - ys.fold(f64::MAX, f64::min),
            )
        };
        let r90 = FlexiParams {
            rotation: 90.0,
            ..p
        };
        let (x, y) = span(&hinge_footprint_corners(&p, 0.0));
        assert!((x - p.hinge_length).abs() < 1e-9 && (y - p.hinge_barrel_dia).abs() < 1e-9);
        let (rx, ry) = span(&hinge_footprint_corners(&r90, 0.0));
        assert!((rx - y).abs() < 1e-6 && (ry - x).abs() < 1e-6);
        let (lo, hi) = bounds(&male_bodies(&r90));
        assert!((0.5 * (lo[0] + hi[0]) + hinge_axis_y(&p)).abs() < 1e-3);
        assert!(((hi[1] - lo[1]) - (p.hinge_length - p.gap)).abs() < 1e-3);
        // The two halves' knuckles do not overlap, and an upper knuckle
        // bored by its relief leaves the pin free.
        let lower = male_bodies(&p);
        let (pin, knuckles) = lower.split_last().unwrap();
        let bore = upper_reliefs(&p).pop().unwrap();
        for upper in female_cavities(&p) {
            for l in knuckles {
                assert!(intersection_volume(l, &upper) < 1e-6);
            }
            assert!(intersection_volume(pin, &upper) > 1.0);
            let bored = boolean::difference(&upper, &bore).unwrap();
            assert!(intersection_volume(pin, &bored) < 1e-6);
        }
    }

    #[test]
    fn footprints() {
        let p = chain_params();
        let corners = chain_footprint_corners(&p, 0.0);
        assert_eq!(corners.len(), 4);
        let true_r = corners.iter().map(|c| c[0].hypot(c[1])).fold(0.0, f64::max);
        let fits = |q: &FlexiParams, r: f64| {
            footprint_corners(q, 0.0)
                .iter()
                .all(|c| c[0].hypot(c[1]) <= r)
        };
        assert!(!fits(&p, 0.5 * true_r));
        assert!(fits(&p, true_r + 1e-9));
        let h = FlexiParams {
            hinge_length: 16.0,
            hinge_edge_offset: 0.0,
            ..hinge_params()
        };
        let corner = 8f64.hypot(2.0);
        assert!(fits(&h, corner + 1e-6) && !fits(&h, corner - 0.5));
        let ring = ring_params();
        let fp = footprint_corners(&ring, 0.0);
        assert_eq!(fp.len(), 60);
        for c in &fp {
            assert!((c[0].hypot(c[1]) - outer_extent(&ring)).abs() < 1e-9);
        }
        for t in [thread_params(), bayonet_params()] {
            let want = thread_bore_radius(&t) + thread_bore_wall();
            assert!((outer_extent(&t) - want).abs() < 1e-12);
            assert!(fits(&t, 10.0));
            for c in footprint_corners(&t, 0.0) {
                assert!((c[0].hypot(c[1]) - want).abs() < 1e-9);
            }
            for r_in in [6.0, 10.0, 25.0] {
                let a = auto_size(t, r_in);
                assert_eq!(validate(&a), None, "{r_in} {:?}", t.kind);
                assert!(outer_extent(&a) <= r_in);
                assert!(a.thread_major_dia > 0.4 * r_in);
            }
        }
    }

    #[test]
    fn the_thread_groove_is_the_thread_grown_by_the_clearance() {
        let p = thread_params();
        let lid = upper_bodies(&p);
        let relief = lower_reliefs(&p);
        let starts = thread_start_count(&p) as usize;
        assert_eq!(lid.len(), 1 + starts);
        assert_eq!(relief.len(), 1 + starts);
        assert!(lower_bodies(&p).is_empty() && upper_reliefs(&p).is_empty());
        assert!(relief[1].signed_volume() > lid[1].signed_volume());
        let plain = thread_profile(&p, 0.0);
        let grown = thread_profile_grown(&p, 0.0, p.clearance);
        let most =
            |prof: &[[f64; 2]], k: usize| prof.iter().map(|v| v[k].abs()).fold(0.0, f64::max);
        assert!((most(&grown, 0) - most(&plain, 0) - p.clearance).abs() < 1e-9);
        assert!(most(&grown, 1) > most(&plain, 1) + 0.5 * p.clearance);
        // Every edge of the thread, and every corner, clears the groove by
        // the clearance.
        for i in 0..4 {
            let (a, b) = (plain[i], plain[(i + 1) % 4]);
            let mid = [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])];
            let e = [b[0] - a[0], b[1] - a[1]];
            let len = e[0].hypot(e[1]);
            let n = [e[1] / len, -e[0] / len];
            let mut reach = f64::INFINITY;
            for j in 0..4 {
                let (c, d) = (grown[j], grown[(j + 1) % 4]);
                let f = [d[0] - c[0], d[1] - c[1]];
                let den = n[0] * f[1] - n[1] * f[0];
                if den.abs() < 1e-12 {
                    continue;
                }
                let w = [c[0] - mid[0], c[1] - mid[1]];
                let t = (w[0] * f[1] - w[1] * f[0]) / den;
                let s = (w[0] * n[1] - w[1] * n[0]) / -den;
                if t > 1e-9 && (-1e-9..=1.0 + 1e-9).contains(&s) {
                    reach = reach.min(t);
                }
            }
            assert!(reach >= p.clearance - 1e-6, "edge {i}: {reach}");
        }
        for q in &plain {
            for j in 0..4 {
                let (c, d) = (grown[j], grown[(j + 1) % 4]);
                let f = [d[0] - c[0], d[1] - c[1]];
                let t = (((q[0] - c[0]) * f[0] + (q[1] - c[1]) * f[1])
                    / (f[0] * f[0] + f[1] * f[1]))
                    .clamp(0.0, 1.0);
                let gap = (q[0] - c[0] - t * f[0]).hypot(q[1] - c[1] - t * f[1]);
                assert!(gap >= p.clearance - 1e-6);
            }
        }
        // Anchored above the lid face, reaching down through the band.
        let face = 0.5 * effective_gap(&p);
        let (lo, hi) = bounds(&lid);
        assert!(hi[2] > face && lo[2] < face - thread_axial_length(&p));
        // The lid below: mirrored.
        let below = FlexiParams {
            thread_lid_upper: false,
            ..p
        };
        let male = lower_bodies(&below);
        assert_eq!(male.len(), 1 + starts);
        assert!(upper_bodies(&below).is_empty());
        let (lo, hi) = bounds(&male);
        assert!(lo[2] < -face && hi[2] > -face + thread_axial_length(&p));
        // The rotation is the start angle: the strand's lowest ring turns
        // with it.
        let start = |q: &FlexiParams| {
            let strand = &upper_bodies(q)[1];
            let low = strand
                .vertices
                .iter()
                .map(|v| v[2])
                .fold(f32::MAX, f32::min);
            let (mut x, mut y) = (0.0, 0.0);
            for v in strand.vertices.iter().filter(|v| v[2] < low + 0.3) {
                x += f64::from(v[0]);
                y += f64::from(v[1]);
            }
            y.atan2(x).to_degrees()
        };
        let turned = FlexiParams {
            rotation: 90.0,
            ..p
        };
        let diff = (start(&turned) - start(&p) + 540.0).rem_euclid(360.0) - 180.0;
        assert!((diff - 90.0).abs() < 3.0, "{diff}");
    }

    /// Every lid body, less the body half's reliefs, leaves nothing on the
    /// body's side of the cut: the strands, lead-ins included, sit wholly
    /// in their grooves. The reference's radial-only fade fails this at
    /// the strands' ends.
    #[test]
    fn the_thread_lies_inside_its_groove_on_the_body_side() {
        let base = FlexiParams {
            kind: FlexiKind::Thread,
            ..FlexiParams::default()
        };
        let cases = [
            auto_size(base, 20.0),
            auto_size(base, 6.0),
            thread_params(),
            FlexiParams {
                thread_starts: 1,
                thread_pitch: 3.0,
                thread_left_hand: true,
                ..thread_params()
            },
            FlexiParams {
                thread_starts: 3,
                thread_pitch: 9.0,
                thread_lid_upper: false,
                rotation: 42.5,
                ..thread_params()
            },
            FlexiParams {
                thread_lid_upper: false,
                thread_left_hand: true,
                thread_turns: 2.0,
                thread_lead_turns: 0.25,
                ..auto_size(base, 12.0)
            },
        ];
        for p in cases {
            assert_eq!(validate(&p), None, "{p:?}");
            let face = 0.5 * effective_gap(&p);
            let (bodies, reliefs, side) = if p.thread_lid_upper {
                (upper_bodies(&p), lower_reliefs(&p), [-1e3, -face])
            } else {
                (lower_bodies(&p), upper_reliefs(&p), [face, 1e3])
            };
            let body_side = Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
                [-1e3, -1e3, side[0] as f32],
                [1e3, 1e3, side[1] as f32],
            ));
            for (i, body) in bodies.iter().enumerate() {
                let mut left = body.clone();
                for relief in &reliefs {
                    if left.triangles.is_empty() {
                        break;
                    }
                    left = boolean::difference(&left, relief).unwrap();
                }
                let stray = if left.triangles.is_empty() {
                    0.0
                } else {
                    intersection_volume(&left, &body_side)
                };
                assert!(stray < 1e-6, "body {i}: {stray} mm³ of {p:?}");
            }
        }
    }

    #[test]
    fn bayonet_lugs_stand_on_the_plug_and_the_slots_are_cut() {
        let p = bayonet_params();
        let lid = upper_bodies(&p);
        assert_eq!(lid.len(), 1 + bayonet_lug_count(&p) as usize);
        assert!(lower_bodies(&p).is_empty());
        let rmax = lid[1..]
            .iter()
            .flat_map(|m| &m.vertices)
            .map(|v| f64::from(v[0]).hypot(f64::from(v[1])))
            .fold(0.0, f64::max);
        assert!((rmax - thread_major_radius(&p)).abs() < 0.05);
        // The bore, then per lug the entry, the two runs of track and the
        // sliver inboard of the detent; without the detent, one track.
        assert_eq!(lower_reliefs(&p).len(), 1 + 3 * 4);
        let plain = FlexiParams {
            bayonet_detent: 0.0,
            ..p
        };
        assert_eq!(lower_reliefs(&plain).len(), 1 + 3 * 2);
        // Locked, each lug is clear of its own slot by the clearance and
        // inside it entirely.
        let lugs = &lid[1..];
        let reliefs = lower_reliefs(&plain);
        for (i, lug) in lugs.iter().enumerate() {
            let track = &reliefs[2 + 2 * i];
            let inside = boolean::intersection(lug, track).unwrap().signed_volume();
            assert!((inside - lug.signed_volume()).abs() < 1e-3 * lug.signed_volume());
        }
        assert!((bayonet_effective_lock_angle(&p) - 75.0).abs() < 1e-12);
        assert!((bayonet_lug_angle_deg(&p, 1) - 120.0).abs() < 1e-12);
    }

    #[test]
    fn attributes_round_trip_and_clamp_as_the_reader_does() {
        let mut thread = thread_params();
        thread.thread_major_dia = 13.5;
        thread.thread_pitch = 4.0;
        thread.thread_starts = 3;
        thread.thread_turns = 1.75;
        thread.thread_lead_turns = 0.375;
        thread.thread_left_hand = true;
        thread.thread_lid_upper = false;
        thread.rotation = 42.5;
        let mut hinge = hinge_params();
        hinge.hinge_knuckles = 5;
        hinge.hinge_pin_dia = 2.5;
        hinge.hinge_barrel_dia = 5.5;
        hinge.hinge_length = 15.0;
        hinge.hinge_edge_offset = 2.75;
        hinge.hinge_fold_upper = true;
        hinge.rotation = 37.5;
        let chain = FlexiParams {
            rotation: 62.5,
            wire: 1.1,
            stem: 1.7,
            ..chain_params()
        };
        let bayonet = FlexiParams {
            thread_major_dia: 14.5,
            thread_lid_upper: false,
            bayonet_lugs: 4,
            bayonet_lug_height: 1.25,
            bayonet_lug_thickness: 3.25,
            bayonet_lug_arc: 22.5,
            bayonet_lock_angle: 45.0,
            bayonet_entry_depth: 5.5,
            bayonet_detent: 0.45,
            rotation: 17.5,
            ..bayonet_params()
        };
        for p in [ring_params(), ball_params(), chain, hinge, thread, bayonet] {
            assert_eq!(validate(&p), None, "{:?}", p.kind);
            let written = attributes(&p);
            assert_eq!(written.len(), 36);
            let back = from_attributes(|name| {
                written
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| *value)
            });
            assert_eq!(back, p);
            // As the reference writes them: in single precision.
            let back = from_attributes(|name| {
                written
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| f64::from(*value as f32))
            });
            assert_eq!(back, p);
        }
        // An older file: every value its default.
        assert_eq!(from_attributes(|_| None), FlexiParams::default());
        assert_eq!(FlexiParams::default().rotation, 0.0);
        // Untrusted values: a kind out of range, counts out of range, a
        // count that is not whole, a value that is not a number.
        let odd = from_attributes(|name| match name {
            "flexi_kind" => Some(7.0),
            "flexi_hinge_knuckles" => Some(12.0),
            "flexi_thread_starts" => Some(0.0),
            "flexi_bayonet_lugs" => Some(2.5),
            "flexi_wire" => Some(f64::NAN),
            "flexi_hinge_fold_upper" => Some(2.0),
            _ => None,
        });
        assert_eq!(odd.kind, FlexiKind::DoubleRing);
        assert_eq!(odd.hinge_knuckles, 9);
        assert_eq!(odd.thread_starts, 1);
        assert_eq!(odd.bayonet_lugs, 3);
        assert_eq!(odd.wire, 1.0);
        assert!(odd.hinge_fold_upper);
        assert_eq!(
            from_attributes(|name| (name == "flexi_kind").then_some(5.0)).kind,
            FlexiKind::Bayonet
        );
        assert_eq!(
            from_attributes(|name| (name == "flexi_bayonet_lugs").then_some(1.0)).bayonet_lugs,
            2
        );
    }

    #[test]
    fn measures() {
        let ring = ring_params();
        assert!((ring.hub_radius - 0.0).abs() < 1e-12); // 4 - 2 - 0.35 - 2 < 0.2
        assert!((protrusion_height(&ring) - (1.5 + 0.3)).abs() < 1e-12);
        let hinge = hinge_params();
        assert_eq!(protrusion_height(&hinge), 2.0);
        assert!(is_two_sided(&hinge) && !is_two_sided(&ring));
        let t = thread_params();
        assert!((protrusion_height(&t) - (thread_axial_length(&t) + 0.6 + 0.2)).abs() < 1e-12);
        assert!(thread_helix_angle_deg(&t) > 0.0 && thread_helix_angle_deg(&t) < 45.0);
        assert!(thread_minor_radius(&t) > 1.2);
        // The same input, the same bytes.
        for kind in FlexiKind::ALL {
            let p = FlexiParams {
                kind,
                ..FlexiParams::default()
            };
            let (a, b) = (preview_body(&p), preview_body(&p));
            assert_eq!(a.vertices, b.vertices);
            assert_eq!(a.triangles, b.triangles);
        }
    }
}
