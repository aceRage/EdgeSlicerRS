//! Triangulating a flat polygon with holes by cutting off ears, every
//! corner of the polygon kept: the cap over a cut ([`crate::cut`]).
//!
//! The method is Mapbox's earcut (ISC licence), whose steps this follows:
//! holes joined to the outline by bridges from their leftmost corners, ears
//! taken off while there are any, and a polygon with none split along a
//! diagonal that lies inside it. Unlike earcut it never drops a corner: a
//! corner on a line between its neighbours is a corner of the side of the
//! half next to it, and the cap has to meet it there or the half is not
//! closed. So no triangle of three corners on a line is taken (an ear has
//! to turn), and nothing is "cured" by leaving corners out.

/// One corner of the ring being cut down.
struct Node {
    /// The vertex, as given.
    v: u32,
    x: f64,
    y: f64,
    prev: usize,
    next: usize,
}

struct Ring {
    nodes: Vec<Node>,
}

/// The sign of the turn at `q` from `p` to `r`, as earcut measures it:
/// negative for a turn to the left (a convex corner of an anticlockwise
/// ring).
fn area(p: &Node, q: &Node, r: &Node) -> f64 {
    (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
}

/// How small a turn's sine is taken as straight on.
const STRAIGHT: f64 = 1e-9;

fn length(p: &Node, q: &Node) -> f64 {
    (q.x - p.x).hypot(q.y - p.y)
}

fn same(p: &Node, q: &Node) -> bool {
    p.x == q.x && p.y == q.y
}

#[allow(clippy::too_many_arguments)]
fn in_triangle(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64, px: f64, py: f64) -> bool {
    (cx - px) * (ay - py) >= (ax - px) * (cy - py)
        && (ax - px) * (by - py) >= (bx - px) * (ay - py)
        && (bx - px) * (cy - py) >= (cx - px) * (by - py)
}

/// Whether `p` is in the anticlockwise triangle `a b c` or on its edges,
/// or so near an edge that it may have been worked out to lie on it.
fn near_triangle(a: &Node, b: &Node, c: &Node, p: &Node) -> bool {
    let side = |s: &Node, e: &Node| {
        let cross = (e.x - s.x) * (p.y - s.y) - (e.y - s.y) * (p.x - s.x);
        cross >= -STRAIGHT * length(s, e) * length(s, p)
    };
    side(a, b) && side(b, c) && side(c, a)
}

impl Ring {
    fn n(&self, i: usize) -> &Node {
        &self.nodes[i]
    }

    fn insert(&mut self, v: u32, [x, y]: [f64; 2], last: Option<usize>) -> usize {
        let at = self.nodes.len();
        match last {
            None => self.nodes.push(Node {
                v,
                x,
                y,
                prev: at,
                next: at,
            }),
            Some(last) => {
                let next = self.nodes[last].next;
                self.nodes.push(Node {
                    v,
                    x,
                    y,
                    prev: last,
                    next,
                });
                self.nodes[next].prev = at;
                self.nodes[last].next = at;
            }
        }
        at
    }

    fn remove(&mut self, i: usize) {
        let (prev, next) = (self.nodes[i].prev, self.nodes[i].next);
        self.nodes[next].prev = prev;
        self.nodes[prev].next = next;
    }

    /// Drops a corner that follows itself (two nodes of one vertex next to
    /// each other, as a bridge can leave). Corners on a line stay.
    fn tidy(&mut self, start: usize) -> usize {
        let mut start = start;
        let mut p = start;
        loop {
            let next = self.nodes[p].next;
            if next != p && self.nodes[p].v == self.nodes[next].v {
                self.remove(next);
                if next == start {
                    start = p;
                }
                if self.nodes[p].next == p {
                    return p;
                }
                continue;
            }
            p = next;
            if p == start {
                return start;
            }
        }
    }

    fn is_ear(&self, ear: usize) -> bool {
        let b = self.n(ear);
        let a = self.n(b.prev);
        let c = self.n(b.next);
        // A turn too slight to tell from a straight line, where the corners
        // were worked out to lie on one, is taken as none: its triangle
        // would have no area once its corners are rounded.
        if area(a, b, c) >= -STRAIGHT * length(a, b) * length(b, c) {
            return false;
        }
        let (x0, x1) = (a.x.min(b.x).min(c.x), a.x.max(b.x).max(c.x));
        let (y0, y1) = (a.y.min(b.y).min(c.y), a.y.max(b.y).max(c.y));
        let mut p = c.next;
        while p != b.prev {
            let q = self.n(p);
            if q.x >= x0
                && q.x <= x1
                && q.y >= y0
                && q.y <= y1
                && !same(q, a)
                && !same(q, c)
                && near_triangle(a, b, c, q)
            {
                return false;
            }
            p = q.next;
        }
        true
    }

    /// Takes ears off the ring until it is gone; false when it could not be
    /// cut down whole.
    fn cut_down(&mut self, ear: usize, out: &mut Vec<[u32; 3]>, pass: u8) -> bool {
        let mut ear = ear;
        let mut stop = ear;
        loop {
            let (prev, next) = (self.n(ear).prev, self.n(ear).next);
            if prev == next {
                // Two corners left: nothing to cover.
                return true;
            }
            if self.n(prev).prev == next {
                // The last three: a triangle, however thin, unless two of
                // its corners are one vertex.
                let (a, b, c) = (self.n(prev), self.n(ear), self.n(next));
                if a.v == b.v || b.v == c.v || c.v == a.v {
                    return true;
                }
                if area(a, b, c) < 0.0 {
                    out.push([a.v, b.v, c.v]);
                    return true;
                }
                return false;
            }
            if self.is_ear(ear) {
                out.push([self.n(prev).v, self.n(ear).v, self.n(next).v]);
                self.remove(ear);
                ear = self.n(next).next;
                stop = ear;
                continue;
            }
            ear = next;
            if ear == stop {
                return match pass {
                    0 => {
                        let start = self.tidy(ear);
                        self.cut_down(start, out, 1)
                    }
                    _ => self.split(ear, out),
                };
            }
        }
    }

    /// Splits a ring with no ear along a diagonal inside it and cuts down
    /// each side.
    fn split(&mut self, start: usize, out: &mut Vec<[u32; 3]>) -> bool {
        let mut a = start;
        loop {
            let mut b = self.n(self.n(a).next).next;
            while b != self.n(a).prev {
                if self.n(a).v != self.n(b).v && self.valid_diagonal(a, b) {
                    let c = self.join(a, b);
                    let a = self.tidy(a);
                    let c = self.tidy(c);
                    return self.cut_down(a, out, 1) && self.cut_down(c, out, 1);
                }
                b = self.n(b).next;
            }
            a = self.n(a).next;
            if a == start {
                return false;
            }
        }
    }

    /// Joins `a` and `b` by two edges, one each way, making two rings of
    /// one (or one of two, for a hole): answers the copy of `b` that starts
    /// the second.
    fn join(&mut self, a: usize, b: usize) -> usize {
        let (av, ax, ay) = (self.n(a).v, self.n(a).x, self.n(a).y);
        let (bv, bx, by) = (self.n(b).v, self.n(b).x, self.n(b).y);
        let an = self.n(a).next;
        let bp = self.n(b).prev;
        let a2 = self.nodes.len();
        self.nodes.push(Node {
            v: av,
            x: ax,
            y: ay,
            prev: a2,
            next: a2,
        });
        let b2 = self.nodes.len();
        self.nodes.push(Node {
            v: bv,
            x: bx,
            y: by,
            prev: b2,
            next: b2,
        });
        self.nodes[a].next = b;
        self.nodes[b].prev = a;
        self.nodes[a2].next = an;
        self.nodes[an].prev = a2;
        self.nodes[b2].next = a2;
        self.nodes[a2].prev = b2;
        self.nodes[bp].next = b2;
        self.nodes[b2].prev = bp;
        b2
    }

    fn valid_diagonal(&self, a: usize, b: usize) -> bool {
        let (na, nb) = (self.n(a), self.n(b));
        self.n(na.next).v != nb.v
            && self.n(na.prev).v != nb.v
            && !self.crosses_ring(a, b)
            && ((self.locally_inside(a, b)
                && self.locally_inside(b, a)
                && self.middle_inside(a, b)
                && (area(self.n(na.prev), na, self.n(nb.prev)) != 0.0
                    || area(na, self.n(nb.prev), nb) != 0.0))
                || (same(na, nb)
                    && area(self.n(na.prev), na, self.n(na.next)) > 0.0
                    && area(self.n(nb.prev), nb, self.n(nb.next)) > 0.0))
    }

    fn crosses_ring(&self, a: usize, b: usize) -> bool {
        let (va, vb) = (self.n(a).v, self.n(b).v);
        let mut p = a;
        loop {
            let q = self.n(p).next;
            let (vp, vq) = (self.n(p).v, self.n(q).v);
            if vp != va
                && vq != va
                && vp != vb
                && vq != vb
                && intersects(self.n(p), self.n(q), self.n(a), self.n(b))
            {
                return true;
            }
            p = q;
            if p == a {
                return false;
            }
        }
    }

    fn locally_inside(&self, a: usize, b: usize) -> bool {
        let (na, nb) = (self.n(a), self.n(b));
        let (prev, next) = (self.n(na.prev), self.n(na.next));
        if area(prev, na, next) < 0.0 {
            area(na, nb, next) >= 0.0 && area(na, prev, nb) >= 0.0
        } else {
            area(na, nb, prev) < 0.0 || area(na, next, nb) < 0.0
        }
    }

    fn middle_inside(&self, a: usize, b: usize) -> bool {
        let (na, nb) = (self.n(a), self.n(b));
        let (px, py) = ((na.x + nb.x) / 2.0, (na.y + nb.y) / 2.0);
        let mut inside = false;
        let mut p = a;
        loop {
            let q = self.n(p).next;
            let (np, nq) = (self.n(p), self.n(q));
            if (np.y > py) != (nq.y > py)
                && nq.y != np.y
                && px < (nq.x - np.x) * (py - np.y) / (nq.y - np.y) + np.x
            {
                inside = !inside;
            }
            p = q;
            if p == a {
                return inside;
            }
        }
    }

    fn leftmost(&self, start: usize) -> usize {
        let mut p = start;
        let mut left = start;
        loop {
            let (np, nl) = (self.n(p), self.n(left));
            if np.x < nl.x || (np.x == nl.x && np.y < nl.y) {
                left = p;
            }
            p = np.next;
            if p == start {
                return left;
            }
        }
    }

    /// The corner of the outline a hole's leftmost corner is bridged to.
    fn bridge(&self, hole: usize, outer: usize) -> Option<usize> {
        let (hx, hy) = (self.n(hole).x, self.n(hole).y);
        let mut p = outer;
        let mut qx = f64::NEG_INFINITY;
        let mut m: Option<usize> = None;
        loop {
            let (np, nn) = (self.n(p), self.n(self.n(p).next));
            if hy <= np.y && hy >= nn.y && nn.y != np.y {
                let x = np.x + (hy - np.y) * (nn.x - np.x) / (nn.y - np.y);
                if x <= hx && x > qx {
                    qx = x;
                    let pick = if np.x < nn.x { p } else { np.next };
                    m = Some(pick);
                    if x == hx {
                        return m;
                    }
                }
            }
            p = np.next;
            if p == outer {
                break;
            }
        }
        let mut m = m?;
        let stop = m;
        let (mx, my) = (self.n(m).x, self.n(m).y);
        let mut tan_min = f64::INFINITY;
        let mut p = m;
        loop {
            let np = self.n(p);
            if hx >= np.x && np.x >= mx && hx != np.x && {
                let (ax, cx) = if hy < my { (hx, qx) } else { (qx, hx) };
                in_triangle(ax, hy, mx, my, cx, hy, np.x, np.y)
            } {
                let tan = (hy - np.y).abs() / (hx - np.x);
                if self.locally_inside(p, hole)
                    && (tan < tan_min
                        || (tan == tan_min
                            && (np.x > self.n(m).x
                                || (np.x == self.n(m).x && self.sector_contains(m, p)))))
                {
                    m = p;
                    tan_min = tan;
                }
            }
            p = np.next;
            if p == stop {
                return Some(m);
            }
        }
    }

    fn sector_contains(&self, m: usize, p: usize) -> bool {
        let (nm, np) = (self.n(m), self.n(p));
        area(self.n(nm.prev), nm, self.n(np.prev)) < 0.0
            && area(self.n(np.next), nm, self.n(nm.next)) < 0.0
    }
}

fn sign(v: f64) -> i8 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

fn on_segment(p: &Node, q: &Node, r: &Node) -> bool {
    q.x <= p.x.max(r.x) && q.x >= p.x.min(r.x) && q.y <= p.y.max(r.y) && q.y >= p.y.min(r.y)
}

fn intersects(p1: &Node, q1: &Node, p2: &Node, q2: &Node) -> bool {
    let o1 = sign(area(p1, q1, p2));
    let o2 = sign(area(p1, q1, q2));
    let o3 = sign(area(p2, q2, p1));
    let o4 = sign(area(p2, q2, q1));
    (o1 != o2 && o3 != o4)
        || (o1 == 0 && on_segment(p1, p2, q1))
        || (o2 == 0 && on_segment(p1, q2, q1))
        || (o3 == 0 && on_segment(p2, p1, q2))
        || (o4 == 0 && on_segment(p2, q1, q2))
}

/// Triangulates an outline (anticlockwise) with its holes (clockwise),
/// each a ring of vertices placed by `xy`, adding the triangles
/// (anticlockwise) to `out`. False when it could not be done whole.
pub(crate) fn triangulate(
    outline: &[u32],
    holes: &[Vec<u32>],
    xy: &impl Fn(u32) -> [f64; 2],
    out: &mut Vec<[u32; 3]>,
) -> bool {
    if outline.len() < 3 {
        return false;
    }
    let mut ring = Ring { nodes: Vec::new() };
    let mut last = None;
    for v in outline {
        last = Some(ring.insert(*v, xy(*v), last));
    }
    let mut outer = last.expect("an outline has corners");
    outer = ring.tidy(outer);
    let mut starts: Vec<usize> = Vec::new();
    for hole in holes {
        if hole.len() < 3 {
            return false;
        }
        let mut last = None;
        for v in hole {
            last = Some(ring.insert(*v, xy(*v), last));
        }
        let start = ring.tidy(last.expect("a hole has corners"));
        starts.push(ring.leftmost(start));
    }
    starts.sort_by(|a, b| {
        let (na, nb) = (ring.n(*a), ring.n(*b));
        na.x.total_cmp(&nb.x)
            .then(na.y.total_cmp(&nb.y))
            .then(na.v.cmp(&nb.v))
    });
    for hole in starts {
        let Some(m) = ring.bridge(hole, outer) else {
            return false;
        };
        let back = ring.join(m, hole);
        ring.tidy(back);
        outer = ring.tidy(m);
    }
    ring.cut_down(outer, out, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(points: &[[f64; 2]], outline: &[u32], holes: &[Vec<u32>]) -> Vec<[u32; 3]> {
        let mut out = Vec::new();
        assert!(triangulate(
            outline,
            holes,
            &|v| points[v as usize],
            &mut out
        ));
        out
    }

    fn area_of(points: &[[f64; 2]], t: [u32; 3]) -> f64 {
        let [a, b, c] = t.map(|v| points[v as usize]);
        ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2.0
    }

    #[test]
    fn keeps_corners_on_a_line() {
        // A square with a corner halfway along each side.
        let points = [
            [0.0, 0.0],
            [1.0, 0.0],
            [2.0, 0.0],
            [2.0, 1.0],
            [2.0, 2.0],
            [1.0, 2.0],
            [0.0, 2.0],
            [0.0, 1.0],
        ];
        let out = run(&points, &[0, 1, 2, 3, 4, 5, 6, 7], &[]);
        assert_eq!(out.len(), 6);
        let total: f64 = out.iter().map(|t| area_of(&points, *t)).sum();
        assert_eq!(total, 4.0);
        assert!(out.iter().all(|t| area_of(&points, *t) > 0.0));
        let mut used: Vec<u32> = out.iter().flatten().copied().collect();
        used.sort_unstable();
        used.dedup();
        assert_eq!(used.len(), 8);
    }

    #[test]
    fn bridges_holes() {
        let points = [
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [2.0, 2.0],
            [2.0, 4.0],
            [4.0, 4.0],
            [4.0, 2.0],
            [6.0, 6.0],
            [6.0, 8.0],
            [8.0, 8.0],
            [8.0, 6.0],
        ];
        let out = run(
            &points,
            &[0, 1, 2, 3],
            &[vec![4, 5, 6, 7], vec![8, 9, 10, 11]],
        );
        let total: f64 = out.iter().map(|t| area_of(&points, *t)).sum();
        assert!((total - 92.0).abs() < 1e-9);
        assert!(out.iter().all(|t| area_of(&points, *t) > 0.0));
        // n corners and h holes make n + 2h - 2 triangles.
        assert_eq!(out.len(), 12 + 4 - 2);
    }
}
