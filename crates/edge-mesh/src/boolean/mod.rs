//! Solid booleans of closed meshes: union, difference and intersection.
//!
//! What the reference does with two meshes (MeshBoolean.cpp
//! `mfd::make_boolean`, the Manifold library first, mcut as its
//! fallback) is done here with [`manifold-rust`], a pure-Rust port of that
//! same Manifold library (v3.5.4), behind this module alone: no other
//! code names the library, so it can be replaced in one place. Its exact
//! engine decides every intersection and every inside-or-outside with
//! exact predicates and a symbolic perturbation, so coplanar faces,
//! shared edges and corners and touching solids are resolved one
//! consistent way rather than by a tolerance.
//!
//! # What goes in
//!
//! Each operand must bound a volume ([`Solid::new`]):
//!
//! - not empty, every corner a finite number;
//! - closed and consistently wound: every edge is run once each way by
//!   exactly two triangles. A mesh that is not is first welded where its
//!   open edges meet (corners at the same place made one, as the
//!   reference's `MeshGL::Merge` does); one still open is refused with
//!   [`InputError::NotClosed`], the reference's "does not bound a volume";
//! - wound outward: a mesh whose volume comes out negative is turned the
//!   right way out first (the reference's `orient_outward`).
//!
//! A mesh that crosses itself is looked at more closely, as real meshes
//! often do it in a few places (the 3DBenchy does, in a handful of tiny
//! folds). One made of several shells that cross one another is first
//! made one solid, the union of its shells (as the reference's export
//! splits a mesh and unions the pieces, `mpartsDoSplits`). One that still
//! crosses itself is taken as it is, under [`SelfIntersections::Allow`]
//! (the default; the reference's own Mesh Boolean and cut tools ask no
//! more of a mesh): the boolean is then checked against its complement,
//! `|A ∪ B| + |A ∩ B| = |A| + |B|` and `|A − B| + |A ∩ B| = |A|`, and
//! refused with [`BooleanError::Inconsistent`] when that does not hold.
//! Where such a mesh crosses itself away from the other operand, the
//! crossing is carried into the result unchanged; the slicer resolves it
//! layer by layer as it does for the input. [`SelfIntersections::Refuse`]
//! refuses such a mesh instead, as the reference's export does
//! (`check_csgmesh_booleans`, CGAL's `does_self_intersect`).
//!
//! # What comes out
//!
//! A result is always closed and consistently wound, with no triangle of
//! zero area, or it is an error: never a mesh with a hole in it. The
//! library works in double precision; the result is rounded to this
//! crate's single-precision corners, and what rounding makes degenerate is
//! cleaned up and the whole checked again before it is returned
//! ([`check::defects`]). Corners where the result touches itself (two
//! solids meeting along an edge or at a point) stay separate corners at
//! the same place, so every edge still belongs to two triangles. From
//! operands that do not cross themselves, the result does not cross
//! itself either (in double precision; see the tests for single). The
//! same input gives the same bytes every time: the library runs on one
//! thread here.
//!
//! An empty result (two solids that do not meet, intersected) is an empty
//! mesh, not an error.
//!
//! # For the tools that come later
//!
//! - The Mesh Boolean gizmo: [`union`], [`difference`], [`intersection`]
//!   on two parts in one frame, or [`boolean`] with [`Op`]; the parts are
//!   left as they are, so "Delete input" is the caller's.
//! - The Cut tool's curved and drawn surfaces (CurvedCut.cpp
//!   `cut_with_solid`): `intersection(object, solid)` and
//!   `difference(object, solid)` with the closed cutter built from the
//!   surface; prepare the object once with [`Solid::new`] and use
//!   [`Solid::boolean`] for both halves.
//! - Negative parts and a model's parts generally (`csg` in Model.cpp,
//!   CSGMesh): [`csg`], the parts in order.
//!
//! [`manifold-rust`]: https://github.com/larsbrubaker/manifold-rust

pub mod check;

use manifold_rust::manifold::Manifold;
use manifold_rust::types::{BooleanEngine, Error as LibraryError, MeshGL64, OpType};

use crate::Mesh;

/// Which boolean.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// Everything in either.
    Union,
    /// The first less the second.
    Difference,
    /// What both hold.
    Intersection,
}

