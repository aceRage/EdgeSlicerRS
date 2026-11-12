//! The optimiser the placer slides an item along a no-fit polygon's edge
//! with: NLopt's subplex (`LN_SBPLX`) on one variable, as libnest2d asks
//! for it (`nfpplacer.hpp` `_NofitPolyPlacer::Optimizer`: the position in
//! 0..1 along the polygon's outline, at most `1000 * accuracy`
//! evaluations, a relative tolerance on the score of 1e-20, no tolerance
//! on the position, NLopt's own first step).
//!
//! Ported from NLopt 2.x `src/algs/neldermead/sbplx.c` and `nldrmd.c`
//! (MIT licence, Steven G. Johnson) and `src/api/options.c`
//! (`nlopt_set_default_initial_step`), cut down to one dimension: subplex
//! on one variable is one Nelder-Mead subspace of one dimension, restarted
//! with a step a quarter of the last until the simplex's two scores agree
//! or the evaluations run out. Every step is the reference's, in the same
//! order and with the same arithmetic, so the same objective gives the
//! same optimum to the bit. NLopt's red-black tree ordering the simplex
//! by score (ties broken by the point's place in memory) is the order of
//! the two points by score and then by slot.

/// Subplex's step scale and Nelder-Mead's convergence factor (`psi`).
const PSI: f64 = 0.25;
/// Nelder-Mead's reflection, expansion, contraction and shrink factors.
const ALPHA: f64 = 1.0;
const GAMMA: f64 = 2.0;
const BETA: f64 = 0.5;
const DELTA: f64 = 0.5;

/// What the search found: the best place and its score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Optimum {
    pub x: f64,
    pub score: f64,
}

/// Why one inner Nelder-Mead run stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stop {
    /// The simplex shrank enough (or collapsed): go on with subplex.
    Converged,
    /// The evaluations ran out: the search is over.
    Exhausted,
    /// The first step made no simplex (NLOPT_FAILURE), which subplex takes
    /// as converged and ends the search on.
    Failed,
}

/// `close` in nldrmd.c: equal to floating-point precision.
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-13 * (a.abs() + b.abs())
}

/// `reflectpt` in nldrmd.c: `c + scale * (c - old)` pinned to the bounds,
/// with whether it is a new point (neither `c` nor `old`).
fn reflect(c: f64, scale: f64, old: f64, lb: f64, ub: f64) -> (f64, bool) {
    let x = (c + scale * (c - old)).clamp(lb, ub);
    (x, !(close(x, c) || close(x, old)))
}

/// `relstop` in stop.c.
fn relstop(old: f64, new: f64, reltol: f64, abstol: f64) -> bool {
    if old.is_infinite() {
        return false;
    }
    (new - old).abs() < abstol
        || (new - old).abs() < reltol * (new.abs() + old.abs()) * 0.5
        || (reltol > 0.0 && new == old)
}

/// NLopt's first step for a variable bounded by `lb` and `ub` that starts
/// at `x` (`nlopt_set_default_initial_step`).
fn initial_step(x: f64, lb: f64, ub: f64) -> f64 {
    let mut step = f64::INFINITY;
    if ub.is_finite() && lb.is_finite() && (ub - lb) * 0.25 < step && ub > lb {
        step = (ub - lb) * 0.25;
    }
    if ub.is_finite() && ub - x < step && ub > x {
        step = (ub - x) * 0.75;
    }
    if lb.is_finite() && x - lb < step && x > lb {
        step = (x - lb) * 0.75;
    }
    if step.is_infinite() {
        if ub.is_finite() && (ub - x).abs() < step.abs() {
            step = (ub - x) * 1.1;
        }
        if lb.is_finite() && (x - lb).abs() < step.abs() {
            step = (x - lb) * 1.1;
        }
    }
    if step.is_infinite() || step == 0.0 || step.is_subnormal() {
        step = x;
    }
    if step.is_infinite() || step == 0.0 {
        step = 1.0;
    }
    step
}

/// The running state the evaluations share: the best point so far, as
/// NLopt keeps it (`*minf` and `x`, updated on ties), and the count.
struct Search<'a, F: FnMut(f64) -> f64> {
    f: &'a mut F,
    best: Optimum,
    evaluations: usize,
    budget: usize,
}

impl<F: FnMut(f64) -> f64> Search<'_, F> {
    /// One evaluation (`CHECK_EVAL`): Err when the budget is spent.
    fn eval(&mut self, x: f64) -> Result<f64, Stop> {
        let score = (self.f)(x);
        self.evaluations += 1;
        if score <= self.best.score {
            self.best = Optimum { x, score };
        }
        if self.evaluations >= self.budget {
            return Err(Stop::Exhausted);
        }
        Ok(score)
    }
}

