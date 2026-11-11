//! GCode/WipePathHelpers.cpp's own tests (tests/libslic3r/test_wipe_path.cpp),
//! case for case and under their names, so the port answers as the
//! reference does on the walls its tests were written from.

use edge_geom::Point2;

use super::*;

const S: i64 = 1_000_000;

fn p(x: i64, y: i64) -> Point2 {
    Point2 { x, y }
}

/// Point.hpp `Point::new_scale`: millimetres to units, cut, not rounded.
fn mm(x: f64, y: f64) -> Point2 {
    Point2 {
        x: (x / 0.000_001) as i64,
        y: (y / 0.000_001) as i64,
    }
}

fn scale(mm: f64) -> f64 {
    mm / 0.000_001
}

fn units(points: &[(i64, i64)], mirror: i64) -> Vec<Point2> {
    points.iter().map(|&(x, y)| p(mirror * x, y)).collect()
}

fn mms(points: &[(f64, f64)]) -> Vec<Point2> {
    points.iter().map(|&(x, y)| mm(x, y)).collect()
}

fn line(a: Point2, b: Point2) -> Line {
    Line { a, b }
}

/// A stored path's length once the nozzle's place is put for its first
/// point, as `Wipe::wipe` does.
fn run(path: &[Point2], from: Point2) -> f64 {
    let mut path = path.to_vec();
    path[0] = from;
    length(&path)
}

fn joined(a: &[Line], b: &[Line]) -> Vec<Line> {
    a.iter().chain(b).copied().collect()
}

#[test]
fn stored_wipe_path_retains_its_length_around_a_curved_wall_after_a_seam_gap() {
    for mirror in [1, -1] {
        for wipe_length in [0.8, 1.0] {
            let original = units(
                &[
                    (671861, 7772276),
                    (688586, 7765098),
                    (781082, 7687014),
                    (852059, 7608861),
                    (889773, 7556912),
                    (958919, 7382963),
                    (977048, 7259018),
                    (977325, 7173839),
                    (944250, 7039370),
                    (911230, 6952087),
                    (880323, 6894944),
                    (760243, 6763860),
                    (598587, 6641719),
                    (492626, 6593610),
                    (362533, 6543938),
                    (170173, 6513482),
                    (114917, 6509380),
                    (18418, 6513666),
                    (-145550, 6537251),
                    (-259087, 6580797),
                    (-413987, 6690495),
                    (-485220, 6767022),
                    (-561189, 6893573),
                    (-576897, 6965717),
                    (-595201, 7089303),
                    (-597977, 7164172),
                    (-590614, 7239553),
                    (-574031, 7305533),
                    (-539668, 7383293),
                    (-442552, 7550465),
                    (-332173, 7659644),
                    (-257819, 7717153),
                    (-209522, 7749563),
                    (-121695, 7793438),
                    (399, 7844160),
                    (228137, 7880068),
                    (363721, 7881585),
                    (431909, 7865985),
                    (569648, 7816149),
                    (653482, 7780164),
                ],
                mirror,
            );
            let inner = units(
                &[
                    (739251, 7241332),
                    (739377, 7202281),
                    (716708, 7110113),
                    (694416, 7051190),
                    (685088, 7033943),
                    (599527, 6940541),
                    (476243, 6847393),
                    (400952, 6813209),
                    (300851, 6774988),
                    (142724, 6749952),
                    (111379, 6747625),
                    (40681, 6750765),
                    (-85281, 6768883),
                    (-146010, 6792175),
                    (-256555, 6870462),
                    (-295251, 6912034),
                    (-336173, 6978125),
                    (-357996, 7111200),
                    (-359693, 7156985),
                    (-355612, 7198777),
                    (-348288, 7227916),
                    (-327412, 7275156),
                    (-252784, 7403618),
                    (-175201, 7480358),
                    (-89616, 7543582),
                    (-22802, 7576960),
                    (65459, 7613627),
                    (248096, 7642423),
                    (338175, 7643431),
                    (364673, 7637369),
                    (482204, 7594844),
                    (562221, 7560499),
                    (615604, 7515433),
                    (679764, 7441323),
                    (727601, 7320981),
                    (739251, 7241332),
                ],
                mirror,
            );
            let (seam_start, seam_end) = (original[0], original[original.len() - 1]);
            let offset = scale(0.239999);
            assert!(
                offset_wipe_path(
                    &original,
                    seam_start,
                    seam_end,
                    seam_end,
                    -mirror as f64,
                    offset,
                    scale(wipe_length)
                )
                .is_none()
            );
            let inner = lines_of(&inner);
            let current = lines_of(&original);
            let path = offset_wipe_path_toward_support(
                &original,
                seam_start,
                seam_end,
                seam_end,
                -mirror as f64,
                offset,
                scale(wipe_length),
                &inner,
                &inner,
                &current,
                offset,
            )
            .unwrap();
            assert_eq!(path[0], seam_start);
            assert!(path.len() > 2);
            assert!((run(&path, seam_end) / 1e6 - wipe_length).abs() < 0.0004);
            assert!(mirror * (path[1].x - seam_end.x) < 0);
            assert!(path[1].y < seam_end.y);
            assert!(
                wipe_path_support_score(&path, seam_end, &inner, &joined(&inner, &current), offset)
                    .is_some()
            );
        }
    }
}

