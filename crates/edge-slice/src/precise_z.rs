//! Layer heights tuned so that a part comes out exactly as tall as it is
//! modelled (`precise_z_height`): a port of the reference's
//! `adjust_layer_series_to_align_object_height` (Slicing.cpp).
//!
//! Layers are laid from the bed up at the heights asked for, and one is
//! added for as long as its middle is still inside the part, so the last
//! of them ends up to half a layer above or below the part's top. With
//! this option the last five layers share that difference out between
//! them, each made a little thinner or thicker, within the heights the
//! nozzle can lay.

/// How close two heights have to be to count as the same, in mm
/// (libslic3r.h `EPSILON`).
const SAME: f64 = 1e-4;

/// How many layers, counted from the top, share out the difference.
pub const ADJUSTED_LAYERS: usize = 5;

/// What came of bringing a stack of layers to a part's height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aligned {
    /// The layers already end where the part does.
    Exact,
    /// The heights the last five layers are to have, the lowest first.
    Adjusted([f64; ADJUSTED_LAYERS]),
    /// There are fewer than six layers: five are adjusted, and the first
    /// layer is never one of them.
    TooFew,
    /// The five cannot take up the difference between them without one
    /// going under the least height allowed or over the greatest.
    /// Nothing is changed then.
    OutOfRange,
}

