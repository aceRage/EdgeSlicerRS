//! An object's outline on the bed and the allowance kept round it for
//! supports (Model.cpp `ModelObject::convex_hull_2d`,
//! `ModelInstance::get_arrange_polygon`; ModelArrange.cpp
//! `get_instance_arrange_poly`, `instance_may_get_support`).

use edge_geom::{Point2, Polygon, convex_hull};

/// The convex hull of points projected onto the bed (mm in, nanometres
/// out): the outline the reference arranges an object by.
pub fn outline(points: impl IntoIterator<Item = [f64; 3]>) -> Polygon {
    convex_hull(points.into_iter().map(|p| Point2::from_mm(p[0], p[1])))
}

/// What kind of supports an object would print with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportKind {
    /// Normal supports, placed where the object needs them.
    NormalAuto,
    /// Tree supports, placed where the object needs them.
    TreeAuto,
    /// Normal supports only where painted.
    NormalManual,
    /// Tree supports only where painted.
    TreeManual,
}

impl SupportKind {
    /// From the `support_type` setting's value.
    pub fn of(value: &str) -> Self {
        match value {
            "tree(auto)" => Self::TreeAuto,
            "normal(manual)" => Self::NormalManual,
            "tree(manual)" => Self::TreeManual,
            _ => Self::NormalAuto,
        }
    }

    fn automatic(self) -> bool {
        matches!(self, Self::NormalAuto | Self::TreeAuto)
    }

    fn tree(self) -> bool {
        matches!(self, Self::TreeAuto | Self::TreeManual)
    }
}