#[test]
fn stored_wipe_path_retains_its_length_after_a_loop_pre_move_at_a_curved_seam() {
    for mirror in [1, -1] {
        for pre_move in [false, true] {
            let original = units(
                &[
                    (686772, 7813199),
                    (516334, 7887188),
                    (411017, 7915098),
                    (346190, 7926015),
                    (246957, 7925330),
                    (-16945, 7881611),
                    (-116443, 7842513),
                    (-255907, 7773886),
                    (-378126, 7681053),
                    (-499258, 7552879),
                    (-574414, 7438250),
                    (-613558, 7370259),
                    (-626313, 7341394),
                    (-650774, 7263424),
                    (-669964, 7169919),
                    (-666798, 7058662),
                    (-631336, 6876547),
                    (-624410, 6852946),
                    (-577832, 6762704),
                    (-517493, 6693143),
                    (-455794, 6631765),
                    (-315549, 6531304),
                    (-169627, 6468424),
                    (-1908, 6443726),
                    (143562, 6438395),
                    (314277, 6465470),
                    (380448, 6480795),
                    (519922, 6526617),
                    (673990, 6611110),
                    (801581, 6705610),
                    (927505, 6840928),
                    (969616, 6912718),
                    (1004269, 7009155),
                    (1044144, 7171737),
                    (1046617, 7228598),
                    (1028598, 7358360),
                    (950280, 7560914),
                    (867133, 7675444),
                    (732946, 7788889),
                    (706168, 7804780),
                    (705118, 7805235),
                ],
                mirror,
            );
            let inner = units(
                &[
                    (808905, 7211140),
                    (796801, 7298317),
                    (739603, 7446245),
                    (691575, 7512401),
                    (595745, 7593416),
                    (438069, 7661865),
                    (360694, 7682370),
                    (327119, 7688024),
                    (266586, 7687607),
                    (53302, 7653657),
                    (-20276, 7624745),
                    (-130307, 7570601),
                    (-218687, 7503470),
                    (-311907, 7404832),
                    (-371722, 7313599),
                    (-401113, 7262547),
                    (-420207, 7203763),
                    (-431422, 7149118),
                    (-429596, 7084952),
                    (-401187, 6939055),
                    (-379519, 6897073),
                    (-343546, 6855603),
                    (-301665, 6813940),
                    (-197879, 6739595),
                    (-104131, 6699197),
                    (19838, 6680942),
                    (129154, 6676936),
                    (268758, 6699077),
                    (316366, 6710103),
                    (424812, 6745731),
                    (545430, 6811879),
                    (642396, 6883697),
                    (735572, 6983824),
                    (753259, 7013976),
                    (776223, 7077887),
                    (808905, 7211140),
                ],
                mirror,
            );
            let (seam_start, seam_end) = (original[0], original[original.len() - 1]);
            let wipe_start = if pre_move {
                p(mirror * 652751, 7792162)
            } else {
                seam_end
            };
            let offset = scale(0.239999);
            let inner = lines_of(&inner);
            let current = lines_of(&original);
            let path = offset_wipe_path_toward_support(
                &original,
                seam_start,
                seam_end,
                wipe_start,
                mirror as f64,
                offset,
                scale(0.8),
                &inner,
                &inner,
                &current,
                offset,
            )
            .unwrap();
            assert_eq!(path[0], seam_start);
            assert!((run(&path, wipe_start) / 1e6 - 0.8).abs() < 0.0004);
            assert!(
                wipe_path_support_score(
                    &path,
                    wipe_start,
                    &inner,
                    &joined(&inner, &current),
                    offset
                )
                .is_some()
            );
        }
    }
}

#[test]
fn wipe_offset_direction_follows_the_material_side() {
    assert_eq!(offset_direction(true, false), 1.0);
    assert_eq!(offset_direction(false, false), -1.0);
    assert_eq!(offset_direction(true, true), -1.0);
    assert_eq!(offset_direction(false, true), 1.0);
}

#[test]
fn stored_wipe_path_leaves_source_crossings_to_support_validation() {
    let path = [p(10 * S, 0), p(100 * S, 0), p(13_400_000, 50 * S)];
    // "crossing the just-printed wall is harmless for a non-extruding wipe."
    assert!(offset_wipe_path(&path, p(10 * S, 0), p(0, 0), p(0, 0), 1.0, 5e6, 1000e6).is_some());
}

#[test]
fn stored_wipe_path_builds_the_join_after_a_nonzero_seam_gap() {
    let path = [p(10 * S, 0), p(10 * S, 0), p(10 * S, 100 * S)];
    let path = offset_wipe_path(&path, p(10 * S, 0), p(0, 0), p(0, 0), 1.0, 5e6, 1000e6).unwrap();
    assert_eq!(path.len(), 3);
    assert_eq!(path[1], p(5 * S, 5 * S));
    assert_eq!(path[2], p(5 * S, 100 * S));
}

