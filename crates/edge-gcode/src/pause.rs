//! The minimum layer time pause (`min_layer_time_pause`): a layer the
//! cooling slowdown cannot stretch to that many seconds, being too small
//! to slow enough above `slow_down_min_speed`, ends with the head pulled
//! back, lifted and parked off the print while the rest of the time runs
//! out, and brought back.
//!
//! Ported from the reference's GCode/MinLayerTimePause.cpp (where the head
//! parks and how long it waits), GCodeWriter.cpp `park_and_return` (the
//! moves) and GCode.cpp `min_layer_time_pause_block` (when): the block is
//! laid out with the layer and kept, with its wait, or dropped once the
//! layer's time after its slowdown is known, as the reference's
//! CoolingBuffer keeps or drops it.

/// What the head parks clear of, in bed coordinates (mm), each a box as
/// min x, min y, max x, max y.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sites {
    /// The box round the printable area, and the area's corners where it
    /// is a polygon of three or more.
    pub bed: Option<[f64; 4]>,
    pub bed_outline: Vec<[f64; 2]>,
    /// Every copy's footprint.
    pub objects: Vec<[f64; 4]>,
    /// The prime tower's, brim included.
    pub tower: Option<[f64; 4]>,
    /// The box round `bed_exclude_area`.
    pub exclusions: Vec<[f64; 4]>,
}

/// Where the head waits (`min_layer_time_pause_park`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Park {
    /// The nearest free bed edge or corner clear of the print.
    Away,
    /// Beside the prime tower, or away without one.
    PrimeTower,
    /// At this point of the bed (`min_layer_time_pause_park_x`, `_y`).
    Custom([f64; 2]),
}

/// The reference's `EPSILON` (libslic3r.h).
const EPSILON: f64 = 1e-4;
/// Clearance kept from objects, the prime tower and excluded areas, and
/// from the bed's edge.
const OBJECT_MARGIN: f64 = 10.0;
const EDGE_MARGIN: f64 = 5.0;
/// A shortfall below this many seconds is not worth stopping for.
const MIN_PAUSE_S: f32 = 0.5;

/// The markers the block is laid out between until it is kept or
/// dropped; none reaches the file.
pub(crate) const START: &[u8] = b";_MIN_LAYER_TIME_PAUSE_START ";
pub(crate) const DWELL: &[u8] = b";_MIN_LAYER_TIME_PAUSE_DWELL\n";
pub(crate) const END: &[u8] = b";_MIN_LAYER_TIME_PAUSE_END\n";

fn inflated(b: [f64; 4], by: f64) -> [f64; 4] {
    [b[0] - by, b[1] - by, b[2] + by, b[3] + by]
}

fn contains(b: [f64; 4], p: [f64; 2]) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

fn centre(b: [f64; 4]) -> [f64; 2] {
    [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]
}

fn merged(a: Option<[f64; 4]>, b: [f64; 4]) -> [f64; 4] {
    match a {
        None => b,
        Some(a) => [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ],
    }
}

/// The four corners and the four middles of the sides of `b`.
fn ring(b: [f64; 4]) -> Vec<[f64; 2]> {
    let c = centre(b);
    vec![
        [b[0], b[1]],
        [b[2], b[1]],
        [b[2], b[3]],
        [b[0], b[3]],
        [c[0], b[1]],
        [b[2], c[1]],
        [c[0], b[3]],
        [b[0], c[1]],
    ]
}

/// Whether `p` is inside the polygon through `points` (crossing count).
fn inside(points: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut odd = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let (a, b) = (points[i], points[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            odd = !odd;
        }
        j = i;
    }
    odd
}

impl Sites {
    fn on_bed(&self, p: [f64; 2]) -> bool {
        let Some(bed) = self.bed else {
            return false;
        };
        contains(inflated(bed, -EDGE_MARGIN), p)
            && (self.bed_outline.len() < 3 || inside(&self.bed_outline, p))
    }

    /// Usable for a park: on the bed, clear of the excluded areas, the
    /// objects and the tower. (The reference also asks whether the tool
    /// in use reaches it, on printers whose tools each reach part of the
    /// bed, `extruder_printable_area`; on every printer this engine
    /// writes for, each reaches all of it.)
    fn free(&self, p: [f64; 2]) -> bool {
        self.on_bed(p)
            && !(self.exclusions.iter()).any(|b| contains(inflated(*b, OBJECT_MARGIN * 0.5), p))
            && !(self.objects.iter()).any(|b| contains(inflated(*b, OBJECT_MARGIN - EPSILON), p))
            && !self
                .tower
                .is_some_and(|b| contains(inflated(b, OBJECT_MARGIN - EPSILON), p))
    }

