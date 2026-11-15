//! "Simplify": the quadric edge collapse of libslic3r/QuadricEdgeCollapse.cpp
//! (`its_quadric_edge_collapse`), ported step by step so the same mesh and
//! parameters give the same triangles, corners and order.
//!
//! Each triangle carries the error of its cheapest edge (Garland-Heckbert
//! quadrics summed at the corners); the triangles wait in the reference's
//! cache-friendly heap of 32-wide miniheaps (MutablePriorityQueue.hpp
//! `MutableSkipHeapPriorityQueue`), whose order on equal errors decides which
//! edge goes first, so it is ported too. An edge is collapsed unless it is on
//! a border, would make a triangle with a corner twice, would lay two
//! triangles back to back, or would turn a triangle round (or make it a
//! needle); then the triangle tries its next edge. It stops at the triangle
//! count asked for ("Decimate ratio") or at the first edge whose error
//! reaches the maximum ("Detail level").
//!
//! The reference works out normals and errors on several threads, each
//! triangle on its own; here they are worked out in order, which gives the
//! same numbers.

use crate::Mesh;

/// The collapse was called off by `cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// What the collapse did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub triangles_before: usize,
    pub triangles_after: usize,
    /// The error of the last edge collapsed (the reference's `*max_error` out).
    pub last_error: f32,
}

/// `its_quadric_edge_collapse(its, triangle_count, max_error, throw_on_cancel, status_fn)`.
/// `max_error` None is the reference's null pointer (no limit). `status`
/// gets percents as the reference reports them; `cancel` is asked as often
/// as the reference's `throw_on_cancel` and stops the collapse when it says
/// true, the mesh then left as it was.
pub fn quadric_edge_collapse(
    mesh: &mut Mesh,
    triangle_count: u32,
    max_error: Option<f32>,
    status: &mut dyn FnMut(i32),
    cancel: &dyn Fn() -> bool,
) -> Result<Stats, Cancelled> {
    let before = mesh.triangles.len();
    let unchanged = Stats {
        triangles_before: before,
        triangles_after: before,
        last_error: max_error.unwrap_or(f32::MAX),
    };
    if triangle_count as usize >= mesh.triangles.len() {
        return Ok(unchanged);
    }
    let maximal_error = max_error.unwrap_or(f32::MAX);
    if maximal_error <= 0.0 {
        return Ok(unchanged);
    }
    let mut work = mesh.clone();
    let last = collapse(&mut work, triangle_count, maximal_error, status, cancel)?;
    *mesh = work;
    Ok(Stats {
        triangles_before: before,
        triangles_after: mesh.triangles.len(),
        last_error: last,
    })
}

// The reference's constants (QuadricEdgeCollapse.cpp l.146-155).
const CHECK_CANCEL_PERIOD: u32 = 16;
const STATUS_INIT_SIZE: i32 = 10;
const STATUS_NORMAL_SIZE: i32 = 25;
const STATUS_SUM_QUADRIC: i32 = 25;
const STATUS_SET_OFFSETS: i32 = 10;
const STATUS_CALC_ERRORS: i32 = 30;
const STATUS_CREATE_REFS: i32 = 10;

/// `SymMat`: the ten numbers of a symmetric 4 x 4 quadric.
#[derive(Clone, Copy, Default)]
struct SymMat([f64; 10]);

impl SymMat {
    fn plane(a: f64, b: f64, c: f64, d: f64) -> Self {
        Self([
            a * a,
            a * b,
            a * c,
            a * d,
            b * b,
            b * c,
            b * d,
            c * c,
            c * d,
            d * d,
        ])
    }

    #[allow(clippy::too_many_arguments)]
    fn det(
        &self,
        a11: usize,
        a12: usize,
        a13: usize,
        a21: usize,
        a22: usize,
        a23: usize,
        a31: usize,
        a32: usize,
        a33: usize,
    ) -> f64 {
        // As clang contracts the reference's one expression (see `CONTRACTION`).
        let m = &self.0;
        let s = (m[a11] * m[a22]).mul_add(m[a33], m[a13] * m[a21] * m[a32]);
        let s = (m[a12] * m[a23]).mul_add(m[a31], s);
        let s = (-(m[a13] * m[a22])).mul_add(m[a31], s);
        let s = (-(m[a11] * m[a23])).mul_add(m[a32], s);
        (-(m[a12] * m[a21])).mul_add(m[a33], s)
    }

    fn add(&mut self, other: &SymMat) {
        for i in 0..10 {
            self.0[i] += other.0[i];
        }
    }

    fn sum(&self, other: &SymMat) -> SymMat {
        let mut out = *self;
        out.add(other);
        out
    }
}