#[test]
fn stored_wipe_path_rejects_an_offset_seam_join_that_turns_backward() {
    let (seam_start, seam_end) = (p(S, S), p(0, 0));
    let path = [seam_start, p(S, -10 * S), p(S, -20 * S)];
    assert!(offset_wipe_path(&path, seam_start, seam_end, seam_end, 1.0, 1e6, 5e6).is_none());
}

#[test]
fn stored_wipe_path_continues_after_an_inward_pre_move() {
    let (seam_start, seam_end, wipe_start) = (p(S, S), p(0, 0), p(2 * S, 2 * S));
    let path = [seam_start, p(S, -10 * S), p(S, -20 * S)];
    let path = offset_wipe_path(&path, seam_start, seam_end, wipe_start, 1.0, 1e6, 5e6).unwrap();
    assert!(path.len() >= 3);
    assert!((run(&path, wipe_start) - 5e6).abs() <= 2.0);
}

#[test]
fn stored_wipe_path_does_not_retrace_a_translated_seam_gap() {
    let (seam_start, seam_end) = (p(S, 0), p(0, 0));
    let original = [seam_start, seam_end, p(-10 * S, 0)];
    let support = [line(p(-10 * S, S), p(10 * S, S))];
    // "the exact reversal at seam_start forces the translated fallback."
    let path = offset_wipe_path_toward_support(
        &original,
        seam_start,
        seam_end,
        seam_end,
        1.0,
        1e6,
        5e6,
        &support,
        &support,
        &lines_of(&original),
        1e6,
    )
    .unwrap();
    assert_eq!(path.len(), 2);
    assert!(path[1].y > seam_end.y);
}

#[test]
fn stored_wipe_path_keeps_its_first_offset_point_when_seam_gap_is_zero() {
    let path = [
        p(0, 0),
        p(100 * S, 0),
        p(100 * S, 100 * S),
        p(0, 100 * S),
        p(0, 0),
    ];
    let path = offset_wipe_path(&path, p(0, 0), p(0, 0), p(0, 0), 1.0, 5e6, 20e6).unwrap();
    assert!(path.len() >= 3);
    assert_eq!(path[0], p(0, 0));
    assert!((path[1].x - 5 * S).abs() <= 2 && (path[1].y - 5 * S).abs() <= 2);
}

#[test]
fn stored_wipe_path_ignores_unsafe_geometry_beyond_the_used_prefix() {
    let path = [
        p(0, 0),
        p(1000 * S, 0),
        p(1000 * S, 20 * S),
        p(900 * S, 20 * S),
        p(0, 20 * S),
        p(0, 0),
    ];
    let path = offset_wipe_path(&path, p(0, 0), p(0, 0), p(0, 0), 1.0, 30e6, 10e6).unwrap();
    assert_eq!(path.len(), 2);
    assert!((length(&path) - 10e6).abs() <= 2.0);
}

#[test]
fn stored_wipe_path_grows_its_source_until_the_offset_reaches_the_requested_length() {
    let path = [p(0, 0), p(100 * S, 0), p(100 * S, 100 * S), p(0, 100 * S)];
    // "two inward corners shorten this offset by more than 2 * offset_dist."
    let path = offset_wipe_path(&path, p(0, 0), p(0, 0), p(0, 0), 1.0, 10e6, 250e6).unwrap();
    assert!((length(&path) - 250e6).abs() <= 2.0);
}

#[test]
fn stored_wipe_path_is_unchanged_when_wipe_distance_is_zero() {
    let path = [p(0, 0), p(100 * S, 0)];
    assert!(offset_wipe_path(&path, p(0, 0), p(0, 0), p(0, 0), 1.0, 5e6, 0.0).is_none());
}

#[test]
fn stored_wipe_path_defers_actual_start_crossings_to_support_validation() {
    let original = [
        p(0, 0),
        p(100 * S, 0),
        p(100 * S, 100 * S),
        p(0, 100 * S),
        p(0, 0),
    ];
    let current = lines_of(&original);
    let remote = [line(p(0, 50 * S), p(100 * S, 50 * S))];
    let wipe_start = p(50 * S, -10 * S);
    let path = offset_wipe_path(&original, p(0, 0), p(0, 0), wipe_start, 1.0, 5e6, 100e6).unwrap();
    assert!(
        wipe_path_support_score(&path, wipe_start, &remote, &joined(&remote, &current), 5e6)
            .is_none()
    );
}

#[test]
fn stored_wipe_path_keeps_the_closing_join_when_its_prefix_ends_at_the_closing_vertex() {
    let path = [
        p(0, 0),
        p(0, 100 * S),
        p(100 * S, 100 * S),
        p(100 * S, 0),
        p(0, 0),
    ];
    let path = offset_wipe_path(&path, p(0, 0), p(0, 0), p(0, 0), 1.0, 5e6, 300e6).unwrap();
    assert!(path.len() >= 2);
    assert_eq!(path[1], p(-5 * S, -5 * S));
}

