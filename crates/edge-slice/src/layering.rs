//! An object's own layer heights: the profile its heights are drawn from
//! and the layers laid by it (a port of the reference's Slicing.cpp
//! `layer_height_profile_from_ranges`, `layer_height_profile_adaptive`,
//! `smooth_height_profile`, `adjust_layer_height_profile`,
//! `generate_object_layers`, `check_object_layers_fixed`, and of
//! SlicingAdaptive.cpp).
//!
//! A profile is the reference's flat list of pairs: a height in the
//! object (from its bottom, in mm) and the layer height there, from the
//! bottom up and ending at the object's top. Between two pairs the layer
//! height runs straight from one to the other; two pairs at one height
//! make a step.

use edge_mesh::Mesh;

use crate::precise_z::{ADJUSTED_LAYERS, Aligned, align_to_height};

/// libslic3r.h `EPSILON`.
const EPSILON: f64 = 1e-4;

/// Slicing.cpp `LAYER_HEIGHT_CHANGE_STEP`: how much an adaptive profile's
/// layer height may change from one layer to the next.
const LAYER_HEIGHT_CHANGE_STEP: f64 = 0.04;

/// What an object's layers are laid by (Slicing.hpp `SlicingParameters`),
/// as far as its layers' heights go. No raft is printed here, so the
/// first layer is always the first object layer, at its own height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlicingParams {
    /// The object's `layer_height`.
    pub layer_height: f64,
    /// The first layer's height (`initial_layer_print_height`, or the
    /// layer height where that is 0).
    pub first_layer_height: f64,
    /// The least and greatest layer height the nozzles in use allow,
    /// never narrower than `layer_height` itself.
    pub min_layer_height: f64,
    pub max_layer_height: f64,
    /// How tall the object is, in mm.
    pub height: f64,
}

impl SlicingParams {
    /// Whether the first layer is laid at its own height whatever the
    /// profile says (Slicing.hpp `first_object_layer_height_fixed`):
    /// always, without a raft.
    pub fn first_fixed(&self) -> bool {
        true
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    (1.0 - t) * a + t * b
}

fn is_approx(a: f64, b: f64) -> bool {
    (a - b).abs() < EPSILON
}

/// One of an object's height ranges with its layer height, in the
/// object's own heights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightRange {
    pub low: f64,
    pub high: f64,
    pub layer_height: f64,
}

/// The profile an object's height ranges give it, the rest at its layer
/// height (Slicing.cpp `layer_height_profile_from_ranges`). `ranges` are
/// sorted by their low and high ends, as the reference keeps them.
pub fn profile_from_ranges(params: &SlicingParams, ranges: &[HeightRange]) -> Vec<f64> {
    // 1) Trim the ranges one by the other, the first layer first.
    let mut kept: Vec<(f64, f64, f64)> = Vec::with_capacity(ranges.len() + 1);
    if params.first_fixed() {
        kept.push((0.0, params.first_layer_height, params.first_layer_height));
    }
    for range in ranges {
        let mut low = range.low;
        let high = range.high.min(params.height);
        if let Some(last) = kept.last() {
            low = low.max(last.1);
        }
        // Ranges too narrow are left out.
        if low + EPSILON < high {
            kept.push((low, high, range.layer_height));
        }
    }
    // 2) Steps of the ranges' heights, the gaps between them at the
    // object's layer height.
    let mut profile: Vec<f64> = Vec::new();
    let last_z = |profile: &[f64]| {
        if profile.is_empty() {
            0.0
        } else {
            profile[profile.len() - 2]
        }
    };
    let append = |profile: &mut Vec<f64>, z: f64, height: f64| {
        let n = profile.len();
        if n > 0 {
            let z_matches = is_approx(profile[n - 2], z);
            let h_matches = is_approx(profile[n - 1], height);
            if h_matches {
                if z_matches {
                    // A duplicate.
                    return;
                }
                if n >= 4 && is_approx(profile[n - 3], height) {
                    // A third pair of the same height moves the second up.
                    profile[n - 2] = z;
                    return;
                }
            }
        }
        profile.push(z);
        profile.push(height);
    };
    for (low, high, height) in kept {
        let z = last_z(&profile);
        if low > z + EPSILON {
            append(&mut profile, z, params.layer_height);
            append(&mut profile, low, params.layer_height);
        }
        append(&mut profile, low, height);
        append(&mut profile, high, height);
    }
    let z = last_z(&profile);
    if z < params.height {
        append(&mut profile, z, params.layer_height);
        append(&mut profile, params.height, params.layer_height);
    }
    profile
}

