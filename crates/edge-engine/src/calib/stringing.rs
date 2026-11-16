//! "Stringing (temp × retraction)" (calib.cpp `make_stringing_grid` and
//! the `stringing_grid_*` functions, Plater.cpp `calib_temp_retraction`,
//! calib_dlg.cpp `TempRetract_Calibration_Dlg`): a row of columns, each a
//! base plate with two square pillars 20 mm apart, in bands of height
//! printed at falling temperatures, each column retracting its own
//! length on the travels between its pillars. The model is made here as
//! the reference makes it, of boxes (and, under each pillar, a fillet);
//! the writer then sets each band's temperature and each column's
//! retraction (`edge_gcode::StringingGrid`).

use edge_config::Settings;
use edge_mesh::{Mesh, Transform as MeshTransform, primitives::cuboid};

use super::{
    CalibMode, CalibParams, CalibProject, Set, bed_box, pairs, raised_max_layer_height, written,
};
use crate::{Bed, Failure, Model, Transform};

/// The grid's own values (calib.hpp `Calib_Params`, the
/// `Calib_Temp_Retract_Grid` fields); the temperatures are the
/// parameters' `start`, `end` and `step`.
#[derive(Clone, Debug, PartialEq)]
pub struct GridParams {
    pub retract_start: f64,
    pub retract_end: f64,
    pub retract_step: f64,
    /// Each band's height, mm (0: 8).
    pub band_height: f64,
    /// `M109` at each band, else `M104`.
    pub wait_for_temperature: bool,
    /// The raised values: each column's retraction on its base, each
    /// band's temperature on a tower of its own.
    pub print_labels: bool,
    pub fillet_base: bool,
    pub fillet_radius: f64,
    /// `z_hop` and `filament_z_hop` 0 for the test.
    pub disable_z_hop: bool,
}

impl Default for GridParams {
    fn default() -> Self {
        Self {
            retract_start: 0.2,
            retract_end: 1.6,
            retract_step: 0.2,
            band_height: 8.0,
            wait_for_temperature: true,
            print_labels: false,
            fillet_base: true,
            fillet_radius: 3.0,
            disable_z_hop: false,
        }
    }
}

// calib.hpp: the grid's measures.
const PILLAR_XY: f64 = 4.0;
const PILLAR_GAP: f64 = 20.0;
const COLUMN_PITCH: f64 = 12.0;
const BASE_X: f64 = 10.0;
const BASE_Y: f64 = 30.0;
pub const BASE_HEIGHT: f64 = 1.0;
const NOTCH_DEPTH: f64 = 0.4;
const LABEL_HEIGHT: f64 = 0.6;
const LABEL_EMBED: f64 = 0.2;
const GLYPH_HEIGHT: f64 = 5.0;
const GLYPH_WIDTH: f64 = 3.0;
const LABEL_STROKE: f64 = 1.0;
const LABEL_TOWER_X: f64 = 10.5;
const LABEL_TOWER_Y: f64 = 6.0;
const BED_MARGIN: f64 = 15.0;
/// Kept off the bed's width for a prime tower (calib.hpp
/// `stringing_grid_prime_tower_size`), which the grid itself never has.
const PRIME_TOWER_SIZE: f64 = 35.0;
const GLYPH_GAP: f64 = 0.4;

fn band_height(params: &CalibParams) -> f64 {
    if params.grid.band_height > 0.0 {
        params.grid.band_height
    } else {
        8.0
    }
}

/// calib.cpp `stringing_grid_band_count`.
pub fn band_count(params: &CalibParams) -> usize {
    if params.step <= 0.0 {
        return 1;
    }
    (((params.end - params.start).abs() / params.step).round() as i64 + 1).max(1) as usize
}

/// calib.cpp `stringing_grid_retract_schedule`: each column's length.
pub fn retract_schedule(params: &CalibParams) -> Vec<f64> {
    let grid = &params.grid;
    if grid.retract_step <= 0.0 {
        return vec![grid.retract_start];
    }
    let n = (((grid.retract_end - grid.retract_start).abs() / grid.retract_step).round() as i64
        + 1)
    .max(1);
    let dir = if grid.retract_end >= grid.retract_start {
        1.0
    } else {
        -1.0
    };
    (0..n)
        .map(|i| grid.retract_start + dir * i as f64 * grid.retract_step)
        .collect()
}