#[test]
fn stored_wipe_path_rejects_a_two_point_zero_gap_loop() {
    let path = [p(0, 0), p(100 * S, 0), p(0, 0)];
    assert!(offset_wipe_path(&path, p(0, 0), p(0, 0), p(0, 0), 1.0, 5e6, 100e6).is_none());
}

#[test]
fn stored_wipe_path_tolerates_quantized_contact_at_its_actual_start() {
    let q = (SCALED_EPSILON / 2.0) as i64;
    let path = [
        p(0, q),
        p(100 * S, q),
        p(100 * S, 100 * S + q),
        p(0, 100 * S + q),
        p(0, q),
    ];
    assert!(offset_wipe_path(&path, p(0, q), p(0, q), p(0, 0), 1.0, 5e6, 20e6).is_some());
}

#[test]
fn stored_wipe_path_requires_nearby_generated_perimeter_geometry() {
    let path = [p(0, 0), p(0, 2 * S), p(10 * S, 2 * S)];
    let adjacent = [line(p(0, 4 * S), p(10 * S, 4 * S))];
    let remote = [line(p(0, 20 * S), p(10 * S, 20 * S))];
    let all_support = joined(&remote, &lines_of(&path));
    let score = wipe_path_support_score(&path, p(0, 2 * S), &adjacent, &adjacent, 3e6).unwrap();
    assert!((score - 2e6).abs() <= 2.0);
    assert!(wipe_path_support_score(&path, p(0, 2 * S), &adjacent, &adjacent, 0.0).is_none());
    assert!(wipe_path_support_score(&path, p(0, 2 * S), &remote, &remote, 3e6).is_none());
    assert!(wipe_path_support_score(&path, p(0, 2 * S), &remote, &all_support, 3e6).is_none());
    assert!(wipe_path_support_score(&path, p(0, 2 * S), &[], &all_support, 3e6).is_none());
}

#[test]
fn stored_wipe_path_checks_the_first_segment_from_its_actual_start() {
    let path = [p(0, 0), p(10 * S, 0)];
    let support = [line(p(0, -S), p(0, S)), line(p(10 * S, -S), p(10 * S, S))];
    // "both endpoints are supported, but the middle of the executable
    // segment from wipe_start is not."
    assert!(wipe_path_support_score(&path, p(0, 0), &support, &support, 2e6).is_none());
}

#[test]
fn stored_wipe_path_rejects_unsupported_gaps_between_nearby_samples() {
    for support_y in [0.8, 0.95] {
        let (start, end) = (mm(0.0, 0.0), mm(0.8, 0.0));
        let support = [
            line(mm(0.0, support_y), mm(0.0, 2.0)),
            line(mm(0.8, support_y), mm(0.8, 2.0)),
        ];
        let supported =
            wipe_path_support_score(&[start, end], start, &support, &support, scale(1.0)).is_some();
        assert_eq!(supported, support_y < 0.9);
    }
}

#[test]
fn stored_wipe_path_checks_support_at_the_actual_nozzle_position() {
    let end = mm(0.0, 0.0);
    let support = [line(mm(-1.0, 0.0), mm(1.0, 0.0))];
    assert!(
        wipe_path_support_score(&[end, end], mm(0.0, -2.0), &support, &support, scale(1.0))
            .is_none()
    );
}

#[test]
fn direct_inward_fallback_respects_a_short_wipe_distance_before_validation() {
    for pre_move in [false, true] {
        let seam = mm(0.0, 0.0);
        let original = [seam, mm(10.0, 0.0), mm(10.0, 10.0), mm(0.0, 10.0), seam];
        let support = [line(mm(0.4, 0.4), mm(9.6, 0.4))];
        let wipe_start = if pre_move { mm(0.02, 0.02) } else { seam };
        let wipe_length = scale(0.05);
        let path = offset_wipe_path_toward_support(
            &original,
            seam,
            seam,
            wipe_start,
            1.0,
            scale(0.2),
            wipe_length,
            &support,
            &support,
            &lines_of(&original),
            scale(0.4),
        )
        .unwrap();
        assert_eq!(path.len(), 2);
        assert!((run(&path, wipe_start) - wipe_length).abs() <= 2.0);
        assert!(path[1].x > wipe_start.x && path[1].y > wipe_start.y);
    }
}

#[test]
fn stored_wipe_path_uses_a_stable_zero_gap_join_for_nearly_parallel_segments() {
    let original = mms(&[
        (58.777, 61.985),
        (58.822, 61.918),
        (58.900, 61.789),
        (58.980, 61.641),
        (59.054, 61.480),
        (59.260, 60.980),
        (58.412, 62.485),
        (58.631, 62.202),
        (58.777, 61.985),
    ]);
    let seam = original[0];
    let path =
        offset_wipe_path(&original, seam, seam, seam, -1.0, scale(0.23), scale(0.8)).unwrap();
    assert!(path.len() >= 3);
    assert!(V::between(seam, path[1]).dot(V::between(path[1], path[2])) >= 0.0);
}