/// `Error`: a triangle's smallest edge error.
#[derive(Clone, Copy, Default)]
struct Error {
    value: f32,
    triangle: u32,
}

/// `TriangleInfo`: the unit normal, a deleted triangle marked by x > 2, and
/// which edge has the smallest error (3: none).
#[derive(Clone, Copy, Default)]
struct TriangleInfo {
    n: [f32; 3],
    min_index: u8,
}

impl TriangleInfo {
    fn is_deleted(&self) -> bool {
        self.n[0] > 2.0
    }
    fn set_deleted(&mut self) {
        self.n[0] = 3.0;
    }
}

/// `VertexInfo`: the summed quadric and the run of `EdgeInfo`s round it.
#[derive(Clone, Copy, Default)]
struct VertexInfo {
    q: SymMat,
    start: u32,
    count: u32,
}

impl VertexInfo {
    fn is_deleted(&self) -> bool {
        self.count == 0
    }
}

/// `EdgeInfo`: a triangle round a corner and which of its corners it is.
#[derive(Clone, Copy, Default)]
struct EdgeInfo {
    t_index: u32,
    edge: u8,
}

/// CONTRACTION: the reference is built with clang's default
/// `-ffp-contract=on`, which fuses a product and the sum or difference it
/// is added to within one source expression into a fused multiply-add
/// (`a * b + c` with the left product taken first, else the right). Where
/// the reference's code has such an expression, it is written here with
/// `mul_add` the way clang fuses it, so the numbers are the reference's to
/// the bit; `mul_add` is the exact fused operation on every platform.
type V3 = [f64; 3];

fn d3(v: [f32; 3]) -> V3 {
    v.map(f64::from)
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
/// Eigen's `cross` (OrthoMethods.h), each component one expression that
/// clang contracts: the first product fused with the second subtracted.
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1].mul_add(b[2], -(a[2] * b[1])),
        a[2].mul_add(b[0], -(a[0] * b[2])),
        a[0].mul_add(b[1], -(a[1] * b[0])),
    ]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
/// Eigen's `normalize()`: divided by the norm when the norm is not zero.
fn normalized(v: V3) -> V3 {
    let z = dot(v, v);
    if z > 0.0 {
        let n = z.sqrt();
        [v[0] / n, v[1] / n, v[2] / n]
    } else {
        v
    }
}

type F3 = [f32; 3];
fn subf(a: F3, b: F3) -> F3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn crossf(a: F3, b: F3) -> F3 {
    [
        a[1].mul_add(b[2], -(a[2] * b[1])),
        a[2].mul_add(b[0], -(a[0] * b[2])),
        a[0].mul_add(b[1], -(a[1] * b[0])),
    ]
}
/// Eigen's sum of three `f32`s: a 3-vector of `float` is too short for its
/// four-wide packet, so its unrolled reduction adds the first to the sum of
/// the other two (Redux.h `redux_novec_unroller`). For `double` the two-wide
/// packet takes the first two, and the third comes last, as written in `dot`.
fn dotf(a: F3, b: F3) -> f32 {
    a[0] * b[0] + (a[1] * b[1] + a[2] * b[2])
}
fn normalizedf(v: F3) -> F3 {
    let z = dotf(v, v);
    if z > 0.0 {
        let n = z.sqrt();
        [v[0] / n, v[1] / n, v[2] / n]
    } else {
        v
    }
}

/// `create_normal`.
fn create_normal(t: [u32; 3], vertices: &[[f32; 3]]) -> V3 {
    let v0 = d3(vertices[t[0] as usize]);
    let v1 = d3(vertices[t[1] as usize]);
    let v2 = d3(vertices[t[2] as usize]);
    normalized(cross(sub(v1, v0), sub(v2, v0)))
}

/// `create_quadric`.
fn create_quadric(t: [u32; 3], n: V3, vertices: &[[f32; 3]]) -> SymMat {
    let v0 = d3(vertices[t[0] as usize]);
    SymMat::plane(n[0], n[1], n[2], -dot(n, v0))
}

/// `calculate_determinant`.
fn calculate_determinant(q: &SymMat) -> f64 {
    q.det(0, 1, 2, 1, 4, 5, 2, 5, 7)
}

/// `calculate_vertex(det, q)`.
fn vertex_from_det(det: f64, q: &SymMat) -> V3 {
    let det_1 = -1.0 / det;
    let det_x = q.det(1, 2, 3, 4, 5, 6, 5, 7, 8);
    let det_y = q.det(0, 2, 3, 1, 5, 6, 2, 7, 8);
    let det_z = q.det(0, 1, 3, 1, 4, 6, 2, 5, 8);
    [det_1 * det_x, -det_1 * det_y, det_1 * det_z]
}