/// `nldrmd_minimize_` with `psi` > 0 on one variable, from the best point
/// so far. Answers why it stopped and the spread of the last simplex's
/// scores (`fdiff`).
fn nelder_mead<F: FnMut(f64) -> f64>(
    search: &mut Search<'_, F>,
    step: f64,
    lb: f64,
    ub: f64,
) -> (Stop, f64) {
    let mut fdiff = f64::INFINITY;
    let x = search.best.x;
    // The simplex: two (score, place) slots; slot 0 is the start.
    let mut pts = [(search.best.score, x), (0.0, 0.0)];
    let mut second = x + step;
    if second > ub {
        second = if ub - x > step.abs() * 0.1 {
            ub
        } else {
            x - step.abs()
        };
    }
    if second < lb {
        if x - lb > step.abs() * 0.1 {
            second = lb;
        } else {
            second = x + step.abs();
            if second > ub {
                second = 0.5 * (if ub - x > x - lb { ub } else { lb } + x);
            }
        }
    }
    if close(second, x) {
        return (Stop::Failed, fdiff);
    }
    pts[1].1 = second;
    pts[1].0 = match search.eval(second) {
        Ok(score) => score,
        Err(stop) => return (stop, fdiff),
    };
    let mut init_diam = 0.0;
    loop {
        // The low and the high point; ties go by slot.
        let (low, high) = if pts[1].0 < pts[0].0 { (1, 0) } else { (0, 1) };
        let (fl, xl) = pts[low];
        let (mut fh, xh) = pts[high];
        fdiff = fh - fl;
        if init_diam == 0.0 {
            init_diam = (xl - xh).abs();
        }
        // The centroid of all but the high point is the low point.
        let c = xl;
        if (xl - xh).abs() < PSI * init_diam {
            return (Stop::Converged, fdiff);
        }
        let (xr, moved) = reflect(c, ALPHA, xh, lb, ub);
        if !moved {
            return (Stop::Converged, fdiff);
        }
        let fr = match search.eval(xr) {
            Ok(score) => score,
            Err(stop) => return (stop, fdiff),
        };
        if fr < fl {
            // A new best point: expand.
            let (xe, moved) = reflect(c, GAMMA, xh, lb, ub);
            pts[high].1 = xe;
            if !moved {
                return (Stop::Converged, fdiff);
            }
            fh = match search.eval(xe) {
                Ok(score) => score,
                Err(stop) => return (stop, fdiff),
            };
            if fh >= fr {
                fh = fr;
                pts[high].1 = xr;
            }
        } else {
            // nldrmd.c would accept the reflection here when it beat the
            // next to worst point, which on one variable is the low point,
            // so never after the test above.
            // A new worst point: contract, inside or outside.
            let (xc, moved) = reflect(c, if fh <= fr { -BETA } else { BETA }, xh, lb, ub);
            if !moved {
                return (Stop::Converged, fdiff);
            }
            let fc = match search.eval(xc) {
                Ok(score) => score,
                Err(stop) => return (stop, fdiff),
            };
            if fc < fr && fc < fh {
                pts[high].1 = xc;
                fh = fc;
            } else {
                // Shrink towards the low point and start over.
                let (xs, moved) = reflect(xl, -DELTA, pts[high].1, lb, ub);
                pts[high].1 = xs;
                if !moved {
                    return (Stop::Converged, fdiff);
                }
                pts[high].0 = match search.eval(xs) {
                    Ok(score) => score,
                    Err(stop) => return (stop, fdiff),
                };
                continue;
            }
        }
        pts[high].0 = fh;
    }
}

/// Minimises `f` over `lb..=ub` from `x0` with subplex, evaluating it at
/// most `budget` times (NLopt's `maxeval`) and stopping when the last
/// simplex's scores agree to `ftol_rel` (NLopt's `ftol_rel`).
pub fn minimize<F: FnMut(f64) -> f64>(
    mut f: F,
    x0: f64,
    lb: f64,
    ub: f64,
    budget: usize,
    ftol_rel: f64,
) -> Optimum {
    let first = f(x0);
    let mut search = Search {
        f: &mut f,
        best: Optimum {
            x: x0,
            score: first,
        },
        evaluations: 1,
        budget: budget.max(1),
    };
    if search.evaluations >= search.budget {
        return search.best;
    }
    let mut step = initial_step(x0, lb, ub);
    loop {
        let previous = search.best.x;
        let (stop, fdiff) = nelder_mead(&mut search, step, lb, ub);
        if stop != Stop::Converged {
            return search.best;
        }
        let minf = search.best.score;
        if relstop(minf + fdiff, minf, ftol_rel, 0.0) {
            return search.best;
        }
        // No tolerance on the place (NLopt's default), so subplex goes on
        // with a quarter of the step, towards where the best point moved.
        let dx = search.best.x - previous;
        step = if dx == 0.0 {
            -(step * PSI)
        } else {
            (step * PSI).copysign(dx)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parabola_is_minimised_where_it_is_lowest() {
        let optimum = minimize(|x| (x - 0.3) * (x - 0.3) + 1.0, 0.9, 0.0, 1.0, 1000, 1e-20);
        assert!((optimum.x - 0.3).abs() < 1e-6, "{optimum:?}");
        assert!((optimum.score - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_bound_holds_the_optimum() {
        let optimum = minimize(|x| -x, 0.2, 0.0, 1.0, 1000, 1e-20);
        assert_eq!(optimum.x, 1.0);
    }

    #[test]
    fn the_budget_is_kept() {
        let mut count = 0;
        minimize(
            |x| {
                count += 1;
                (x * 37.0).sin()
            },
            0.5,
            0.0,
            1.0,
            25,
            1e-20,
        );
        assert!(count <= 25);
    }

    #[test]
    fn a_flat_score_stops_at_once() {
        let mut count = 0;
        let optimum = minimize(
            |_| {
                count += 1;
                4.0
            },
            0.5,
            0.0,
            1.0,
            1000,
            1e-20,
        );
        // A tie moves NLopt's best point to the newer one.
        assert_eq!(optimum.score, 4.0);
        assert!(count < 20, "{count}");
    }

    #[test]
    fn the_first_step_is_nlopts() {
        assert_eq!(initial_step(0.0, 0.0, 1.0), 0.25);
        assert_eq!(initial_step(0.5, 0.0, 1.0), 0.25);
        assert_eq!(initial_step(0.9, 0.0, 1.0), (1.0 - 0.9) * 0.75);
        assert_eq!(initial_step(0.1, 0.0, 1.0), 0.1 * 0.75);
    }
}