#[test]
fn stored_wipe_path_follows_the_inner_wall_at_a_narrow_external_cusp() {
    let original = mms(&[
        (55.139, 60.077),
        (55.156, 60.010),
        (55.205, 59.961),
        (55.237, 59.934),
        (55.304, 59.907),
        (55.392, 59.872),
        (55.630, 59.791),
        (56.564, 59.430),
        (55.061, 59.956),
        (55.108, 60.008),
        (55.139, 60.077),
    ]);
    let seam = original[0];
    let target = [line(mm(54.983, 59.648), mm(55.121, 59.745))];
    let printed = [target[0], line(mm(54.75, 60.25), mm(55.50, 60.10))];
    let path = offset_wipe_path_toward_support(
        &original,
        seam,
        seam,
        seam,
        -1.0,
        scale(0.270341),
        scale(0.8),
        &target,
        &printed,
        &lines_of(&original),
        scale(0.4),
    )
    .unwrap();
    assert!(path.len() >= 2);
    assert!((path[1].y as f64) < seam.y as f64 - scale(0.2));
    assert!(((path[1].x - seam.x).abs() as f64) < scale(0.1));
}

#[test]
fn stored_wipe_path_keeps_a_supported_zero_gap_join_that_initially_backtracks() {
    let original = mms(&[
        (56.737, 62.049),
        (56.759, 62.142),
        (56.727, 62.294),
        (56.682, 62.447),
        (56.631, 62.570),
        (56.581, 62.669),
        (56.512, 62.776),
        (54.0, 64.0),
        (50.0, 60.0),
        (54.0, 58.0),
        (56.773, 62.031),
        (56.737, 62.049),
    ]);
    let seam = original[0];
    let target = lines_of(&mms(&[
        (56.546, 62.012),
        (56.534, 62.104),
        (56.506, 62.238),
        (56.467, 62.371),
        (56.424, 62.474),
        (56.382, 62.556),
    ]));
    let inward =
        offset_wipe_path(&original, seam, seam, seam, 1.0, scale(0.23), scale(0.8)).unwrap();
    assert!(inward.len() >= 3);
    assert!(V::between(seam, inward[1]).dot(V::between(inward[1], inward[2])) < 0.0);
    let path = offset_wipe_path_toward_support(
        &original,
        seam,
        seam,
        seam,
        1.0,
        scale(0.23),
        scale(0.8),
        &target,
        &target,
        &lines_of(&original),
        scale(0.4),
    )
    .unwrap();
    assert!((path[1].x as f64) < seam.x as f64 - scale(0.1));
}

#[test]
fn stored_wipe_path_leaves_a_narrow_cusp_directly_after_a_seam_gap() {
    let original = mms(&[
        (55.139, 60.077),
        (55.107, 60.008),
        (55.061, 59.956),
        (55.027, 59.943),
        (54.982, 59.924),
        (54.922, 59.879),
        (54.868, 59.845),
        (54.754, 59.783),
        (54.391, 59.635),
        (54.053, 59.471),
    ]);
    let (seam_start, seam_end) = (original[0], mm(55.141, 60.067));
    let target = lines_of(&mms(&[
        (55.132, 59.744),
        (55.121, 59.745),
        (54.983, 59.648),
        (54.938, 59.623),
        (54.866, 59.584),
        (54.483, 59.427),
        (54.157, 59.268),
    ]));
    let path = offset_wipe_path_toward_support(
        &original,
        seam_start,
        seam_end,
        seam_end,
        -1.0,
        scale(0.270341),
        scale(0.8),
        &target,
        &target,
        &lines_of(&original),
        scale(0.4),
    )
    .unwrap();
    assert!(path.len() >= 2);
    assert!((path[1].y as f64) < seam_end.y as f64 - scale(0.2));
    assert!(((path[1].x - seam_end.x).abs() as f64) < scale(0.05));
    // "the inward connector must not run back through the first extruded
    // point after the gap"
    assert!(line(seam_end, path[1]).distance_to(original[1]) > scale(0.02));
}

