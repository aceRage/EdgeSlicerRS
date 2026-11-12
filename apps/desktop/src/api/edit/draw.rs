//! "Cut"'s drawn surface while it is drawn (GLGizmoCut.cpp
//! `draw_on_mouse`, `draw_sample_at`, `refresh_draw_stroke`,
//! `close_draw_chain_on_surface`, `commit_draw_points`): where the
//! pointer's rays meet the copy, and the line as its chain of strokes,
//! finished and checked against the copy, with the surface it makes. The
//! sums are `edge_engine::drawn`'s; the page keeps the chain and its undo.

use std::sync::Arc;

use edge_engine::drawn::{
    self, DrawChainEnd, DrawCutChain, DrawCutError, DrawCutSample, DrawCutStroke, RecipeStroke,
};
use edge_engine::raycast::Tree;
use edge_engine::{Mesh, Transform};
use serde_json::{Value, json};

use super::super::{Session, lock, whole};
use super::{drawn_of, transform_of};

/// A copy's printed parts in a cut plane's frame, with the tree its rays
/// are cast in; kept while the copy and the plane stay as they are
/// (`update_draw_raycaster`: one mesh a gesture).
pub(in crate::api) struct InPlane {
    key: String,
    mesh: Mesh,
    tree: Tree,
}

/// A sample as the page takes it: `[x, y, z, nx, ny, nz, facet]`.
fn sample_json(s: &DrawCutSample) -> Value {
    json!([
        s.pos[0],
        s.pos[1],
        s.pos[2],
        s.normal[0],
        s.normal[1],
        s.normal[2],
        s.facet
    ])
}