/// calib.cpp `stringing_grid_max_columns`: the columns at 12 mm that fit
/// the bed's width, less the margins and room for a prime tower.
pub fn max_columns(bed_width: f64) -> usize {
    let usable = bed_width - 2.0 * BED_MARGIN - PRIME_TOWER_SIZE;
    if usable < COLUMN_PITCH {
        return 0;
    }
    (usable / COLUMN_PITCH).floor() as usize
}

/// calib.cpp `stringing_grid_bed_width`: the printable area's width, 270
/// without one.
pub fn bed_width(settings: &Settings) -> f64 {
    match bed_box(settings) {
        Ok([x0, _, x1, _]) => x1 - x0,
        Err(_) => 270.0,
    }
}

/// calib.cpp `stringing_grid_max_fillet_radius`: 3 mm, which keeps the
/// foot on the plate.
pub fn max_fillet_radius() -> f64 {
    let y0 = (BASE_Y - PILLAR_GAP) * 0.5;
    (0.5 * (BASE_X - PILLAR_XY)).min(y0 - 0.5 * PILLAR_XY)
}

fn fillet_radius(params: &CalibParams) -> f64 {
    if !params.grid.fillet_base || params.grid.fillet_radius <= 0.0 {
        return 0.0;
    }
    params.grid.fillet_radius.min(max_fillet_radius())
}

/// What is wrong with the grid's values, as the reference says it: its
/// message id, and the value its `%1%` takes.
#[derive(Clone, Debug, PartialEq)]
pub struct GridError {
    pub message: &'static str,
    pub value: Option<String>,
}

fn fail(message: &'static str) -> Result<(), GridError> {
    Err(GridError {
        message,
        value: None,
    })
}

/// calib.cpp `stringing_grid_params_valid`, for a bed `bed_width` wide.
pub fn params_valid(params: &CalibParams, bed_width: f64) -> Result<(), GridError> {
    let (start, end, step) = (params.start, params.end, params.step);
    if !(150.0..=350.0).contains(&start) || !(150.0..=350.0).contains(&end) {
        return fail("Temperature must be between 150 and 350 C");
    }
    if end > start {
        return fail("Start temperature must be >= end temperature");
    }
    if !(1.0..=20.0).contains(&step) {
        return fail("Temperature step must be between 1 and 20 C");
    }
    let steps = (start - end) / step;
    if (steps - steps.round()).abs() > 1e-6 {
        return fail("The temperature range must be a multiple of the temperature step");
    }
    let bands = band_count(params);
    if !(2..=10).contains(&bands) {
        return fail("Band count must be between 2 and 10");
    }
    let band_h = band_height(params);
    if !(5.0..=20.0).contains(&band_h) {
        return fail("Band height must be between 5 and 20 mm");
    }
    if bands as f64 * band_h > 150.0 {
        return fail("Total height must be <= 150 mm");
    }
    let grid = &params.grid;
    if !(0.0..=10.0).contains(&grid.retract_start) || !(0.0..=10.0).contains(&grid.retract_end) {
        return fail("Retraction must be between 0 and 10 mm");
    }
    if grid.retract_end < grid.retract_start {
        return fail("Retraction end must be >= start");
    }
    if grid.retract_step <= 0.0 {
        return fail("Retraction step must be > 0");
    }
    if grid.fillet_base {
        let most = max_fillet_radius();
        if grid.fillet_radius < 0.5 - 1e-9 || grid.fillet_radius > most + 1e-9 {
            return Err(GridError {
                message: "Fillet radius must be between 0.5 and %1% mm",
                value: Some(written(most)),
            });
        }
    }
    let columns = retract_schedule(params).len();
    let most = max_columns(bed_width).min(17);
    if columns < 2 {
        return fail("Column count must be at least 2");
    }
    if columns > most {
        return Err(GridError {
            message: "Grid does not fit the bed (max %1% columns at 12 mm pitch)",
            value: Some(most.to_string()),
        });
    }
    Ok(())
}

