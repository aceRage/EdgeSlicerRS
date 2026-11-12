//! Every OBJ the reference's own tests read (`tests/data/*.obj`), read
//! here as the C++ slicer reads it: the number of triangles and the
//! bounds its `--info` gives for each (the build of the pinned commit,
//! run with a data folder of its own and `--no-hub`).

use std::path::Path;

/// File, triangles, least and greatest corner, as the C++ slicer's
/// `--info` printed them.
const READ: [(&str, usize, [f64; 3], [f64; 3]); 21] = [
    (
        "20mm_cube.obj",
        12,
        [0.000000, 0.000000, 0.000000],
        [20.000000, 20.000000, 20.000000],
    ),
    (
        "2x20x10.obj",
        12,
        [0.000000, 0.000000, 0.000000],
        [2.000000, 20.000000, 10.000000],
    ),
    (
        "A.obj",
        1246,
        [488.632568, 47.732437, 24.001102],
        [517.188416, 81.599236, 36.000900],
    ),
    (
        "A_upsidedown.obj",
        1246,
        [46.202087, 28.020100, 0.000000],
        [74.757912, 40.019901, 33.866802],
    ),
    (
        "V.obj",
        24,
        [41.479996, 24.020000, 0.000000],
        [79.480003, 44.020000, 15.000000],
    ),
    (
        "V_standing.obj",
        24,
        [-14.000000, 0.000000, 0.000000],
        [24.000000, 15.000000, 20.000000],
    ),
    (
        "bridge.obj",
        28,
        [75.000000, 84.500000, 0.000000],
        [125.000000, 94.500000, 8.000000],
    ),
    (
        "cube_with_concave_hole.obj",
        48,
        [-10.000000, -10.000000, -5.000000],
        [10.000000, 10.000000, 5.000000],
    ),
    (
        "cube_with_concave_hole_enlarged.obj",
        48,
        [44.325836, 17.865837, 0.000000],
        [76.634163, 50.174164, 16.154167],
    ),
    (
        "cube_with_concave_hole_enlarged_standing.obj",
        48,
        [52.402916, 17.865833, 0.000000],
        [68.557083, 50.174164, 32.308331],
    ),
    (
        "cube_with_hole.obj",
        32,
        [0.000000, 0.000000, 0.000000],
        [20.000000, 20.000000, 10.000000],
    ),
    (
        "extruder_idler.obj",
        3586,
        [44.275475, 25.600410, 0.000000],
        [76.697266, 43.439590, 37.951313],
    ),
    (
        "extruder_idler_quads.obj",
        7344,
        [44.275475, 25.600410, 0.000000],
        [76.697266, 43.439590, 37.951313],
    ),
    (
        "frog_legs.obj",
        18334,
        [34.436432, 9.138266, 0.000000],
        [86.482773, 58.860577, 2.075428],
    ),
    (
        "ipadstand.obj",
        56,
        [0.000000, 0.000000, 0.000000],
        [98.207970, 10.000000, 30.000000],
    ),
    (
        "overhang.obj",
        118,
        [1346.560059, 564.398987, 17.001499],
        [1389.685059, 614.398010, 23.300499],
    ),
    (
        "pyramid.obj",
        6,
        [0.000000, 0.000000, 0.000000],
        [20.000000, 20.000000, 40.000000],
    ),
    (
        "simplification.obj",
        28,
        [39.349007, -54.069000, -199.878998],
        [39.937004, -53.723007, -199.723022],
    ),
    (
        "sloping_hole.obj",
        136,
        [-20.000000, -20.000000, -5.000000],
        [20.000000, 20.000000, 5.000000],
    ),
    (
        "small_dorito.obj",
        8,
        [-9.999578, -49.999874, 0.000000],
        [22.001024, -22.998207, 28.001925],
    ),
    (
        "two_hollow_squares.obj",
        96,
        [65.666687, 94.666672, 0.000000],
        [134.333313, 105.333344, 2.800000],
    ),
];

#[test]
fn every_obj_of_the_references_tests_reads_as_the_reference_reads_it() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/EdgeSlicer/tests/data");
    for (name, triangles, low, high) in READ {
        let mesh =
            edge_mesh::obj::load(data.join(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(mesh.triangles.len(), triangles, "{name}");
        let (min, max) = mesh.bounds().unwrap();
        for axis in 0..3 {
            assert!(
                (f64::from(min[axis]) - low[axis]).abs() < 1e-4,
                "{name}: {min:?} {low:?}"
            );
            assert!(
                (f64::from(max[axis]) - high[axis]).abs() < 1e-4,
                "{name}: {max:?} {high:?}"
            );
        }
    }
}
