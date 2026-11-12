//! "Cut" along a line drawn on the part (the reference fork's Draw cut,
//! libslic3r/DrawCut.cpp and DrawCut.hpp; the stored form from
//! CutRecipe.cpp). The user draws a stroke on a part's skin; a cut surface
//! is built from it, closed into a solid, and the part is cut with that
//! solid by [`crate::curved::cut_with_solid`], the boolean the curved cut
//! uses.
//!
//! An OPEN stroke gives a ruled strip: every sample gets a straight ruling
//! along the cut direction (the inward surface normal, the view, or an
//! axis), from `extension` out of the skin to `depth` (or through the
//! part) into it, the ends carried on along the tangent by `extension`;
//! the strip swept sideways past the part is the cutter, a half-space.
//!
//! A CLOSED loop gives a band and a flat core: the drawn line itself, a
//! skirt `extension` out of the skin, a band `depth` in along the lip
//! angle, and a flat core plate where the band arrives. It is closed into
//! a plug (a loop on a face) or a half-space (a loop round the part), or
//! with "through all" into a prism along the core's normal.
//!
//! Everything is in the cut plane's own frame, as in the reference.
//!
//! Left out, as only the gizmo's interaction needs them: the chain's undo,
//! its close-along-a-path and the screen-space end pick
//! (`undo_last_stroke`, `close_along_path`, `draw_cut_chain_end_at_pixel`,
//! `draw_cut_chain_snap_radius`, `draw_cut_distance_to_polyline`), the
//! capture's skim filter (`draw_cut_filter_capture_skims`), the surface
//! queries connectors stand on (`draw_cut_surface_*`, the curvature and
//! tilt checks) and the shader's voxel field (`draw_cut_inside_field`).

use edge_geom::{ExPolygon, Point2, Polygon, unscaled};
use edge_mesh::Mesh;
use edge_mesh::boolean::{Cancel, Solid};
use edge_mesh::raycast::Tree;

use crate::CutThickness;
use crate::curved::{self, CurvedError};

type V = [f64; 3];

/// A box in the plane's frame, lowest and highest corner.
pub type Bounds = (V, V);

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn neg(a: V) -> V {
    [-a[0], -a[1], -a[2]]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: V) -> f64 {
    dot(a, a).sqrt()
}
/// `a` less its part along the unit `n`.
fn reject(a: V, n: V) -> V {
    sub(a, mul(n, dot(a, n)))
}
/// `safe_normalize`.
fn unit(v: V, fallback: V) -> V {
    let n = norm(v);
    if n > 1e-12 { mul(v, 1.0 / n) } else { fallback }
}
fn f32s(v: V) -> [f32; 3] {
    v.map(|c| c as f32)
}

const X: V = [1.0, 0.0, 0.0];
const Y: V = [0.0, 1.0, 0.0];
const Z: V = [0.0, 0.0, 1.0];

// ---------------------------------------------------------------------------
// The types.
// ---------------------------------------------------------------------------

/// One captured point of a stroke (`DrawCutSample`): where the ray met the
/// model, the surface's unit normal there and which facet it was, in the
/// plane's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawCutSample {
    pub pos: V,
    pub normal: V,
    pub facet: usize,
}

impl Default for DrawCutSample {
    fn default() -> Self {
        Self {
            pos: [0.0; 3],
            normal: Z,
            facet: 0,
        }
    }
}

/// Which way the cut surface reaches into the part (`DrawCutDirection`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawCutDirection {
    /// Along the inward surface normal at each sample.
    #[default]
    SurfaceNormal,
    /// Along the camera's direction, `DrawCutParams::view_dir`.
    View,
    AxisX,
    AxisY,
    AxisZ,
}

impl DrawCutDirection {
    /// The direction a stored number names, as the recipe reads it
    /// (`CutRecipe::draw_params`): anything out of range is the surface
    /// normal.
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::View,
            2 => Self::AxisX,
            3 => Self::AxisY,
            4 => Self::AxisZ,
            _ => Self::SurfaceNormal,
        }
    }

    pub fn index(self) -> i32 {
        match self {
            Self::SurfaceNormal => 0,
            Self::View => 1,
            Self::AxisX => 2,
            Self::AxisY => 3,
            Self::AxisZ => 4,
        }
    }
}

/// What the surface is made with (`DrawCutParams`); see the reference's
/// header for each value's meaning.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawCutParams {
    pub direction: DrawCutDirection,
    /// For [`DrawCutDirection::View`] (and the plug's lid): the camera's
    /// direction into the part, in the plane's frame; need not be unit.
    pub view_dir: V,
    /// How far the surface reaches past the stroke, mm.
    pub extension: f64,
    /// Per-rail clamp on an open stroke's extension, in rail order: the
    /// front tangent end, samples `0..n`, the back tangent end. Empty (or
    /// of another length) is no clamp.
    pub extension_clearance: Vec<f64>,
    /// The lip angle, degrees, -90..90: positive leans the band along -n.
    pub angle_deg: f64,
    /// The skirt's own angle; none is "continue the band" (the same as
    /// `angle_deg`).
    pub extension_angle_deg: Option<f64>,
    /// No core: a prism along the core's normal through the part.
    pub through_all: bool,
    /// How far the band travels in before turning onto the core, mm.
    pub depth: f64,
    /// Kerf, mm, and where it is taken from.
    pub thickness: f64,
    pub thickness_offset: CutThickness,
}

impl Default for DrawCutParams {
    fn default() -> Self {
        Self {
            direction: DrawCutDirection::SurfaceNormal,
            view_dir: [0.0, 0.0, -1.0],
            extension: 5.0,
            extension_clearance: Vec::new(),
            angle_deg: 0.0,
            extension_angle_deg: None,
            through_all: false,
            depth: 3.0,
            thickness: 0.0,
            thickness_offset: CutThickness::Centred,
        }
    }
}

/// Why a stroke cannot be cut with (`DrawCutError`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawCutError {
    #[default]
    None,
    TooShort,
    SelfCrossing,
    LeavesMesh,
    CutterDegenerate,
    EmptySide,
    NotClosed,
}

/// The panel's words for an error, untranslated, exactly as the reference
/// has them (`draw_cut_error_message`).
pub fn draw_cut_error_message(error: DrawCutError) -> &'static str {
    match error {
        DrawCutError::None => "",
        DrawCutError::TooShort => "Draw a longer line on the model",
        DrawCutError::SelfCrossing => "The line crosses itself",
        DrawCutError::LeavesMesh => "The line leaves the model",
        DrawCutError::CutterDegenerate => "The line does not make a usable cut surface",
        DrawCutError::EmptySide => "The stroke does not separate the part",
        DrawCutError::NotClosed => "Carry on from an end of the line until it closes on the other",
    }
}

/// What the panel says for an error (GLGizmoCut.cpp, the stroke's status):
/// the translated words, each a message id, where it has them, otherwise
/// [`draw_cut_error_message`]'s.
pub fn draw_cut_error_words(error: DrawCutError) -> &'static str {
    match error {
        DrawCutError::TooShort => "Draw a longer line on the model.",
        DrawCutError::SelfCrossing => "The line crosses itself.",
        DrawCutError::LeavesMesh => "The line leaves the model.",
        DrawCutError::CutterDegenerate => "The line does not make a usable cut surface.",
        DrawCutError::EmptySide => "The stroke does not separate the part.",
        other => draw_cut_error_message(other),
    }
}

impl std::fmt::Display for DrawCutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(draw_cut_error_message(*self))
    }
}

impl std::error::Error for DrawCutError {}

/// Default resample spacing, mm (`DrawCutStroke::DefaultSpacing`).
pub const DEFAULT_SPACING: f64 = 1.0;
/// The fewest samples and the least length a stroke can be cut with.
pub const MIN_SAMPLES: usize = 4;
pub const MIN_LENGTH: f64 = 2.0;
/// The passes the panel's smoothing of 1 asks for.
pub const MAX_SMOOTH_PASSES: i32 = 10;
/// The open strip's draft limit (`DrawCutMaxAngleDeg`).
pub const MAX_ANGLE_DEG: f64 = 60.0;
/// The lip angle's and the skirt angle's range.
pub const MIN_LIP_ANGLE_DEG: f64 = -90.0;
pub const MAX_LIP_ANGLE_DEG: f64 = 90.0;
pub const MIN_EXT_ANGLE_DEG: f64 = -90.0;
pub const MAX_EXT_ANGLE_DEG: f64 = 90.0;
/// The fewest samples a chain can be closed with.
pub const MIN_CHAIN_SAMPLES: usize = 6;

// ---------------------------------------------------------------------------
// Resampling and smoothing.
// ---------------------------------------------------------------------------

/// The polyline resampled to `spacing` of arc length, the normals lerped
/// and made unit again (`draw_cut_resample`). An open stroke keeps its
/// last sample unless it lands within half a spacing of the one before; a
/// closed one walks its closing span too and stays open in the output.
pub fn draw_cut_resample(
    input: &[DrawCutSample],
    spacing: f64,
    closed: bool,
) -> Vec<DrawCutSample> {
    if input.len() < 2 || spacing <= 0.0 {
        return input.to_vec();
    }
    let mut src: Vec<DrawCutSample> = Vec::with_capacity(input.len() + 1);
    src.push(input[0]);
    for s in &input[1..] {
        if norm(sub(s.pos, src[src.len() - 1].pos)) > 1e-12 {
            src.push(*s);
        }
    }
    if closed && norm(sub(src[src.len() - 1].pos, src[0].pos)) > 1e-12 {
        src.push(src[0]);
    }
    if src.len() < 2 {
        return input.to_vec();
    }
    let mut acc = vec![0.0; src.len()];
    for i in 1..src.len() {
        acc[i] = acc[i - 1] + norm(sub(src[i].pos, src[i - 1].pos));
    }
    let total = acc[acc.len() - 1];
    if total < 1e-12 {
        return input.to_vec();
    }
    let mut hint = 0usize;
    let mut sample_at = |d: f64| {
        while hint + 2 < src.len() && acc[hint + 1] < d {
            hint += 1;
        }
        let span = acc[hint + 1] - acc[hint];
        let t = if span > 1e-12 {
            ((d - acc[hint]) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (a, b) = (src[hint], src[hint + 1]);
        DrawCutSample {
            pos: add(a.pos, mul(sub(b.pos, a.pos), t)),
            normal: unit(add(mul(a.normal, 1.0 - t), mul(b.normal, t)), a.normal),
            facet: if t < 0.5 { a.facet } else { b.facet },
        }
    };
    let steps = (total / spacing).floor() as usize;
    let mut out: Vec<DrawCutSample> = Vec::with_capacity(steps + 2);
    for k in 0..=steps {
        out.push(sample_at(k as f64 * spacing));
    }
    if closed {
        while out.len() > 3 && norm(sub(out[out.len() - 1].pos, out[0].pos)) < 0.5 * spacing {
            out.pop();
        }
    } else {
        let last = src[src.len() - 1];
        if norm(sub(out[out.len() - 1].pos, last.pos)) > 0.5 * spacing {
            out.push(last);
        }
    }
    out
}

/// `passes` windowed-average passes over positions and normals, each
/// sample blended `strength` towards the mean of its neighbours
/// (`draw_cut_smooth`); an open stroke's ends are held.
pub fn draw_cut_smooth(path: &mut Vec<DrawCutSample>, passes: i32, closed: bool, strength: f64) {
    if passes <= 0 || path.len() < 3 {
        return;
    }
    let w = strength.clamp(0.0, 1.0);
    if w <= 0.0 {
        return;
    }
    let n = path.len();
    let mut next = vec![DrawCutSample::default(); n];
    for _ in 0..passes {
        for i in 0..n {
            if !closed && (i == 0 || i + 1 == n) {
                next[i] = path[i];
                continue;
            }
            let (prev, cur, nxt) = (path[(i + n - 1) % n], path[i], path[(i + 1) % n]);
            next[i] = DrawCutSample {
                pos: add(mul(cur.pos, 1.0 - w), mul(add(prev.pos, nxt.pos), 0.5 * w)),
                normal: unit(
                    add(
                        mul(cur.normal, 1.0 - w),
                        mul(add(prev.normal, nxt.normal), 0.5 * w),
                    ),
                    cur.normal,
                ),
                facet: cur.facet,
            };
        }
        std::mem::swap(path, &mut next);
    }
}

/// The passes a panel smoothing in [0, 1] asks for (`draw_cut_smooth_passes`).
pub fn draw_cut_smooth_passes(smoothing: f64) -> i32 {
    (smoothing.clamp(0.0, 1.0) * f64::from(MAX_SMOOTH_PASSES)).round() as i32
}

/// How near its first sample a stroke's last must come for it to be a
/// loop (`draw_cut_closing_tolerance`).
pub fn draw_cut_closing_tolerance(spacing: f64) -> f64 {
    (3.0 * spacing.max(0.0)).max(2.0)
}

// ---------------------------------------------------------------------------
// The stroke.
// ---------------------------------------------------------------------------

/// A stroke (`DrawCutStroke`): the raw samples as captured and, once
/// [`DrawCutStroke::finish`] has run, the resampled and smoothed path and
/// its binormals.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawCutStroke {
    samples: Vec<DrawCutSample>,
    path: Vec<DrawCutSample>,
    binormal: Vec<V>,
    closed: bool,
    error: DrawCutError,
}

impl Default for DrawCutStroke {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            path: Vec::new(),
            binormal: Vec::new(),
            closed: false,
            error: DrawCutError::TooShort,
        }
    }
}