/// The dialog's values when it opens (calib.cpp
/// `stringing_grid_default_params`): bands built outward from the
/// filament's temperature (`center`), up a step then down a step, within
/// its range and 150 to 350 °C (and the printer's most, when over 0), at
/// most 10; 230 to 205 when the filament's values do not make sense.
/// Says too whether the range holds fewer than two bands, when the
/// dialog asks for one.
pub fn default_params(
    center: f64,
    range_low: f64,
    range_high: f64,
    step: f64,
    printer_max: f64,
) -> (CalibParams, bool) {
    let mut params = CalibParams::new(
        CalibMode::StringingGrid,
        230.0,
        205.0,
        if step > 0.0 { step } else { 5.0 },
    );
    params.grid = GridParams {
        fillet_radius: max_fillet_radius(),
        ..GridParams::default()
    };
    let eps = 1e-6;
    let (abs_low, abs_high, max_bands) = (150.0, 350.0, 10);
    if center <= 0.0
        || range_low <= 0.0
        || range_high <= 0.0
        || range_low > range_high
        || center < range_low - eps
        || center > range_high + eps
    {
        return (params, false);
    }
    let mut upper = range_high.min(abs_high);
    if printer_max > 0.0 {
        upper = upper.min(printer_max);
    }
    if upper < abs_low {
        return (params, false);
    }
    let in_low = range_low.max(abs_low);
    if in_low > upper + eps {
        params.start = upper;
        params.end = upper;
        return (params, true);
    }
    let c = center.clamp(in_low, upper);
    let (mut hi, mut lo, mut bands) = (c, c, 1);
    let step = params.step;
    while bands < max_bands {
        let can_up = hi + step <= upper + eps;
        let can_down = lo - step >= in_low - eps;
        if !can_up && !can_down {
            break;
        }
        if can_up {
            hi += step;
            bands += 1;
            if bands >= max_bands {
                break;
            }
        }
        if can_down {
            lo -= step;
            bands += 1;
        }
    }
    params.start = hi;
    params.end = lo;
    if bands < 2 {
        return (params, true);
    }
    if params_valid(&params, 1000.0).is_err() {
        params.start = 230.0;
        params.end = 205.0;
    }
    (params, false)
}

/// calib.cpp `stringing_grid_column_name`.
pub fn column_name(column: usize) -> String {
    format!("SG_col_{column:02}")
}

/// The name of the tower of the bands' temperatures.
pub const LABEL_NAME: &str = "SG_label";

/// calib.cpp `stringing_grid_retract_label`: "0.2" … "9.8", from 10 mm
/// whole.
pub fn retract_label(length: f64) -> String {
    if length >= 10.0 - 1e-6 {
        format!("{}", length.round() as i64)
    } else {
        format!("{length:.1}")
    }
}

/// A box of the given size with its least corner at `at`, its corners in
/// single precision as the reference's `make_cube` and `translate` make
/// them, as a shell of its own beside the others (`TriangleMesh::merge`).
fn add_box(mesh: &mut Mesh, at: [f64; 3], size: [f64; 3]) {
    if size.iter().any(|s| *s <= 0.0) {
        return;
    }
    let low = at.map(|v| v as f32);
    let high: [f32; 3] = std::array::from_fn(|axis| size[axis] as f32 + at[axis] as f32);
    let shell = Mesh::from_triangle_soup(cuboid(low, high));
    mesh.append(&shell, &MeshTransform::IDENTITY);
}

/// The segments of each digit, A to G (calib.cpp `k_digit_seg`).
const DIGIT_SEGMENTS: [u8; 10] = [
    0b1111110, 0b0110000, 0b1101101, 0b1111001, 0b0110011, 0b1011011, 0b1011111, 0b1110000,
    0b1111111, 0b1111011,
];

/// calib.cpp `merge_digit`: a digit or a point of boxes, `t` thick.
fn add_digit(mesh: &mut Mesh, o: [f64; 3], ch: char, w: f64, h: f64, t: f64) {
    let s = LABEL_STROKE;
    let [ox, oy, oz] = o;
    if ch == '.' {
        add_box(mesh, [ox + 0.5 * (w - s), oy, oz], [s, s, t]);
        return;
    }
    let Some(digit) = ch.to_digit(10) else {
        return;
    };
    let bits = DIGIT_SEGMENTS[digit as usize];
    let mid = h * 0.5;
    let up = h - (mid - 0.5 * s);
    let low = mid + 0.5 * s;
    let segments: [(u8, [f64; 2], [f64; 2]); 7] = [
        (0b1000000, [ox, oy + h - s], [w, s]),
        (0b0100000, [ox + w - s, oy + mid - 0.5 * s], [s, up]),
        (0b0010000, [ox + w - s, oy], [s, low]),
        (0b0001000, [ox, oy], [w, s]),
        (0b0000100, [ox, oy], [s, low]),
        (0b0000010, [ox, oy + mid - 0.5 * s], [s, up]),
        (0b0000001, [ox, oy + mid - 0.5 * s], [w, s]),
    ];
    for (bit, [x, y], [sx, sy]) in segments {
        if bits & bit != 0 {
            add_box(mesh, [x, y, oz], [sx, sy, t]);
        }
    }
}