#[test]
fn stored_wipe_path_does_not_reverse_after_an_inward_pre_move_at_a_wide_gap() {
    let (seam_end, wipe_start) = (mm(55.163, 60.002), mm(55.142, 60.037));
    let mut original = mms(&[
        (55.139, 60.077),
        (55.107, 60.008),
        (55.061, 59.956),
        (55.027, 59.943),
        (54.982, 59.924),
        (54.922, 59.879),
        (54.868, 59.845),
        (54.754, 59.783),
        (54.391, 59.635),
        (54.053, 59.471),
        (50.2, 55.0),
        (50.2, 50.0),
        (60.8, 50.0),
        (60.8, 55.0),
        (56.564, 59.430),
        (55.824, 59.708),
        (55.392, 59.872),
        (55.237, 59.934),
        (55.205, 59.961),
    ]);
    original.push(seam_end);
    let seam_start = original[0];
    let target = lines_of(&mms(&[
        (55.132, 59.744),
        (55.121, 59.745),
        (54.983, 59.648),
        (54.866, 59.584),
        (54.483, 59.427),
        (54.157, 59.268),
    ]));
    let path = offset_wipe_path_toward_support(
        &original,
        seam_start,
        seam_end,
        wipe_start,
        -1.0,
        scale(0.270341),
        scale(0.8),
        &target,
        &target,
        &lines_of(&original),
        scale(0.4),
    )
    .unwrap();
    assert!(path.len() >= 3);
    assert!(V::between(wipe_start, path[1]).dot(V::between(path[1], path[2])) >= 0.0);
    assert!((run(&path, wipe_start) - scale(0.8)).abs() <= 2.0);
}

#[test]
fn stored_wipe_path_follows_the_incoming_wall_when_a_corner_gap_truncates_the_forward_path() {
    let (seam_start, seam_end) = (mm(46.047, 61.988), mm(46.118, 61.917));
    let original = [
        seam_start,
        mm(39.139, 55.080),
        mm(46.047, 48.171),
        mm(52.956, 55.080),
        seam_end,
    ];
    let target = lines_of(&mms(&[
        (46.047, 61.672),
        (39.461, 55.080),
        (46.047, 48.493),
        (52.633, 55.080),
        (46.047, 61.672),
    ]));
    let path = offset_wipe_path_toward_support(
        &original,
        seam_start,
        seam_end,
        seam_end,
        1.0,
        scale(0.23),
        scale(0.8),
        &target,
        &target,
        &lines_of(&original),
        scale(0.4),
    )
    .unwrap();
    assert!((run(&path, seam_end) - scale(0.8)).abs() <= 2.0);
    assert!(path.len() >= 3);
    assert!(path[1].x < seam_end.x && path[1].y < seam_end.y);
}

#[test]
fn stored_wipe_path_prefers_support_on_the_material_side_of_a_seam_gap() {
    let (seam_start, seam_end) = (p(S, 0), p(0, 0));
    let original = [seam_start, p(S, 10 * S), p(S, 20 * S)];
    let target = [
        line(p(0, S), p(0, 3 * S)),
        line(p(S / 2, -S / 10), p(3 * S / 2, -S / 10)),
    ];
    // "the lower line is closest at the cusp and the preferred winding
    // points toward it, but the outgoing wall is adjacent to the upper line."
    let path = offset_wipe_path_toward_support(
        &original,
        seam_start,
        seam_end,
        seam_end,
        -1.0,
        1e6,
        5e6,
        &target,
        &target,
        &lines_of(&original),
        2e6,
    )
    .unwrap();
    assert!(path[1].y > seam_end.y);
}

#[test]
fn stored_wipe_path_rejects_an_outward_offset_at_a_reflex_seam_gap() {
    for offset in [0.2, 0.4] {
        for mirror in [1.0, -1.0] {
            let point = |x: f64, y: f64| mm(mirror * x, y);
            let seam_start = point(0.0, 0.0);
            let gap = 0.04 / 2f64.sqrt();
            let seam_end = point(-gap, -gap);
            let original = [seam_start, point(0.0, -10.0)];
            let support = [line(point(0.4, -10.0), point(0.4, 1.0))];
            let current = lines_of(&original);
            // "The inward miter backtracks. The opposite offset can still
            // be supported by the outer bead, so support alone must not
            // make it an inward candidate."
            let outward = offset_wipe_path(
                &original,
                seam_start,
                seam_end,
                seam_end,
                -mirror,
                scale(offset),
                scale(2.0),
            )
            .unwrap();
            assert!(
                wipe_path_support_score(
                    &outward,
                    seam_end,
                    &support,
                    &joined(&support, &current),
                    scale(0.4)
                )
                .is_some()
            );
            assert!(mirror * (outward[1].x as f64) < 0.0);
            if let Some(path) = offset_wipe_path_toward_support(
                &original,
                seam_start,
                seam_end,
                seam_end,
                mirror,
                scale(offset),
                scale(2.0),
                &support,
                &support,
                &current,
                scale(0.4),
            ) {
                assert!(path.len() >= 2);
                assert!(mirror * (path[1].x as f64) > 0.0);
            }
            // "An inward pre-move provides a clear connector to the direct
            // fallback."
            let wipe_start = point(0.05, -0.04);
            let path = offset_wipe_path_toward_support(
                &original,
                seam_start,
                seam_end,
                wipe_start,
                mirror,
                scale(offset),
                scale(2.0),
                &support,
                &support,
                &current,
                scale(0.4),
            )
            .unwrap();
            assert_eq!(path.len(), 2);
            assert!(mirror * (path[1].x as f64) > mirror * (wipe_start.x as f64));
        }
    }
}