/// Which of the two operands an error is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operand {
    First,
    Second,
}

impl std::fmt::Display for Operand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Operand::First => "first",
            Operand::Second => "second",
        })
    }
}

/// Why a mesh cannot be an operand.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    #[error("is empty")]
    Empty,
    #[error("has a corner that is not a finite number")]
    NotFinite,
    /// The reference's "does not bound a volume".
    #[error("does not bound a volume")]
    NotClosed,
    /// Only under [`SelfIntersections::Refuse`].
    #[error("has self intersection")]
    SelfIntersecting,
}

/// Why a boolean gave no mesh.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum BooleanError {
    #[error("the {operand} mesh {error}")]
    Input { operand: Operand, error: InputError },
    /// An operand crosses itself and the result does not add up: the
    /// volumes of the result and of its complementary boolean differ from
    /// what the operands' volumes say they must be (in mm³).
    #[error("the result does not add up: {got} mm³ where {expected} mm³ was expected")]
    Inconsistent { got: f64, expected: f64 },
    /// The library failed, or its result did not pass the checks.
    #[error("the boolean failed: {0}")]
    Failed(String),
}

/// What to do with an operand that crosses itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelfIntersections {
    /// Take it, and check the result adds up (see the module's words).
    #[default]
    Allow,
    /// Refuse it, as the reference's export does.
    Refuse,
}

/// How a boolean is done.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub self_intersections: SelfIntersections,
    /// Stops a long boolean from another thread: the boolean then fails.
    pub cancel: Option<Cancel>,
}

/// A handle that stops a boolean under way ([`Options::cancel`]); clones
/// share it.
#[derive(Clone, Debug, Default)]
pub struct Cancel(manifold_rust::cancel::CancelToken);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

/// A mesh checked and made ready as an operand, in double precision. A
/// result kept as a `Solid` goes into the next boolean without being
/// rounded to single precision first.
#[derive(Clone)]
pub struct Solid {
    manifold: Manifold,
    /// Whether it crosses itself (it, or an operand it was made from).
    self_intersecting: bool,
}

impl std::fmt::Debug for Solid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Solid")
            .field("triangles", &self.manifold.num_tri())
            .field("volume", &self.volume())
            .field("self_intersecting", &self.self_intersecting)
            .finish()
    }
}

impl Solid {
    /// Checks a mesh and makes it an operand: see the module's words for
    /// what is asked of it and what is done to it.
    pub fn new(mesh: &Mesh) -> Result<Solid, InputError> {
        Self::with(mesh, SelfIntersections::Allow)
    }

    /// [`Solid::new`], refusing a mesh that crosses itself when asked to.
    pub fn with(mesh: &Mesh, self_intersections: SelfIntersections) -> Result<Solid, InputError> {
        if mesh.triangles.is_empty() {
            return Err(InputError::Empty);
        }
        if (mesh.vertices.iter().flatten()).any(|c| !c.is_finite()) {
            return Err(InputError::NotFinite);
        }
        let manifold = import(mesh)?;
        if manifold.is_empty() {
            // Nothing but degenerate triangles.
            return Err(InputError::NotClosed);
        }
        if !manifold.has_self_intersections() {
            return Ok(Solid {
                manifold,
                self_intersecting: false,
            });
        }
        // Several shells crossing one another: their union is one solid.
        if let Some(manifold) = union_of_shells(mesh) {
            return Ok(Solid {
                manifold,
                self_intersecting: false,
            });
        }
        match self_intersections {
            SelfIntersections::Refuse => Err(InputError::SelfIntersecting),
            SelfIntersections::Allow => Ok(Solid {
                manifold,
                self_intersecting: true,
            }),
        }
    }

    /// Enclosed volume in mm³.
    pub fn volume(&self) -> f64 {
        self.manifold.volume()
    }

    pub fn is_empty(&self) -> bool {
        self.manifold.is_empty()
    }

    /// Whether it crosses itself, or was made from an operand that did.
    pub fn is_self_intersecting(&self) -> bool {
        self.self_intersecting
    }

    /// Whether it crosses itself in the library's double precision: an
    /// exact test, cached.
    pub fn crosses_itself(&self) -> bool {
        self.manifold.has_self_intersections()
    }