/// `vertex_error`.
fn vertex_error(q: &SymMat, v: V3) -> f64 {
    // As clang contracts the reference's one expression (see `CONTRACTION`).
    let m = &q.0;
    let [x, y, z] = v;
    let s = (m[0] * x).mul_add(x, 2.0 * m[1] * x * y);
    let s = (2.0 * m[2] * x).mul_add(z, s);
    let s = (2.0 * m[3]).mul_add(x, s);
    let s = (m[4] * y).mul_add(y, s);
    let s = (2.0 * m[5] * y).mul_add(z, s);
    let s = (2.0 * m[6]).mul_add(y, s);
    let s = (m[7] * z).mul_add(z, s);
    let s = (2.0 * m[8]).mul_add(z, s);
    s + m[9]
}

/// `create_vertices`: the two ends and their middle.
fn create_vertices(v1: u32, v2: u32, vertices: &[[f32; 3]]) -> [V3; 3] {
    let a = d3(vertices[v1 as usize]);
    let b = d3(vertices[v2 as usize]);
    let m = [
        (a[0] + b[0]) / 2.0,
        (a[1] + b[1]) / 2.0,
        (a[2] + b[2]) / 2.0,
    ];
    [a, b, m]
}

/// The first of the smallest (`std::min_element`).
fn min_element(errors: [f64; 3]) -> usize {
    let mut best = 0;
    for i in 1..3 {
        if errors[i] < errors[best] {
            best = i;
        }
    }
    best
}

/// `calculate_error(id_v1, id_v2, q, vertices)`.
fn edge_error(v1: u32, v2: u32, q: &SymMat, vertices: &[[f32; 3]]) -> f64 {
    let det = calculate_determinant(q);
    if det.abs() < f64::EPSILON {
        let verts = create_vertices(v1, v2, vertices);
        let errors = verts.map(|v| vertex_error(q, v));
        return errors[min_element(errors)];
    }
    vertex_error(q, vertex_from_det(det, q))
}

/// `calculate_vertex(id_v1, id_v2, q, vertices)`.
fn new_vertex(v1: u32, v2: u32, q: &SymMat, vertices: &[[f32; 3]]) -> [f32; 3] {
    let det = calculate_determinant(q);
    if det.abs() < f64::EPSILON {
        let verts = create_vertices(v1, v2, vertices);
        let errors = verts.map(|v| vertex_error(q, v));
        return verts[min_element(errors)].map(|c| c as f32);
    }
    vertex_from_det(det, q).map(|c| c as f32)
}

/// `calculate_3errors`.
fn three_errors(t: [u32; 3], vertices: &[[f32; 3]], v_infos: &[VertexInfo]) -> [f64; 3] {
    let mut error = [0.0; 3];
    for j in 0..3 {
        let j2 = if j == 2 { 0 } else { j + 1 };
        let (vi0, vi1) = (t[j], t[j2]);
        let q = v_infos[vi0 as usize].q.sum(&v_infos[vi1 as usize].q);
        error[j] = edge_error(vi0, vi1, &q, vertices);
    }
    error
}

/// `calculate_error(ti, t, vertices, v_infos, min_index)`.
fn triangle_error(
    ti: u32,
    t: [u32; 3],
    vertices: &[[f32; 3]],
    v_infos: &[VertexInfo],
    min_index: &mut u8,
) -> Error {
    let e = three_errors(t, vertices, v_infos);
    let m = if e[0] < e[1] {
        if e[0] < e[2] { 0 } else { 2 }
    } else if e[1] < e[2] {
        1
    } else {
        2
    };
    *min_index = m as u8;
    Error {
        value: e[m] as f32,
        triangle: ti,
    }
}

/// `MutableSkipHeapPriorityQueue<Error, ..., 32, false>` with the setter
/// writing each triangle's place into `places` and `less` on the error.
struct Heap {
    heap: Vec<Error>,
}

const BLOCK: usize = 32;
const MASK: usize = BLOCK - 1;

// `SkipHeapAddressing<32>`.
fn is_block_leaf(n: usize) -> bool {
    n & (BLOCK >> 1) != 0
}
fn is_padding(n: usize) -> bool {
    n & MASK == 0
}
fn is_block_root(n: usize) -> bool {
    n & MASK == 1
}
fn block_offset(n: usize) -> usize {
    n & MASK
}
fn block_base(n: usize) -> usize {
    n & !MASK
}
fn child_no(n: usize) -> usize {
    n & (MASK >> 1)
}
fn child_of(n: usize) -> usize {
    if !is_block_leaf(n) {
        return n + block_offset(n);
    }
    (block_base(n) + 1 + child_no(n) * 2) * BLOCK + 1
}
fn parent_of(n: usize) -> usize {
    let node_root = block_base(n);
    if !is_block_root(n) {
        return node_root + block_offset(n) / 2;
    }
    let parent_base = block_base(node_root / BLOCK - 1);
    let child = ((n - BLOCK) / BLOCK - parent_base) / 2;
    parent_base + BLOCK / 2 + child
}