#[test]
fn direct_inward_wipes_respect_the_nozzle_position_and_intervening_walls() {
    for mirror in [1.0, -1.0] {
        for crossing_wall in [false, true] {
            let point = |x: f64, y: f64| mm(mirror * x, y);
            let seam_start = point(0.0, 0.0);
            let gap = 0.04 / 2f64.sqrt();
            let seam_end = point(-gap, -gap);
            let original = [seam_start, point(0.0, -10.0)];
            let support = [line(point(0.4, -10.0), point(0.4, 1.0))];
            let mut current = lines_of(&original);
            let wipe_start = point(if crossing_wall { 0.05 } else { 0.3 }, -0.04);
            if crossing_wall {
                current.push(line(point(0.1, -0.2), point(0.1, 0.2)));
            }
            assert!(
                offset_wipe_path_toward_support(
                    &original,
                    seam_start,
                    seam_end,
                    wipe_start,
                    mirror,
                    scale(0.2),
                    scale(2.0),
                    &support,
                    &support,
                    &current,
                    scale(0.4),
                )
                .is_none()
            );
        }
    }
}

#[test]
fn inward_wipe_checks_the_material_side_after_leaving_an_open_wall_endpoint() {
    for require_clearance in [false, true] {
        let seam = mm(0.0, 0.0);
        let current = [line(seam, mm(2.0, 0.0))];
        let support = [line(mm(0.0, 0.4), mm(2.0, 0.4))];
        let up = V { x: 0.0, y: 1.0 };
        let mut path = vec![seam, mm(0.1, 0.2), mm(0.5, 0.2), mm(-0.2, 0.2)];
        assert!(wipe_path_stays_on_material_side(
            &path,
            seam,
            up,
            &support,
            &current,
            scale(0.2),
            require_clearance
        ));
        // "Rounding the open endpoint keeps 0.2 mm of unsigned clearance
        // while moving to the air side."
        path.extend([mm(-0.2, -0.2), mm(0.5, -0.2)]);
        assert!(!wipe_path_stays_on_material_side(
            &path,
            seam,
            up,
            &support,
            &current,
            scale(0.2),
            require_clearance
        ));
    }
}

#[test]
fn direct_inward_fallbacks_check_the_material_side_without_requiring_clearance() {
    let seam = mm(0.0, 0.0);
    let current = [line(mm(-2.0, 0.0), mm(2.0, 0.0))];
    let support = [line(mm(-2.0, 0.4), mm(2.0, 0.4))];
    assert!(wipe_path_stays_on_material_side(
        &[seam, mm(0.0, 0.05)],
        seam,
        V { x: 0.0, y: 1.0 },
        &support,
        &current,
        scale(0.2),
        false
    ));
    assert!(!wipe_path_stays_on_material_side(
        &[seam, mm(0.0, -0.05)],
        seam,
        V { x: 0.0, y: -1.0 },
        &support,
        &current,
        scale(0.2),
        false
    ));
}

#[test]
fn stored_wipe_path_may_return_to_the_current_wall_after_reaching_an_earlier_wall() {
    let path = [p(0, 0), p(0, 2 * S), p(10 * S, 0)];
    let earlier = [line(p(0, 2 * S), p(10 * S, 2 * S))];
    let current = [line(p(0, 0), p(10 * S, 0))];
    assert!(
        wipe_path_support_score(&path, p(0, 0), &earlier, &joined(&earlier, &current), 1e6)
            .is_some()
    );
}

#[test]
fn stored_wipe_path_tolerates_compounded_coordinate_quantization() {
    let destination = p(0, 2 * S + (3.5 * SCALED_EPSILON) as i64);
    let earlier = [line(p(-S, 0), p(S, 0))];
    assert!(
        wipe_path_support_score(
            &[p(0, 0), destination],
            destination,
            &earlier,
            &earlier,
            2e6
        )
        .is_some()
    );
}