/// calib.cpp `text_width`.
fn text_width(text: &str) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    text.chars()
        .map(|ch| if ch == '.' { LABEL_STROKE } else { GLYPH_WIDTH } + GLYPH_GAP)
        .sum::<f64>()
        - GLYPH_GAP
}

/// calib.cpp `merge_text`: laid flat, along +X from `o`, up along +Y.
fn add_text(mesh: &mut Mesh, o: [f64; 3], text: &str, t: f64) {
    let mut x = o[0];
    for ch in text.chars() {
        let w = if ch == '.' { LABEL_STROKE } else { GLYPH_WIDTH };
        add_digit(mesh, [x, o[1], o[2]], ch, w, GLYPH_HEIGHT, t);
        x += w + GLYPH_GAP;
    }
}

/// calib.cpp `add_pillar`: the bands stacked, each but the last with a
/// notch at its top.
fn add_pillar(mesh: &mut Mesh, cx: f64, cy: f64, params: &CalibParams) {
    let bands = band_count(params);
    let band_h = band_height(params);
    let half = PILLAR_XY * 0.5;
    let inset = PILLAR_XY - 2.0 * NOTCH_DEPTH;
    for band in 0..bands {
        let z0 = BASE_HEIGHT + band as f64 * band_h;
        if band + 1 < bands && band_h > NOTCH_DEPTH {
            add_box(
                mesh,
                [cx - half, cy - half, z0],
                [PILLAR_XY, PILLAR_XY, band_h - NOTCH_DEPTH],
            );
            add_box(
                mesh,
                [
                    cx - inset * 0.5,
                    cy - inset * 0.5,
                    z0 + band_h - NOTCH_DEPTH,
                ],
                [inset, inset, NOTCH_DEPTH],
            );
        } else {
            add_box(
                mesh,
                [cx - half, cy - half, z0],
                [PILLAR_XY, PILLAR_XY, band_h],
            );
        }
    }
}

/// calib.cpp `make_stringing_grid_pillar_fillet_mesh`: a square loft
/// round the pillar's foot, following a quarter circle.
fn add_fillet(mesh: &mut Mesh, cx: f64, cy: f64, radius: f64) {
    if radius <= 0.0 {
        return;
    }
    let inset = 0.05;
    let half = 0.5 * PILLAR_XY - inset;
    let steps = 12;
    let mut rings = vec![(BASE_HEIGHT - LABEL_EMBED, half + radius)];
    for i in 0..=steps {
        let t = radius * f64::from(i) / f64::from(steps);
        let extra = radius
            - (radius * radius - (radius - t) * (radius - t))
                .max(0.0)
                .sqrt();
        rings.push((BASE_HEIGHT + t, half + extra));
    }
    let mut shell = Mesh::default();
    for (z, h) in &rings {
        for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            shell
                .vertices
                .push([(cx + dx * h) as f32, (cy + dy * h) as f32, *z as f32]);
        }
    }
    let n = rings.len() as u32;
    for r in 0..n - 1 {
        for j in 0..4 {
            let (a0, a1) = (4 * r + j, 4 * r + (j + 1) % 4);
            let (b0, b1) = (a0 + 4, a1 + 4);
            shell.triangles.push([a0, a1, b1]);
            shell.triangles.push([a0, b1, b0]);
        }
    }
    shell.triangles.push([0, 2, 1]);
    shell.triangles.push([0, 3, 2]);
    let top = 4 * (n - 1);
    shell.triangles.push([top, top + 1, top + 2]);
    shell.triangles.push([top, top + 2, top + 3]);
    mesh.append(&shell, &MeshTransform::IDENTITY);
}