impl Heap {
    fn less(a: &Error, b: &Error) -> bool {
        a.value < b.value
    }

    fn push(&mut self, item: Error, places: &mut [usize]) {
        if is_padding(self.heap.len()) {
            self.heap.push(Error::default());
        }
        let idx = self.heap.len();
        self.heap.push(item);
        places[item.triangle as usize] = idx;
        self.up(1, idx, places);
    }

    fn top(&self) -> Error {
        self.heap[1]
    }

    fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    fn pop_back(&mut self) {
        self.heap.pop();
        if !self.heap.is_empty() && is_padding(self.heap.len() - 1) {
            self.heap.pop();
        }
    }

    fn pop(&mut self, places: &mut [usize]) {
        if self.heap.len() > 2 {
            self.heap[1] = *self.heap.last().expect("heap has a last element");
            self.pop_back();
            places[self.heap[1].triangle as usize] = 1;
            let bottom = self.heap.len() - 1;
            self.down(1, bottom, places);
        } else {
            self.heap.clear();
        }
    }

    fn remove(&mut self, idx: usize, places: &mut [usize]) {
        if idx + 1 == self.heap.len() {
            self.pop_back();
            return;
        }
        self.heap[idx] = *self.heap.last().expect("heap has a last element");
        places[self.heap[idx].triangle as usize] = idx;
        self.pop_back();
        let bottom = self.heap.len() - 1;
        self.down(idx, bottom, places);
        self.up(1, idx, places);
    }

    fn update(&mut self, idx: usize, places: &mut [usize]) {
        let item = self.heap[idx];
        self.remove(idx, places);
        self.push(item, places);
    }

    fn up(&mut self, top: usize, bottom: usize, places: &mut [usize]) {
        let mut child_idx = bottom;
        loop {
            // The reference works out node 1's parent (a wrapped number) before
            // it looks at whether it is node 1; the order changes nothing.
            if child_idx == 1 {
                break;
            }
            let parent_idx = parent_of(child_idx);
            if parent_idx < top {
                break;
            }
            let (parent, child) = (self.heap[parent_idx], self.heap[child_idx]);
            // The reference swaps when the parent is not less, equal ones too,
            // and goes on to the top whether it swapped or not.
            if !Self::less(&parent, &child) {
                places[parent.triangle as usize] = child_idx;
                places[child.triangle as usize] = parent_idx;
                self.heap[parent_idx] = child;
                self.heap[child_idx] = parent;
            }
            child_idx = parent_idx;
        }
    }

    fn down(&mut self, top: usize, bottom: usize, places: &mut [usize]) {
        let mut parent_idx = top;
        loop {
            let mut child_idx = child_of(parent_idx);
            if child_idx > bottom {
                break;
            }
            let child2_idx = child_idx + if is_block_leaf(parent_idx) { BLOCK } else { 1 };
            if child2_idx <= bottom && !Self::less(&self.heap[child_idx], &self.heap[child2_idx]) {
                child_idx = child2_idx;
            }
            let (parent, child) = (self.heap[parent_idx], self.heap[child_idx]);
            if Self::less(&parent, &child) {
                return;
            }
            places[parent.triangle as usize] = child_idx;
            places[child.triangle as usize] = parent_idx;
            self.heap[parent_idx] = child;
            self.heap[child_idx] = parent;
            parent_idx = child_idx;
        }
    }
}

