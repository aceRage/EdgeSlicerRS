//! The shapes a cut's connectors are made of, each closed and wound
//! outward, at unit size (TriangleMesh.cpp `its_make_cylinder`,
//! `its_make_frustum`, `its_make_frustum_dowel`, `its_make_snap`, as
//! GLGizmoCut.cpp `get_connector_mesh` calls them).

use crate::Mesh;

/// The circle's sides: a "Circle" connector is a polygon of 360.
pub const CIRCLE: usize = 360;

/// A ring of `sides` corners at height `z`, radius `r`, the first at
/// `start` radians from +x, counter-clockwise.
fn ring(mesh: &mut Mesh, sides: usize, r: f64, z: f64, start: f64) -> u32 {
    let first = mesh.vertices.len() as u32;
    let step = std::f64::consts::TAU / sides as f64;
    for i in 0..sides {
        let a = start + step * i as f64;
        mesh.vertices
            .push([(r * a.cos()) as f32, (r * a.sin()) as f32, z as f32]);
    }
    first
}

/// A prism from `z = 0` to `z = 1` whose bottom has radius 1 and top
/// `top`, `sides` corners, the first on +y: a "Prism" (`top` 1) or a
/// plug's "Frustum" (`top` 0.5, `its_make_frustum`).
pub fn prism(sides: usize, top: f64) -> Mesh {
    let sides = sides.max(3);
    let mut mesh = Mesh::default();
    let start = std::f64::consts::FRAC_PI_2;
    let low = ring(&mut mesh, sides, 1.0, 0.0, start);
    let high = ring(&mut mesh, sides, top, 1.0, start);
    let (bottom, roof) = (mesh.vertices.len() as u32, mesh.vertices.len() as u32 + 1);
    mesh.vertices.push([0.0, 0.0, 0.0]);
    mesh.vertices.push([0.0, 0.0, 1.0]);
    let n = sides as u32;
    for i in 0..n {
        let j = (i + 1) % n;
        mesh.triangles.push([bottom, low + j, low + i]);
        mesh.triangles.push([roof, high + i, high + j]);
        mesh.triangles.push([low + i, low + j, high + j]);
        mesh.triangles.push([low + i, high + j, high + i]);
    }
    mesh
}

/// A dowel's "Frustum": two cones base to base, the ring of radius 1 at
/// `z = 0` (its first corner 45° from +x), the points at `z = ±1`
/// (`its_make_frustum_dowel`).
pub fn bicone(sides: usize) -> Mesh {
    let sides = sides.max(3);
    let mut mesh = Mesh::default();
    let around = ring(&mut mesh, sides, 1.0, 0.0, std::f64::consts::FRAC_PI_4);
    let (top, bottom) = (mesh.vertices.len() as u32, mesh.vertices.len() as u32 + 1);
    mesh.vertices.push([0.0, 0.0, 1.0]);
    mesh.vertices.push([0.0, 0.0, -1.0]);
    let n = sides as u32;
    for i in 0..n {
        let j = (i + 1) % n;
        mesh.triangles.push([top, around + i, around + j]);
        mesh.triangles.push([bottom, around + j, around + i]);
    }
    mesh
}