/// calib.cpp `make_stringing_grid_column_mesh`.
pub fn column_mesh(params: &CalibParams, column: usize) -> Mesh {
    let mut mesh = Mesh::default();
    add_box(&mut mesh, [0.0; 3], [BASE_X, BASE_Y, BASE_HEIGHT]);
    let cx = BASE_X * 0.5;
    let y0 = (BASE_Y - PILLAR_GAP) * 0.5;
    add_pillar(&mut mesh, cx, y0, params);
    add_pillar(&mut mesh, cx, y0 + PILLAR_GAP, params);
    let radius = fillet_radius(params);
    if radius > 0.0 {
        add_fillet(&mut mesh, cx, y0, radius);
        add_fillet(&mut mesh, cx, y0 + PILLAR_GAP, radius);
    }
    if params.grid.print_labels {
        // calib.cpp `make_stringing_grid_column_label_mesh`.
        let schedule = retract_schedule(params);
        let length = schedule
            .get(column)
            .or(schedule.last())
            .copied()
            .unwrap_or(0.0);
        let label = retract_label(length);
        let ox = 0.5 * (BASE_X - text_width(&label));
        let oy = 0.5 * (BASE_Y - GLYPH_HEIGHT);
        add_text(
            &mut mesh,
            [ox, oy, BASE_HEIGHT - LABEL_EMBED],
            &label,
            LABEL_HEIGHT + LABEL_EMBED,
        );
    }
    mesh
}

/// calib.cpp `make_stringing_grid_label_tower_mesh`: a tower as tall as
/// the pillars, each band's temperature upright on its front face.
pub fn label_tower_mesh(params: &CalibParams, grid: &edge_gcode::StringingGrid) -> Mesh {
    let mut mesh = Mesh::default();
    let bands = band_count(params);
    let band_h = band_height(params);
    let height = BASE_HEIGHT + bands as f64 * band_h;
    add_box(&mut mesh, [0.0; 3], [LABEL_TOWER_X, LABEL_TOWER_Y, height]);
    if params.grid.print_labels {
        for band in 0..bands {
            // calib.cpp `make_stringing_grid_band_label_mesh`: the band of
            // the band's middle, as the G-code finds it.
            let z_mid = BASE_HEIGHT + (band as f64 + 0.5) * band_h;
            let label = format!("{}", grid.temperature(grid.band(z_mid)).round() as i64);
            let mut flat = Mesh::default();
            add_text(
                &mut flat,
                [0.0, 0.0, -LABEL_EMBED],
                &label,
                LABEL_HEIGHT + LABEL_EMBED,
            );
            // +90° about X: (x, y, z) to (x, -z, y), then placed.
            let ox = 0.5 * (LABEL_TOWER_X - text_width(&label));
            let oz = z_mid - 0.5 * GLYPH_HEIGHT;
            for v in &mut flat.vertices {
                *v = [
                    (f64::from(v[0]) + ox) as f32,
                    -v[2],
                    (f64::from(v[1]) + oz) as f32,
                ];
            }
            mesh.append(&flat, &MeshTransform::IDENTITY);
        }
    }
    mesh
}

/// What the writer follows (`edge_gcode::StringingGrid`).
pub fn for_writer(params: &CalibParams, settings: &Settings) -> edge_gcode::StringingGrid {
    edge_gcode::StringingGrid {
        start: params.start,
        end: params.end,
        bands: band_count(params),
        band_height: band_height(params),
        base_height: BASE_HEIGHT,
        retractions: retract_schedule(params),
        wait: params.grid.wait_for_temperature,
        flavor: settings.text("gcode_flavor").unwrap_or("marlin").to_owned(),
    }
}