/// The body of `its_quadric_edge_collapse`; the mesh is changed in place,
/// and on a cancel it is the caller's copy that is dropped.
fn collapse(
    its: &mut Mesh,
    triangle_count: u32,
    maximal_error: f32,
    status: &mut dyn FnMut(i32),
    cancel: &dyn Fn() -> bool,
) -> Result<f32, Cancelled> {
    let check = || if cancel() { Err(Cancelled) } else { Ok(()) };
    let (mut t_infos, mut v_infos, mut e_infos, errors) = {
        let mut init_status = |percent: i32| {
            let n = percent as f32 * STATUS_INIT_SIZE as f32 / 100.0;
            status(n.round() as i32);
        };
        init(its, &check, &mut init_status)?
    };
    check()?;
    status(STATUS_INIT_SIZE);

    let mut places = vec![0usize; its.triangles.len()];
    let mut mpq = Heap {
        heap: Vec::with_capacity(its.triangles.len() + its.triangles.len().div_ceil(BLOCK - 1)),
    };
    for e in errors {
        mpq.push(e, &mut places);
    }

    let mut room = Room::new(&v_infos, e_infos.len());
    let mut e_infos_swap: Vec<EdgeInfo> = Vec::new();
    let mut changed: Vec<u32> = Vec::new();

    let mut actual = its.triangles.len() as u32;
    let to_reduce = actual - triangle_count;
    let status_mod = 16u32.max(to_reduce / (100 - STATUS_INIT_SIZE as u32));

    let mut iteration = 0u32;
    let mut last_collapsed_error = 0.0f32;
    while actual > triangle_count && !mpq.is_empty() {
        iteration = iteration.wrapping_add(1);
        if iteration.is_multiple_of(status_mod) {
            // `increase_status`.
            let reduced = (actual - triangle_count) as f64 / to_reduce as f64;
            let s = STATUS_INIT_SIZE as f64 + (100 - STATUS_INIT_SIZE) as f64 * (1.0 - reduced);
            status(s.round() as i32);
        }
        if iteration.is_multiple_of(CHECK_CANCEL_PERIOD) {
            check()?;
        }

        let mut e = mpq.top();
        if e.value >= maximal_error {
            break;
        }
        mpq.pop(&mut places);
        let ti0 = e.triangle;
        if t_infos[ti0 as usize].is_deleted() {
            continue;
        }
        let t0 = its.triangles[ti0 as usize];
        let min_index = t_infos[ti0 as usize].min_index as usize;
        let mut vi0 = t0[min_index];
        let mut vi1 = t0[(min_index + 1) % 3];
        if vi0 > vi1 {
            std::mem::swap(&mut vi0, &mut vi1);
        }

        let q = v_infos[vi0 as usize].q.sum(&v_infos[vi1 as usize].q);
        let new_vertex0 = new_vertex(vi0, vi1, &q, &its.vertices);
        let ti1_opt = if v_infos[vi0 as usize].count < v_infos[vi1 as usize].count {
            find_triangle_index1(vi1, &v_infos[vi0 as usize], ti0, &e_infos, &its.triangles)
        } else {
            find_triangle_index1(vi0, &v_infos[vi1 as usize], ti0, &e_infos, &its.triangles)
        };
        let mut ti1 = u32::MAX;
        if let Some(t) = ti1_opt {
            ti1 = t;
            reorder_edges(&mut e_infos, &v_infos[vi0 as usize], ti0, ti1);
            reorder_edges(&mut e_infos, &v_infos[vi1 as usize], ti0, ti1);
        }
        let v_info0 = v_infos[vi0 as usize];
        let v_info1 = v_infos[vi1 as usize];
        if ti1_opt.is_none()
            || degenerate(vi0, &v_info1, &e_infos, &its.triangles)
            || degenerate(vi1, &v_info0, &e_infos, &its.triangles)
            || create_no_volume(vi0, vi1, &v_info0, &v_info1, &e_infos, &its.triangles)
            || is_flipped(new_vertex0, &v_info0, &t_infos, &e_infos, its)
            || is_flipped(new_vertex0, &v_info1, &t_infos, &e_infos, its)
        {
            // Try the triangle's next edge.
            let errors = three_errors(t0, &its.vertices, &v_infos);
            let ord: [usize; 3] = if errors[0] < errors[1] {
                if errors[0] < errors[2] {
                    if errors[1] < errors[2] {
                        [0, 1, 2]
                    } else {
                        [0, 2, 1]
                    }
                } else {
                    [2, 0, 1]
                }
            } else if errors[1] < errors[2] {
                if errors[0] < errors[2] {
                    [1, 0, 2]
                } else {
                    [1, 2, 0]
                }
            } else {
                [2, 1, 0]
            };
            let info = &mut t_infos[ti0 as usize];
            if info.min_index as usize == ord[0] {
                info.min_index = ord[1] as u8;
                e.value = errors[info.min_index as usize] as f32;
            } else if info.min_index as usize == ord[1] {
                info.min_index = ord[2] as u8;
                e.value = errors[info.min_index as usize] as f32;
            } else {
                // Changed again when an edge round it is collapsed.
                info.min_index = 3;
                e.value = maximal_error;
            }
            mpq.push(e, &mut places);
            continue;
        }

        last_collapsed_error = e.value;
        changed.clear();
        let end0 = v_info0.start + v_info0.count - 2;
        for di in v_info0.start..end0 {
            changed.push(e_infos[di as usize].t_index);
        }
        let end1 = v_info1.start + v_info1.count - 2;
        for di in v_info1.start..end1 {
            let info = e_infos[di as usize];
            its.triangles[info.t_index as usize][info.edge as usize] = vi0;
            changed.push(info.t_index);
        }
        v_infos[vi0 as usize].q = q;

        let vi_top0 = t0[(min_index + 2) % 3];
        let t1 = its.triangles[ti1 as usize];
        change_neighbors(
            &mut e_infos,
            &mut v_infos,
            &mut room,
            [ti0, ti1],
            [vi0, vi1],
            vi_top0,
            t1,
            &mut e_infos_swap,
        );
        its.vertices[vi0 as usize] = new_vertex0;

        mpq.remove(places[ti1 as usize], &mut places);
        for &ti in &changed {
            let at = places[ti as usize];
            let t = its.triangles[ti as usize];
            let info = &mut t_infos[ti as usize];
            info.n = create_normal(t, &its.vertices).map(|c| c as f32);
            mpq.heap[at] = triangle_error(ti, t, &its.vertices, &v_infos, &mut info.min_index);
            mpq.update(at, &mut places);
        }
        t_infos[ti0 as usize].set_deleted();
        t_infos[ti1 as usize].set_deleted();
        actual -= 2;
    }

    compact(&v_infos, &t_infos, &e_infos, its);
    Ok(last_collapsed_error)
}