    /// This boolean the other.
    pub fn boolean(&self, other: &Solid, op: Op) -> Result<Solid, BooleanError> {
        self.boolean_with(other, op, &Options::default())
    }

    /// [`Solid::boolean`] with options. The self-intersection option is
    /// for [`Solid::new`]; here only `cancel` counts.
    pub fn boolean_with(
        &self,
        other: &Solid,
        op: Op,
        options: &Options,
    ) -> Result<Solid, BooleanError> {
        let result = run(&self.manifold, &other.manifold, op, options)?;
        let self_intersecting = self.self_intersecting || other.self_intersecting;
        if self_intersecting {
            // Without operands that are surfaces of solids, nothing in
            // the library promises the result is right. Check it against
            // its complement: misclassified pieces do not add up.
            let (a, b) = (self.volume(), other.volume());
            let both = match op {
                Op::Intersection => result.volume(),
                _ => run(&self.manifold, &other.manifold, Op::Intersection, options)?.volume(),
            };
            let (got, expected) = match op {
                Op::Union => (result.volume() + both, a + b),
                Op::Difference => (result.volume() + both, a),
                Op::Intersection => {
                    let rest = run(&self.manifold, &other.manifold, Op::Difference, options)?;
                    (rest.volume() + both, a)
                }
            };
            if (got - expected).abs() > VOLUME_SLACK * (a.abs() + b.abs()) {
                return Err(BooleanError::Inconsistent { got, expected });
            }
        }
        Ok(Solid {
            manifold: result,
            self_intersecting,
        })
    }

    /// The solid as a mesh in this crate's single precision: closed,
    /// consistently wound, no triangle of zero area, or an error.
    pub fn mesh(&self) -> Result<Mesh, BooleanError> {
        finish(self)
    }
}

/// How far the volumes may be from adding up, relative to the operands'.
/// Far above the library's rounding (around 1e-12 here) and far below
/// what a misplaced piece of a solid moves them by.
const VOLUME_SLACK: f64 = 1e-7;

/// `a ∪ b`.
pub fn union(a: &Mesh, b: &Mesh) -> Result<Mesh, BooleanError> {
    boolean(a, b, Op::Union)
}

/// `a − b`.
pub fn difference(a: &Mesh, b: &Mesh) -> Result<Mesh, BooleanError> {
    boolean(a, b, Op::Difference)
}

/// `a ∩ b`.
pub fn intersection(a: &Mesh, b: &Mesh) -> Result<Mesh, BooleanError> {
    boolean(a, b, Op::Intersection)
}

/// `a op b`, with the default [`Options`].
pub fn boolean(a: &Mesh, b: &Mesh, op: Op) -> Result<Mesh, BooleanError> {
    boolean_with(a, b, op, &Options::default())
}

/// `a op b`.
pub fn boolean_with(a: &Mesh, b: &Mesh, op: Op, options: &Options) -> Result<Mesh, BooleanError> {
    let solid = |mesh: &Mesh, operand: Operand| {
        Solid::with(mesh, options.self_intersections)
            .map_err(|error| BooleanError::Input { operand, error })
    };
    let (a, b) = (solid(a, Operand::First)?, solid(b, Operand::Second)?);
    a.boolean_with(&b, op, options)?.mesh()
}

/// Why a [`csg`] gave no mesh: the error, and the part it came at (its
/// place in the list).
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("part {part}: {error}")]
pub struct CsgError {
    pub part: usize,
    pub error: BooleanError,
}