/// "Stringing (temp × retraction)" (Plater.cpp `calib_temp_retraction`):
/// the columns 12 mm apart, the label tower before them when labelled,
/// the whole in the middle of the bed; the settings the grid needs and
/// nothing else (calib.cpp `apply_stringing_grid_slicer_config`), first
/// layer and layers of 0.2 mm, and on each object two walls, three bottom
/// layers and nothing on top or inside, printed with the first filament.
pub fn stringing_grid(
    params: &CalibParams,
    settings: &Settings,
    bed: Option<&Bed>,
) -> Result<CalibProject, Failure> {
    if let Err(error) = params_valid(params, bed_width(settings)) {
        let mut said = error.message.to_owned();
        if let Some(value) = error.value {
            said = said.replace("%1%", &value);
        }
        return Err(said.into());
    }
    let grid = for_writer(params, settings);
    let schedule = retract_schedule(params);
    // Each object: its name, its mesh and where its mesh's origin goes.
    let mut parts: Vec<(String, Mesh, f64)> = (0..schedule.len())
        .map(|column| {
            (
                column_name(column),
                column_mesh(params, column),
                column as f64 * COLUMN_PITCH,
            )
        })
        .collect();
    if params.grid.print_labels {
        parts.push((
            LABEL_NAME.to_owned(),
            label_tower_mesh(params, &grid),
            -COLUMN_PITCH,
        ));
    }
    // The group's box, its foot on the bed (`ensure_on_bed`), centred on
    // the bed.
    let mut low = [f64::MAX; 2];
    let mut high = [f64::MIN; 2];
    let mut feet = Vec::new();
    for (_, mesh, offset) in &parts {
        let (lo, hi) = mesh
            .bounds()
            .ok_or("the stringing grid has an empty part")?;
        low[0] = low[0].min(f64::from(lo[0]) + offset);
        low[1] = low[1].min(f64::from(lo[1]));
        high[0] = high[0].max(f64::from(hi[0]) + offset);
        high[1] = high[1].max(f64::from(hi[1]));
        feet.push([f64::from(lo[0]) + offset, f64::from(lo[1])]);
    }
    let area = bed_box(settings)?;
    let shift = [
        (area[0] + area[2]) / 2.0 - (low[0] + high[0]) / 2.0,
        (area[1] + area[3]) / 2.0 - (low[1] + high[1]) / 2.0,
    ];
    let mut model = Model::new();
    let mut moves = Vec::new();
    for ((name, mesh, _), foot) in parts.into_iter().zip(feet) {
        let copies = model.import(
            bed,
            &Model::from(crate::load::Opened::mesh(&name, mesh)),
            None,
        )?;
        let copy = copies[0];
        let Some((scene, Some(reach))) = model.in_scene(bed, copy) else {
            return Err("a column of the stringing grid has no printed material".into());
        };
        let to = Transform::translation([
            foot[0] + shift[0] - reach[0][0],
            foot[1] + shift[1] - reach[0][1],
            0.0,
        ]);
        moves.push((copy, scene.then(&to)));
    }
    model.set_transforms(bed, &moves)?;

    let mut process: Set = pairs(&[
        ("print_sequence", "by layer"),
        ("enable_prime_tower", "0"),
        ("wipe_inward", "0"),
        ("raft_layers", "0"),
        ("enable_support", "0"),
        ("reduce_crossing_wall", "0"),
    ]);
    let mut printer: Set = pairs(&[("use_firmware_retraction", "0")]);
    let mut filament: Set = Vec::new();
    if params.grid.disable_z_hop {
        let hops = settings.floats("z_hop")?.unwrap_or_default();
        let zeros = vec!["0"; hops.len().max(1)].join(",");
        printer.push(("z_hop".to_owned(), zeros));
        filament.push(("filament_z_hop".to_owned(), "0".to_owned()));
    }
    printer.extend(raised_max_layer_height(settings, 0.2)?);
    process.push(("initial_layer_print_height".to_owned(), "0.2".to_owned()));
    let own = [
        ("extruder", "1"),
        ("wall_loops", "2"),
        ("top_shell_layers", "0"),
        ("bottom_shell_layers", "3"),
        ("sparse_infill_density", "0%"),
        ("layer_height", "0.2"),
        ("alternate_extra_wall", "0"),
        ("seam_position", "aligned"),
        ("wall_sequence", "inner wall/outer wall"),
        ("overhang_reverse", "0"),
        ("enable_support", "0"),
        ("raft_layers", "0"),
    ];
    // The same on every object: one this engine does not follow set on an
    // object goes over the process, which gives the same print.
    let mut on_object: Vec<(String, Option<String>)> = Vec::new();
    for (key, value) in own {
        if crate::settable(key).followed_on_object {
            on_object.push((key.to_owned(), Some(value.to_owned())));
        } else if !process.iter().any(|(own, _)| own == key) {
            process.push((key.to_owned(), value.to_owned()));
        }
    }
    let objects: Vec<u64> = model.objects().iter().map(|object| object.id()).collect();
    for object in objects {
        model.set_object_settings(bed, object, &on_object)?;
    }
    Ok(CalibProject {
        name: "Stringing temp × retraction".to_owned(),
        model,
        process,
        filament,
        printer,
        params: Some(params.clone()),
        notes: Vec::new(),
    })
}
