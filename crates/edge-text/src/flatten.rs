//! A glyph's outline as stb_truetype holds it and flattens it
//! (imstb_truetype.h `stbtt__GetGlyphShapeTT`, `stbtt_FlattenCurves`,
//! `stbtt__tesselate_curve`, `stbtt__tesselate_cubic`), in single
//! precision as there, so that the points are the reference's.

/// One step of an outline (`stbtt_vertex`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Vertex {
    Move(f32, f32),
    Line(f32, f32),
    /// A quadratic curve: its control point, then where it ends.
    Curve(f32, f32, f32, f32),
    /// A cubic curve (CFF fonts): two control points, then the end.
    Cubic(f32, f32, f32, f32, f32, f32),
}

/// The outline `ttf-parser` reads, as stb_truetype's vertices.
#[derive(Default)]
pub(crate) struct Outline {
    pub vertices: Vec<Vertex>,
}

/// stb_truetype finds the on-curve point implied between two off-curve
/// ones with integers, `(a + b) >> 1`, rounding half down, where
/// `ttf-parser` keeps the half: a coordinate ending in a half is such a
/// point, and is taken down to stb's.
fn stb(value: f32) -> f32 {
    if value.fract().abs() == 0.5 {
        value.floor()
    } else {
        value
    }
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.vertices.push(Vertex::Move(stb(x), stb(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.vertices.push(Vertex::Line(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.vertices.push(Vertex::Curve(x1, y1, stb(x), stb(y)));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.vertices.push(Vertex::Cubic(x1, y1, x2, y2, x, y));
    }

    fn close(&mut self) {}
}

#[allow(clippy::too_many_arguments)]
fn tesselate_curve(
    points: &mut Vec<[f32; 2]>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    flatness_squared: f32,
    n: u32,
) {
    // midpoint
    let mx = (x0 + 2.0 * x1 + x2) / 4.0;
    let my = (y0 + 2.0 * y1 + y2) / 4.0;
    // versus directly drawn line
    let dx = (x0 + x2) / 2.0 - mx;
    let dy = (y0 + y2) / 2.0 - my;
    if n > 16 {
        return;
    }
    if dx * dx + dy * dy > flatness_squared {
        tesselate_curve(
            points,
            x0,
            y0,
            (x0 + x1) / 2.0,
            (y0 + y1) / 2.0,
            mx,
            my,
            flatness_squared,
            n + 1,
        );
        tesselate_curve(
            points,
            mx,
            my,
            (x1 + x2) / 2.0,
            (y1 + y2) / 2.0,
            x2,
            y2,
            flatness_squared,
            n + 1,
        );
    } else {
        points.push([x2, y2]);
    }
}

#[allow(clippy::too_many_arguments)]
fn tesselate_cubic(
    points: &mut Vec<[f32; 2]>,
    [x0, y0]: [f32; 2],
    [x1, y1]: [f32; 2],
    [x2, y2]: [f32; 2],
    [x3, y3]: [f32; 2],
    flatness_squared: f32,
    n: u32,
) {
    let (dx0, dy0) = (x1 - x0, y1 - y0);
    let (dx1, dy1) = (x2 - x1, y2 - y1);
    let (dx2, dy2) = (x3 - x2, y3 - y2);
    let (dx, dy) = (x3 - x0, y3 - y0);
    // `STBTT_sqrt` is the C library's, in double precision.
    let root = |v: f32| f64::from(v).sqrt();
    let longlen = (root(dx0 * dx0 + dy0 * dy0)
        + root(dx1 * dx1 + dy1 * dy1)
        + root(dx2 * dx2 + dy2 * dy2)) as f32;
    let shortlen = root(dx * dx + dy * dy) as f32;
    let flat = longlen * longlen - shortlen * shortlen;
    if n > 16 {
        return;
    }
    if flat > flatness_squared {
        let (x01, y01) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (x12, y12) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
        let (x23, y23) = ((x2 + x3) / 2.0, (y2 + y3) / 2.0);
        let (xa, ya) = ((x01 + x12) / 2.0, (y01 + y12) / 2.0);
        let (xb, yb) = ((x12 + x23) / 2.0, (y12 + y23) / 2.0);
        let (mx, my) = ((xa + xb) / 2.0, (ya + yb) / 2.0);
        tesselate_cubic(
            points,
            [x0, y0],
            [x01, y01],
            [xa, ya],
            [mx, my],
            flatness_squared,
            n + 1,
        );
        tesselate_cubic(
            points,
            [mx, my],
            [xb, yb],
            [x23, y23],
            [x3, y3],
            flatness_squared,
            n + 1,
        );
    } else {
        points.push([x3, y3]);
    }
}

/// The outline as closed contours of points, each ending where it began
/// (`stbtt_FlattenCurves`).
pub(crate) fn flatten(vertices: &[Vertex], flatness: f32) -> Vec<Vec<[f32; 2]>> {
    let flatness_squared = flatness * flatness;
    let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    for vertex in vertices {
        match *vertex {
            Vertex::Move(px, py) => {
                contours.push(vec![[px, py]]);
                (x, y) = (px, py);
            }
            Vertex::Line(px, py) => {
                if let Some(points) = contours.last_mut() {
                    points.push([px, py]);
                }
                (x, y) = (px, py);
            }
            Vertex::Curve(cx, cy, px, py) => {
                if let Some(points) = contours.last_mut() {
                    tesselate_curve(points, x, y, cx, cy, px, py, flatness_squared, 0);
                }
                (x, y) = (px, py);
            }
            Vertex::Cubic(c1x, c1y, c2x, c2y, px, py) => {
                if let Some(points) = contours.last_mut() {
                    tesselate_cubic(
                        points,
                        [x, y],
                        [c1x, c1y],
                        [c2x, c2y],
                        [px, py],
                        flatness_squared,
                        0,
                    );
                }
                (x, y) = (px, py);
            }
        }
    }
    contours
}