/// Whether a profile read from a project can be used for an object of
/// this height: pairs, ending at its top (PrintObject.cpp
/// `update_layer_height_profile`, "Verify the layer_height_profile").
pub fn profile_fits(params: &SlicingParams, profile: &[f64]) -> bool {
    profile.len() >= 2
        && profile.len().is_multiple_of(2)
        && (profile[profile.len() - 2] - params.height).abs() <= 1e-3
}

/// One layer: its bottom and its top, in the object's heights.
pub type Span = (f64, f64);

/// The layers a profile lays (Slicing.cpp `generate_object_layers`): from
/// the first layer up, each as thick as the profile asks at its middle,
/// for as long as the middle is inside the object. With `precise_z` the
/// last five are tuned so that the last ends at the object's top
/// (`adjust_layer_series_to_align_object_height`, [`align_to_height`]).
pub fn object_layers(params: &SlicingParams, profile: &[f64], precise_z: bool) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut print_z = 0.0;
    if params.first_fixed() {
        print_z = params.first_layer_height;
        out.push((0.0, print_z));
    }
    let top = params.height;
    let min = params.min_layer_height;
    let mut at = 0;
    let mut slice_z = print_z + 0.5 * min;
    while slice_z < top {
        let mut height = min;
        if at < profile.len() {
            let mut next = at + 2;
            while next < profile.len() && slice_z >= profile[next] {
                at = next;
                next += 2;
            }
            let (z1, h1) = (profile[at], profile[at + 1]);
            height = h1;
            if next < profile.len() {
                let (z2, h2) = (profile[next], profile[next + 1]);
                height = lerp(h1, h2, (slice_z - z1) / (z2 - z1));
            }
        }
        slice_z = print_z + 0.5 * height;
        if slice_z >= top {
            break;
        }
        let bottom = print_z;
        print_z += height;
        slice_z = print_z + 0.5 * min;
        out.push((bottom, print_z));
    }
    if precise_z && out.len() > ADJUSTED_LAYERS {
        // The reference needs six layers: five adjusted over the first.
        let heights: Vec<f64> = out.iter().map(|(low, high)| high - low).collect();
        let ends = out.last().map_or(0.0, |span| span.1);
        if let Aligned::Adjusted(last) = align_to_height(
            &heights,
            ends,
            top,
            params.min_layer_height,
            params.max_layer_height,
        ) {
            let from = out.len() - ADJUSTED_LAYERS;
            for (offset, height) in last.iter().enumerate() {
                let index = from + offset;
                let low = if offset > 0 {
                    out[index - 1].1
                } else {
                    out[index].0
                };
                out[index] = (low, low + height);
            }
        }
    }
    out
}

/// Whether a profile is the plain one: the first layer and then one
/// layer height to the top (Slicing.cpp `check_object_layers_fixed`).
pub fn layers_fixed(params: &SlicingParams, profile: &[f64]) -> bool {
    if profile.len() != 4 && profile.len() != 8 {
        return false;
    }
    let fixed_step1 = is_approx(profile[1], profile[3]);
    let fixed_step2 =
        profile.len() == 4 || (profile[2] == profile[4] && is_approx(profile[5], profile[7]));
    if !fixed_step1 || !fixed_step2 {
        return false;
    }
    if profile[2] < 0.5 * params.first_layer_height + EPSILON
        || !is_approx(profile[3], params.first_layer_height)
    {
        return false;
    }
    let z_max = profile[profile.len() - 2];
    let z_2nd = params.first_layer_height + 0.5 * params.layer_height;
    if z_2nd > z_max {
        return true;
    }
    !(z_2nd < profile[profile.len() - 4] + EPSILON
        || !is_approx(profile[profile.len() - 1], params.layer_height))
}