/// A "Snap": two prongs either side of a slit `space` wide (of the
/// radius), each half a round post from `z = 0` to `z = 1` that swells by
/// `bulge` (of the radius) at mid-height and narrows to half the radius at
/// the top (`its_make_snap`).
pub fn snap(space: f64, bulge: f64) -> Mesh {
    // Ten steps across each prong, as the reference.
    const STEPS: usize = 10;
    let space = space.clamp(0.0, 0.49);
    let (b_len, m_len, t_len) = (1.0, 1.0 + bulge.max(0.0), 0.5);
    let b_angle = (space / b_len).acos();
    let t_angle = (space / t_len).acos();
    // The bulge's ellipse (`get_m_len`), flattened to the radius at the
    // slit.
    let f = (b_len - m_len) / m_len;
    let m_at = |angle: f64| {
        let s = angle.sin().powi(2);
        (b_len * b_len / (1.0 + (1.0 / ((1.0 - f) * (1.0 - f)) - 1.0) * s)).sqrt()
    };
    let at = |len: f64, angle: f64| [-len * angle.sin(), len * angle.cos()];
    let mut mesh = Mesh::default();
    for (centre_x, turn) in [
        (-space, std::f64::consts::FRAC_PI_2),
        (space, 3.0 * std::f64::consts::FRAC_PI_2),
    ] {
        // Each prong: a run of columns (bottom, middle, top) from one side
        // of the slit round to the other, closed by its flat face on the
        // slit, a bottom and a top.
        let first = mesh.vertices.len() as u32;
        let column = |mesh: &mut Mesh, i: usize| {
            let t = i as f64 / STEPS as f64;
            let b = turn - b_angle + 2.0 * b_angle * t;
            let top = turn - t_angle + 2.0 * t_angle * t;
            let [bx, by] = at(b_len, b);
            let [mx, my] = at(m_at(b), b);
            let [tx, ty] = at(t_len, top);
            mesh.vertices.push([bx as f32, by as f32, 0.0]);
            mesh.vertices.push([mx as f32, my as f32, 0.5]);
            mesh.vertices.push([tx as f32, ty as f32, 1.0]);
        };
        for i in 0..=STEPS {
            column(&mut mesh, i);
        }
        let (bottom, roof) = (mesh.vertices.len() as u32, mesh.vertices.len() as u32 + 1);
        mesh.vertices.push([centre_x as f32, 0.0, 0.0]);
        mesh.vertices.push([centre_x as f32, 0.0, 1.0]);
        let c = |i: usize, level: u32| first + 3 * i as u32 + level;
        for i in 0..STEPS {
            let j = i + 1;
            // The round side: bottom to middle, middle to top.
            for level in 0..2 {
                mesh.triangles
                    .push([c(i, level), c(j, level), c(j, level + 1)]);
                mesh.triangles
                    .push([c(i, level), c(j, level + 1), c(i, level + 1)]);
            }
            mesh.triangles.push([bottom, c(j, 0), c(i, 0)]);
            mesh.triangles.push([roof, c(i, 2), c(j, 2)]);
        }
        // The flat face on the slit, joining the last column to the first
        // through the centre line.
        let last = STEPS;
        mesh.triangles.push([c(last, 0), bottom, c(last, 1)]);
        mesh.triangles.push([bottom, roof, c(last, 1)]);
        mesh.triangles.push([c(last, 1), roof, c(last, 2)]);
        mesh.triangles.push([bottom, c(0, 0), c(0, 1)]);
        mesh.triangles.push([bottom, c(0, 1), roof]);
        mesh.triangles.push([roof, c(0, 1), c(0, 2)]);
    }
    // Wound outward whichever way the columns run.
    if mesh.signed_volume() < 0.0 {
        for triangle in &mut mesh.triangles {
            triangle.swap(1, 2);
        }
    }
    mesh
}