type Init = (
    Vec<TriangleInfo>,
    Vec<VertexInfo>,
    Vec<EdgeInfo>,
    Vec<Error>,
);

/// `init`.
fn init(
    its: &Mesh,
    check: &dyn Fn() -> Result<(), Cancelled>,
    status: &mut dyn FnMut(i32),
) -> Result<Init, Cancelled> {
    let n = its.triangles.len();
    let mut offset = 0;
    let mut t_infos = vec![TriangleInfo::default(); n];
    let mut v_infos = vec![VertexInfo::default(); its.vertices.len()];
    {
        let mut quadrics = Vec::with_capacity(n);
        for (i, &t) in its.triangles.iter().enumerate() {
            let normal = create_normal(t, &its.vertices);
            t_infos[i].n = normal.map(|c| c as f32);
            quadrics.push(create_quadric(t, normal, &its.vertices));
            if i % 1_000_000 == 0 {
                check()?;
                status(offset + (i as i64 * STATUS_NORMAL_SIZE as i64 / n as i64) as i32);
            }
        }
        offset += STATUS_NORMAL_SIZE;
        for (i, (&t, q)) in its.triangles.iter().zip(&quadrics).enumerate() {
            for &v in &t {
                let info = &mut v_infos[v as usize];
                info.q.add(q);
                info.count += 1;
            }
            if i % 1_000_000 == 0 {
                check()?;
                status(offset + (i as i64 * STATUS_SUM_QUADRIC as i64 / n as i64) as i32);
            }
        }
        offset += STATUS_SUM_QUADRIC;
    }
    let mut start = 0u32;
    for info in &mut v_infos {
        info.start = start;
        start += info.count;
        info.count = 0;
    }
    offset += STATUS_SET_OFFSETS;
    check()?;
    status(offset);

    let mut errors = Vec::with_capacity(n);
    for (i, &t) in its.triangles.iter().enumerate() {
        errors.push(triangle_error(
            i as u32,
            t,
            &its.vertices,
            &v_infos,
            &mut t_infos[i].min_index,
        ));
        if i % 1_000_000 == 0 {
            check()?;
            status(offset + (i as i64 * STATUS_CALC_ERRORS as i64 / n as i64) as i32);
        }
    }
    offset += STATUS_CALC_ERRORS;

    let mut e_infos = vec![EdgeInfo::default(); n * 3];
    for (i, &t) in its.triangles.iter().enumerate() {
        for (j, &v) in t.iter().enumerate() {
            let info = &mut v_infos[v as usize];
            e_infos[(info.start + info.count) as usize] = EdgeInfo {
                t_index: i as u32,
                edge: j as u8,
            };
            info.count += 1;
        }
        if i % 1_000_000 == 0 {
            check()?;
            status(offset + (i as i64 * STATUS_CREATE_REFS as i64 / n as i64) as i32);
        }
    }
    check()?;
    status(100);
    Ok((t_infos, v_infos, e_infos, errors))
}

/// `find_triangle_index1`: the other triangle on the edge, None on a border.
fn find_triangle_index1(
    vi: u32,
    v_info: &VertexInfo,
    ti0: u32,
    e_infos: &[EdgeInfo],
    indices: &[[u32; 3]],
) -> Option<u32> {
    for e in &e_infos[v_info.start as usize..(v_info.start + v_info.count) as usize] {
        if e.t_index == ti0 {
            continue;
        }
        let t = indices[e.t_index as usize];
        if t[(e.edge as usize + 1) % 3] == vi || t[(e.edge as usize + 2) % 3] == vi {
            return Some(e.t_index);
        }
    }
    None
}