impl DrawCutStroke {
    pub fn new() -> Self {
        Self::default()
    }

    /// One raw sample, its normal made unit.
    pub fn append(&mut self, pos: V, normal: V, facet: usize) {
        self.samples.push(DrawCutSample {
            pos,
            normal: unit(normal, Z),
            facet,
        });
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn samples(&self) -> &[DrawCutSample] {
        &self.samples
    }

    /// Resample to `spacing`, smooth by `smoothing` in [0, 1], decide open
    /// or closed, and check the result; always from the raw samples, so the
    /// same arguments give the same path (`DrawCutStroke::finish`).
    pub fn finish(&mut self, spacing: f64, smoothing: f64, force_closed: bool) -> DrawCutError {
        self.path.clear();
        self.binormal.clear();
        self.closed = false;
        self.error = DrawCutError::None;
        if self.samples.len() < 2 {
            self.error = DrawCutError::TooShort;
            return self.error;
        }
        let sp = if spacing > 0.0 {
            spacing
        } else {
            DEFAULT_SPACING
        };
        let tol = draw_cut_closing_tolerance(sp);

        // The jump test: keep the longest run with no gap over 8 spacings.
        let jump = 8.0 * sp;
        let n = self.samples.len();
        let (mut best_begin, mut best_end, mut begin) = (0usize, 0usize, 0usize);
        let (mut best_len, mut cur_len) = (-1.0f64, 0.0f64);
        for i in 1..=n {
            let step = if i < n {
                norm(sub(self.samples[i].pos, self.samples[i - 1].pos))
            } else {
                0.0
            };
            if i < n && step <= jump {
                cur_len += step;
                continue;
            }
            if cur_len > best_len {
                best_len = cur_len;
                best_begin = begin;
                best_end = i;
            }
            begin = i;
            cur_len = 0.0;
        }
        let run = self.samples[best_begin..best_end].to_vec();
        let discarded = best_end - best_begin < n;
        let short_error = if discarded {
            DrawCutError::LeavesMesh
        } else {
            DrawCutError::TooShort
        };
        if run.len() < 2 {
            self.error = short_error;
            return self.error;
        }
        let gap = norm(sub(run[run.len() - 1].pos, run[0].pos));
        self.closed = (force_closed || gap <= tol) && run.len() >= 3;
        self.path = draw_cut_resample(&run, sp, self.closed);
        draw_cut_smooth(
            &mut self.path,
            draw_cut_smooth_passes(smoothing),
            self.closed,
            0.5,
        );
        if self.path.len() < MIN_SAMPLES || self.length() < MIN_LENGTH {
            self.error = short_error;
            return self.error;
        }
        self.compute_binormals();
        if draw_cut_self_crossing(self) {
            self.error = DrawCutError::SelfCrossing;
        }
        self.error
    }

    /// The finished path; empty until [`DrawCutStroke::finish`] has run.
    pub fn path(&self) -> &[DrawCutSample] {
        &self.path
    }

    /// The finished path replaced in place (the gizmo's re-projection onto
    /// the mesh), the binormals rebuilt; a path of another length is
    /// refused (`set_path`).
    pub fn set_path(&mut self, path: &[DrawCutSample]) {
        if path.len() != self.path.len() {
            return;
        }
        self.path = path.to_vec();
        for s in &mut self.path {
            s.normal = unit(s.normal, Z);
        }
        self.compute_binormals();
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn error(&self) -> DrawCutError {
        self.error
    }

    pub fn valid(&self) -> bool {
        self.error == DrawCutError::None && self.path.len() >= MIN_SAMPLES
    }

    /// The finished path's arc length, the closing span included for a loop.
    pub fn length(&self) -> f64 {
        if self.path.len() < 2 {
            return 0.0;
        }
        let mut len: f64 = self
            .path
            .windows(2)
            .map(|w| norm(sub(w[1].pos, w[0].pos)))
            .sum();
        if self.closed {
            len += norm(sub(self.path[0].pos, self.path[self.path.len() - 1].pos));
        }
        len
    }

    pub fn centroid(&self) -> V {
        if self.path.is_empty() {
            return [0.0; 3];
        }
        let c = self.path.iter().fold([0.0; 3], |c, s| add(c, s.pos));
        mul(c, 1.0 / self.path.len() as f64)
    }

    /// The unit tangent at path sample `i`, central-differenced.
    pub fn tangent(&self, i: usize) -> V {
        let p = &self.path;
        let n = p.len();
        if n < 2 {
            return X;
        }
        if self.closed {
            return unit(sub(p[(i + 1) % n].pos, p[(i + n - 1) % n].pos), X);
        }
        if i == 0 {
            return unit(sub(p[1].pos, p[0].pos), X);
        }
        if i + 1 >= n {
            return unit(sub(p[n - 1].pos, p[n - 2].pos), X);
        }
        unit(sub(p[i + 1].pos, p[i - 1].pos), X)
    }

    /// The outward binormal at path sample `i`.
    pub fn binormal(&self, i: usize) -> V {
        self.binormal.get(i).copied().unwrap_or(X)
    }

    fn compute_binormals(&mut self) {
        let n = self.path.len();
        self.binormal = vec![[0.0; 3]; n];
        if n == 0 {
            return;
        }
        let c = self.centroid();
        for i in 0..n {
            let t = self.tangent(i);
            let nrm = self.path[i].normal;
            let mut b = cross(t, nrm);
            if norm(b) < 1e-9 {
                b = reject(sub(self.path[i].pos, c), nrm);
            }
            self.binormal[i] = unit(b, X);
        }
        if self.closed {
            let agree: f64 = (0..n)
                .map(|i| {
                    let radial = reject(sub(self.path[i].pos, c), self.path[i].normal);
                    dot(self.binormal[i], radial)
                })
                .sum();
            if agree < 0.0 {
                for b in &mut self.binormal {
                    *b = neg(*b);
                }
            }
        } else {
            for i in 1..n {
                if dot(self.binormal[i], self.binormal[i - 1]) < 0.0 {
                    self.binormal[i] = neg(self.binormal[i]);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Self-crossing.
// ---------------------------------------------------------------------------

/// Newell's normal of the path's polygon.
fn newell(path: &[DrawCutSample]) -> V {
    let n = path.len();
    let mut nw = [0.0; 3];
    for i in 0..n {
        let (a, b) = (path[i].pos, path[(i + 1) % n].pos);
        nw[0] += (a[1] - b[1]) * (a[2] + b[2]);
        nw[1] += (a[2] - b[2]) * (a[0] + b[0]);
        nw[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    nw
}

/// The path's best-fit plane, as an origin and two in-plane axes
/// (`stroke_plane`).
fn stroke_plane(stroke: &DrawCutStroke) -> (V, V, V) {
    let p = stroke.path();
    let origin = stroke.centroid();
    let mut nrm = newell(p);
    if norm(nrm) < 1e-12 {
        nrm = p.iter().fold([0.0; 3], |a, s| add(a, s.normal));
    }
    let nrm = unit(nrm, Z);
    let seed = if nrm[0].abs() < 0.9 { X } else { Y };
    let ax = unit(reject(seed, nrm), X);
    let ay = cross(nrm, ax);
    (origin, ax, ay)
}

/// Whether segments ab and cd cross properly; touching at an end does not
/// count (`segments_cross`).
fn segments_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let cr = |u: [f64; 2], v: [f64; 2]| u[0] * v[1] - u[1] * v[0];
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let denom = cr(r, s);
    if denom.abs() < 1e-12 {
        return false;
    }
    let ca = [c[0] - a[0], c[1] - a[1]];
    let t = cr(ca, s) / denom;
    let u = cr(ca, r) / denom;
    let eps = 1e-9;
    t > eps && t < 1.0 - eps && u > eps && u < 1.0 - eps
}

/// Whether the path, laid on its best-fit plane, crosses itself
/// (`draw_cut_self_crossing`).
pub fn draw_cut_self_crossing(stroke: &DrawCutStroke) -> bool {
    let p = stroke.path();
    let n = p.len();
    if n < 4 {
        return false;
    }
    let (origin, ax, ay) = stroke_plane(stroke);
    let flat: Vec<[f64; 2]> = p
        .iter()
        .map(|s| {
            let d = sub(s.pos, origin);
            [dot(d, ax), dot(d, ay)]
        })
        .collect();
    let segments = if stroke.is_closed() { n } else { n - 1 };
    for i in 0..segments.saturating_sub(1) {
        let (a, b) = (flat[i], flat[(i + 1) % n]);
        for j in i + 2..segments {
            if stroke.is_closed() && i == 0 && j + 1 == segments {
                continue;
            }
            if segments_cross(a, b, flat[j], flat[(j + 1) % n]) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// The open strip's cut direction, and its fold gates.
// ---------------------------------------------------------------------------

/// The unit ray into the part at path sample `i`: the constant direction,
/// or the inward normal turned towards the outward binormal by the draft
/// angle (`draw_cut_inward_dir`).
pub fn draw_cut_inward_dir(stroke: &DrawCutStroke, params: &DrawCutParams, i: usize) -> V {
    match params.direction {
        DrawCutDirection::View => return unit(params.view_dir, neg(Z)),
        DrawCutDirection::AxisX => return neg(X),
        DrawCutDirection::AxisY => return neg(Y),
        DrawCutDirection::AxisZ => return neg(Z),
        DrawCutDirection::SurfaceNormal => {}
    }
    let Some(sample) = stroke.path().get(i) else {
        return neg(Z);
    };
    let inward = neg(sample.normal);
    let theta = params
        .angle_deg
        .clamp(-MAX_ANGLE_DEG, MAX_ANGLE_DEG)
        .to_radians();
    if theta.abs() < 1e-12 {
        return inward;
    }
    let b = stroke.binormal(i);
    if dot(b, inward).abs() > 0.999 {
        return inward;
    }
    unit(add(mul(inward, theta.cos()), mul(b, theta.sin())), inward)
}

/// Whether a loop's binormals disagree in sign between neighbours
/// (`draw_cut_frame_holonomy_flips`).
pub fn draw_cut_frame_holonomy_flips(stroke: &DrawCutStroke) -> bool {
    if !stroke.is_closed() {
        return false;
    }
    let n = stroke.path().len();
    if n < 3 {
        return false;
    }
    (0..n).any(|i| dot(stroke.binormal(i), stroke.binormal((i + 1) % n)) < 0.0)
}

/// Whether the ruled strip folds where the stroke turns tighter than the
/// rulings reach sideways, and the worst curvature found
/// (`draw_cut_strip_folds`).
pub fn draw_cut_strip_folds(
    stroke: &DrawCutStroke,
    extension: f64,
    angle_deg: f64,
    depth: f64,
) -> (bool, f64) {
    let p = stroke.path();
    let n = p.len();
    let theta = angle_deg.clamp(-MAX_ANGLE_DEG, MAX_ANGLE_DEG).to_radians();
    let lean = theta.sin().abs();
    let reach = extension.max(0.0).max(depth.max(0.0) * lean);
    if n < 3 || reach <= 0.0 {
        return (false, 0.0);
    }
    let mut worst: f64 = 0.0;
    let (first, last) = if stroke.is_closed() {
        (0, n)
    } else {
        (1, n - 1)
    };
    for i in first..last {
        let (a, b, c) = (p[(i + n - 1) % n].pos, p[i % n].pos, p[(i + 1) % n].pos);
        let (ab, bc, ca) = (norm(sub(b, a)), norm(sub(c, b)), norm(sub(a, c)));
        if ab < 1e-12 || bc < 1e-12 || ca < 1e-12 {
            continue;
        }
        let area = 0.5 * norm(cross(sub(b, a), sub(c, a)));
        let kappa = 4.0 * area / (ab * bc * ca);
        let turn = sub(mul(sub(c, b), 1.0 / bc), mul(sub(b, a), 1.0 / ab));
        let toward_out = dot(turn, stroke.binormal(i % n));
        if toward_out > 0.0 || (lean > 1e-9 && toward_out < 0.0) {
            worst = worst.max(kappa);
        }
    }
    (reach * worst > 1.0, worst)
}

// ---------------------------------------------------------------------------
// The core plane and the band.
// ---------------------------------------------------------------------------

/// A closed stroke's best-fit plane: Newell's normal (an axis direction
/// replaces it) and the path's centroid (`draw_cut_core_plane`). None for
/// a stroke that is not a valid loop.
pub fn draw_cut_core_plane(stroke: &DrawCutStroke, params: &DrawCutParams) -> Option<(V, V)> {
    if !stroke.valid() || !stroke.is_closed() {
        return None;
    }
    let p = stroke.path();
    if p.len() < 3 {
        return None;
    }
    let centroid = stroke.centroid();
    match params.direction {
        DrawCutDirection::AxisX => return Some((X, centroid)),
        DrawCutDirection::AxisY => return Some((Y, centroid)),
        DrawCutDirection::AxisZ => return Some((Z, centroid)),
        _ => {}
    }
    let nw = newell(p);
    if norm(nw) < 1e-9 {
        let avg = p.iter().fold([0.0; 3], |a, s| add(a, s.normal));
        return Some((unit(avg, Z), centroid));
    }
    Some((unit(nw, Z), centroid))
}

/// The in-plane inward direction at sample `i`: towards the loop's axis
/// (`draw_cut_core_inward`); an open stroke's own ruling.
pub fn draw_cut_core_inward(
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
    normal: V,
    centroid: V,
    i: usize,
) -> V {
    let Some(sample) = stroke.path().get(i) else {
        return neg(Z);
    };
    if !stroke.is_closed() {
        return draw_cut_inward_dir(stroke, params, i);
    }
    let mut inward = reject(sub(centroid, sample.pos), normal);
    if norm(inward) < 1e-9 {
        let d = draw_cut_inward_dir(stroke, params, i);
        inward = reject(d, normal);
        if norm(inward) < 1e-9 {
            return neg(normal);
        }
    }
    unit(inward, X)
}

/// The band's travel: `inward` tilted by the lip angle towards -normal
/// (`draw_cut_band_dir`).
pub fn draw_cut_band_dir(inward: V, normal: V, angle_deg: f64) -> V {
    let a = angle_deg
        .clamp(MIN_LIP_ANGLE_DEG, MAX_LIP_ANGLE_DEG)
        .to_radians();
    unit(sub(mul(inward, a.cos()), mul(normal, a.sin())), inward)
}

/// The angle the skirt leaves at, "the same as Angle" resolved
/// (`draw_cut_extension_angle`).
pub fn draw_cut_extension_angle(params: &DrawCutParams) -> f64 {
    params
        .extension_angle_deg
        .unwrap_or(params.angle_deg)
        .clamp(MIN_EXT_ANGLE_DEG, MAX_EXT_ANGLE_DEG)
}

/// The skirt's travel: the reverse of a band ruling at the extension angle
/// (`draw_cut_skirt_dir`).
pub fn draw_cut_skirt_dir(inward: V, normal: V, params: &DrawCutParams) -> V {
    neg(draw_cut_band_dir(
        inward,
        normal,
        draw_cut_extension_angle(params),
    ))
}

/// The side of the core plane out of the part where the loop was drawn,
/// as +n or -n: the samples' mean normal, confirmed against the mesh by a
/// handful of parity probes when the samples do not agree
/// (`draw_cut_outward_side`).
pub fn draw_cut_outward_side(stroke: &DrawCutStroke, n: V, mesh: Option<&Mesh>) -> V {
    let nn = unit(n, Z);
    if !stroke.valid() {
        return nn;
    }
    let p = stroke.path();
    let avg = p.iter().fold([0.0; 3], |a, s| add(a, s.normal));
    let along = dot(avg, nn) / p.len() as f64;
    if along.abs() > 0.5 {
        return if along > 0.0 { nn } else { neg(nn) };
    }
    let mut cand = if along >= 0.0 { nn } else { neg(nn) };
    let Some(mesh) = mesh.filter(|m| !m.triangles.is_empty()) else {
        return cand;
    };
    let c = stroke.centroid();
    let r = p
        .iter()
        .map(|s| norm(reject(sub(s.pos, c), nn)))
        .fold(0.0, f64::max);
    let step = (0.1 * r).max(0.05);
    let probes = p.len().min(8);
    let (mut in_pos, mut in_neg) = (0i64, 0i64);
    for k in 0..probes {
        let q = p[(k * p.len()) / probes].pos;
        let reach_pos = ray_first_hit(mesh, q, cand);
        let reach_neg = ray_first_hit(mesh, q, neg(cand));
        let dp = if reach_pos < step {
            0.5 * reach_pos
        } else {
            step
        };
        let dn = if reach_neg < step {
            0.5 * reach_neg
        } else {
            step
        };
        if point_in_solid(mesh, add(q, mul(cand, dp))) {
            in_pos += 1;
        }
        if point_in_solid(mesh, sub(q, mul(cand, dn))) {
            in_neg += 1;
        }
        let left = (probes - k - 1) as i64;
        if (in_pos - in_neg).abs() > left {
            break;
        }
    }
    if in_pos > in_neg {
        cand = neg(cand);
    }
    cand
}

/// A closed loop's band, worked out once (`CoreBand`).
struct CoreBand {
    normal: V,
    centroid: V,
    /// A point on the core plane: the centroid moved to the band's mean
    /// arrival height.
    plane_pt: V,
    ex: V,
    ey: V,
    /// The skirt's far edge.
    out: Vec<V>,
    /// The drawn line, kerf-shifted.
    line: Vec<V>,
    /// The band's inner curve, on the core plane.
    inner: Vec<V>,
}

/// The band of a closed loop (`build_core_band`): the core normal pinned
/// to the skin (or, for a plug whose samples cancel, to the mesh), and for
/// each sample the line, the skirt tip and the band's arrival, flattened
/// onto the core plane at the mean arrival height.
fn build_core_band(
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
    ext: f64,
    depth_along: f64,
    face_offset: f64,
    pin_mesh: Option<&Mesh>,
) -> Option<CoreBand> {
    let (mut normal, centroid) = draw_cut_core_plane(stroke, params)?;
    let p = stroke.path();
    let n = p.len();
    let avg_skin = p.iter().fold([0.0; 3], |a, s| add(a, s.normal));
    if norm(avg_skin) > 0.25 * n as f64 {
        if dot(normal, avg_skin) < 0.0 {
            normal = neg(normal);
        }
    } else if let Some(mesh) = pin_mesh.filter(|m| !m.triangles.is_empty()) {
        let outward = draw_cut_outward_side(stroke, normal, Some(mesh));
        if dot(outward, normal) < 0.0 {
            normal = neg(normal);
        }
    }
    let (mut out, mut line, mut inner) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let shift = mul(normal, face_offset);
    for (i, sample) in p.iter().enumerate() {
        let inward = draw_cut_core_inward(stroke, params, normal, centroid, i);
        let d = draw_cut_band_dir(inward, normal, params.angle_deg);
        let mut outward = sample.normal;
        if norm(outward) < 1e-9 {
            outward = neg(inward);
        }
        let outward = unit(outward, neg(inward));
        line.push(add(sample.pos, shift));
        let mut skirt = draw_cut_skirt_dir(inward, normal, params);
        if norm(skirt) < 1e-9 {
            skirt = outward;
        }
        out.push(add(add(sample.pos, mul(unit(skirt, outward), ext)), shift));
        inner.push(add(add(sample.pos, mul(d, depth_along)), shift));
    }
    let mean_h = inner
        .iter()
        .map(|v| dot(sub(*v, centroid), normal))
        .sum::<f64>()
        / inner.len() as f64;
    let plane_pt = add(centroid, mul(normal, mean_h));
    for v in &mut inner {
        *v = sub(*v, mul(normal, dot(sub(*v, plane_pt), normal)));
    }
    let seed = if normal[2].abs() < 0.9 { Z } else { X };
    let ex = unit(cross(seed, normal), X);
    let ey = cross(normal, ex);
    Some(CoreBand {
        normal,
        centroid,
        plane_pt,
        ex,
        ey,
        out,
        line,
        inner,
    })
}

/// The core plate (`CorePlate`): its boundary ring on the plane,
/// counter-clockwise about +n, and its triangles as indices into the ring,
/// counter-clockwise about +n too.
struct CorePlate {
    ring: Vec<V>,
    triangles: Vec<[usize; 3]>,
}

/// The core plate over the band's inner curve (`build_core_plate`): the
/// curve laid on the plane and made simple by a non-zero union, the
/// largest contour kept, and its interior triangulated.
///
/// The reference triangulates with its tesselator and matches the
/// triangles' corners back to the ring by position; the ear clipping here
/// adds no corners, so its triangles index the ring directly.
fn build_core_plate(inner: &[V], plane_pt: V, ex: V, ey: V) -> Option<CorePlate> {
    if inner.len() < 3 {
        return None;
    }
    let mut proj = Polygon::default();
    for v in inner {
        let q = sub(*v, plane_pt);
        let pt = Point2::from_mm(dot(q, ex), dot(q, ey));
        let far = |a: Point2, b: Point2| ((a.x - b.x) as f64).hypot((a.y - b.y) as f64) > 1e3;
        if proj.points.last().is_none_or(|last| far(pt, *last)) {
            proj.points.push(pt);
        }
    }
    if proj.points.len() >= 2 {
        let (first, last) = (proj.points[0], proj.points[proj.points.len() - 1]);
        if ((first.x - last.x) as f64).hypot((first.y - last.y) as f64) <= 1e3 {
            proj.points.pop();
        }
    }
    if proj.points.len() < 3 {
        return None;
    }
    if proj.signed_area_mm2() < 0.0 {
        proj.points.reverse();
    }
    let simple = edge_clip::union([&proj]).ok()?;
    let mut keep: Option<&ExPolygon> = None;
    for region in &simple {
        let a = region.contour.signed_area_mm2().abs();
        if keep.is_none_or(|k| a > k.contour.signed_area_mm2().abs()) {
            keep = Some(region);
        }
    }
    let mut contour = keep?.contour.clone();
    if contour.points.len() < 3 || contour.signed_area_mm2() == 0.0 {
        return None;
    }
    if contour.signed_area_mm2() < 0.0 {
        contour.points.reverse();
    }
    let flat: Vec<[f64; 2]> = contour
        .points
        .iter()
        .map(|p| [unscaled(p.x), unscaled(p.y)])
        .collect();
    let triangles = triangulate_simple(&flat)?;
    let ring = flat
        .iter()
        .map(|q| add(plane_pt, add(mul(ex, q[0]), mul(ey, q[1]))))
        .collect();
    Some(CorePlate { ring, triangles })
}

/// Twice the signed area of the turn a, b, c.
fn turn2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// A simple counter-clockwise polygon cut into triangles by ear clipping,
/// every triangle counter-clockwise, every corner kept. None for fewer
/// than three corners.
fn triangulate_simple(points: &[[f64; 2]]) -> Option<Vec<[usize; 3]>> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    let mut ring: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let inside = |p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        turn2(a, b, p) >= 0.0 && turn2(b, c, p) >= 0.0 && turn2(c, a, p) >= 0.0
    };
    let mut at = 0usize;
    let mut misses = 0usize;
    while ring.len() > 3 {
        let len = ring.len();
        let (ip, ic, inx) = (
            ring[(at + len - 1) % len],
            ring[at % len],
            ring[(at + 1) % len],
        );
        let (a, b, c) = (points[ip], points[ic], points[inx]);
        let convex = turn2(a, b, c) > 0.0;
        let ear = convex
            && ring.iter().all(|&o| {
                o == ip
                    || o == ic
                    || o == inx
                    || points[o] == a
                    || points[o] == b
                    || points[o] == c
                    || !inside(points[o], a, b, c)
            });
        if ear || misses > len {
            // With no ear in a whole round (rounding on a nearly
            // degenerate ring), the corner is taken anyway, so the ring's
            // edges are all still used once.
            out.push([ip, ic, inx]);
            ring.remove(at % len);
            at = (at + len - 2) % (len - 1);
            misses = 0;
        } else {
            at = (at + 1) % len;
            misses += 1;
        }
    }
    out.push([ring[0], ring[1], ring[2]]);
    Some(out)
}

/// The closed cutter of a loop (`draw_cut_band_core_solid`): the prism for
/// through all, else the skirt, band and core plate, closed as a plug (a
/// lid over the skin inside the loop) or as a half-space (a flange out
/// past the part, a wall down and a cap) for a loop round the part.
/// `surface_only` stops before the closure (the preview).
fn draw_cut_band_core_solid(
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
    bbox: Option<Bounds>,
    face_offset: f64,
    mesh: Option<&Mesh>,
    surface_only: bool,
) -> Mesh {
    let mut its = Mesh::default();
    if !stroke.valid() || !stroke.is_closed() {
        return its;
    }
    let ext = params.extension.max(0.0);
    let diag = bbox.map_or(100.0, |(lo, hi)| norm(sub(hi, lo)));
    let reach = 1.05 * diag + 1.0;
    let depth_along = if params.through_all {
        reach
    } else {
        params.depth.max(0.01)
    };
    let wraps = mesh.is_some_and(|m| draw_cut_loop_separates(m, stroke, params, 0.5));
    let Some(cb) = build_core_band(
        stroke,
        params,
        ext,
        depth_along,
        face_offset,
        if !params.through_all && !wraps {
            mesh
        } else {
            None
        },
    ) else {
        return its;
    };
    let m = cb.out.len();
    if m < 3 {
        return its;
    }
    let vertex = |its: &mut Mesh, v: V| {
        its.vertices.push(f32s(v));
        (its.vertices.len() - 1) as u32
    };

    if params.through_all {
        let p = stroke.path();
        let outward_n = draw_cut_outward_side(stroke, cb.normal, mesh);
        let into = neg(outward_n);
        let nudge = (1e-4 * diag).max(1e-3);
        let travel = if wraps { neg(cb.normal) } else { into };
        let lift_n = if wraps { cb.normal } else { outward_n };
        let lift = if wraps {
            (1e-4 * diag).max(1e-3)
        } else {
            ext.max((1e-3 * diag).max(1e-2))
        };
        let (mut ring_a, mut ring_b) = (Vec::with_capacity(m), Vec::with_capacity(m));
        for (i, sample) in p.iter().enumerate().take(m) {
            let inward = draw_cut_core_inward(stroke, params, cb.normal, cb.centroid, i);
            let shift = sub(mul(cb.normal, face_offset), mul(inward, nudge));
            ring_a.push(add(add(sample.pos, mul(lift_n, lift)), shift));
            ring_b.push(add(add(sample.pos, mul(travel, reach)), shift));
        }
        for v in ring_a.iter().chain(&ring_b) {
            vertex(&mut its, *v);
        }
        let mu = m as u32;
        for i in 0..mu {
            let j = (i + 1) % mu;
            its.triangles.push([i, mu + i, mu + j]);
            its.triangles.push([i, mu + j, j]);
        }
        let ca = mul(
            ring_a.iter().fold([0.0; 3], |a, v| add(a, *v)),
            1.0 / m as f64,
        );
        let cbc = mul(
            ring_b.iter().fold([0.0; 3], |a, v| add(a, *v)),
            1.0 / m as f64,
        );
        let c_a = vertex(&mut its, ca);
        let c_b = vertex(&mut its, cbc);
        for i in 0..mu {
            let j = (i + 1) % mu;
            its.triangles.push([c_a, i, j]);
            its.triangles.push([c_b, mu + j, mu + i]);
        }
    } else {
        let Some(plate) = build_core_plate(&cb.inner, cb.plane_pt, cb.ex, cb.ey) else {
            return Mesh::default();
        };
        let k = plate.ring.len();
        if k < 3 {
            return Mesh::default();
        }
        let ang = |v: V| {
            let q = sub(v, cb.plane_pt);
            dot(q, cb.ey).atan2(dot(q, cb.ex))
        };
        let ao: Vec<f64> = cb.line.iter().map(|v| ang(*v)).collect();
        let mut ai: Vec<f64> = plate.ring.iter().map(|v| ang(*v)).collect();
        let winding = |a: &[f64]| {
            let mut sum = 0.0;
            for i in 0..a.len() {
                let mut d = a[(i + 1) % a.len()] - a[i];
                while d > std::f64::consts::PI {
                    d -= std::f64::consts::TAU;
                }
                while d < -std::f64::consts::PI {
                    d += std::f64::consts::TAU;
                }
                sum += d;
            }
            sum
        };
        let mut inner_ring = plate.ring.clone();
        let ring_reversed = winding(&ao) * winding(&ai) < 0.0;
        if ring_reversed {
            inner_ring.reverse();
            ai.reverse();
        }
        let mut i0 = 0usize;
        let mut best = f64::MAX;
        for (i, a) in ai.iter().enumerate() {
            let mut d = (a - ao[0]).abs();
            if d > std::f64::consts::PI {
                d = std::f64::consts::TAU - d;
            }
            if d < best {
                best = d;
                i0 = i;
            }
        }
        for v in cb.line.iter().chain(&inner_ring).chain(&cb.out) {
            vertex(&mut its, *v);
        }
        let o = |i: usize| i as u32;
        let ii = |i: usize| (m + (i0 + i) % k) as u32;
        let t = |i: usize| (m + k + i) as u32;
        // The band, from the drawn line to the plate's ring, walking both
        // rings by fractional position.
        let (mut a, mut b) = (0usize, 0usize);
        while a < m || b < k {
            let take_outer =
                b >= k || (a < m && (a + 1) as f64 / m as f64 <= (b + 1) as f64 / k as f64);
            if take_outer {
                its.triangles.push([o(a % m), ii(b % k), o((a + 1) % m)]);
                a += 1;
            } else {
                its.triangles.push([o(a % m), ii(b % k), ii((b + 1) % k)]);
                b += 1;
            }
        }
        // The skirt, from the line out to the tip.
        for i in 0..m {
            let j = (i + 1) % m;
            its.triangles.push([o(i), o(j), t(i)]);
            its.triangles.push([o(j), t(j), t(i)]);
        }
        // The core plate, walking its ring against the band's walk.
        let at_ring = |r: usize| (m + if ring_reversed { k - 1 - r } else { r }) as u32;
        for tri in &plate.triangles {
            let [a0, a1, a2] = tri.map(at_ring);
            its.triangles.push(if ring_reversed {
                [a0, a1, a2]
            } else {
                [a0, a2, a1]
            });
        }
        if surface_only {
            return its;
        }
        let tip = &cb.out;
        if !wraps {
            // The plug's lid: a height field over the loop along the view,
            // hugging the skin the camera saw inside it.
            let vd = unit(params.view_dir, neg(Z));
            let w = neg(vd);
            let ev1 = unit(cross(if w[2].abs() < 0.9 { Z } else { X }, w), X);
            let ev2 = cross(w, ev1);
            let path = stroke.path();
            let o3 = mul(tip.iter().fold([0.0; 3], |a, v| add(a, *v)), 1.0 / m as f64);
            let to2 = |v: V| {
                let q = sub(v, o3);
                [dot(q, ev1), dot(q, ev2)]
            };
            let to3 = |uv: [f64; 2], d: f64| {
                add(o3, add(add(mul(ev1, uv[0]), mul(ev2, uv[1])), mul(w, d)))
            };
            let tip_w: Vec<f64> = tip.iter().map(|v| dot(sub(*v, o3), w)).collect();
            let tip2: Vec<[f64; 2]> = tip.iter().map(|v| to2(*v)).collect();
            let w_near = tip_w.iter().copied().fold(f64::MIN, f64::max);
            let mut c2 = [0.0, 0.0];
            for t2 in &tip2 {
                c2 = [c2[0] + t2[0], c2[1] + t2[1]];
            }
            let c2 = [c2[0] / m as f64, c2[1] / m as f64];
            let r_mean = tip2
                .iter()
                .map(|t2| (t2[0] - c2[0]).hypot(t2[1] - c2[1]))
                .sum::<f64>()
                / m as f64;
            let kk = ((r_mean / 1.5).round() as i64).clamp(4, 24) as usize;
            let lift = (0.005 * diag).max(0.5);
            let ring_sp = r_mean / kk as f64;
            let tree = mesh
                .filter(|m| !m.triangles.is_empty())
                .map(|m| (m, Tree::new(m)));
            let w_flat = match bbox {
                Some((lo, hi)) => {
                    let mut wf = w_near;
                    for cx in 0..2 {
                        for cy in 0..2 {
                            for cz in 0..2 {
                                let corner = [
                                    if cx == 1 { hi[0] } else { lo[0] },
                                    if cy == 1 { hi[1] } else { lo[1] },
                                    if cz == 1 { hi[2] } else { lo[2] },
                                ];
                                wf = wf.max(dot(sub(corner, o3), w));
                            }
                        }
                    }
                    wf + (0.01 * diag).max(1.0)
                }
                None => w_near + reach,
            };
            let first_hit = |uv: [f64; 2]| {
                let (mesh, tree) = tree.as_ref()?;
                let hit = tree.hit(to3(uv, reach), vd, (0.0, f64::INFINITY), &|_| true)?;
                Some((*mesh, hit))
            };
            let lid_pt = |uv: [f64; 2]| -> V {
                if tree.is_none() {
                    return to3(uv, w_flat);
                }
                if let Some((mesh, hit)) = first_hit(uv) {
                    let [a, b, c] = mesh.triangle(hit.triangle).map(|p| p.map(f64::from));
                    let fnrm = unit(cross(sub(b, a), sub(c, a)), w);
                    let cos_t = dot(fnrm, w).abs().max(0.15);
                    return add(
                        to3(uv, reach - hit.t + 0.25 * ring_sp / cos_t),
                        mul(fnrm, lift),
                    );
                }
                to3(uv, w_near + lift)
            };
            let skin_w = |uv: [f64; 2]| first_hit(uv).map(|(_, hit)| reach - hit.t);
            let clear_chord = |uv_a: [f64; 2], w_a: f64, uv_b: [f64; 2], w_b: f64| {
                let mut raise: f64 = 0.0;
                for x in [0.2, 0.4, 0.6, 0.8] {
                    let at = [
                        uv_a[0] + x * (uv_b[0] - uv_a[0]),
                        uv_a[1] + x * (uv_b[1] - uv_a[1]),
                    ];
                    let Some(sw) = skin_w(at) else {
                        continue;
                    };
                    let chord = w_a + x * (w_b - w_a);
                    if sw + lift > chord {
                        raise = raise.max((sw + lift - chord) / x);
                    }
                }
                raise
            };
            let mut ring_pts = vec![[0.0; 3]; (kk - 1) * m];
            let mut centre_pt = lid_pt(c2);
            let mut centre_raise: f64 = 0.0;
            for i in 0..m {
                let (mut prev_uv, mut prev_w) = (tip2[i], tip_w[i]);
                for ring in 1..kk {
                    let f = ring as f64 / kk as f64;
                    let uv = [
                        tip2[i][0] * (1.0 - f) + c2[0] * f,
                        tip2[i][1] * (1.0 - f) + c2[1] * f,
                    ];
                    let mut pt = lid_pt(uv);
                    if ring == 1 {
                        let cos_t = dot(unit(path[i].normal, w), w).clamp(-1.0, 1.0);
                        let theta =
                            (cos_t.abs().acos() + 10f64.to_radians()).min(75f64.to_radians());
                        let run = (uv[0] - tip2[i][0]).hypot(uv[1] - tip2[i][1]);
                        let floor_w = (tip_w[i] + run * theta.tan()).max(tip_w[i] + lift);
                        let pt_w = dot(sub(pt, o3), w);
                        if pt_w < floor_w {
                            pt = add(pt, mul(w, floor_w - pt_w));
                        }
                    }
                    let pt_uv = to2(pt);
                    let mut pt_w = dot(sub(pt, o3), w);
                    let raise = clear_chord(prev_uv, prev_w, pt_uv, pt_w);
                    if raise > 0.0 {
                        pt = add(pt, mul(w, raise));
                        pt_w += raise;
                    }
                    ring_pts[(ring - 1) * m + i] = pt;
                    prev_uv = pt_uv;
                    prev_w = pt_w;
                }
                centre_raise = centre_raise.max(clear_chord(
                    prev_uv,
                    prev_w,
                    to2(centre_pt),
                    dot(sub(centre_pt, o3), w),
                ));
            }
            centre_pt = add(centre_pt, mul(w, centre_raise));
            let base = its.vertices.len();
            for pt in &ring_pts {
                vertex(&mut its, *pt);
            }
            let c_top = vertex(&mut its, centre_pt);
            let l = |ring: usize, i: usize| (base + (ring - 1) * m + i) as u32;
            for ring in 0..kk - 1 {
                for i in 0..m {
                    let j = (i + 1) % m;
                    let (ai, aj) = if ring == 0 {
                        (t(i), t(j))
                    } else {
                        (l(ring, i), l(ring, j))
                    };
                    its.triangles.push([ai, aj, l(ring + 1, i)]);
                    its.triangles.push([aj, l(ring + 1, j), l(ring + 1, i)]);
                }
            }
            for i in 0..m {
                its.triangles
                    .push([l(kk - 1, i), l(kk - 1, (i + 1) % m), c_top]);
            }
        } else {
            // The half-space: a flange out past the part in the core
            // plane, a wall down along -n, and a cap.
            let fbase = its.vertices.len();
            let mut far_ring = Vec::with_capacity(m);
            for (i, tip_i) in tip.iter().enumerate() {
                let inward = draw_cut_core_inward(stroke, params, cb.normal, cb.centroid, i);
                let f = sub(*tip_i, mul(inward, reach));
                far_ring.push(f);
                vertex(&mut its, f);
            }
            let sbase = its.vertices.len();
            for f in &far_ring {
                vertex(&mut its, sub(*f, mul(cb.normal, reach)));
            }
            let ff = |i: usize| (fbase + i) as u32;
            let ss = |i: usize| (sbase + i) as u32;
            for i in 0..m {
                let j = (i + 1) % m;
                its.triangles.push([t(i), t(j), ff(i)]);
                its.triangles.push([t(j), ff(j), ff(i)]);
            }
            for i in 0..m {
                let j = (i + 1) % m;
                its.triangles.push([ff(i), ff(j), ss(i)]);
                its.triangles.push([ff(j), ss(j), ss(i)]);
            }
            let bot = far_ring
                .iter()
                .fold([0.0; 3], |a, f| add(a, sub(*f, mul(cb.normal, reach))));
            let c_bot = vertex(&mut its, mul(bot, 1.0 / m as f64));
            for i in 0..m {
                its.triangles.push([c_bot, ss(i), ss((i + 1) % m)]);
            }
        }
    }
    outward(its)
}

/// The mesh turned outward-wound if it encloses negative volume, as the
/// reference does before handing a cutter over (`its_volume` check).
fn outward(mut its: Mesh) -> Mesh {
    if its.signed_volume() < 0.0 {
        for t in &mut its.triangles {
            t.swap(1, 2);
        }
    }
    its
}

/// Whether a closed loop goes ROUND the part rather than sitting on it
/// (`draw_cut_loop_separates`): its sample normals cancel, and some slice
/// of the part's section at the loop's plane lies more than
/// `contain_frac` inside the loop.
pub fn draw_cut_loop_separates(
    mesh: &Mesh,
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
    contain_frac: f64,
) -> bool {
    if mesh.triangles.is_empty() || !stroke.valid() || !stroke.is_closed() {
        return false;
    }
    let Some((n, c)) = draw_cut_core_plane(stroke, params) else {
        return false;
    };
    let p = stroke.path();
    let avg_skin = p.iter().fold([0.0; 3], |a, s| add(a, s.normal));
    if norm(avg_skin) > 0.25 * p.len() as f64 {
        return false;
    }
    let ax = if n[2].abs() < 0.9 { Z } else { X };
    let ex = cross(ax, n);
    if norm(ex) < 1e-9 {
        return false;
    }
    let ex = unit(ex, X);
    let ey = cross(n, ex);
    let frame = |v: V| {
        let q = sub(v, c);
        [dot(ex, q), dot(ey, q), dot(n, q)]
    };
    let flat = Mesh {
        vertices: mesh
            .vertices
            .iter()
            .map(|v| f32s(frame(v.map(f64::from))))
            .collect(),
        triangles: mesh.triangles.clone(),
    };
    let layers = edge_slice::slice(&flat, &[0.0]);
    let Some(layer) = layers.first() else {
        return false;
    };
    let Ok(section) = edge_clip::union(&layer.polygons) else {
        return false;
    };
    if section.is_empty() {
        return false;
    }
    let mut lp = Polygon {
        points: p
            .iter()
            .map(|s| {
                let q = frame(s.pos);
                Point2::from_mm(q[0], q[1])
            })
            .collect(),
    };
    if lp.points.len() < 3 {
        return false;
    }
    if lp.signed_area_mm2() < 0.0 {
        lp.points.reverse();
    }
    let loop_area = lp.signed_area_mm2().abs();
    // The reference's EPSILON is on its scaled (nanometre²) area.
    let epsilon = 1e-4 / (edge_geom::UNITS_PER_MM * edge_geom::UNITS_PER_MM);
    if loop_area < epsilon {
        return false;
    }
    let loop_region = [ExPolygon {
        contour: lp,
        holes: Vec::new(),
    }];
    for slice in &section {
        let slice_area = slice.area_mm2().abs();
        if slice_area < epsilon || slice_area < 0.25 * loop_area {
            continue;
        }
        let Ok(inside) = edge_clip::intersection(std::slice::from_ref(slice), &loop_region) else {
            continue;
        };
        let inside_area = inside.iter().map(ExPolygon::area_mm2).sum::<f64>().abs();
        if inside_area > contain_frac * slice_area {
            return true;
        }
    }
    false
}

/// Where the flat core sits: its normal and a point on it
/// (`draw_cut_core_face`). None for a stroke that is not a valid loop, and
/// for through all.
///
/// DIFFERS FROM THE REFERENCE in taking the mesh. The reference builds
/// this band without it, but the cutter pins a plug's core normal against
/// the mesh when the samples' normals cancel (a loop over a strongly
/// curved patch); there the reference's core face can have the other
/// normal, and with it the band leaning the other way, so a different
/// plane from the one the cut makes. Given the mesh the cutter is built
/// with, this asks the same question the cutter does.
pub fn draw_cut_core_face(
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
    mesh: Option<&Mesh>,
) -> Option<(V, V)> {
    if !stroke.valid() || !stroke.is_closed() || params.through_all {
        return None;
    }
    let wraps = mesh.is_some_and(|m| draw_cut_loop_separates(m, stroke, params, 0.5));
    let cb = build_core_band(
        stroke,
        params,
        params.extension.max(0.0),
        params.depth.max(0.01),
        0.0,
        if wraps { None } else { mesh },
    )?;
    Some((cb.normal, cb.plane_pt))
}

/// The loop's in-plane radius at the 5th percentile (`loop_inplane_radius`).
fn loop_inplane_radius(stroke: &DrawCutStroke, n: V, c: V) -> f64 {
    let mut r: Vec<f64> = stroke
        .path()
        .iter()
        .map(|s| norm(reject(sub(c, s.pos), n)))
        .collect();
    if r.is_empty() {
        return 0.0;
    }
    let k = (r.len() - 1).min((0.05 * r.len() as f64) as usize);
    let (_, kth, _) = r.select_nth_unstable_by(k, f64::total_cmp);
    *kth
}

/// Whether the surface the stroke gets folds (`draw_cut_band_folds`): for
/// a loop, whether the band's inset nears the loop's own radius, with the
/// fraction it reaches; for an open stroke, the strip test on a smoothed
/// path, with the worst curvature.
pub fn draw_cut_band_folds(stroke: &DrawCutStroke, params: &DrawCutParams) -> (bool, f64) {
    if !stroke.valid() {
        return (false, 0.0);
    }
    if !stroke.is_closed() {
        let mut smoothed = stroke.clone();
        let mut path = stroke.path().to_vec();
        draw_cut_smooth(&mut path, 4, false, 0.5);
        smoothed.set_path(&path);
        return draw_cut_strip_folds(
            &smoothed,
            params.extension,
            params.angle_deg,
            params.depth.max(0.0),
        );
    }
    if params.through_all {
        return (false, 0.0);
    }
    let Some((n, c)) = draw_cut_core_plane(stroke, params) else {
        return (false, 0.0);
    };
    let radius = loop_inplane_radius(stroke, n, c);
    if radius <= 1e-9 {
        return (false, 0.0);
    }
    let inset = params.depth.max(0.01)
        * params
            .angle_deg
            .clamp(MIN_LIP_ANGLE_DEG, MAX_LIP_ANGLE_DEG)
            .to_radians()
            .cos()
            .abs();
    let frac = inset / radius;
    (frac >= 0.95, frac)
}

// ---------------------------------------------------------------------------
// The cutter.
// ---------------------------------------------------------------------------

fn draw_cut_cutter_solid_impl(
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
    bbox: Option<Bounds>,
    face_offset: f64,
    mesh: Option<&Mesh>,
    surface_only: bool,
) -> Mesh {
    if !stroke.valid() {
        return Mesh::default();
    }
    if stroke.is_closed() {
        return draw_cut_band_core_solid(stroke, params, bbox, face_offset, mesh, surface_only);
    }
    // The open stroke: a ruled strip. (The reference's ruled-strip branch
    // for a closed stroke cannot be reached, a loop having returned above,
    // and is not ported.)
    let p = stroke.path();
    let ext = params.extension.max(0.0);
    let diag = bbox.map_or(100.0, |(lo, hi)| norm(sub(hi, lo)));
    let depth = if params.through_all {
        1.05 * diag + 1.0
    } else {
        params.depth.max(0.01)
    };
    let n = p.len();
    // One sweep direction for the whole strip: the strip normals t x d,
    // kept coherent.
    let mut sweep = [0.0; 3];
    for i in 0..n {
        let d = draw_cut_inward_dir(stroke, params, i);
        let sn = cross(stroke.tangent(i), d);
        if norm(sn) < 1e-9 {
            continue;
        }
        let mut sn = unit(sn, X);
        if sweep != [0.0; 3] && dot(sn, sweep) < 0.0 {
            sn = neg(sn);
        }
        sweep = add(sweep, sn);
    }
    let sweep = unit(sweep, Y);
    let shift = mul(neg(sweep), face_offset);
    let clearance = &params.extension_clearance;
    let clamp_ext = clearance.len() == n + 2;
    let clamped = |k: usize| {
        if clamp_ext {
            ext.min(clearance[k].max(0.0))
        } else {
            ext
        }
    };
    // Each rail: (out, in) on the ruling through the point.
    let rail = |pos: V, out_ext: f64, d: V| {
        (
            add(sub(pos, mul(d, out_ext)), shift),
            add(add(pos, mul(d, depth)), shift),
        )
    };
    let mut rails: Vec<(V, V)> = Vec::with_capacity(n + 2);
    let t0 = stroke.tangent(0);
    rails.push(rail(
        sub(p[0].pos, mul(t0, clamped(0))),
        clamped(1),
        draw_cut_inward_dir(stroke, params, 0),
    ));
    for (i, sample) in p.iter().enumerate() {
        rails.push(rail(
            sample.pos,
            clamped(i + 1),
            draw_cut_inward_dir(stroke, params, i),
        ));
    }
    let tn = stroke.tangent(n - 1);
    rails.push(rail(
        add(p[n - 1].pos, mul(tn, clamped(n + 1))),
        clamped(n),
        draw_cut_inward_dir(stroke, params, n - 1),
    ));
    let m = rails.len();
    let mut its = Mesh::default();
    let mu = m as u32;
    let a = |i: u32| i;
    let b = |i: u32| mu + i;
    if surface_only {
        its.vertices.extend(rails.iter().map(|r| f32s(r.0)));
        its.vertices.extend(rails.iter().map(|r| f32s(r.1)));
        for i in 0..mu - 1 {
            let j = i + 1;
            its.triangles.push([a(i), b(i), b(j)]);
            its.triangles.push([a(i), b(j), a(j)]);
        }
        return its;
    }
    let reach = 1.05 * diag + 1.0;
    let far = mul(sweep, reach);
    its.vertices.extend(rails.iter().map(|r| f32s(r.0)));
    its.vertices.extend(rails.iter().map(|r| f32s(r.1)));
    its.vertices
        .extend(rails.iter().map(|r| f32s(add(r.0, far))));
    its.vertices
        .extend(rails.iter().map(|r| f32s(add(r.1, far))));
    let a2 = |i: u32| 2 * mu + i;
    let b2 = |i: u32| 3 * mu + i;
    for i in 0..mu - 1 {
        let j = i + 1;
        its.triangles.push([a(i), b(i), b(j)]);
        its.triangles.push([a(i), b(j), a(j)]);
        its.triangles.push([a2(i), b2(j), b2(i)]);
        its.triangles.push([a2(i), a2(j), b2(j)]);
    }
    for i in 0..mu - 1 {
        let j = i + 1;
        its.triangles.push([a(i), a(j), a2(j)]);
        its.triangles.push([a(i), a2(j), a2(i)]);
        its.triangles.push([b(i), b2(i), b2(j)]);
        its.triangles.push([b(i), b2(j), b(j)]);
    }
    let l = mu - 1;
    its.triangles.push([a(0), a2(0), b2(0)]);
    its.triangles.push([a(0), b2(0), b(0)]);
    its.triangles.push([a(l), b(l), b2(l)]);
    its.triangles.push([a(l), b2(l), a2(l)]);
    outward(its)
}

/// The closed, outward-wound cutter for `stroke` (`draw_cut_cutter_solid`),
/// in the plane's frame: `bbox` is the object's box there (none: a 100 mm
/// diagonal is assumed), `face_offset` moves the surface for a kerf, and
/// `mesh` (the object, in the same frame) lets a loop be told a plug from
/// a loop round the part and lets the plug's lid follow the skin. Empty
/// for a stroke that is not valid and for a loop whose depth eats its
/// core.
pub fn draw_cut_cutter_solid(
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
    bbox: Option<Bounds>,
    face_offset: f64,
    mesh: Option<&Mesh>,
) -> Mesh {
    draw_cut_cutter_solid_impl(stroke, params, bbox, face_offset, mesh, false)
}

/// Only the surface the user drew, for the preview: skirt, band and core,
/// or the open strip, without what closes it for the boolean
/// (`draw_cut_preview_surface`). Not a closed mesh.
pub fn draw_cut_preview_surface(
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
    bbox: Option<Bounds>,
    face_offset: f64,
    mesh: Option<&Mesh>,
) -> Mesh {
    draw_cut_cutter_solid_impl(stroke, params, bbox, face_offset, mesh, true)
}

// ---------------------------------------------------------------------------
// Inside or out.
// ---------------------------------------------------------------------------

/// The barycentric crossing of a ray from `pt` along the unit `dir` with
/// each triangle, as the reference's parity tests solve it: the distance
/// along the ray of each one met (behind the point too).
fn ray_crossings(solid: &Mesh, pt: V, dir: V, mut each: impl FnMut(f64)) {
    let ax = if dir[0].abs() < 0.9 { X } else { Y };
    let e0 = unit(reject(ax, dir), X);
    let e1 = cross(dir, e0);
    for tri in &solid.triangles {
        let uvw = tri.map(|v| {
            let rel = sub(solid.vertices[v as usize].map(f64::from), pt);
            [dot(rel, e0), dot(rel, e1), dot(rel, dir)]
        });
        let det = (uvw[1][1] - uvw[2][1]) * (uvw[0][0] - uvw[2][0])
            + (uvw[2][0] - uvw[1][0]) * (uvw[0][1] - uvw[2][1]);
        if det.abs() < 1e-12 {
            continue;
        }
        let l0 =
            ((uvw[1][1] - uvw[2][1]) * (-uvw[2][0]) + (uvw[2][0] - uvw[1][0]) * (-uvw[2][1])) / det;
        let l1 =
            ((uvw[2][1] - uvw[0][1]) * (-uvw[2][0]) + (uvw[0][0] - uvw[2][0]) * (-uvw[2][1])) / det;
        let l2 = 1.0 - l0 - l1;
        if l0 < 0.0 || l1 < 0.0 || l2 < 0.0 {
            continue;
        }
        each(l0 * uvw[0][2] + l1 * uvw[1][2] + l2 * uvw[2][2]);
    }
}

/// Whether `pt` is inside a closed mesh, by the parity of a ray along a
/// fixed direction with no simple ratios between its parts
/// (`point_in_solid`).
fn point_in_solid(solid: &Mesh, pt: V) -> bool {
    let dir = unit([0.3131592653, 0.4271828182, 0.8481471805], Z);
    let mut crossings = 0usize;
    ray_crossings(solid, pt, dir, |w| {
        if w > 0.0 {
            crossings += 1;
        }
    });
    crossings % 2 == 1
}

/// How far along the unit `dir` from `pt` the nearest surface is, past
/// 0.05 mm; infinity when none (`ray_first_hit`).
fn ray_first_hit(solid: &Mesh, pt: V, dir: V) -> f64 {
    let mut best = f64::INFINITY;
    ray_crossings(solid, pt, dir, |w| {
        if w > 0.05 && w < best {
            best = w;
        }
    });
    best
}

/// Which half a point falls in, by `draw_cut_split`'s convention: true is
/// the upper half, which for a loop is inside the cutter and for an open
/// stroke outside it (`draw_cut_classify_upper`).
pub fn draw_cut_classify_upper(cutter: &Mesh, closed: bool, p: V) -> bool {
    if cutter.triangles.is_empty() {
        return false;
    }
    let inside = point_in_solid(cutter, p);
    if closed { inside } else { !inside }
}

/// The heights of the lower and the upper cut face off the surface
/// (CurvedCut.cpp `curved_cut_thickness_faces`).
fn thickness_faces(thickness: f64, offset: CutThickness) -> [f64; 2] {
    let t = thickness.max(0.0);
    match offset {
        CutThickness::Above => [0.0, t],
        CutThickness::Below => [-t, 0.0],
        CutThickness::Centred => [-0.5 * t, 0.5 * t],
    }
}

fn bounds_of(mesh: &Mesh) -> Option<Bounds> {
    mesh.bounds()
        .map(|(lo, hi)| (lo.map(f64::from), hi.map(f64::from)))
}

/// Which side a drawn cut would leave empty, `[upper empty, lower empty]`,
/// from the mesh's vertices (at most 20 000, strided) against the cutter
/// (`draw_cut_empty_sides`). Both true for a stroke that cannot be cut
/// with.
pub fn draw_cut_empty_sides(
    mesh: &Mesh,
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
) -> [bool; 2] {
    if mesh.vertices.is_empty() || !stroke.valid() {
        return [true, true];
    }
    let bbox = bounds_of(mesh);
    let [face_lo, face_hi] = thickness_faces(params.thickness, params.thickness_offset);
    let kerf = params.thickness > 0.0;
    let cutter_lo = draw_cut_cutter_solid(stroke, params, bbox, face_lo, Some(mesh));
    if cutter_lo.triangles.is_empty() {
        return [true, true];
    }
    let cutter_hi = if kerf {
        draw_cut_cutter_solid(stroke, params, bbox, face_hi, Some(mesh))
    } else {
        Mesh::default()
    };
    const MAX_PROBES: usize = 20000;
    let n = mesh.vertices.len();
    let stride = if n > MAX_PROBES {
        n.div_ceil(MAX_PROBES)
    } else {
        1
    };
    let (mut inside_empty, mut outside_empty) = (true, true);
    for v in mesh.vertices.iter().step_by(stride) {
        let pt = v.map(f64::from);
        if inside_empty && point_in_solid(&cutter_lo, pt) {
            inside_empty = false;
        }
        if outside_empty && !point_in_solid(if kerf { &cutter_hi } else { &cutter_lo }, pt) {
            outside_empty = false;
        }
        if !inside_empty && !outside_empty {
            break;
        }
    }
    // DIFFERS FROM THE REFERENCE, which asks only the mesh's vertices. A
    // plug on a face of a coarse mesh (a loop on a box's top) holds none
    // of them, so the reference reports its side empty ("The stroke does
    // not separate the part") for a cut that does separate it. A point of
    // the cutter's own surface that lies well inside the mesh has material
    // on both sides of that surface round it, so a side still empty is
    // also asked of the cutters' vertices: the lower face's for the inside,
    // the upper face's for the outside (the same one without a kerf).
    const MAX_SURFACE_PROBES: usize = 64;
    let deep_inside = |pt: V| {
        let step = 0.05;
        point_in_solid(mesh, pt)
            && [X, Y, Z].iter().all(|axis| {
                point_in_solid(mesh, add(pt, mul(*axis, step)))
                    && point_in_solid(mesh, sub(pt, mul(*axis, step)))
            })
    };
    let any_deep = |cutter: &Mesh| {
        let stride = cutter.vertices.len().div_ceil(MAX_SURFACE_PROBES).max(1);
        cutter
            .vertices
            .iter()
            .step_by(stride)
            .any(|v| deep_inside(v.map(f64::from)))
    };
    if inside_empty && any_deep(&cutter_lo) {
        inside_empty = false;
    }
    if outside_empty && any_deep(if kerf { &cutter_hi } else { &cutter_lo }) {
        outside_empty = false;
    }
    if stroke.is_closed() {
        [inside_empty, outside_empty]
    } else {
        [outside_empty, inside_empty]
    }
}

/// Why a drawn cut gave no halves.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawSplitError {
    /// The stroke or the cut (the reference's `DrawCutError`).
    Draw(DrawCutError),
    /// The boolean.
    Cut(CurvedError),
}

impl std::fmt::Display for DrawSplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Draw(error) => write!(f, "{error}"),
            Self::Cut(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DrawSplitError {}

/// The cutter(s) a split uses: the one at the lower face and, with a kerf,
/// the one at the upper, built against `mesh` (the object in the plane's
/// frame).
pub fn draw_cut_cutters(
    mesh: &Mesh,
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
) -> Result<(Mesh, Option<Mesh>), DrawCutError> {
    if mesh.triangles.is_empty() {
        return Err(DrawCutError::CutterDegenerate);
    }
    if !stroke.valid() {
        return Err(match stroke.error() {
            DrawCutError::None => DrawCutError::TooShort,
            error => error,
        });
    }
    let bbox = bounds_of(mesh);
    let t = params.thickness.max(0.0);
    let [face_lo, face_hi] = thickness_faces(t, params.thickness_offset);
    let cutter_lo = draw_cut_cutter_solid(stroke, params, bbox, face_lo, Some(mesh));
    if cutter_lo.triangles.is_empty() {
        return Err(DrawCutError::CutterDegenerate);
    }
    if t > 0.0 {
        let cutter_hi = draw_cut_cutter_solid(stroke, params, bbox, face_hi, Some(mesh));
        if cutter_hi.triangles.is_empty() {
            return Err(DrawCutError::CutterDegenerate);
        }
        return Ok((cutter_lo, Some(cutter_hi)));
    }
    Ok((cutter_lo, None))
}

/// The part cut along the drawn stroke, `(upper, lower)`
/// (`draw_cut_split`). `object` is `mesh` made a solid; both in the
/// plane's frame. For a loop the upper half is the plug, what the cutter
/// holds; for an open stroke it is what the swept cutter does not hold,
/// the plane's own convention. A side the boolean leaves empty is
/// [`DrawCutError::EmptySide`], as the reference reports it.
///
/// The reference also logs a fold warning here (`draw_cut_band_folds`);
/// the panel asks that itself, so it is not repeated.
pub fn draw_cut_split(
    object: &Solid,
    mesh: &Mesh,
    stroke: &DrawCutStroke,
    params: &DrawCutParams,
    cancel: Option<&Cancel>,
) -> Result<(Mesh, Mesh), DrawSplitError> {
    let (cutter_lo, cutter_hi) =
        draw_cut_cutters(mesh, stroke, params).map_err(DrawSplitError::Draw)?;
    // cut_with_solid's lower half is the object in the cutter, its upper
    // the object out of it.
    let (outside, inside) = curved::cut_with_solid(object, &cutter_lo, cutter_hi.as_ref(), cancel)
        .map_err(DrawSplitError::Cut)?;
    let (upper, lower) = if stroke.is_closed() {
        (inside, outside)
    } else {
        (outside, inside)
    };
    if upper.triangles.is_empty() || lower.triangles.is_empty() {
        return Err(DrawSplitError::Draw(DrawCutError::EmptySide));
    }
    Ok((upper, lower))
}

// ---------------------------------------------------------------------------
// The chain, as far as the stored form needs it.
// ---------------------------------------------------------------------------

/// Which end of a chain a stroke continues (`DrawChainEnd`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawChainEnd {
    None,
    Front,
    Back,
}

/// The line as a chain of strokes (`DrawCutChain`): the raw samples in
/// order, each appended stroke's range, and whether it is a loop or a line
/// the user finished open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawCutChain {
    samples: Vec<DrawCutSample>,
    bounds: Vec<(usize, usize)>,
    closed: bool,
    finished_open: bool,
}

impl DrawCutChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn samples(&self) -> &[DrawCutSample] {
        &self.samples
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn is_finished_open(&self) -> bool {
        self.finished_open
    }

    pub fn stroke_count(&self) -> usize {
        self.bounds.len()
    }

    /// `[begin, end)` of each appended stroke, newest last.
    pub fn stroke_bounds(&self) -> &[(usize, usize)] {
        &self.bounds
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The chain rebuilt from stored samples as one stroke
    /// (`set_samples`).
    pub fn set_samples(&mut self, samples: &[DrawCutSample], closed: bool, finished_open: bool) {
        self.samples = samples.to_vec();
        self.bounds.clear();
        if !self.samples.is_empty() {
            self.bounds.push((0, self.samples.len()));
        }
        self.closed = closed && self.samples.len() >= MIN_CHAIN_SAMPLES;
        self.finished_open =
            !self.closed && finished_open && self.samples.len() >= MIN_CHAIN_SAMPLES;
    }

    /// Which end a stroke starting at `p` continues (`end_for_start`).
    pub fn end_for_start(&self, p: V, snap_radius: f64) -> DrawChainEnd {
        if self.samples.is_empty() {
            return DrawChainEnd::Back;
        }
        if self.closed {
            return DrawChainEnd::None;
        }
        let r = snap_radius.max(1e-6);
        let d_bk = norm(sub(p, self.samples[self.samples.len() - 1].pos));
        let d_fr = norm(sub(p, self.samples[0].pos));
        if d_bk <= r && d_bk <= d_fr {
            DrawChainEnd::Back
        } else if d_fr <= r {
            DrawChainEnd::Front
        } else {
            DrawChainEnd::None
        }
    }

    /// A stroke appended at the end its first sample continues (`append`).
    pub fn append(&mut self, stroke: &[DrawCutSample], snap_radius: f64) -> DrawChainEnd {
        if stroke.len() < 2 {
            return DrawChainEnd::None;
        }
        let at = self.end_for_start(stroke[0].pos, snap_radius);
        self.append_at(stroke, at, snap_radius)
    }

    /// A stroke appended at a given end, a front one reversed; it closes
    /// the chain when its last sample is within `snap_radius` of the far
    /// end (`append_at`).
    pub fn append_at(
        &mut self,
        stroke: &[DrawCutSample],
        at: DrawChainEnd,
        snap_radius: f64,
    ) -> DrawChainEnd {
        if stroke.len() < 2 || at == DrawChainEnd::None || self.closed {
            return DrawChainEnd::None;
        }
        let r = snap_radius.max(1e-6);
        let first = self.samples.is_empty();
        let far_end = if first {
            stroke[0].pos
        } else if at == DrawChainEnd::Back {
            self.samples[0].pos
        } else {
            self.samples[self.samples.len() - 1].pos
        };
        let mut skip = 0;
        if !first {
            let join = if at == DrawChainEnd::Back {
                self.samples[self.samples.len() - 1].pos
            } else {
                self.samples[0].pos
            };
            if norm(sub(stroke[0].pos, join)) < 1e-9 {
                skip = 1;
            }
        }
        let added = &stroke[skip..];
        let closes = self.samples.len() + added.len() >= MIN_CHAIN_SAMPLES
            && norm(sub(stroke[stroke.len() - 1].pos, far_end)) <= r;
        if at == DrawChainEnd::Back {
            let begin = self.samples.len();
            self.samples.extend_from_slice(added);
            self.bounds.push((begin, self.samples.len()));
        } else {
            let mut rev = added.to_vec();
            rev.reverse();
            let count = rev.len();
            rev.extend_from_slice(&self.samples);
            self.samples = rev;
            for b in &mut self.bounds {
                b.0 += count;
                b.1 += count;
            }
            self.bounds.push((0, count));
        }
        if closes {
            self.closed = true;
        }
        self.finished_open = false;
        at
    }

    /// The panel's "Close loop" (`force_close`).
    pub fn force_close(&mut self) -> bool {
        if self.samples.len() < MIN_CHAIN_SAMPLES {
            return false;
        }
        self.closed = true;
        self.finished_open = false;
        true
    }

    /// The panel's "Cut along the line" (`finish_open`).
    pub fn finish_open(&mut self) -> bool {
        if self.closed || self.samples.len() < MIN_CHAIN_SAMPLES {
            return false;
        }
        self.finished_open = true;
        true
    }

    /// The stroke the chain makes, and its error (`DrawCutChain::finish`):
    /// none for a chain neither closed nor finished open.
    pub fn finish(&self, spacing: f64, smoothing: f64) -> (DrawCutStroke, DrawCutError) {
        let mut out = DrawCutStroke::new();
        if !self.closed && !self.finished_open {
            return (out, DrawCutError::NotClosed);
        }
        if self.samples.len() < MIN_CHAIN_SAMPLES {
            return (out, DrawCutError::TooShort);
        }
        for s in &self.samples {
            out.append(s.pos, s.normal, s.facet);
        }
        if self.finished_open {
            let error = out.finish(spacing, smoothing, false);
            if error == DrawCutError::None && out.is_closed() {
                return (DrawCutStroke::new(), DrawCutError::NotClosed);
            }
            return (out, error);
        }
        let error = out.finish(spacing, smoothing, true);
        (out, error)
    }
}

// ---------------------------------------------------------------------------
// The stored form (CutRecipe.cpp).
// ---------------------------------------------------------------------------

/// The stroke as a recipe stores it (`CutRecipeStroke`).
#[derive(Clone, Debug, PartialEq)]
pub struct RecipeStroke {
    pub samples: Vec<DrawCutSample>,
    pub closed: bool,
    /// The panel's 0..1 smoothing, an input to the finish.
    pub smoothing: f64,
    /// Each stroke's range in `samples`, in sample order; empty for a
    /// version 1 recipe.
    pub stroke_bounds: Vec<(u32, u32)>,
    pub finished_open: bool,
}

impl Default for RecipeStroke {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            closed: false,
            smoothing: 0.2,
            stroke_bounds: Vec::new(),
            finished_open: false,
        }
    }
}

/// The chain a stored stroke describes (`cut_recipe_stroke_to_chain`):
/// the strokes replayed when their ranges tile the samples, else one
/// stroke; an unclosed stored line is finished open.
pub fn recipe_stroke_to_chain(stored: &RecipeStroke) -> DrawCutChain {
    let mut out = DrawCutChain::new();
    if stored.samples.is_empty() {
        return out;
    }
    let n = stored.samples.len();
    let mut tiles = !stored.stroke_bounds.is_empty();
    let mut expect = 0usize;
    for &(b0, b1) in &stored.stroke_bounds {
        let (b0, b1) = (b0 as usize, b1 as usize);
        if b0 != expect || b1 <= b0 || b1 > n {
            tiles = false;
            break;
        }
        expect = b1;
    }
    if tiles && expect != n {
        tiles = false;
    }
    let fin_open = !stored.closed && (stored.finished_open || n >= MIN_CHAIN_SAMPLES);
    let fallback =
        |out: &mut DrawCutChain| out.set_samples(&stored.samples, stored.closed, fin_open);
    if !tiles {
        fallback(&mut out);
        return out;
    }
    // Every replayed stroke is a back append with a tiny snap radius: it
    // passes every start test (the joins are exact) and fails every
    // closure test; the closure is restored from the stored flag.
    let any = 1e-9;
    for &(b0, b1) in &stored.stroke_bounds {
        let mut stroke = stored.samples[b0 as usize..b1 as usize].to_vec();
        if stroke.len() < 2 {
            fallback(&mut out);
            return out;
        }
        if !out.is_empty() {
            stroke.insert(0, out.samples[out.len() - 1]);
        }
        if out.append(&stroke, any) == DrawChainEnd::None {
            fallback(&mut out);
            return out;
        }
    }
    if out.len() != n {
        fallback(&mut out);
        return out;
    }
    if stored.closed {
        out.force_close();
    } else if fin_open {
        out.finish_open();
    }
    out
}

/// A chain as a recipe stores it (`cut_recipe_stroke_from_chain`): its
/// ranges sorted by position.
pub fn recipe_stroke_from_chain(chain: &DrawCutChain, smoothing: f64) -> RecipeStroke {
    let mut stroke_bounds: Vec<(u32, u32)> = chain
        .stroke_bounds()
        .iter()
        .map(|&(a, b)| (a as u32, b as u32))
        .collect();
    stroke_bounds.sort_unstable();
    RecipeStroke {
        samples: chain.samples().to_vec(),
        closed: chain.is_closed(),
        smoothing,
        stroke_bounds,
        finished_open: chain.is_finished_open(),
    }
}

/// A drawn cut stored before version 3 mapped onto the band-and-core
/// surface (`CutRecipe::migrate_draw_v2`): the old draft becomes a lip of
/// 90 - |draft| (0..90), the depth the default 3 mm, through all off.
/// Nothing for a later version or another kind of cut; the caller stamps
/// the current version.
pub fn migrate_draw_v2(version: u32, drawn: bool, params: &mut DrawCutParams) {
    if version <= 2 && drawn {
        params.angle_deg = (90.0 - params.angle_deg.abs()).clamp(0.0, MAX_LIP_ANGLE_DEG);
        params.depth = 3.0;
        params.through_all = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use edge_mesh::Transform;
    use edge_mesh::primitives::cuboid;

    fn sample(pos: V, normal: V) -> DrawCutSample {
        DrawCutSample {
            pos,
            normal,
            facet: 0,
        }
    }

    fn block() -> Mesh {
        Mesh::from_triangle_soup(cuboid([-10.0, -10.0, -10.0], [10.0, 10.0, 10.0]))
    }

    /// A closed cylinder of `sides` round the z axis.
    fn cylinder(r: f64, z0: f64, z1: f64, sides: usize) -> Mesh {
        let mut mesh = Mesh::default();
        for k in 0..sides {
            let a = std::f64::consts::TAU * k as f64 / sides as f64;
            mesh.vertices.push(f32s([r * a.cos(), r * a.sin(), z0]));
            mesh.vertices.push(f32s([r * a.cos(), r * a.sin(), z1]));
        }
        let (bc, tc) = (2 * sides as u32, 2 * sides as u32 + 1);
        mesh.vertices.push(f32s([0.0, 0.0, z0]));
        mesh.vertices.push(f32s([0.0, 0.0, z1]));
        let s = sides as u32;
        for k in 0..s {
            let j = (k + 1) % s;
            let (b0, t0, b1, t1) = (2 * k, 2 * k + 1, 2 * j, 2 * j + 1);
            mesh.triangles.push([b0, b1, t1]);
            mesh.triangles.push([b0, t1, t0]);
            mesh.triangles.push([bc, b1, b0]);
            mesh.triangles.push([tc, t0, t1]);
        }
        mesh
    }

    fn sound(cutter: &Mesh) {
        assert!(!cutter.triangles.is_empty());
        assert!(edge_mesh::cut::is_closed(cutter), "cutter not closed");
        assert!(cutter.signed_volume() > 0.0, "cutter wound inward");
        assert_eq!(edge_mesh::cut::flat_triangles(cutter), 0, "flat triangles");
        assert!(Solid::new(cutter).is_ok());
    }

    fn halves_sound(upper: &Mesh, lower: &Mesh) {
        for half in [upper, lower] {
            assert!(edge_mesh::cut::is_closed(half));
            assert!(half.signed_volume() > 0.0);
        }
    }

    fn line_across_top(y: f64) -> DrawCutStroke {
        let mut stroke = DrawCutStroke::new();
        for k in 0..=38 {
            stroke.append([-9.5 + 0.5 * k as f64, y, 10.0], Z, 0);
        }
        assert_eq!(
            stroke.finish(DEFAULT_SPACING, 0.2, false),
            DrawCutError::None
        );
        assert!(!stroke.is_closed());
        stroke
    }

    fn ring_round_cylinder(r: f64, z: f64, count: usize, wave: f64) -> DrawCutStroke {
        let mut stroke = DrawCutStroke::new();
        for k in 0..count {
            let a = std::f64::consts::TAU * k as f64 / count as f64;
            stroke.append(
                [r * a.cos(), r * a.sin(), z + wave * (3.0 * a).sin()],
                [a.cos(), a.sin(), 0.0],
                0,
            );
        }
        stroke
    }

    #[test]
    fn resampling_keeps_spacing_and_ends() {
        let line: Vec<DrawCutSample> = (0..=7)
            .map(|k| sample([k as f64 * 1.4, 0.0, 0.0], Z))
            .collect();
        let out = draw_cut_resample(&line, 1.0, false);
        assert_eq!(out[0].pos, [0.0; 3]);
        assert_eq!(out[out.len() - 1].pos, line[7].pos);
        // A last sample within half a spacing of the one before is not kept.
        let short = draw_cut_resample(&line[..7], 1.0, false);
        assert_eq!(short[short.len() - 1].pos, [8.0, 0.0, 0.0]);
        for w in out[..out.len() - 1].windows(2) {
            assert!((norm(sub(w[1].pos, w[0].pos)) - 1.0).abs() < 1e-9);
        }
        // A ring: the closing span walked, no duplicate of the first, the
        // closing span from half to one and a half spacings.
        let ring: Vec<DrawCutSample> = (0..12)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 12.0;
                sample([5.0 * a.cos(), 5.0 * a.sin(), 0.0], Z)
            })
            .collect();
        let out = draw_cut_resample(&ring, 0.5, true);
        assert!(norm(sub(out[out.len() - 1].pos, out[0].pos)) >= 0.25);
        assert!(norm(sub(out[out.len() - 1].pos, out[0].pos)) < 0.75);
        // Smoothing: an open stroke's ends held, a loop stays a loop.
        let mut zig: Vec<DrawCutSample> = (0..10)
            .map(|k| sample([k as f64, if k % 2 == 0 { 0.0 } else { 1.0 }, 0.0], Z))
            .collect();
        let (first, last) = (zig[0], zig[9]);
        draw_cut_smooth(&mut zig, 3, false, 0.5);
        assert_eq!((zig[0], zig[9]), (first, last));
        assert!(zig[4].pos[1] > 0.2 && zig[4].pos[1] < 0.8);
        assert_eq!(draw_cut_smooth_passes(0.2), 2);
        assert_eq!(draw_cut_closing_tolerance(1.0), 3.0);
        let mut stroke = ring_round_cylinder(10.0, 0.0, 40, 0.0);
        assert_eq!(stroke.finish(1.0, 1.0, false), DrawCutError::None);
        assert!(stroke.is_closed());
        let step = norm(sub(stroke.path()[1].pos, stroke.path()[0].pos));
        let close = norm(sub(
            stroke.path()[0].pos,
            stroke.path()[stroke.path().len() - 1].pos,
        ));
        assert!(close < 2.0 * step);
    }

    #[test]
    fn finish_reports_the_references_errors() {
        let mut stroke = DrawCutStroke::new();
        stroke.append([0.0; 3], Z, 0);
        assert_eq!(stroke.finish(1.0, 0.2, false), DrawCutError::TooShort);
        stroke.append([1.0, 0.0, 0.0], Z, 0);
        assert_eq!(stroke.finish(1.0, 0.2, false), DrawCutError::TooShort);
        assert!(!stroke.valid());
        // A long line with a jump in it: the longest run kept, and when that
        // is too short the line left the model.
        let mut jumpy = DrawCutStroke::new();
        jumpy.append([0.0; 3], Z, 0);
        jumpy.append([1.0, 0.0, 0.0], Z, 0);
        jumpy.append([50.0, 0.0, 0.0], Z, 0);
        assert_eq!(jumpy.finish(1.0, 0.2, false), DrawCutError::LeavesMesh);
        // A figure of eight crosses itself. (Started off the crossing: a
        // crossing exactly at a sample is a touch at a segment's end, which
        // the reference's test does not count either.)
        let mut eight = DrawCutStroke::new();
        for k in 0..80 {
            let a = std::f64::consts::TAU * k as f64 / 80.0 + 0.3;
            eight.append([10.0 * a.sin(), 5.0 * (2.0 * a).sin(), 0.0], Z, 0);
        }
        assert_eq!(eight.finish(1.0, 0.0, false), DrawCutError::SelfCrossing);
        assert_eq!(
            draw_cut_error_message(DrawCutError::SelfCrossing),
            "The line crosses itself"
        );
        // A chain neither closed nor finished makes no stroke.
        let mut chain = DrawCutChain::new();
        let samples: Vec<DrawCutSample> =
            (0..10).map(|k| sample([k as f64, 0.0, 0.0], Z)).collect();
        assert_eq!(chain.append(&samples, 1.0), DrawChainEnd::Back);
        assert_eq!(chain.finish(1.0, 0.2).1, DrawCutError::NotClosed);
        assert!(chain.finish_open());
        let (stroke, error) = chain.finish(1.0, 0.2);
        assert_eq!(error, DrawCutError::None);
        assert!(stroke.valid() && !stroke.is_closed());
    }

    #[test]
    fn a_stored_chain_comes_back_as_it_was() {
        let samples: Vec<DrawCutSample> =
            (0..12).map(|k| sample([k as f64, 0.0, 0.0], Z)).collect();
        let stored = RecipeStroke {
            samples: samples.clone(),
            stroke_bounds: vec![(0, 5), (5, 12)],
            ..RecipeStroke::default()
        };
        let chain = recipe_stroke_to_chain(&stored);
        assert_eq!(chain.samples(), &samples[..]);
        assert_eq!(chain.stroke_bounds(), &[(0, 5), (5, 12)]);
        assert!(chain.is_finished_open() && !chain.is_closed());
        let back = recipe_stroke_from_chain(&chain, 0.2);
        assert_eq!(
            back,
            RecipeStroke {
                finished_open: true,
                ..stored.clone()
            }
        );
        // Ranges that do not tile: one stroke.
        let broken = RecipeStroke {
            stroke_bounds: vec![(0, 5), (6, 12)],
            closed: true,
            ..stored
        };
        let chain = recipe_stroke_to_chain(&broken);
        assert_eq!(chain.stroke_bounds(), &[(0, 12)]);
        assert!(chain.is_closed());
        let mut params = DrawCutParams {
            angle_deg: -20.0,
            depth: 40.0,
            through_all: true,
            ..DrawCutParams::default()
        };
        migrate_draw_v2(3, true, &mut params);
        assert_eq!(params.angle_deg, -20.0);
        migrate_draw_v2(2, true, &mut params);
        assert_eq!(
            (params.angle_deg, params.depth, params.through_all),
            (70.0, 3.0, false)
        );
    }

    #[test]
    fn an_open_line_across_a_box_halves_it_like_a_plane() {
        let mesh = block();
        let object = Solid::new(&mesh).unwrap();
        let params = DrawCutParams {
            through_all: true,
            ..DrawCutParams::default()
        };
        for y in [0.0, 3.0] {
            let stroke = line_across_top(y);
            let (cutter, _) = draw_cut_cutters(&mesh, &stroke, &params).unwrap();
            sound(&cutter);
            sound_preview(&stroke, &params, &mesh);
            let (upper, lower) = draw_cut_split(&object, &mesh, &stroke, &params, None).unwrap();
            halves_sound(&upper, &lower);
            let (vu, vl) = (upper.signed_volume(), lower.signed_volume());
            assert!((vu + vl - 8000.0).abs() < 1e-2, "{vu} + {vl}");
            // A plane at y, its normal along the cut's sweep.
            let to_plane = Transform {
                rows: [
                    [1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [0.0, -1.0, 0.0],
                    [0.0, 0.0, -y],
                ],
            };
            let flat = edge_mesh::cut::cut(&mesh, &to_plane, [0.0, 0.0], &Transform::IDENTITY);
            let mut plane = [
                flat.upper.mesh.signed_volume(),
                flat.lower.mesh.signed_volume(),
            ];
            plane.sort_by(f64::total_cmp);
            let mut drawn = [vu, vl];
            drawn.sort_by(f64::total_cmp);
            assert!((plane[0] - drawn[0]).abs() < 1e-2, "{plane:?} {drawn:?}");
            assert_eq!(
                draw_cut_empty_sides(&mesh, &stroke, &params),
                [false, false]
            );
        }
        // A line beside the box: its swept cutter (towards +y) holds none
        // of it, so the lower side is empty.
        let beside = line_across_top(15.0);
        assert_eq!(draw_cut_empty_sides(&mesh, &beside, &params), [false, true]);
        assert_eq!(
            draw_cut_split(&object, &mesh, &beside, &params, None).err(),
            Some(DrawSplitError::Draw(DrawCutError::EmptySide))
        );
        // A kerf takes a band 1 mm wide out of the middle.
        let stroke = line_across_top(0.0);
        let kerf = DrawCutParams {
            thickness: 1.0,
            ..params
        };
        let (lo, hi) = draw_cut_cutters(&mesh, &stroke, &kerf).unwrap();
        sound(&lo);
        sound(hi.as_ref().unwrap());
        let (upper, lower) = draw_cut_split(&object, &mesh, &stroke, &kerf, None).unwrap();
        let sum = upper.signed_volume() + lower.signed_volume();
        assert!((sum - (8000.0 - 400.0)).abs() < 1e-2, "{sum}");
    }

    fn sound_preview(stroke: &DrawCutStroke, params: &DrawCutParams, mesh: &Mesh) {
        let preview = draw_cut_preview_surface(stroke, params, bounds_of(mesh), 0.0, Some(mesh));
        assert!(!preview.triangles.is_empty());
        assert_eq!(edge_mesh::cut::flat_triangles(&preview), 0);
    }

    #[test]
    fn a_loop_round_a_cylinder_halves_it_on_a_flat_core() {
        let mesh = cylinder(10.0, -15.0, 15.0, 96);
        let whole = mesh.signed_volume();
        let object = Solid::new(&mesh).unwrap();
        for wave in [0.0, 1.0] {
            let mut stroke = ring_round_cylinder(10.0, 2.0, 120, wave);
            assert_eq!(stroke.finish(1.0, 0.2, false), DrawCutError::None);
            assert!(stroke.is_closed());
            for angle in [0.0, 45.0, -30.0] {
                let params = DrawCutParams {
                    angle_deg: angle,
                    ..DrawCutParams::default()
                };
                assert!(draw_cut_loop_separates(&mesh, &stroke, &params, 0.5));
                let (cutter, _) = draw_cut_cutters(&mesh, &stroke, &params).unwrap();
                sound(&cutter);
                sound_preview(&stroke, &params, &mesh);
                let (upper, lower) =
                    draw_cut_split(&object, &mesh, &stroke, &params, None).unwrap();
                halves_sound(&upper, &lower);
                let sum = upper.signed_volume() + lower.signed_volume();
                assert!((sum - whole).abs() < 1e-2, "{angle}: {sum} against {whole}");
                // The core is flat: whatever of either half lies within the
                // core's radius near the plane lies on it.
                let (n, at) = draw_cut_core_face(&stroke, &params, Some(&mesh)).unwrap();
                let core_r = 10.0 - 3.0 * angle.to_radians().cos() - 1.5;
                for half in [&upper, &lower] {
                    for v in &half.vertices {
                        let v = v.map(f64::from);
                        let h = dot(sub(v, at), n);
                        if norm(reject(sub(v, at), n)) < core_r && h.abs() < 2.0 {
                            assert!(h.abs() < 1e-3, "{angle}: {h}");
                        }
                    }
                }
                // A kerf takes out roughly the band.
                let kerf = DrawCutParams {
                    thickness: 1.0,
                    ..params
                };
                let (lo, hi) = draw_cut_cutters(&mesh, &stroke, &kerf).unwrap();
                sound(&lo);
                sound(hi.as_ref().unwrap());
                let (upper, lower) = draw_cut_split(&object, &mesh, &stroke, &kerf, None).unwrap();
                halves_sound(&upper, &lower);
                let gone = whole - upper.signed_volume() - lower.signed_volume();
                let band = std::f64::consts::PI * 100.0;
                assert!(gone > 0.8 * band && gone < 1.2 * band, "{angle}: {gone}");
            }
        }
    }

    #[test]
    fn a_loop_round_a_cylinder_through_all_halves_it() {
        let mesh = cylinder(10.0, -15.0, 15.0, 96);
        let object = Solid::new(&mesh).unwrap();
        let mut stroke = ring_round_cylinder(10.0, 2.0, 120, 1.0);
        assert_eq!(stroke.finish(1.0, 0.2, false), DrawCutError::None);
        let params = DrawCutParams {
            through_all: true,
            ..DrawCutParams::default()
        };
        let (cutter, _) = draw_cut_cutters(&mesh, &stroke, &params).unwrap();
        sound(&cutter);
        let (upper, lower) = draw_cut_split(&object, &mesh, &stroke, &params, None).unwrap();
        halves_sound(&upper, &lower);
        let sum = upper.signed_volume() + lower.signed_volume();
        assert!((sum - mesh.signed_volume()).abs() < 1e-2, "{sum}");
        assert!(draw_cut_core_face(&stroke, &params, Some(&mesh)).is_none());
    }

    #[test]
    fn a_loop_on_a_face_cuts_a_plug() {
        let mesh = block();
        let object = Solid::new(&mesh).unwrap();
        let mut stroke = DrawCutStroke::new();
        for k in 0..60 {
            let a = std::f64::consts::TAU * k as f64 / 60.0;
            stroke.append([5.0 * a.cos(), 5.0 * a.sin(), 10.0], Z, 0);
        }
        assert_eq!(stroke.finish(1.0, 0.2, false), DrawCutError::None);
        let params = DrawCutParams {
            angle_deg: 90.0,
            ..DrawCutParams::default()
        };
        assert!(!draw_cut_loop_separates(&mesh, &stroke, &params, 0.5));
        let (cutter, _) = draw_cut_cutters(&mesh, &stroke, &params).unwrap();
        sound(&cutter);
        let (upper, lower) = draw_cut_split(&object, &mesh, &stroke, &params, None).unwrap();
        halves_sound(&upper, &lower);
        let plug = std::f64::consts::PI * 25.0 * 3.0;
        assert!(
            (upper.signed_volume() - plug).abs() < 0.05 * plug,
            "{}",
            upper.signed_volume()
        );
        assert!((upper.signed_volume() + lower.signed_volume() - 8000.0).abs() < 1e-2);
        // No vertex of the box is in the plug; the cutter's own surface
        // tells the plug's side is not empty (see draw_cut_empty_sides).
        assert_eq!(
            draw_cut_empty_sides(&mesh, &stroke, &params),
            [false, false]
        );
        // Through all: a prism straight in, through the box.
        let through = DrawCutParams {
            through_all: true,
            ..params.clone()
        };
        let (prism_cutter, _) = draw_cut_cutters(&mesh, &stroke, &through).unwrap();
        sound(&prism_cutter);
        let (upper, lower) = draw_cut_split(&object, &mesh, &stroke, &through, None).unwrap();
        halves_sound(&upper, &lower);
        let prism = std::f64::consts::PI * 25.0 * 20.0;
        assert!((upper.signed_volume() - prism).abs() < 0.05 * prism);
        assert!((upper.signed_volume() + lower.signed_volume() - 8000.0).abs() < 1e-2);
        assert!(draw_cut_classify_upper(&cutter, true, [0.0, 0.0, 9.0]));
        assert!(!draw_cut_classify_upper(&cutter, true, [0.0, 0.0, 0.0]));
    }

    #[test]
    fn the_same_line_cuts_the_same_bytes() {
        let mesh = cylinder(10.0, -15.0, 15.0, 64);
        let object = Solid::new(&mesh).unwrap();
        let run = || {
            let mut stroke = ring_round_cylinder(10.0, 0.0, 90, 0.5);
            stroke.finish(1.0, 0.2, false);
            let params = DrawCutParams {
                angle_deg: 30.0,
                ..DrawCutParams::default()
            };
            let (cutter, _) = draw_cut_cutters(&mesh, &stroke, &params).unwrap();
            let (upper, lower) = draw_cut_split(&object, &mesh, &stroke, &params, None).unwrap();
            [cutter, upper, lower].map(|m| edge_mesh::stl::to_binary(&m))
        };
        assert_eq!(run(), run());
    }
}