/// A solid of revolution about z (TriangleMesh.cpp `its_make_revolved`):
/// `profile` is a closed outline in (radius, z), its ends on the axis, cut
/// into `sectors` around. Wound outward whichever way the outline runs.
pub fn revolved(profile: &[[f64; 2]], sectors: usize) -> Mesh {
    let mut mesh = Mesh::default();
    let n = profile.len();
    if n < 3 || sectors < 3 {
        return mesh;
    }
    let mut prof = profile.to_vec();
    let twice: f64 = (0..n)
        .map(|i| {
            let (a, b) = (prof[i], prof[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if twice < 0.0 {
        prof.reverse();
    }
    let on_axis: Vec<bool> = prof.iter().map(|p| p[0] <= 1e-9).collect();
    let mut base = Vec::with_capacity(n);
    for (i, p) in prof.iter().enumerate() {
        base.push(mesh.vertices.len() as u32);
        if on_axis[i] {
            mesh.vertices.push([0.0, 0.0, p[1] as f32]);
        } else {
            for j in 0..sectors {
                let a = std::f64::consts::TAU * j as f64 / sectors as f64;
                mesh.vertices.push([
                    (p[0] * a.cos()) as f32,
                    (p[0] * a.sin()) as f32,
                    p[1] as f32,
                ]);
            }
        }
    }
    let id = |i: usize, j: usize| {
        if on_axis[i] {
            base[i]
        } else {
            base[i] + j as u32
        }
    };
    for i in 0..n {
        let i2 = (i + 1) % n;
        if on_axis[i] && on_axis[i2] {
            continue;
        }
        for j in 0..sectors {
            let j2 = (j + 1) % sectors;
            let (a, b, c, d) = (id(i, j), id(i, j2), id(i2, j2), id(i2, j));
            if !on_axis[i] {
                mesh.triangles.push([a, b, c]);
            }
            if !on_axis[i2] {
                mesh.triangles.push([a, c, d]);
            }
        }
    }
    if mesh.signed_volume() < 0.0 {
        for triangle in &mut mesh.triangles {
            triangle.swap(1, 2);
        }
    }
    mesh
}

/// A prism over a convex outline in (x, y), from `z0` to `z1` (PushFit.cpp
/// `extrude_convex`), wound outward.
pub fn extruded(outline: &[[f64; 2]], z0: f64, z1: f64) -> Mesh {
    let mut mesh = Mesh::default();
    let n = outline.len();
    if n < 3 {
        return mesh;
    }
    for z in [z0, z1] {
        for p in outline {
            mesh.vertices.push([p[0] as f32, p[1] as f32, z as f32]);
        }
    }
    let n32 = n as u32;
    for i in 0..n32 {
        let i2 = (i + 1) % n32;
        mesh.triangles.push([i, i2, n32 + i2]);
        mesh.triangles.push([i, n32 + i2, n32 + i]);
    }
    for i in 1..n32 - 1 {
        mesh.triangles.push([0, i + 1, i]);
        mesh.triangles.push([n32, n32 + i, n32 + i + 1]);
    }
    if mesh.signed_volume() < 0.0 {
        for triangle in &mut mesh.triangles {
            triangle.swap(1, 2);
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cut::{flat_triangles, is_closed};

    fn sound(mesh: &Mesh) {
        assert!(is_closed(mesh));
        assert_eq!(flat_triangles(mesh), 0);
        assert!(mesh.signed_volume() > 0.0);
    }

    #[test]
    fn every_shape_is_closed_wound_outward_and_the_size_it_should_be() {
        use std::f64::consts::PI;
        for sides in [3, 4, 6, CIRCLE] {
            let polygon = 0.5 * sides as f64 * (2.0 * PI / sides as f64).sin();
            let mesh = prism(sides, 1.0);
            sound(&mesh);
            assert!((mesh.signed_volume() - polygon).abs() < 1e-5, "{sides}");
            // The frustum: a quarter of the bottom's area at the top.
            let mesh = prism(sides, 0.5);
            sound(&mesh);
            let expected = polygon * (1.0 + 0.25 + 0.5) / 3.0;
            assert!((mesh.signed_volume() - expected).abs() < 1e-5, "{sides}");
            let mesh = bicone(sides);
            sound(&mesh);
            assert!(
                (mesh.signed_volume() - 2.0 * polygon / 3.0).abs() < 1e-5,
                "{sides}"
            );
        }
        // A prism's first corner is on +y (`its_make_cylinder`).
        let triangle = prism(3, 1.0);
        assert!((triangle.vertices[0][1] - 1.0).abs() < 1e-6);
        let snap = snap(0.3, 0.15);
        sound(&snap);
        let (low, high) = snap
            .vertices
            .iter()
            .fold(([f32::MAX; 3], [f32::MIN; 3]), |(l, h), v| {
                (
                    std::array::from_fn(|i| l[i].min(v[i])),
                    std::array::from_fn(|i| h[i].max(v[i])),
                )
            });
        assert_eq!((low[2], high[2]), (0.0, 1.0));
        // A revolved cylinder and a prism over a square.
        let cylinder = revolved(&[[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]], 96);
        sound(&cylinder);
        let area = 0.5 * 96.0 * (std::f64::consts::TAU / 96.0).sin();
        assert!((cylinder.signed_volume() - 2.0 * area).abs() < 1e-4);
        let square = extruded(&[[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]], -1.0, 1.0);
        sound(&square);
        assert!((square.signed_volume() - 2.0).abs() < 1e-6);
        // The bulge reaches past the radius; the slit splits it in two.
        assert!(high[0] > 1.1 && high[0] < 1.16, "{high:?}");
        assert!(!snap.vertices.iter().any(|v| v[0].abs() < 0.29));
    }
}