    /// The free one of `candidates` nearest `from`, the first of any
    /// equally near.
    fn nearest_free(&self, candidates: &[[f64; 2]], from: [f64; 2]) -> Option<[f64; 2]> {
        let mut best = None;
        let mut best_d2 = f64::MAX;
        for &p in candidates {
            if self.free(p) {
                let d2 = (p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2);
                if d2 < best_d2 - EPSILON {
                    best_d2 = d2;
                    best = Some(p);
                }
            }
        }
        best
    }

    /// The bed's edge nearest the print, or failing that a strip just
    /// beside the print.
    fn away(&self, from: [f64; 2]) -> Option<[f64; 2]> {
        let inner = inflated(self.bed?, -EDGE_MARGIN);
        let mut print: Option<[f64; 4]> = None;
        for b in &self.objects {
            print = Some(merged(print, *b));
        }
        if let Some(tower) = self.tower {
            print = Some(merged(print, tower));
        }
        let mut candidates = ring(inner);
        let Some(print) = print else {
            return self.nearest_free(&candidates, from);
        };
        let c = centre(print);
        let around = inflated(print, OBJECT_MARGIN);
        candidates.extend([
            [c[0], inner[1]],
            [c[0], inner[3]],
            [inner[0], c[1]],
            [inner[2], c[1]],
        ]);
        let beside = [
            [c[0], around[1]],
            [c[0], around[3]],
            [around[0], c[1]],
            [around[2], c[1]],
        ];
        self.nearest_free(&candidates, from)
            .or_else(|| self.nearest_free(&beside, from))
    }

    /// Where to park, from `from`. `current` is the footprint of the copy
    /// being printed when printing object by object: the head then stays
    /// beside it, as the copies finished may stand taller than the head
    /// (`park_point`). None where no free spot is found.
    pub fn park_point(
        &self,
        park: Park,
        from: [f64; 2],
        current: Option<[f64; 4]>,
    ) -> Option<[f64; 2]> {
        if let Park::Custom(at) = park {
            return Some(match self.bed {
                None => at,
                Some(bed) => [at[0].clamp(bed[0], bed[2]), at[1].clamp(bed[1], bed[3])],
            });
        }
        if let Some(current) = current {
            return self.nearest_free(&ring(inflated(current, OBJECT_MARGIN)), from);
        }
        if park == Park::PrimeTower
            && let Some(tower) = self.tower
            && let Some(p) = self.nearest_free(&ring(inflated(tower, OBJECT_MARGIN)), from)
        {
            return Some(p);
        }
        self.away(from)
    }
}

/// The target as the block carries it: written to three places and read
/// back in single precision, as the reference's CoolingBuffer has it.
pub(crate) fn target_as_read(target: f64) -> f32 {
    format!("{target:.3}").parse().unwrap_or(0.0)
}

/// What stands for the wait when the layer took `layer_time` of `target`
/// seconds: the comment and the wait for the shortfall, or none when it
/// is not worth stopping for (`dwell_gcode`).
pub(crate) fn dwell(target: f32, layer_time: f32) -> Option<String> {
    let missing = target - layer_time.max(0.0);
    if !(missing >= MIN_PAUSE_S) {
        return None;
    }
    let ms = (f64::from(missing) * 1000.0).round() as i64;
    Some(format!(
        ";MIN_LAYER_TIME_PAUSE {:.2}s\nG4 P{ms}\n",
        ms as f64 / 1000.0
    ))
}