/// Samples as the page gives them.
fn samples_of(value: Option<&Value>) -> Result<Vec<DrawCutSample>, String> {
    let Some(list) = value.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    list.iter()
        .map(|sample| {
            let n: Vec<f64> = sample
                .as_array()
                .map(|list| list.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default();
            if n.len() != 7 || n.iter().any(|v| !v.is_finite()) {
                return Err("a drawn cut's point is not seven numbers".to_string());
            }
            Ok(DrawCutSample {
                pos: [n[0], n[1], n[2]],
                normal: [n[3], n[4], n[5]],
                facet: n[6].max(0.0) as usize,
            })
        })
        .collect()
}

/// A chain as the page keeps it (the recipe's form: `samples`, `strokes`,
/// `closed`, `finished_open`, `smoothing`).
fn chain_json(chain: &DrawCutChain, smoothing: f64) -> Value {
    let stored = drawn::recipe_stroke_from_chain(chain, smoothing);
    json!({
        "samples": stored.samples.iter().map(sample_json).collect::<Vec<_>>(),
        "strokes": stored.stroke_bounds.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
        "closed": stored.closed,
        "finished_open": stored.finished_open,
        "smoothing": smoothing,
    })
}

/// The words for an error, as the panel shows them.
fn error_words(error: DrawCutError) -> Option<String> {
    (error != DrawCutError::None)
        .then(|| super::super::i18n::tr(drawn::draw_cut_error_words(error)))
}

impl Session {
    /// The copy's printed parts in the plane's frame, from the cache when
    /// the copy and the plane are as they were.
    fn in_plane(&self, args: &Value) -> Result<(Arc<InPlane>, Transform), String> {
        let stage = self.stage_of(args)?;
        let file = stage.file.as_ref().ok_or("the project has no model")?;
        let copy = whole(args, "copy")?;
        let plane = transform_of(args.get("plane").unwrap_or(&Value::Null))?;
        let key = format!("{}|{copy}|{:?}", Arc::as_ptr(file) as usize, plane.rows);
        if let Some(held) = lock(&self.cut_preview).in_plane.clone()
            && held.key == key
        {
            return Ok((held, plane));
        }
        let mesh = file
            .model
            .cut_object_in_plane(stage.bed().as_ref(), copy, &plane)
            .map_err(|error| error.to_string())?;
        let tree = Tree::new(&mesh);
        let made = Arc::new(InPlane { key, mesh, tree });
        lock(&self.cut_preview).in_plane = Some(made.clone());
        Ok((made, plane))
    }

    /// Where rays from the pointer meet the copy (`draw_sample_at`):
    /// `rays`, each `[x, y, z, dx, dy, dz]` in the scene; each answered
    /// with a sample in the plane's frame, or null where it misses or meets
    /// the part from inside (`unproject_on_mesh`). Also the radius a
    /// stroke closes the loop within (`draw_chain_snap_radius`).
    pub(in crate::api) fn cut_draw_hits(&self, args: &Value) -> Result<Value, String> {
        let (held, plane) = self.in_plane(args)?;
        let to_plane = plane.inverse().ok_or("that is no plane to cut by")?;
        let rays = args
            .get("rays")
            .and_then(Value::as_array)
            .ok_or("rays were not given")?;
        let hits: Vec<Value> = rays
            .iter()
            .map(|ray| {
                let r: Vec<f64> = ray
                    .as_array()
                    .map(|list| list.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                if r.len() != 6 {
                    return Value::Null;
                }
                let origin = to_plane.apply([r[0], r[1], r[2]]);
                let ahead = to_plane.apply([r[0] + r[3], r[1] + r[4], r[2] + r[5]]);
                let dir: [f64; 3] = std::array::from_fn(|axis| ahead[axis] - origin[axis]);
                let Some(hit) = held.tree.hit(origin, dir, (0.0, f64::INFINITY), &|_| true) else {
                    return Value::Null;
                };
                let normal = held.tree.normal(hit.triangle);
                if normal.iter().zip(dir).map(|(n, d)| n * d).sum::<f64>() >= 0.0 {
                    return Value::Null;
                }
                sample_json(&DrawCutSample {
                    pos: hit.point,
                    normal,
                    facet: hit.triangle,
                })
            })
            .collect();
        let bounds = held
            .mesh
            .bounds()
            .map(|(lo, hi)| (lo.map(f64::from), hi.map(f64::from)));
        Ok(json!({ "hits": hits, "snap": drawn::draw_cut_chain_snap_radius(bounds) }))
    }

    /// The line drawn so far, changed by `op` and made ready to cut with
    /// (`refresh_draw_stroke`). `drawn` as [`drawn_of`] takes it, the chain
    /// and the sweep; `op`:
    /// - `append`: `stroke` (samples) carried on from `at` (`front` or
    ///   `back`), its skims onto other faces taken out, the loop closed
    ///   when it ends near the other end;
    /// - `close`: "Close loop", along `path` (samples from the back end to
    ///   the front), or as it is when the ends are near;
    /// - `finish_open`: "Cut along the line";
    /// - `points`: the line's points as edited (`points`), one stroke,
    ///   open or closed as it was;
    /// - none: as it is.
    ///
    /// Answers `drawn` (the chain), `done` (whether the op took), `path`
    /// (the finished line, put back onto the copy), `closed`, `error` (why
    /// there is no cut surface), `empty` (`[upper, lower]`), `folds`,
    /// `clipped`, `flips`, `snap`, and `surface` (the cut surface drawn,
    /// triangles of corners, in the plane's frame).
    pub(in crate::api) fn cut_draw(&self, args: &Value) -> Result<Value, String> {
        let (held, _) = self.in_plane(args)?;
        let given = args.get("drawn").unwrap_or(&Value::Null);
        let samples = if given.get("samples").is_some() {
            given.clone()
        } else {
            let mut with = given.clone();
            if let Value::Object(map) = &mut with {
                map.insert("samples".into(), json!([]));
            } else {
                with = json!({ "samples": [] });
            }
            with
        };
        let mut cut = drawn_of(&samples)?;
        let bounds = held
            .mesh
            .bounds()
            .map(|(lo, hi)| (lo.map(f64::from), hi.map(f64::from)));
        let snap = drawn::draw_cut_chain_snap_radius(bounds);
        let mut chain = drawn::recipe_stroke_to_chain(&cut.stroke);
        let done = match args.get("op").and_then(Value::as_str) {
            None => true,
            Some("append") => {
                let stroke = drawn::draw_cut_filter_capture_skims(&samples_of(args.get("stroke"))?);
                let at = match args.get("at").and_then(Value::as_str) {
                    Some("front") => DrawChainEnd::Front,
                    Some("back") => DrawChainEnd::Back,
                    _ => return Err("at is front or back".into()),
                };
                stroke.len() >= 2 && chain.append_at(&stroke, at, snap) != DrawChainEnd::None
            }
            Some("close") => {
                let path = samples_of(args.get("path"))?;
                match (chain.back_pos(), chain.front_pos()) {
                    _ if chain.is_closed() || chain.len() < drawn::MIN_CHAIN_SAMPLES => false,
                    (Some(a), Some(b)) => {
                        let gap = distance(a, b);
                        if gap <= snap {
                            chain.close_along_path(&[])
                        } else {
                            path.len() >= 2
                                && distance(path[0].pos, a) <= gap
                                && distance(path[path.len() - 1].pos, b) <= gap
                                && chain.close_along_path(&path)
                        }
                    }
                    _ => false,
                }
            }
            Some("finish_open") => chain.finish_open(),
            Some("points") => {
                // DIFFERS FROM THE REFERENCE, whose edit closes the line
                // whatever it was (`commit_draw_points`, l.3559): an open
                // line stays open here.
                let points = samples_of(args.get("points"))?;
                let (closed, open) = (chain.is_closed(), chain.is_finished_open());
                if points.len() < 2 {
                    false
                } else {
                    chain.set_samples(&points, closed, open);
                    true
                }
            }
            Some(other) => return Err(format!("there is no drawing step named {other}")),
        };
        cut.stroke = RecipeStroke {
            smoothing: cut.stroke.smoothing,
            ..drawn::recipe_stroke_from_chain(&chain, cut.stroke.smoothing)
        };
        let mut params = cut.params.clone();
        params.thickness = args
            .get("thickness")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0);
        params.thickness_offset = match args.get("thickness_from").and_then(Value::as_str) {
            Some("above") => edge_engine::CutThickness::Above,
            Some("below") => edge_engine::CutThickness::Below,
            _ => edge_engine::CutThickness::Centred,
        };
        let line = drawn::draw_cut_line(
            &held.mesh,
            &held.tree,
            &chain,
            cut.stroke.smoothing,
            &params,
        );
        let valid = line.stroke.valid();
        let empty = if valid {
            drawn::draw_cut_empty_sides(&held.mesh, &line.stroke, &line.params)
        } else {
            [false, false]
        };
        Ok(json!({
            "drawn": chain_json(&chain, cut.stroke.smoothing),
            "done": done,
            "path": line.stroke.path().iter().map(sample_json).collect::<Vec<_>>(),
            "closed": line.stroke.is_closed(),
            "valid": valid,
            "error": if chain.is_empty() || (!chain.is_closed() && !chain.is_finished_open()) {
                None
            } else {
                error_words(line.error)
            },
            "empty": empty,
            "folds": line.folds,
            "clipped": line.clipped,
            "flips": line.flips,
            "snap": snap,
            "surface": if valid { surface(&line.stroke, &line.params, &held.mesh) } else { Vec::new() },
        }))
    }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3)
        .map(|axis| (a[axis] - b[axis]).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// The cut surface as the tool draws it (`update_draw_preview_models`):
/// the surface alone, its triangles' corners one after another.
fn surface(stroke: &DrawCutStroke, params: &drawn::DrawCutParams, mesh: &Mesh) -> Vec<f32> {
    let bounds = mesh
        .bounds()
        .map(|(lo, hi)| (lo.map(f64::from), hi.map(f64::from)));
    let shown = drawn::draw_cut_preview_surface(stroke, params, bounds, 0.0, Some(mesh));
    let mut out = Vec::with_capacity(shown.triangles.len() * 9);
    for at in 0..shown.triangles.len() {
        for corner in shown.triangle(at) {
            out.extend_from_slice(&corner);
        }
    }
    out
}