/// One face of a mesh as the adaptive heights see it
/// (SlicingAdaptive.hpp `FaceZ`): its span in height, and how far its
/// normal leans from upright and from flat.
#[derive(Clone, Copy, Debug)]
struct FaceZ {
    low: f32,
    high: f32,
    n_cos: f32,
    n_sin: f32,
}

/// The faces of a mesh by their height, for [`adaptive_profile`]
/// (SlicingAdaptive.cpp `prepare`).
pub struct Adaptive {
    faces: Vec<FaceZ>,
}

impl Adaptive {
    /// `mesh` is the object's printed parts as it stands, its bottom at
    /// height 0.
    pub fn new(mesh: &Mesh) -> Self {
        let mut faces: Vec<FaceZ> = mesh
            .triangles
            .iter()
            .map(|triangle| {
                let [a, b, c] = triangle.map(|index| mesh.vertices[index as usize]);
                // TriangleMesh.hpp `face_normal`, in single precision.
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let mut n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if length > 0.0 {
                    n = n.map(|component| component / length);
                }
                FaceZ {
                    low: a[2].min(b[2]).min(c[2]),
                    high: a[2].max(b[2]).max(c[2]),
                    n_cos: n[2].abs(),
                    n_sin: (n[0] * n[0] + n[1] * n[1]).sqrt(),
                }
            })
            .collect();
        // By their span, the lower end first.
        faces.sort_by(|f1, f2| {
            (f1.low, f1.high)
                .partial_cmp(&(f2.low, f2.high))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Self { faces }
    }

    /// The height of the next layer from `print_z` up, for `quality` from
    /// 0 (finest) to 1 (fastest) (SlicingAdaptive.cpp `next_layer_height`).
    /// `current` is where the search of the faces starts, and is moved on.
    fn next_layer_height(
        &self,
        params: &SlicingParams,
        print_z: f32,
        quality: f32,
        current: &mut usize,
    ) -> f32 {
        let mut height = params.max_layer_height as f32;
        let deviation = if quality < 0.5 {
            lerp(
                params.min_layer_height,
                params.layer_height,
                2.0 * f64::from(quality),
            )
        } else {
            lerp(
                params.max_layer_height,
                params.layer_height,
                2.0 * (1.0 - f64::from(quality)),
            )
        } as f32;
        let epsilon = EPSILON as f32;
        let min = params.min_layer_height as f32;
        let mut ordered = *current;
        let mut first_hit = false;
        while ordered < self.faces.len() {
            let face = &self.faces[ordered];
            if face.low >= print_z {
                break;
            }
            if face.high > print_z {
                if !first_hit {
                    first_hit = true;
                    *current = ordered;
                }
                // Faces that only touch would give small heights.
                if face.high >= print_z + epsilon {
                    height = height.min(from_slope(face, deviation));
                }
            }
            ordered += 1;
        }
        height = height.max(min);
        if height > min {
            while ordered < self.faces.len() {
                let face = &self.faces[ordered];
                if face.low >= print_z + height {
                    break;
                }
                if face.high < print_z + epsilon {
                    ordered += 1;
                    continue;
                }
                let reduced = from_slope(face, deviation);
                let z_diff = face.low - print_z;
                if reduced < z_diff {
                    height = z_diff;
                } else if reduced < height {
                    height = reduced;
                }
                ordered += 1;
            }
            height = height.max(min);
        }
        height
    }
}

/// The greatest height a face allows within the surface error asked for
/// (SlicingAdaptive.cpp `layer_height_from_slope`, the area of the error
/// triangle, held to the roughness of an upright wall).
fn from_slope(face: &FaceZ, deviation: f32) -> f32 {
    let by_slope = if face.n_cos > 1e-5 {
        (1.44 * f64::from(deviation) * f64::from(face.n_sin / face.n_cos).sqrt()) as f32
    } else {
        f32::MAX
    };
    (deviation / 0.184).min(by_slope)
}

/// An adaptive profile: each layer as thick as the slopes of the faces
/// it crosses allow for `quality` (0 finest, 1 fastest), changing by no
/// more than 0.04 mm from one layer to the next, and a height range's
/// own height kept within it (Slicing.cpp `layer_height_profile_adaptive`).
pub fn adaptive_profile(
    params: &SlicingParams,
    faces: &Adaptive,
    quality: f32,
    ranges: &[HeightRange],
) -> Vec<f64> {
    let mut profile = vec![0.0, params.first_layer_height];
    if params.first_fixed() {
        profile.push(params.first_layer_height);
        profile.push(params.first_layer_height);
    }
    let mut print_z = params.first_layer_height;
    let mut current = 0;
    while print_z + EPSILON < params.height {
        let mut height = params.max_layer_height as f32;
        let cusp = faces.next_layer_height(params, print_z as f32, quality, &mut current);
        height = cusp.min(height);
        let mut height = f64::from(height);
        let last = profile[profile.len() - 1];
        if last < height && height - last > LAYER_HEIGHT_CHANGE_STEP {
            height = last + LAYER_HEIGHT_CHANGE_STEP;
        } else if last > height && last - height > LAYER_HEIGHT_CHANGE_STEP {
            height = last - LAYER_HEIGHT_CHANGE_STEP;
        }
        // The reference keeps the layer height in a float.
        let mut height = height as f32 as f64;
        if let Some(range) = ranges
            .iter()
            .find(|range| print_z >= range.low && print_z <= range.high)
        {
            height = range.layer_height;
        }
        profile.push(print_z);
        profile.push(height);
        print_z += height;
    }
    let gap = params.height - profile[profile.len() - 2];
    if gap > 0.0 {
        profile.push(params.height);
        profile.push(gap.clamp(params.min_layer_height, params.max_layer_height));
    }
    profile
}

/// A profile smoothed by a Gaussian blur leaning to the least height,
/// six times over (Slicing.cpp `smooth_height_profile`). `radius` is in
/// pairs of the profile; with `keep_min` no layer is made thicker than
/// it was.
pub fn smooth_profile(
    profile: &[f64],
    params: &SlicingParams,
    radius: u32,
    keep_min: bool,
) -> Vec<f64> {
    let blur = |profile: &[f64]| -> Vec<f64> {
        let skip = if params.first_fixed() { 4 } else { 0 };
        if (profile.len() as i64) - (skip as i64) < 6 {
            return profile.to_vec();
        }
        let radius = radius.max(1);
        let kernel: Vec<f64> = {
            let size = 2 * radius + 1;
            let sigma = 0.3 * (f64::from(radius) - 1.0) + 0.8;
            let two_sq_sigma = 2.0 * sigma * sigma;
            let inv = 1.0 / (std::f64::consts::PI * two_sq_sigma).sqrt();
            (0..size)
                .map(|i| {
                    let x = f64::from(i) - f64::from(radius);
                    inv * (-x * x / two_sq_sigma).exp()
                })
                .collect()
        };
        let two_radius = 2 * radius as i64;
        let size = profile.len();
        let mut out: Vec<f64> = profile[..skip].to_vec();
        let delta_h = params.max_layer_height - params.min_layer_height;
        let inv_delta_h = if delta_h != 0.0 { 1.0 / delta_h } else { 1.0 };
        let max_dz_band = f64::from(radius) * params.layer_height;
        let mut i = skip;
        while i < size {
            let (zi, hi) = (profile[i], profile[i + 1]);
            let begin = (i as i64 - two_radius).max(skip as i64);
            let end = (i as i64 + two_radius).min(size as i64 - 2);
            let (mut height, mut total) = (0.0, 0.0);
            let mut j = begin;
            while j <= end {
                let kernel_id = (i64::from(radius) + (j - i as i64) / 2) as usize;
                let dz = (zi - profile[j as usize]).abs();
                if dz * params.layer_height <= max_dz_band {
                    let dh = (params.max_layer_height - profile[j as usize + 1]).abs();
                    let weight = kernel[kernel_id] * (dh * inv_delta_h).sqrt();
                    height += weight * profile[j as usize + 1];
                    total += weight;
                }
                j += 2;
            }
            let mut height = if total == 0.0 { hi } else { height / total }
                .clamp(params.min_layer_height, params.max_layer_height);
            if keep_min {
                height = height.min(hi);
            }
            out.push(zi);
            out.push(height);
            i += 2;
        }
        out
    };
    let mut out = profile.to_vec();
    for _ in 0..6 {
        out = blur(&out);
    }
    out
}

/// How a stroke of the layer height tool changes a profile
/// (Slicing.hpp `LayerHeightEditActionType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAction {
    Increase,
    Decrease,
    /// Back towards the object's layer height.
    Reduce,
    Smooth,
}

/// The profile changed by one stroke of the layer height tool at height
/// `z` (Slicing.cpp `adjust_layer_height_profile`): within `band_width`
/// round it, by up to `delta`, weighted by a cosine. A stroke over or
/// within a layer of a height range changes nothing. The profile is the
/// object's own, ending at its top.
pub fn adjust_profile(
    params: &SlicingParams,
    ranges: &[HeightRange],
    profile: &mut Vec<f64>,
    z: f64,
    delta: f64,
    band_width: f64,
    action: EditAction,
) {
    let span = (
        if params.first_fixed() {
            params.first_layer_height
        } else {
            0.0
        },
        params.height,
    );
    if z < span.0 || z > span.1 || profile.len() < 4 {
        return;
    }
    // 1) The layer height at z.
    let mut current = params.layer_height;
    let mut i = 0;
    while i < profile.len() {
        if i + 2 == profile.len() {
            current = profile[i + 1];
            break;
        } else if profile[i + 2] > z {
            let (z1, h1, z2, h2) = (profile[i], profile[i + 1], profile[i + 2], profile[i + 3]);
            current = lerp(h1, h2, (z - z1) / (z2 - z1));
            break;
        }
        i += 2;
    }
    if ranges
        .iter()
        .any(|range| z >= range.low - current && z <= range.high + current)
    {
        return;
    }
    // 2) Whether the change can be made.
    let mut delta = delta;
    match action {
        EditAction::Increase | EditAction::Decrease => {
            if action == EditAction::Decrease {
                delta = -delta;
            }
            if delta > 0.0 {
                if current >= params.max_layer_height - EPSILON {
                    return;
                }
                delta = delta.min(params.max_layer_height - current);
            } else {
                if current <= params.min_layer_height + EPSILON {
                    return;
                }
                delta = delta.max(params.min_layer_height - current);
            }
        }
        EditAction::Reduce | EditAction::Smooth => {
            delta = delta.abs().min((params.layer_height - current).abs());
            if delta < EPSILON {
                return;
            }
        }
    }
    // 3) The profile sampled every 0.1 mm across the band.
    let low = span.0.max(z - 0.5 * band_width);
    // The upper side is not limited, so the top point can be changed.
    let high = z + 0.5 * band_width;
    let z_step = 0.1;
    let mut idx: usize = 0;
    while idx < profile.len() && profile[idx] < low {
        idx += 2;
    }
    let mut idx = idx.saturating_sub(2) as isize;
    let mut out: Vec<f64> = Vec::with_capacity(profile.len());
    out.extend_from_slice(&profile[..idx as usize + 2]);
    let mut zz = low;
    let start = out.len();
    while zz < high {
        let at = idx as usize;
        let next = at + 2;
        let (z1, h1) = (profile[at], profile[at + 1]);
        let mut height = h1;
        if next < profile.len() {
            let (z2, h2) = (profile[next], profile[next + 1]);
            height = lerp(h1, h2, (zz - z1) / (z2 - z1));
        }
        let weight = if (zz - z).abs() < 0.5 * band_width {
            0.5 + 0.5 * (2.0 * std::f64::consts::PI * (zz - z) / band_width).cos()
        } else {
            0.0
        };
        match action {
            EditAction::Increase | EditAction::Decrease => height += weight * delta,
            EditAction::Reduce => {
                let off = height - params.layer_height;
                let step = weight * delta;
                height += if off.abs() > step {
                    if off > 0.0 { -step } else { step }
                } else {
                    -off
                };
            }
            EditAction::Smooth => {}
        }
        height = height.clamp(params.min_layer_height, params.max_layer_height);
        if zz == span.1 {
            // The last point of the profile.
            if out[out.len() - 2] + EPSILON > zz {
                out.pop();
                out.pop();
            }
            out.push(zz);
            out.push(height);
            idx = profile.len() as isize;
            break;
        }
        // A segment too short is not entered.
        if out[out.len() - 2] + EPSILON < zz {
            out.push(zz);
            out.push(height);
        }
        // Held to the object's top, so the next round sets the last point.
        zz = (zz + z_step).min(span.1);
        let mut at = next;
        while at < profile.len() && profile[at] < zz {
            at += 2;
        }
        idx = at as isize - 2;
    }
    let idx = (idx + 2) as usize;
    let end = out.len();
    if idx < profile.len() {
        out.extend_from_slice(&profile[idx..]);
    } else if out[out.len() - 2] + 0.5 * EPSILON < span.1 {
        out.extend_from_slice(&profile[profile.len() - 2..]);
    }
    *profile = out;
    if action == EditAction::Smooth {
        let start = start.max(1);
        let end = if end == profile.len() { end - 2 } else { end };
        for _ in 0..6 {
            let was = profile.clone();
            let mut i = start;
            while i < end {
                let zz = was[i];
                let t = if (zz - z).abs() < 0.5 * band_width {
                    0.25 + 0.25 * (2.0 * std::f64::consts::PI * (zz - z) / band_width).cos()
                } else {
                    0.0
                };
                profile[i + 1] = if i == 0 {
                    (1.0 - t) * was[i + 1] + t * was[i + 3]
                } else if i + 1 == was.len() {
                    (1.0 - t) * was[i + 1] + t * was[i - 1]
                } else {
                    (1.0 - t) * was[i + 1] + 0.5 * t * (was[i - 1] + was[i + 3])
                };
                i += 2;
            }
        }
    }
}

/// What a profile is read from and written as in a project
/// (`Metadata/layer_heights_profile.txt`, bbs_3mf.cpp): its numbers
/// separated by `;`, each as `%f` writes it.
pub fn profile_text(profile: &[f64]) -> String {
    profile
        .iter()
        .map(|value| format!("{value:.6}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// A profile as a project holds it; none where it is not one the
/// reference reads (fewer than six numbers or an odd count).
pub fn parse_profile(text: &str) -> Option<Vec<f64>> {
    let values: Vec<f64> = text
        .split(';')
        .map(|value| value.trim().parse::<f64>().unwrap_or(0.0))
        .collect();
    (values.len() > 4 && values.len().is_multiple_of(2)).then_some(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(height: f64) -> SlicingParams {
        SlicingParams {
            layer_height: 0.2,
            first_layer_height: 0.2,
            min_layer_height: 0.08,
            max_layer_height: 0.32,
            height,
        }
    }

    fn tops(layers: &[Span]) -> Vec<f64> {
        layers.iter().map(|span| span.1).collect()
    }

    #[test]
    fn a_plain_profile_lays_the_first_layer_and_then_the_layer_height() {
        let p = params(2.0);
        let profile = profile_from_ranges(&p, &[]);
        assert_eq!(profile, vec![0.0, 0.2, 2.0, 0.2]);
        let layers = object_layers(&p, &profile, false);
        assert_eq!(layers.len(), 10);
        for (index, (low, high)) in layers.iter().enumerate() {
            assert!((high - 0.2 * (index + 1) as f64).abs() < 1e-9);
            assert!((high - low - 0.2).abs() < 1e-9);
        }
        assert!(layers_fixed(&p, &profile));
    }

    #[test]
    fn a_range_lays_its_own_height_within_it() {
        let p = params(10.0);
        let ranges = [HeightRange {
            low: 4.0,
            high: 6.0,
            layer_height: 0.1,
        }];
        let profile = profile_from_ranges(&p, &ranges);
        // A third pair at one height moves the second up.
        assert_eq!(
            profile,
            vec![0.0, 0.2, 4.0, 0.2, 4.0, 0.1, 6.0, 0.1, 6.0, 0.2, 10.0, 0.2]
        );
        let layers = object_layers(&p, &profile, false);
        // 20 layers to 4 mm, 20 of 0.1 to 6 mm, 20 to the top.
        assert_eq!(layers.len(), 60);
        assert!((layers[19].1 - 4.0).abs() < 1e-9);
        assert!((layers[20].1 - layers[20].0 - 0.1).abs() < 1e-9);
        assert!((layers[39].1 - 6.0).abs() < 1e-9);
        assert!((layers[59].1 - 10.0).abs() < 1e-9);
        assert!(!layers_fixed(&p, &profile));
    }

    #[test]
    fn a_profile_ends_where_the_middle_of_a_layer_would_leave_the_object() {
        let p = params(1.05);
        let layers = object_layers(&p, &profile_from_ranges(&p, &[]), false);
        // The sixth layer's middle, at 1.1, is above the top.
        assert_eq!(tops(&layers).len(), 5);
        let p = params(1.15);
        let layers = object_layers(&p, &profile_from_ranges(&p, &[]), false);
        assert_eq!(tops(&layers).len(), 6);
    }

    #[test]
    fn precise_z_brings_the_last_layer_to_the_top() {
        let p = params(2.05);
        let layers = object_layers(&p, &profile_from_ranges(&p, &[]), true);
        assert!((layers.last().unwrap().1 - 2.05).abs() < 1e-9);
    }

    #[test]
    fn a_profile_is_written_as_the_reference_writes_it_and_read_back() {
        let text = profile_text(&[0.0, 0.2, 0.2, 0.2, 10.0, 0.12]);
        assert_eq!(
            text,
            "0.000000;0.200000;0.200000;0.200000;10.000000;0.120000"
        );
        assert_eq!(
            parse_profile(&text),
            Some(vec![0.0, 0.2, 0.2, 0.2, 10.0, 0.12])
        );
        assert_eq!(parse_profile("0;0.2;10;0.2"), None);
        assert!(profile_fits(&params(10.0), &[0.0, 0.2, 10.0005, 0.2]));
        assert!(!profile_fits(&params(10.0), &[0.0, 0.2, 9.9, 0.2]));
    }

    fn cone() -> Mesh {
        // A cone of radius 10 and height 10: faces at 45 degrees.
        let mut mesh = Mesh::default();
        mesh.vertices.push([0.0, 0.0, 10.0]);
        mesh.vertices.push([0.0, 0.0, 0.0]);
        let n = 32;
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            mesh.vertices.push([10.0 * a.cos(), 10.0 * a.sin(), 0.0]);
        }
        for i in 0..n {
            let (a, b) = (2 + i, 2 + (i + 1) % n);
            mesh.triangles.push([0, a, b]);
            mesh.triangles.push([1, b, a]);
        }
        mesh
    }

    #[test]
    fn an_adaptive_profile_is_finer_for_quality_and_coarser_for_speed() {
        let p = params(10.0);
        let faces = Adaptive::new(&cone());
        let fine = adaptive_profile(&p, &faces, 0.0, &[]);
        let fast = adaptive_profile(&p, &faces, 1.0, &[]);
        let count = |profile: &[f64]| object_layers(&p, profile, false).len();
        assert!(
            count(&fine) > count(&fast),
            "{} {}",
            count(&fine),
            count(&fast)
        );
        // Every height within the limits, ending at the top.
        for profile in [&fine, &fast] {
            assert!(profile_fits(&p, profile));
            for pair in profile.chunks(2).skip(2) {
                assert!(pair[1] >= 0.08 - 1e-9 && pair[1] <= 0.32 + 1e-9, "{pair:?}");
            }
        }
        // A range keeps its own height.
        let ranges = [HeightRange {
            low: 3.0,
            high: 5.0,
            layer_height: 0.1,
        }];
        let with = adaptive_profile(&p, &faces, 1.0, &ranges);
        for pair in with.chunks(2) {
            if pair[0] >= 3.0 && pair[0] <= 5.0 {
                assert_eq!(pair[1], 0.1);
            }
        }
    }

    #[test]
    fn smoothing_keeps_the_ends_and_the_limits() {
        let p = params(10.0);
        let faces = Adaptive::new(&cone());
        let profile = adaptive_profile(&p, &faces, 0.5, &[]);
        let smooth = smooth_profile(&profile, &p, 5, false);
        assert_eq!(smooth.len(), profile.len());
        assert_eq!(&smooth[..4], &profile[..4]);
        let kept = smooth_profile(&profile, &p, 5, true);
        for (pair, was) in kept.chunks(2).zip(profile.chunks(2)) {
            assert!(pair[1] <= was[1] + 1e-12);
        }
    }

    #[test]
    fn a_stroke_thickens_and_thins_layers_round_where_it_is_made() {
        let p = params(10.0);
        let mut profile = profile_from_ranges(&p, &[]);
        adjust_profile(&p, &[], &mut profile, 5.0, 0.1, 2.0, EditAction::Increase);
        let height_at = |profile: &[f64], z: f64| -> f64 {
            let layers = object_layers(&p, profile, false);
            let (low, high) = layers
                .into_iter()
                .find(|(low, high)| *low <= z && z < *high)
                .unwrap();
            high - low
        };
        assert!(height_at(&profile, 5.0) > 0.25);
        assert!((height_at(&profile, 1.0) - 0.2).abs() < 1e-9);
        assert!(profile_fits(&p, &profile));
        adjust_profile(&p, &[], &mut profile, 5.0, 0.3, 2.0, EditAction::Decrease);
        assert!(height_at(&profile, 5.0) < 0.15);
        // Over a height range nothing changes.
        let ranges = [HeightRange {
            low: 4.0,
            high: 6.0,
            layer_height: 0.1,
        }];
        let before = profile.clone();
        adjust_profile(
            &p,
            &ranges,
            &mut profile,
            5.0,
            0.1,
            2.0,
            EditAction::Increase,
        );
        assert_eq!(profile, before);
        // Smoothing and reducing keep it within the limits.
        adjust_profile(&p, &[], &mut profile, 5.0, 0.1, 2.0, EditAction::Smooth);
        adjust_profile(&p, &[], &mut profile, 5.0, 0.1, 2.0, EditAction::Reduce);
        for pair in profile.chunks(2) {
            assert!(pair[1] >= 0.08 - 1e-9 && pair[1] <= 0.32 + 1e-9);
        }
    }
}