/// Keeps or drops the block in `out` (from `start` on) for a layer that
/// took `layer_time` seconds: kept, its wait put in and the feedrate given
/// back after it; dropped, with nothing of it left. Returns whether it was
/// kept.
pub(crate) fn settle(out: &mut Vec<u8>, start: usize, layer_time: f64) -> bool {
    let find = |from: usize, what: &[u8], out: &[u8]| {
        out[from..]
            .windows(what.len())
            .position(|window| window == what)
            .map(|at| at + from)
    };
    let Some(at) = find(start, START, out) else {
        return false;
    };
    let (Some(dwell_at), Some(end_at)) = (find(at, DWELL, out), find(at, END, out)) else {
        return false;
    };
    let head_end = find(at, b"\n", out).map_or(at, |end| end + 1);
    let header = String::from_utf8_lossy(&out[at + START.len()..head_end]).into_owned();
    // "<target> <feedrate>"
    let mut words = header.split_whitespace();
    let target: f32 = words.next().and_then(|w| w.parse().ok()).unwrap_or(0.0);
    let feedrate = words.next().unwrap_or("").to_owned();
    match dwell(target, layer_time as f32) {
        Some(wait) => {
            let mut kept = Vec::with_capacity(end_at - head_end + wait.len() + 16);
            kept.extend_from_slice(&out[head_end..dwell_at]);
            kept.extend_from_slice(wait.as_bytes());
            kept.extend_from_slice(&out[dwell_at + DWELL.len()..end_at]);
            if !feedrate.is_empty() {
                kept.extend_from_slice(format!("G1 F{feedrate}\n").as_bytes());
            }
            out.splice(at..end_at + END.len(), kept);
            true
        }
        None => {
            out.drain(at..end_at + END.len());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The U1's bed with one copy in the middle.
    fn u1(objects: Vec<[f64; 4]>, tower: Option<[f64; 4]>) -> Sites {
        Sites {
            bed: Some([0.5, 1.0, 270.5, 271.0]),
            bed_outline: vec![[0.5, 1.0], [270.5, 1.0], [270.5, 271.0], [0.5, 271.0]],
            objects,
            tower,
            exclusions: Vec::new(),
        }
    }

    #[test]
    fn the_head_parks_at_the_bed_edge_nearest_it_clear_of_the_print() {
        let sites = u1(vec![[105.0, 155.0, 165.0, 195.0]], None);
        // Off the print's right, at the bed's edge level with its middle.
        assert_eq!(
            sites.park_point(Park::Away, [220.0, 175.0], None),
            Some([265.5, 175.0])
        );
        // Off its back, above its middle.
        assert_eq!(
            sites.park_point(Park::Away, [135.0, 194.0], None),
            Some([135.0, 266.0])
        );
        // Beside the tower where asked, clear of it by 10 mm.
        let towered = u1(
            vec![[105.0, 155.0, 165.0, 195.0]],
            Some([8.0, 209.0, 74.0, 272.0]),
        );
        let at = towered
            .park_point(Park::PrimeTower, [60.0, 200.0], None)
            .unwrap();
        assert_eq!(at, [41.0, 199.0]);
        // A custom place is kept to the bed.
        assert_eq!(
            sites.park_point(Park::Custom([-20.0, 300.0]), [0.0, 0.0], None),
            Some([0.5, 271.0])
        );
        // Printing object by object, beside the copy being printed.
        let copy = [20.0, 20.0, 40.0, 40.0];
        assert_eq!(
            sites.park_point(Park::Away, [39.0, 30.0], Some(copy)),
            Some([50.0, 30.0])
        );
        // A bed with every spot taken has none.
        let full = u1(vec![[0.0, 0.0, 300.0, 300.0]], None);
        assert_eq!(full.park_point(Park::Away, [10.0, 10.0], None), None);
    }

    #[test]
    fn the_wait_is_what_the_layer_lacks_and_the_block_goes_without_it() {
        assert_eq!(
            dwell(20.0, 9.754).as_deref(),
            Some(";MIN_LAYER_TIME_PAUSE 10.25s\nG4 P10246\n")
        );
        assert_eq!(dwell(20.0, 19.6), None);
        assert_eq!(target_as_read(20.0), 20.0);
        let block = b"G1 X1 Y1 E1\n;_MIN_LAYER_TIME_PAUSE_START 20.000 9634\nG1 E-1.5 F1800\nG1 X5 Y5 F30000\n;_MIN_LAYER_TIME_PAUSE_DWELL\nG1 X1 Y1 F30000\nG1 E1.5 F1800\n;_MIN_LAYER_TIME_PAUSE_END\n;LAYER_CHANGE\n";
        let mut kept = block.to_vec();
        assert!(settle(&mut kept, 0, 9.754));
        assert_eq!(
            String::from_utf8(kept).unwrap(),
            "G1 X1 Y1 E1\nG1 E-1.5 F1800\nG1 X5 Y5 F30000\n;MIN_LAYER_TIME_PAUSE 10.25s\nG4 P10246\nG1 X1 Y1 F30000\nG1 E1.5 F1800\nG1 F9634\n;LAYER_CHANGE\n"
        );
        let mut dropped = block.to_vec();
        assert!(!settle(&mut dropped, 0, 25.0));
        assert_eq!(
            String::from_utf8(dropped).unwrap(),
            "G1 X1 Y1 E1\n;LAYER_CHANGE\n"
        );
    }
}