/// `reorder_edges`: the two triangles of the edge moved to the end of the run.
fn reorder_edges(e_infos: &mut [EdgeInfo], v_info: &VertexInfo, ti0: u32, ti1: u32) {
    let end = (v_info.start + v_info.count - 2) as usize;
    let mut swapped = false;
    for ei in v_info.start as usize..end {
        if e_infos[ei].t_index == ti0 {
            e_infos.swap(ei, end);
            if swapped {
                return;
            }
            if e_infos[ei].t_index == ti1 {
                e_infos.swap(ei, end + 1);
                return;
            }
            swapped = true;
        } else if e_infos[ei].t_index == ti1 {
            e_infos.swap(ei, end + 1);
            if swapped {
                return;
            }
            if e_infos[ei].t_index == ti0 {
                e_infos.swap(ei, end);
                return;
            }
            swapped = true;
        }
    }
}

/// `is_flipped`.
fn is_flipped(
    new_vertex: [f32; 3],
    v_info: &VertexInfo,
    t_infos: &[TriangleInfo],
    e_infos: &[EdgeInfo],
    its: &Mesh,
) -> bool {
    let thr_pos = 1.0f32 - f32::EPSILON;
    let thr_neg = -thr_pos;
    let dot_thr = 0.2f32;
    for e in &e_infos[v_info.start as usize..(v_info.start + v_info.count - 2) as usize] {
        let t = its.triangles[e.t_index as usize];
        let normal = t_infos[e.t_index as usize].n;
        let vf = its.vertices[t[(e.edge as usize + 1) % 3] as usize];
        let vs = its.vertices[t[(e.edge as usize + 2) % 3] as usize];
        let d1 = normalizedf(subf(vf, new_vertex));
        let d2 = normalizedf(subf(vs, new_vertex));
        let d = dotf(d1, d2);
        if d > thr_pos || d < thr_neg {
            return true;
        }
        let n = normalizedf(crossf(d1, d2));
        if dotf(n, normal) < dot_thr {
            return true;
        }
    }
    false
}

/// `degenerate`: a triangle round the other corner already has `vi`.
fn degenerate(vi: u32, v_info: &VertexInfo, e_infos: &[EdgeInfo], indices: &[[u32; 3]]) -> bool {
    e_infos[v_info.start as usize..(v_info.start + v_info.count - 2) as usize]
        .iter()
        .any(|e| indices[e.t_index as usize].contains(&vi))
}

/// `create_no_volume`: two triangles would lie back to back.
fn create_no_volume(
    vi0: u32,
    vi1: u32,
    v_info0: &VertexInfo,
    v_info1: &VertexInfo,
    e_infos: &[EdgeInfo],
    indices: &[[u32; 3]],
) -> bool {
    let range1 = v_info1.start as usize..(v_info1.start + v_info1.count - 2) as usize;
    for e0 in &e_infos[v_info0.start as usize..(v_info0.start + v_info0.count - 2) as usize] {
        let t0 = indices[e0.t_index as usize];
        let mut t0i = 0;
        let mut t0vi0 = t0[t0i];
        if t0vi0 == vi0 {
            t0i += 1;
            t0vi0 = t0[t0i];
        }
        t0i += 1;
        let mut t0vi1 = t0[t0i];
        if t0vi1 == vi0 {
            t0i += 1;
            t0vi1 = t0[t0i];
        }
        for e1 in &e_infos[range1.clone()] {
            let t1 = indices[e1.t_index as usize];
            let Some(mut t1i) = t1.iter().position(|&v| v == t0vi1) else {
                continue;
            };
            t1i = (t1i + 1) % 3;
            if t1[t1i] == vi1 {
                t1i = (t1i + 1) % 3;
            }
            if t1[t1i] == t0vi0 {
                return true;
            }
        }
    }
    false
}

/// `remove_triangle`.
fn remove_triangle(e_infos: &mut [EdgeInfo], v_info: &mut VertexInfo, ti: u32) {
    let start = v_info.start as usize;
    let last = start + v_info.count as usize - 1;
    for at in start..last {
        if e_infos[at].t_index == ti {
            e_infos[at] = e_infos[last];
            v_info.count -= 1;
            return;
        }
    }
    v_info.count -= 1;
}