#[test]
fn stored_wipe_path_stays_on_the_inner_side_of_a_short_external_loop() {
    let original = mms(&[
        (55.270, 41.666),
        (55.241, 41.568),
        (55.210, 41.518),
        (55.195, 41.506),
        (55.173, 41.496),
        (55.141, 41.479),
        (55.126, 41.473),
        (55.068, 41.421),
        (55.055, 41.416),
        (55.006, 41.382),
        (54.808, 41.231),
        (54.687, 41.153),
        (54.590, 41.069),
        (54.529, 41.027),
        (54.441, 40.949),
        (54.299, 40.803),
        (54.219, 40.674),
        (54.183, 40.581),
        (54.172, 40.511),
        (54.182, 40.450),
        (54.225, 40.358),
        (54.256, 40.318),
        (54.341, 40.251),
        (54.418, 40.211),
        (54.499, 40.176),
        (54.675, 40.124),
        (54.797, 40.106),
        (54.978, 40.092),
        (55.245, 40.093),
        (55.443, 40.103),
        (55.591, 40.128),
        (55.771, 40.164),
        (55.962, 40.217),
        (56.103, 40.264),
        (56.167, 40.295),
        (56.246, 40.341),
        (56.338, 40.412),
        (56.382, 40.469),
        (56.396, 40.527),
        (56.386, 40.609),
        (56.313, 40.740),
        (56.208, 40.867),
        (56.071, 40.991),
        (55.946, 41.094),
        (55.812, 41.198),
        (55.722, 41.262),
        (55.665, 41.294),
        (55.556, 41.398),
        (55.520, 41.414),
        (55.495, 41.424),
        (55.478, 41.437),
        (55.442, 41.469),
        (55.407, 41.505),
        (55.386, 41.510),
        (55.367, 41.516),
        (55.335, 41.535),
        (55.292, 41.575),
        (55.270, 41.666),
    ]);
    let seam = original[0];
    let inner = mms(&[
        (55.111, 41.176),
        (54.946, 41.050),
        (54.824, 40.970),
        (54.733, 40.892),
        (54.668, 40.846),
        (54.598, 40.784),
        (54.480, 40.662),
        (54.424, 40.572),
        (54.403, 40.516),
        (54.420, 40.479),
        (54.465, 40.443),
        (54.577, 40.390),
        (54.723, 40.347),
        (54.823, 40.333),
        (54.986, 40.320),
        (55.239, 40.321),
        (55.418, 40.330),
        (55.550, 40.352),
        (55.718, 40.386),
        (55.896, 40.435),
        (56.017, 40.476),
        (56.061, 40.497),
        (56.118, 40.530),
        (56.154, 40.558),
        (56.124, 40.611),
        (56.043, 40.709),
        (55.921, 40.819),
        (55.804, 40.916),
        (55.676, 41.015),
        (55.600, 41.069),
        (55.526, 41.114),
        (55.426, 41.207),
        (55.379, 41.235),
        (55.349, 41.259),
        (55.291, 41.317),
        (55.252, 41.293),
        (55.192, 41.238),
        (55.111, 41.176),
    ]);
    let mut target = lines_of(&inner);
    // "a different contour has a slightly closer inner wall on the air
    // side of this short loop. It must not override the loop's material side."
    target.push(line(mm(55.159, 42.147), mm(55.299, 42.011)));
    let path = offset_wipe_path_toward_support(
        &original,
        seam,
        seam,
        seam,
        1.0,
        scale(0.293166),
        scale(0.8),
        &target,
        &target,
        &lines_of(&original),
        scale(0.4),
    )
    .unwrap();
    assert!(path.len() >= 2);
    assert!(path[1].y < seam.y);
}

#[test]
fn stored_wipe_path_does_not_return_to_the_external_wall_after_moving_inward() {
    let original = mms(&[
        (47.451, 54.647),
        (47.370, 54.634),
        (47.345, 54.619),
        (47.333, 54.604),
        (47.322, 54.572),
        (47.312, 54.518),
        (47.315, 54.445),
        (47.345, 54.257),
        (47.357, 54.206),
        (47.380, 54.135),
        (47.418, 54.065),
        (47.514, 53.917),
        (47.537, 53.886),
        (47.597, 53.834),
        (47.705, 53.769),
        (47.747, 53.748),
        (47.785, 53.734),
        (47.825, 53.735),
        (47.862, 53.746),
        (47.889, 53.763),
        (47.939, 53.817),
        (47.964, 53.856),
        (47.979, 53.897),
        (47.986, 53.943),
        (47.986, 54.005),
        (47.977, 54.075),
        (47.949, 54.188),
        (47.902, 54.321),
        (47.871, 54.388),
        (47.835, 54.444),
        (47.765, 54.521),
        (47.741, 54.542),
        (47.675, 54.589),
        (47.615, 54.620),
        (47.518, 54.642),
        (47.451, 54.647),
    ]);
    let seam = original[0];
    let inner = lines_of(&mms(&[
        (47.541, 54.281),
        (47.545, 54.258),
        (47.560, 54.212),
        (47.577, 54.181),
        (47.682, 54.017),
        (47.707, 53.995),
        (47.789, 53.946),
        (47.791, 53.958),
        (47.791, 53.992),
        (47.785, 54.039),
        (47.762, 54.132),
        (47.721, 54.249),
        (47.700, 54.294),
        (47.680, 54.324),
        (47.628, 54.382),
        (47.574, 54.422),
        (47.548, 54.435),
        (47.513, 54.443),
        (47.541, 54.281),
    ]));
    let offset = scale(0.229999);
    let current = lines_of(&original);
    let path = offset_wipe_path_toward_support(
        &original,
        seam,
        seam,
        seam,
        1.0,
        offset,
        scale(0.8),
        &inner,
        &inner,
        &current,
        scale(0.4),
    )
    .unwrap();
    // "after reaching the inner wall, a full-width inward wipe must not
    // collapse back onto the external perimeter at a tight turn."
    for &point in &path[1..] {
        assert!(distance_from(&current, point) >= 0.75 * offset);
    }
}