/// The heights the last five of a stack of layers should have for the
/// stack to end at `wanted` mm, where it now ends at `top` mm. `heights`
/// holds every layer's height, from the bed up; no layer is made thinner
/// than `min` or thicker than `max`.
///
/// The difference is shared out evenly. A layer that would leave the
/// range stops at its edge, and what it could not take is shared out
/// again among those that still can.
pub fn align_to_height(heights: &[f64], top: f64, wanted: f64, min: f64, max: f64) -> Aligned {
    if (top - wanted).abs() < SAME {
        return Aligned::Exact;
    }
    if heights.len() <= ADJUSTED_LAYERS {
        return Aligned::TooFew;
    }
    let mut last = [0.0; ADJUSTED_LAYERS];
    last.copy_from_slice(&heights[heights.len() - ADJUSTED_LAYERS..]);
    // The reference calls this `taller_than_object`; it is the part that
    // is the taller when it is set, and the layers that have to grow.
    let grow = top < wanted;
    let limit = if grow { max } else { min };
    let mut open = [true; ADJUSTED_LAYERS];
    let mut gap = (top - wanted).abs();
    loop {
        let able = open.iter().filter(|open| **open).count();
        if able == 0 {
            return Aligned::OutOfRange;
        }
        let share = gap / able as f64;
        let mut left = 0.0;
        for (height, open) in last.iter_mut().zip(&mut open) {
            // A layer already at the edge of the range takes no part in
            // a later round. The reference's loop falls through to its
            // last branch for such a layer and gives it a share all the
            // same, which puts the layer out of the range and the stack
            // past the height wanted, by that share for each such layer.
            // That is not copied.
            if !*open {
                continue;
            }
            if (*height - limit).abs() < SAME {
                left += share;
                *open = false;
                continue;
            }
            let changed = if grow {
                *height + share
            } else {
                *height - share
            };
            if (grow && changed > max) || (!grow && changed < min) {
                left += (changed - limit).abs();
                *height = limit;
                *open = false;
            } else {
                *height = changed;
            }
        }
        gap = left;
        if gap < SAME {
            return Aligned::Adjusted(last);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(first: f64, rest: f64, count: usize) -> (Vec<f64>, f64) {
        let mut heights = vec![rest; count];
        heights[0] = first;
        (heights, first + rest * (count - 1) as f64)
    }

    fn adjusted(result: Aligned) -> [f64; 5] {
        match result {
            Aligned::Adjusted(last) => last,
            other => panic!("not adjusted: {other:?}"),
        }
    }

    #[test]
    fn a_stack_too_tall_is_thinned_evenly_over_its_last_five_layers() {
        // A 20 mm cube at 0.2 mm on a 0.25 mm first layer: 100 layers,
        // ending at 20.05 mm.
        let (heights, top) = stack(0.25, 0.2, 100);
        let last = adjusted(align_to_height(&heights, top, 20.0, 0.08, 0.32));
        for height in last {
            assert!((height - 0.19).abs() < 1e-9, "{height}");
        }
        let ends = top - 5.0 * 0.2 + last.iter().sum::<f64>();
        assert!((ends - 20.0).abs() < 1e-9, "{ends}");
    }

    #[test]
    fn a_stack_too_short_is_thickened() {
        // 20.14 mm: the hundredth layer's middle, at 19.95, is the last
        // inside, and the stack ends 0.09 mm short.
        let (heights, top) = stack(0.25, 0.2, 100);
        let last = adjusted(align_to_height(&heights, top, 20.14, 0.08, 0.32));
        for height in last {
            assert!((height - 0.218).abs() < 1e-9, "{height}");
        }
    }

    #[test]
    fn a_stack_that_ends_where_the_part_does_is_left_alone() {
        let (heights, top) = stack(0.2, 0.2, 50);
        assert_eq!(
            align_to_height(&heights, top, 10.0, 0.08, 0.32),
            Aligned::Exact
        );
        // Within a tenth of a micron counts as there.
        assert_eq!(
            align_to_height(&heights, top, 10.00005, 0.08, 0.32),
            Aligned::Exact
        );
    }

    #[test]
    fn five_layers_and_the_first_are_needed() {
        let (heights, top) = stack(0.25, 0.2, 5);
        assert_eq!(
            align_to_height(&heights, top, 1.0, 0.08, 0.32),
            Aligned::TooFew
        );
        let (heights, top) = stack(0.25, 0.2, 6);
        let last = adjusted(align_to_height(&heights, top, 1.2, 0.08, 0.32));
        assert!((last.iter().sum::<f64>() - 0.95).abs() < 1e-9);
    }

    #[test]
    fn nothing_is_changed_when_the_range_of_heights_is_too_narrow() {
        // Layers already as thick as the nozzle lays cannot grow.
        let (heights, top) = stack(0.3, 0.3, 40);
        assert_eq!(
            align_to_height(&heights, top, 12.1, 0.08, 0.3),
            Aligned::OutOfRange
        );
        // Nor can 0.28 mm layers find 0.13 mm between them under 0.3.
        let (heights, top) = stack(0.28, 0.28, 40);
        assert_eq!(
            align_to_height(&heights, top, top + 0.13, 0.08, 0.3),
            Aligned::OutOfRange
        );
        // They can find 0.09 mm.
        let last = adjusted(align_to_height(&heights, top, top + 0.09, 0.08, 0.3));
        assert!(last.iter().all(|height| (height - 0.298).abs() < 1e-9));
    }

    #[test]
    fn what_one_layer_cannot_take_goes_to_the_others() {
        // Five layers of different heights, the first near the greatest
        // allowed: it takes 0.01 mm and the other four the rest.
        let mut heights = vec![0.2; 12];
        heights[7] = 0.27;
        let top = heights.iter().sum::<f64>();
        let last = adjusted(align_to_height(&heights, top, top + 0.1, 0.08, 0.28));
        assert!((last[0] - 0.28).abs() < 1e-12, "{}", last[0]);
        for height in &last[1..] {
            assert!((height - 0.2225).abs() < 1e-9, "{height}");
        }
        // The stack ends where it was wanted, and no layer is out of
        // range. (The reference gives the full layer a share of the
        // second round too: 0.2825 mm, and a stack 0.0025 mm too tall.)
        assert!((last.iter().sum::<f64>() - (0.27 + 0.8 + 0.1)).abs() < 1e-9);
        assert!(last.iter().all(|height| *height <= 0.28 + 1e-12));

        // The same downward: one layer near the least allowed.
        let mut heights = vec![0.2; 12];
        heights[11] = 0.09;
        let top = heights.iter().sum::<f64>();
        let last = adjusted(align_to_height(&heights, top, top - 0.09, 0.08, 0.28));
        assert!((last[4] - 0.08).abs() < 1e-12);
        assert!((last.iter().sum::<f64>() - (0.8 + 0.09 - 0.09)).abs() < 1e-9);
        assert!(last.iter().all(|height| *height >= 0.08 - 1e-12));
    }
}