/// Whether supports would be printed under an object standing as its
/// facets (corners in mm, on the bed) stand: for the automatic kinds, a
/// facet facing down more steeply than `threshold_deg` from the
/// horizontal (0: 30 degrees) that is not the object's foot on the bed;
/// for the manual kinds, any support painted on (`painted`)
/// (`instance_may_get_support`).
pub fn may_get_support(
    facets: impl IntoIterator<Item = [[f64; 3]; 3]>,
    kind: SupportKind,
    threshold_deg: f64,
    painted: bool,
) -> bool {
    if !kind.automatic() {
        return painted;
    }
    let facets: Vec<[[f64; 3]; 3]> = facets.into_iter().collect();
    let threshold = if threshold_deg > 0.0 {
        threshold_deg
    } else {
        30.0
    };
    let cos_threshold = threshold.to_radians().cos();
    let z_min = facets
        .iter()
        .flat_map(|f| f.iter().map(|v| v[2]))
        .fold(f64::MAX, f64::min);
    facets.iter().any(|[a, b, c]| {
        let (u, v) = (
            [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
            [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
        );
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if l <= 1e-12 || -n[2] / l <= cos_threshold {
            return false;
        }
        // The foot on the bed is no overhang.
        a[2].max(b[2]).max(c[2]) > z_min + 0.05
    })
}

/// The allowance kept round an object with the spacing at "auto", in mm,
/// and whether it is for tree supports: 1 mm, 6 with normal supports, 24
/// (twice the widest first-layer branch) with tree supports, supports
/// counting only where they would be printed.
pub fn support_allowance(supports: Option<SupportKind>) -> (f64, bool) {
    match supports {
        None => (1.0, false),
        Some(kind) if !kind.tree() => (6.0, false),
        Some(_) => (24.0, true),
    }
}

/// libnest2d's filament temperature kinds (Print.hpp `FilamentTempType`).
pub const HIGH_TEMP: i32 = 0;
pub const LOW_TEMP: i32 = 1;
pub const HIGH_LOW_COMPATIBLE: i32 = 2;
pub const UNDEFINED_TEMP: i32 = 3;

/// The temperature kind of a filament type
/// (`Print::get_filament_temp_type`, from the reference's
/// `resources/info/filament_info.json`).
pub fn filament_temp_type(filament_type: &str) -> i32 {
    const HIGH: &[&str] = &[
        "ABS", "ASA", "PC", "PA", "PA-CF", "PA-GF", "PA6-CF", "PET-CF", "PPS", "PPS-CF", "PPA-CF",
        "PPA-GF", "ABS-GF", "ASA-Aero",
    ];
    const LOW: &[&str] = &["PLA", "TPU", "PLA-CF", "PLA-AERO", "PVA", "BVOH", "SBS"];
    const BOTH: &[&str] = &[
        "HIPS", "PETG", "PE", "PP", "EVA", "PE-CF", "PP-CF", "PP-GF", "PHA",
    ];
    if BOTH.contains(&filament_type) {
        HIGH_LOW_COMPATIBLE
    } else if HIGH.contains(&filament_type) {
        HIGH_TEMP
    } else if LOW.contains(&filament_type) {
        LOW_TEMP
    } else {
        UNDEFINED_TEMP
    }
}

/// The kind of several filaments printed together
/// (`Print::get_compatible_filament_type`).
pub fn compatible_filament_type(kinds: impl IntoIterator<Item = i32>) -> i32 {
    let (mut high, mut low) = (false, false);
    for kind in kinds {
        high |= kind == HIGH_TEMP;
        low |= kind == LOW_TEMP;
    }
    match (high, low) {
        (true, false) => HIGH_TEMP,
        (false, true) => LOW_TEMP,
        _ => HIGH_LOW_COMPATIBLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cube's facets, `z0` to `z0 + 10`.
    fn cube(z0: f64) -> Vec<[[f64; 3]; 3]> {
        let v = |x: f64, y: f64, z: f64| [x, y, z0 + z];
        let (a, b, c, d) = (
            v(0., 0., 0.),
            v(10., 0., 0.),
            v(10., 10., 0.),
            v(0., 10., 0.),
        );
        let (e, f, g, h) = (
            v(0., 0., 10.),
            v(10., 0., 10.),
            v(10., 10., 10.),
            v(0., 10., 10.),
        );
        vec![
            [a, c, b],
            [a, d, c],
            [e, f, g],
            [e, g, h],
            [a, b, f],
            [a, f, e],
            [b, c, g],
            [b, g, f],
            [c, d, h],
            [c, h, g],
            [d, a, e],
            [d, e, h],
        ]
    }

    #[test]
    fn a_cube_on_the_bed_needs_no_support() {
        assert!(!may_get_support(
            cube(0.0),
            SupportKind::NormalAuto,
            0.0,
            false
        ));
    }

    #[test]
    fn an_overhang_needs_support() {
        // A cube with a slab hanging from its side.
        let mut facets = cube(0.0);
        facets.extend(
            cube(5.0)
                .into_iter()
                .map(|f| f.map(|p| [p[0] + 10.0, p[1], p[2]])),
        );
        assert!(may_get_support(facets, SupportKind::TreeAuto, 30.0, false));
        assert_eq!(support_allowance(Some(SupportKind::TreeAuto)), (24.0, true));
        assert_eq!(
            support_allowance(Some(SupportKind::NormalAuto)),
            (6.0, false)
        );
        assert_eq!(support_allowance(None), (1.0, false));
    }

    #[test]
    fn filaments_are_kinds_of_heat() {
        assert_eq!(filament_temp_type("PLA"), LOW_TEMP);
        assert_eq!(filament_temp_type("ABS"), HIGH_TEMP);
        assert_eq!(
            compatible_filament_type([LOW_TEMP, HIGH_TEMP]),
            HIGH_LOW_COMPATIBLE
        );
        assert_eq!(
            compatible_filament_type([UNDEFINED_TEMP]),
            HIGH_LOW_COMPATIBLE
        );
    }

    #[test]
    fn the_outline_is_the_hull_on_the_ground() {
        let hull = outline(cube(3.0).into_iter().flatten());
        assert_eq!(hull.points.len(), 4);
    }
}