/// Parts combined in order, each by its operation with what the parts
/// before it made, as the reference combines a model's parts
/// (CSGMesh/PerformCSGMeshBooleans.hpp `perform_csgmesh_booleans`): the
/// first union starts the solid, a difference or intersection before
/// anything is there does nothing, and a part's error is the whole
/// result's. A part that is an empty mesh is skipped, as the reference
/// skips one. An object is `[(Union, part), (Union, part),
/// (Difference, negative part), ...]` in its parts' order. With nothing
/// to start from, the result is an empty mesh.
pub fn csg<'a>(
    parts: impl IntoIterator<Item = (Op, &'a Mesh)>,
    options: &Options,
) -> Result<Mesh, CsgError> {
    let mut solid: Option<Solid> = None;
    let mut last = 0;
    for (at, (op, mesh)) in parts.into_iter().enumerate() {
        last = at;
        if mesh.triangles.is_empty() || (solid.is_none() && op != Op::Union) {
            continue;
        }
        let part = Solid::with(mesh, options.self_intersections).map_err(|error| CsgError {
            part: at,
            error: BooleanError::Input {
                operand: Operand::Second,
                error,
            },
        })?;
        solid = Some(match solid {
            None => part,
            Some(solid) => solid
                .boolean_with(&part, op, options)
                .map_err(|error| CsgError { part: at, error })?,
        });
    }
    match solid {
        None => Ok(Mesh::default()),
        Some(solid) => solid.mesh().map_err(|error| CsgError { part: last, error }),
    }
}

// --- the library's side

fn to_library(mesh: &Mesh) -> MeshGL64 {
    MeshGL64 {
        num_prop: 3,
        vert_properties: (mesh.vertices.iter())
            .flat_map(|v| v.map(f64::from))
            .collect(),
        tri_verts: (mesh.triangles.iter())
            .flat_map(|t| t.map(u64::from))
            .collect(),
        ..MeshGL64::default()
    }
}

/// The library's own checks after ours: ours say what is wrong in the
/// reference's words, its say whether it can work with it.
fn import(mesh: &Mesh) -> Result<Manifold, InputError> {
    let mut gl = to_library(mesh);
    if !check::is_closed(mesh) {
        // Corners at one place that the file kept apart (`MeshGL::Merge`,
        // as the reference's `to_manifold`). It welds only along open
        // edges, so a mesh closed already is left alone.
        gl.merge();
    }
    if mesh.signed_volume() < 0.0 {
        for t in gl.tri_verts.chunks_mut(3) {
            t.swap(1, 2);
        }
    }
    let manifold = guarded(|| Manifold::from_mesh_gl64(&gl)).map_err(|_| InputError::NotClosed)?;
    match manifold.status() {
        LibraryError::NoError => Ok(manifold),
        LibraryError::NonFiniteVertex => Err(InputError::NotFinite),
        _ => Err(InputError::NotClosed),
    }
}

/// A mesh of several shells, each clean and enclosing material (no
/// cavity among them), as the union of its shells: `None` when it is not
/// one of those, or the union still crosses itself.
fn union_of_shells(mesh: &Mesh) -> Option<Manifold> {
    if !crate::split::is_splittable(mesh) {
        return None;
    }
    let shells = crate::split::split(mesh);
    // `split` turns a cavity the right way out; a mesh with one is not a
    // union of solids, and is left as it is.
    let sum: f64 = shells.iter().map(Mesh::signed_volume).sum();
    let whole = mesh.signed_volume().abs();
    if (sum - whole).abs() > VOLUME_SLACK * whole {
        return None;
    }
    // Every shell checked before any is united, so a mesh that cannot be
    // put right this way costs no booleans.
    let mut solids = Vec::with_capacity(shells.len());
    for shell in &shells {
        let shell = import(shell).ok()?;
        if shell.is_empty() || shell.has_self_intersections() {
            return None;
        }
        solids.push(shell);
    }
    let mut union: Option<Manifold> = None;
    for shell in solids {
        union = Some(match union {
            None => shell,
            Some(union) => {
                let next = guarded(|| {
                    union.boolean_with_engine(&shell, OpType::Add, BooleanEngine::Exact)
                })
                .ok()?;
                if next.status() != LibraryError::NoError {
                    return None;
                }
                next
            }
        });
    }
    let union = union?;
    (!union.has_self_intersections()).then_some(union)
}

fn run(a: &Manifold, b: &Manifold, op: Op, options: &Options) -> Result<Manifold, BooleanError> {
    let op = match op {
        Op::Union => OpType::Add,
        Op::Difference => OpType::Subtract,
        Op::Intersection => OpType::Intersect,
    };
    let token = options.cancel.as_ref().map(|cancel| &cancel.0);
    // The exact engine whatever the process-wide default says; the robust
    // one is slower and its results were not as clean on the test meshes.
    let result = guarded(|| a.boolean_with_engine_and_token(b, op, BooleanEngine::Exact, token))?;
    match result.status() {
        LibraryError::NoError => Ok(result),
        LibraryError::Cancelled => Err(BooleanError::Failed("cancelled".into())),
        status => Err(BooleanError::Failed(status.to_str().into())),
    }
}