/// `change_neighbors`: the triangles round `vi1` moved to the end of
/// `vi0`'s run, in their order.
///
/// Where this differs from the reference, on purpose: the reference makes
/// room for the longer run by shifting every later corner's run along the
/// one array, as far as it takes to find enough of the gaps collapses
/// leave, which on a mesh of many small even triangles (one subdivided)
/// shifts much of the array at each collapse. Here a run that has no room
/// left is moved to the end of the array with room to grow, and the array
/// is packed again in corner order when it has doubled. What the collapse
/// reads of a run is only its triangles and their order, which both keep,
/// so the result is the reference's to the bit (checked against its own
/// code: the tests' REFERENCE).
#[allow(clippy::too_many_arguments)]
fn change_neighbors(
    e_infos: &mut Vec<EdgeInfo>,
    v_infos: &mut [VertexInfo],
    room: &mut Room,
    [ti0, ti1]: [u32; 2],
    [vi0, vi1]: [u32; 2],
    vi_top0: u32,
    t1: [u32; 3],
    e_infos1: &mut Vec<EdgeInfo>,
) {
    let mut vi_top1 = t1[0];
    if vi_top1 == vi0 || vi_top1 == vi1 {
        vi_top1 = t1[1];
        if vi_top1 == vi0 || vi_top1 == vi1 {
            vi_top1 = t1[2];
        }
    }
    remove_triangle(e_infos, &mut v_infos[vi_top0 as usize], ti0);
    remove_triangle(e_infos, &mut v_infos[vi_top1 as usize], ti1);
    remove_triangle(e_infos, &mut v_infos[vi0 as usize], ti0);
    remove_triangle(e_infos, &mut v_infos[vi0 as usize], ti1);

    e_infos1.clear();
    let v1 = v_infos[vi1 as usize];
    for e in &e_infos[v1.start as usize..(v1.start + v1.count) as usize] {
        if e.t_index == ti0 || e.t_index == ti1 {
            continue;
        }
        e_infos1.push(*e);
    }
    v_infos[vi1 as usize].count = 0;

    let total = v_infos[vi0 as usize].count + e_infos1.len() as u32;
    if total > room.slots[vi0 as usize] {
        if e_infos.len() > room.limit {
            room.pack(e_infos, v_infos);
        }
        let v0 = v_infos[vi0 as usize];
        let start = e_infos.len();
        let slots = (total * 2).max(8);
        e_infos.extend_from_within(v0.start as usize..(v0.start + v0.count) as usize);
        e_infos.resize(start + slots as usize, EdgeInfo::default());
        v_infos[vi0 as usize].start = start as u32;
        room.slots[vi0 as usize] = slots;
    }
    let v0 = &mut v_infos[vi0 as usize];
    for e in e_infos1.iter() {
        e_infos[(v0.start + v0.count) as usize] = *e;
        v0.count += 1;
    }
}

/// How many places of the array of runs each corner holds, and how long
/// the array may grow before it is packed again.
struct Room {
    slots: Vec<u32>,
    limit: usize,
}

impl Room {
    fn new(v_infos: &[VertexInfo], len: usize) -> Self {
        Self {
            slots: v_infos.iter().map(|v| v.count).collect(),
            limit: (len * 2).max(64),
        }
    }

    /// Every run put back in corner order, each with room for itself.
    fn pack(&mut self, e_infos: &mut Vec<EdgeInfo>, v_infos: &mut [VertexInfo]) {
        let mut packed = Vec::with_capacity(e_infos.len());
        for (v, slots) in v_infos.iter_mut().zip(&mut self.slots) {
            let start = packed.len();
            packed.extend_from_slice(&e_infos[v.start as usize..(v.start + v.count) as usize]);
            v.start = start as u32;
            *slots = v.count;
        }
        self.limit = (packed.len() * 2).max(64);
        *e_infos = packed;
    }
}

/// `compact`.
fn compact(v_infos: &[VertexInfo], t_infos: &[TriangleInfo], e_infos: &[EdgeInfo], its: &mut Mesh) {
    let mut vi_new = 0u32;
    for (vi, info) in v_infos.iter().enumerate() {
        if info.is_deleted() {
            continue;
        }
        for e in &e_infos[info.start as usize..(info.start + info.count) as usize] {
            its.triangles[e.t_index as usize][e.edge as usize] = vi_new;
        }
        its.vertices[vi_new as usize] = its.vertices[vi];
        vi_new += 1;
    }
    its.vertices.truncate(vi_new as usize);
    let mut ti_new = 0;
    for (ti, info) in t_infos.iter().enumerate() {
        if info.is_deleted() {
            continue;
        }
        its.triangles[ti_new] = its.triangles[ti];
        ti_new += 1;
    }
    its.triangles.truncate(ti_new);
}

#[cfg(test)]
mod tests;