/// Runs the library, a panic in it an error rather than the caller's end.
fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, BooleanError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|panic| {
        let what = (panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        BooleanError::Failed(format!("the library stopped: {what}"))
    })
}

/// The solid in single precision, checked (see the module's words).
///
/// Rounding every corner to single precision moves it by up to half a
/// unit in the last place, which is enough to collapse a sliver or to
/// push a face a fraction of a micron through one it touches. So the
/// mesh is rounded as it is first; when that crosses itself (and the
/// solid did not), it is rounded again after the library has collapsed
/// what is shorter than single precision can tell apart and merged what
/// it cannot tell from flat (`set_tolerance`, which moves the surface by
/// no more than that). The first that does not cross itself is taken;
/// when neither does that, the first closed one is, its crossings no
/// deeper than the rounding.
///
/// "Crosses itself" here is the library's exact test, which also counts
/// two parts of a surface that only touch (an edge lying on a face, as
/// where two solids that touch along an edge are united). Those are part
/// of the shape, not a defect, and are kept.
fn finish(solid: &Solid) -> Result<Mesh, BooleanError> {
    if solid.manifold.is_empty() {
        return Ok(Mesh::default());
    }
    let first = rounded(&solid.manifold);
    if solid.self_intersecting {
        return first;
    }
    if let Ok(mesh) = &first
        && !check::self_intersects(mesh)
    {
        return first;
    }
    let bounds = solid.manifold.bounding_box();
    let scale = [bounds.min, bounds.max]
        .iter()
        .flat_map(|v| [v.x, v.y, v.z])
        .fold(0.0f64, |m, c| m.max(c.abs()));
    let tolerance = f64::from(f32::EPSILON) * scale * 2.0;
    let snapped = guarded(|| solid.manifold.set_tolerance(tolerance))?;
    // The surface moves by no more than the tolerance, so the volume by no
    // more than that over the area. The library's simplification does not
    // always keep to it (on two of three thousand random pairs it moved a
    // face across a solid, `set_tolerance` in manifold-rust 0.19.2), so it
    // is checked.
    let moved = (snapped.volume() - solid.manifold.volume()).abs();
    if snapped.status() == LibraryError::NoError
        && moved <= tolerance * solid.manifold.surface_area()
        && let Ok(mesh) = rounded(&snapped)
        && !check::self_intersects(&mesh)
    {
        return Ok(mesh);
    }
    first
}

/// The library's mesh rounded to single precision: closed, consistently
/// wound, no triangle of zero area, or an error. Rounding can collapse a
/// sliver; the library is then given the rounded mesh to clean up (its
/// import collapses what is shorter than its precision), needles left are
/// turned away by an edge flip, and the result is checked again.
fn rounded(manifold: &Manifold) -> Result<Mesh, BooleanError> {
    let mut current = manifold.clone();
    for _ in 0..4 {
        if current.is_empty() {
            return Ok(Mesh::default());
        }
        let gl = guarded(|| current.get_mesh_gl64(-1))?;
        let stride = gl.num_prop as usize;
        let mut mesh = Mesh {
            vertices: (gl.vert_properties.chunks(stride))
                .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
                .collect(),
            triangles: (gl.tri_verts.chunks(3))
                .map(|t| [t[0] as u32, t[1] as u32, t[2] as u32])
                .collect(),
        };
        check::flip_needles(&mut mesh);
        let defects = check::defects(&mesh);
        if defects.is_clean() {
            return Ok(mesh);
        }
        let gl = to_library(&mesh);
        current = guarded(|| Manifold::from_mesh_gl64(&gl))?;
        if current.status() != LibraryError::NoError {
            return Err(BooleanError::Failed(format!(
                "the result rounded to single precision is not closed ({defects:?})"
            )));
        }
    }
    Err(BooleanError::Failed(
        "the result keeps triangles of no area in single precision".into(),
    ))
}
