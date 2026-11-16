//! "Ironing flow" (calib.cpp `ironing_flow_calib_mesh` and its helpers,
//! Plater.cpp `calib_ironing_flow`, `ironing_flow_test_mesh`,
//! `update_ironing_flow_names`, calib_dlg.cpp `IroningFlow_Test_Dlg`): a
//! plate with two raised bands, ironed from the least flow at one end to
//! the most at the other (the band behind the other way round, to tell the
//! flow from a tilted bed), a ruler under the front band marked with the
//! flows, and the filament's name. The text is cut from the reference's
//! own font (`resources/fonts/HarmonyOS_Sans_SC_Bold.ttf`, found beside
//! the profiles as the calibration models are) by [`edge_text`], as the
//! reference's embosser cuts it. The ramp itself is the infill's
//! ([`edge_infill::FlowRamp`]), switched on by the calibration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use edge_config::Settings;
use edge_geom::{Coord, ExPolygon, Point2, Polygon};
use edge_mesh::Mesh;
use edge_text::{Font, FontProp, HorizontalAlign, VerticalAlign};

use super::{CalibMode, CalibParams, CalibProject, Set, bed_box, nozzle_diameter, pairs};
use crate::{Bed, Failure, Model, Transform};

/// The object's name, and the text of the name strip until the test is
/// sliced (Plater.cpp; "Not translated: the name identifies the test").
pub const OBJECT_NAME: &str = "Ironing flow test";
pub const PLACEHOLDER: &str = "Ironing flow calibration test";
/// The test's layer height, set on its object.
pub const LAYER_HEIGHT: f64 = 0.12;
/// The font the text is cut from, and the one for symbols it lacks.
pub const FONT: &str = "HarmonyOS_Sans_SC_Bold.ttf";
pub const FALLBACK_FONT: &str = "NotoSansSymbols2-Subset.ttf";

/// libslic3r.h `EPSILON`.
const EPSILON: f64 = 1e-4;
/// libslic3r.h `SCALING_FACTOR`.
const SCALING_FACTOR: f64 = 1e-6;

/// calib.cpp `ironing_flow_calib_mark_step`: the step between the
/// ruler's marks, the first of 1, 2, 5, 10 and 20 % that leaves 10 mm
/// between them; 0 when none does.
pub fn mark_step(min_flow: f64, max_flow: f64, length: f64) -> f64 {
    for step in [1.0, 2.0, 5.0, 10.0, 20.0] {
        if step * length / (max_flow - min_flow) >= 10.0 - EPSILON {
            return step;
        }
    }
    0.0
}

/// calib.cpp `ironing_flow_label`: the flow to two places, its trailing
/// zeros and point left off, and "%".
pub fn label(flow: f64) -> String {
    let mut s = format!("{:.2}", (flow * 100.0).round() / 100.0);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s + "%"
}

/// What is wrong with the dialog's values, as the reference says it
/// (`IroningFlow_Test_Dlg::on_start`): the message id and the values its
/// placeholders take, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogError {
    pub message: &'static str,
    pub values: Vec<i64>,
}

/// The dialog's checks, against the printer's nozzle and bed (min x, min
/// y, max x, max y of its printable area).
pub fn check(
    start: f64,
    end: f64,
    length: f64,
    nozzle: f64,
    bed: [f64; 4],
) -> Result<(), DialogError> {
    if !(start.is_finite() && end.is_finite() && length.is_finite())
        || start < 0.0
        || end <= start
        || end > 100.0
        || length < 10.0
    {
        return Err(DialogError {
            message: "Please input valid values:\n0 <= minimum < maximum <= 100\nlength >= 10 mm",
            values: Vec::new(),
        });
    }
    if mark_step(start, end, length) == 0.0 {
        return Err(DialogError {
            message: "Marks every 20%% would be less than 10 mm apart: use a length of at least %1% mm.",
            values: vec![((end - start) / 2.0).ceil() as i64],
        });
    }
    // "The band plus 5 mm at each end. In Y: two 15 mm bands and a 4 mm
    // valley, then in front of them the scale (2 mm marks, 1 mm gap,
    // labels up to 15 mm long), a 1 mm gap, the dividing line (nozzle
    // wide, at least 0.5 mm) and the 7 mm name strip"
    let whole = length + 10.0;
    let line_width = nozzle.max(0.5);
    let depth = 15.0 + 4.0 + 15.0 + (2.0 + 1.0 + 15.0) + 1.0 + line_width + 7.0;
    let (bed_x, bed_y) = (bed[2] - bed[0], bed[3] - bed[1]);
    if whole > bed_x - 10.0 {
        return Err(DialogError {
            message: "A band %1% mm long makes a test %2% mm long, which does not fit on the bed. Use at most %3% mm.",
            values: vec![length as i64, whole as i64, (bed_x - 20.0) as i64],
        });
    }
    if depth > bed_y - 10.0 {
        return Err(DialogError {
            message: "The two-band ironing flow test does not fit on the bed in Y. Use a printer with a deeper bed.",
            values: Vec::new(),
        });
    }
    Ok(())
}

/// The fonts, read once for each folder.
struct Fonts {
    font: Font,
    fallback: Option<Font>,
}

fn fonts(dir: &Path) -> Result<Arc<Fonts>, Failure> {
    static READ: OnceLock<Mutex<HashMap<PathBuf, Arc<Fonts>>>> = OnceLock::new();
    let mut read = READ
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(fonts) = read.get(dir) {
        return Ok(fonts.clone());
    }
    // The reference logs a font it cannot load and makes the test without
    // its text; here that is an error, as the labels are the test.
    let font = Font::open(&dir.join(FONT)).map_err(|e| -> Failure {
        format!("Ironing flow calibration: cannot load the font: {e}").into()
    })?;
    let fallback = Font::open(&dir.join(FALLBACK_FONT)).ok();
    let fonts = Arc::new(Fonts { font, fallback });
    read.insert(dir.to_owned(), fonts.clone());
    Ok(fonts)
}

/// `C++ coord_t(x * factor)`: scaled, truncated.
fn scale(shapes: &mut [ExPolygon], factor: f64) {
    for shape in shapes {
        for ring in std::iter::once(&mut shape.contour).chain(&mut shape.holes) {
            for p in &mut ring.points {
                p.x = (p.x as f64 * factor) as Coord;
                p.y = (p.y as f64 * factor) as Coord;
            }
        }
    }
}

/// `Point::new_scale`: mm to units, truncated.
fn new_scale(x: f64, y: f64) -> Point2 {
    Point2 {
        x: (x / SCALING_FACTOR) as Coord,
        y: (y / SCALING_FACTOR) as Coord,
    }
}

fn size_x(shapes: &[ExPolygon]) -> f64 {
    edge_text::extents(shapes).map_or(0.0, |(lo, hi)| (hi.x - lo.x) as f64 * SCALING_FACTOR)
}

/// The text maker of the test (calib.cpp `ironing_flow_text`).
struct Text<'a> {
    fonts: &'a Fonts,
    prop: FontProp,
    digit_height: f64,
}

impl<'a> Text<'a> {
    fn new(fonts: &'a Fonts) -> Result<Self, Failure> {
        let mut prop = FontProp::new(4.0);
        prop.horizontal = HorizontalAlign::Left;
        prop.vertical = VerticalAlign::Center;
        let mut text = Self {
            fonts,
            prop,
            digit_height: 1.0,
        };
        let zero = text.shapes_mm("0")?;
        text.digit_height =
            edge_text::extents(&zero).map_or(0.0, |(lo, hi)| (hi.y - lo.y) as f64 * SCALING_FACTOR);
        Ok(text)
    }

    fn shapes_mm(&self, s: &str) -> Result<Vec<ExPolygon>, Failure> {
        let mut shapes = edge_text::text_to_shapes(
            &self.fonts.font,
            self.fonts.fallback.as_ref(),
            s,
            &self.prop,
        )
        .map_err(|e| e.to_string())?;
        scale(
            &mut shapes,
            edge_text::shape_scale(&self.fonts.font, &self.prop) / SCALING_FACTOR,
        );
        Ok(shapes)
    }

    /// Text along +X with digits `height` high, shrunk to fit in
    /// `max_length`.
    fn text(&self, text: &str, height: f64, max_length: f64) -> Result<Vec<ExPolygon>, Failure> {
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let mut shapes = self.shapes_mm(text)?;
        let length = size_x(&shapes) / self.digit_height;
        let factor = if length * height > max_length {
            max_length / length
        } else {
            height
        } / self.digit_height;
        scale(&mut shapes, factor);
        Ok(shapes)
    }
}

/// Text reading towards -Y, starting at `top_y` (calib.cpp
/// `ironing_flow_text_down`).
fn text_down(mut text: Vec<ExPolygon>, center_x: f64, top_y: f64) -> Vec<ExPolygon> {
    // `rotate(-M_PI / 2.)`, rounded as MultiPoint::rotate rounds.
    let (s, c) = (-std::f64::consts::FRAC_PI_2).sin_cos();
    for shape in &mut text {
        for ring in std::iter::once(&mut shape.contour).chain(&mut shape.holes) {
            for p in &mut ring.points {
                let (x, y) = (p.x as f64, p.y as f64);
                *p = Point2 {
                    x: (c * x - s * y).round() as Coord,
                    y: (c * y + s * x).round() as Coord,
                };
            }
        }
    }
    if let Some((lo, hi)) = edge_text::extents(&text) {
        let to = new_scale(center_x, top_y);
        edge_text::translate(
            &mut text,
            Point2 {
                x: to.x - (lo.x + hi.x) / 2,
                y: to.y - hi.y,
            },
        );
    }
    text
}

fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
    ExPolygon {
        contour: Polygon {
            points: vec![
                new_scale(x0, y0),
                new_scale(x1, y0),
                new_scale(x1, y1),
                new_scale(x0, y1),
            ],
        },
        holes: Vec::new(),
    }
}

/// The shape raised from `z0` to `z1` (calib.cpp `ironing_flow_prism`),
/// its corners in single precision as the reference's mesh holds them.
fn prism(mut shape: Vec<ExPolygon>, z0: f64, z1: f64) -> Result<Mesh, Failure> {
    // "Scaled text can have duplicate points, which make non-manifold
    // edges."
    edge_text::heal(&mut shape, 10).map_err(|e| e.to_string())?;
    let mm = |p: &Point2| {
        [
            (p.x as f32) as f64 * SCALING_FACTOR,
            (p.y as f32) as f64 * SCALING_FACTOR,
        ]
        .map(|v| v as f32)
    };
    let regions: Vec<edge_mesh::primitives::FlatRegion> = shape
        .iter()
        .map(|region| {
            (
                region.contour.points.iter().map(mm).collect(),
                region
                    .holes
                    .iter()
                    .map(|hole| hole.points.iter().map(mm).collect())
                    .collect(),
            )
        })
        .collect();
    let depth = ((z1 - z0) / SCALING_FACTOR) as f32;
    let bottom = z0 as f32;
    let top = (f64::from(depth) * SCALING_FACTOR + z0) as f32;
    edge_mesh::primitives::prism(&regions, bottom, top)
        .ok_or_else(|| "the ironing flow test's shape could not be closed".into())
}

/// calib.cpp `ironing_flow_calib_mesh`: the test's mesh, its front left
/// corner of the bands at the origin.
#[allow(clippy::too_many_arguments)]
pub fn calib_mesh(
    min_flow: f64,
    max_flow: f64,
    length: f64,
    name: &str,
    nozzle_diameter: f64,
    base_z: f64,
    ruler_z: f64,
    top_z: f64,
    fonts_dir: &Path,
) -> Result<Mesh, Failure> {
    const SIDE: f64 = 5.0;
    const DEPTH: f64 = 15.0;
    const REAR_GAP: f64 = 4.0;
    const NAME_DEPTH: f64 = 7.0;
    const MARGIN: f64 = 1.5;
    const MARK_LENGTH: f64 = 2.0;
    const LABEL_GAP: f64 = 1.0;
    const LABEL_HEIGHT: f64 = 4.2;
    let fonts = fonts(fonts_dir)?;
    let text = Text::new(&fonts)?;
    let total_length = length + 2.0 * SIDE;
    let mark_width = nozzle_diameter.max(0.5);

    // Marks and their flows.
    let mut marks: Vec<Polygon> = Vec::new();
    let mut scale_depth = MARK_LENGTH;
    let mut add_label = |flow: f64, x: f64, marks: &mut Vec<Polygon>| -> Result<(), Failure> {
        let shapes = text.text(&label(flow), LABEL_HEIGHT, DEPTH)?;
        scale_depth = scale_depth.max(MARK_LENGTH + LABEL_GAP + size_x(&shapes));
        let down = text_down(shapes, x, -MARK_LENGTH - LABEL_GAP);
        marks.extend(down.iter().flat_map(|s| s.rings().cloned()));
        Ok(())
    };
    // "The minimum and maximum flows go below the side strips, without
    // marks."
    add_label(min_flow, SIDE / 2.0, &mut marks)?;
    add_label(max_flow, total_length - SIDE / 2.0, &mut marks)?;
    let step = mark_step(min_flow, max_flow, length);
    if step > 0.0 {
        let mut i = (min_flow / step - EPSILON).ceil() as i64;
        while i as f64 * step <= max_flow + EPSILON {
            let flow = i as f64 * step;
            let x = SIDE + (flow - min_flow) * length / (max_flow - min_flow);
            marks.push(
                rectangle(
                    x - mark_width / 2.0,
                    -MARK_LENGTH,
                    x + mark_width / 2.0,
                    0.0,
                )
                .contour,
            );
            // "No label where it would overlap the minimum or maximum flow."
            if (x - SIDE / 2.0).min(total_length - SIDE / 2.0 - x) >= LABEL_HEIGHT + LABEL_GAP {
                add_label(flow, x, &mut marks)?;
            }
            i += 1;
        }
    }

    // "Line between the scale and the filament name."
    let line_y = -scale_depth - LABEL_GAP;
    marks.push(rectangle(MARGIN, line_y - mark_width, total_length - MARGIN, line_y).contour);
    let front = scale_depth + LABEL_GAP + mark_width + NAME_DEPTH;
    let mut name_text = text.text(name, LABEL_HEIGHT, total_length - 2.0 * MARGIN)?;
    if let Some((lo, hi)) = edge_text::extents(&name_text) {
        let to = new_scale(total_length / 2.0, -front + NAME_DEPTH / 2.0);
        // `BoundingBox::center`, halved in integers.
        let centre = Point2 {
            x: (lo.x + hi.x) / 2,
            y: (lo.y + hi.y) / 2,
        };
        edge_text::translate(
            &mut name_text,
            Point2 {
                x: to.x - centre.x,
                y: to.y - centre.y,
            },
        );
        marks.extend(name_text.iter().flat_map(|s| s.rings().cloned()));
    }

    // "Front band: flow ramps left-to-right. Rear band: same height,
    // separated by a valley so make_ironing sees two top surfaces and can
    // reverse the ramp (bed-tilt check)."
    let front_band = rectangle(0.0, 0.0, total_length, DEPTH);
    let rear_band = rectangle(0.0, DEPTH + REAR_GAP, total_length, 2.0 * DEPTH + REAR_GAP);
    let plate = rectangle(0.0, -front, total_length, 2.0 * DEPTH + REAR_GAP);

    let mut ruler: Vec<Polygon> = vec![front_band.contour.clone(), rear_band.contour.clone()];
    ruler.extend(marks);
    let ruler = edge_clip::union(&ruler).map_err(|e| e.to_string())?;

    let mut mesh = prism(vec![plate], 0.0, base_z)?;
    for (shape, z0, z1) in [
        (ruler, base_z, ruler_z),
        (vec![front_band], ruler_z, top_z),
        (vec![rear_band], ruler_z, top_z),
    ] {
        mesh.append(&prism(shape, z0, z1)?, &edge_mesh::Transform::IDENTITY);
    }
    Ok(mesh)
}

/// Plater.cpp `ironing_flow_test_mesh`: the mesh for these values and
/// this name, for a printer with this nozzle and a process with this
/// first layer, on six solid layers and two of marks under the ironed
/// top, centred on its box as the reference centres a new volume, so that
/// a new name keeps the test in place.
pub fn test_mesh(
    params: &CalibParams,
    name: &str,
    nozzle_diameter: f64,
    first_layer: f64,
    fonts_dir: &Path,
) -> Result<Mesh, Failure> {
    let mut mesh = calib_mesh(
        params.start,
        params.end,
        params.step,
        name,
        nozzle_diameter,
        first_layer + 6.0 * LAYER_HEIGHT,
        first_layer + 8.0 * LAYER_HEIGHT,
        first_layer + 9.0 * LAYER_HEIGHT,
        fonts_dir,
    )?;
    let (lo, hi) = mesh.bounds().ok_or("the ironing flow test is empty")?;
    // `bounding_box().center()` in double, moved by it in single precision.
    let shift: [f32; 3] =
        std::array::from_fn(|axis| (-((f64::from(lo[axis]) + f64::from(hi[axis])) / 2.0)) as f32);
    for v in &mut mesh.vertices {
        for axis in 0..3 {
            v[axis] += shift[axis];
        }
    }
    Ok(mesh)
}

/// Plater.cpp `ironing_flow_filament_name`: the filament preset's name
/// without the printer it is for (from `@` on), trimmed.
pub fn filament_name(preset: &str) -> String {
    preset
        .split('@')
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn first_layer(settings: &Settings) -> Result<f64, Failure> {
    Ok(settings.float("initial_layer_print_height")?.unwrap_or(0.2))
}

/// Plater.cpp `calib_ironing_flow`: the test, its text the placeholder
/// until it is sliced, in the middle of the bed; over the process
/// `ironing_type` topmost, `ironing_pattern` rectilinear and the lines
/// along Y (`ironing_angle_fixed`, `ironing_angle` 90), and on the object
/// layers of 0.12 mm.
pub fn ironing_flow(
    params: &CalibParams,
    settings: &Settings,
    fonts_dir: &Path,
    bed: Option<&Bed>,
) -> Result<CalibProject, Failure> {
    let nozzle = nozzle_diameter(settings)?;
    if let Err(error) = check(
        params.start,
        params.end,
        params.step,
        nozzle,
        bed_box(settings)?,
    ) {
        return Err(error.message.into());
    }
    let mesh = test_mesh(
        params,
        PLACEHOLDER,
        nozzle,
        first_layer(settings)?,
        fonts_dir,
    )?;
    let mut model = Model::new();
    let copies = model.import(
        bed,
        &Model::from(crate::load::Opened::mesh(OBJECT_NAME, mesh)),
        None,
    )?;
    let copy = copies[0];
    let Some((scene, Some([low, high]))) = model.in_scene(bed, copy) else {
        return Err("the ironing flow test has no printed material".into());
    };
    // `load_mesh_object(…, center)`: its box's middle at the bed's.
    let area = bed_box(settings)?;
    let to = Transform::translation([
        (area[0] + area[2]) / 2.0 - (low[0] + high[0]) / 2.0,
        (area[1] + area[3]) / 2.0 - (low[1] + high[1]) / 2.0,
        -low[2],
    ]);
    model.set_transforms(bed, &[(copy, scene.then(&to))])?;
    let object = model.objects()[0].id();
    model.set_object_settings(
        bed,
        object,
        &[(
            "layer_height".to_owned(),
            Some(super::written(LAYER_HEIGHT)),
        )],
    )?;
    let process: Set = pairs(&[
        ("ironing_type", "topmost"),
        ("ironing_pattern", "rectilinear"),
        // "Ironing lines along Y, so that the flow changes from one line
        // to the next."
        ("ironing_angle_fixed", "1"),
        ("ironing_angle", "90"),
    ]);
    let mut params = params.clone();
    params.mode = CalibMode::IroningFlow;
    Ok(CalibProject {
        name: OBJECT_NAME.to_owned(),
        model,
        process,
        filament: Vec::new(),
        printer: Vec::new(),
        params: Some(params),
        notes: Vec::new(),
    })
}

/// Plater.cpp `update_ironing_flow_names`, as slicing calls it: every
/// ironing flow test of the model (an object of that name and one part)
/// given its filament's name, `filaments` being the presets' names of the
/// filaments from the first. Answers whether any changed.
pub fn name_for_slice(
    model: &mut Model,
    bed: Option<&Bed>,
    params: &CalibParams,
    settings: &Settings,
    filaments: &[String],
    fonts_dir: &Path,
) -> Result<bool, Failure> {
    if params.mode != CalibMode::IroningFlow {
        return Ok(false);
    }
    let nozzle = nozzle_diameter(settings)?;
    let first = first_layer(settings)?;
    let tests: Vec<(u64, usize)> = model
        .objects()
        .iter()
        .filter(|object| object.name() == OBJECT_NAME && object.parts() == 1)
        .map(|object| (object.id(), object.filament().unwrap_or(1)))
        .collect();
    let mut changed = false;
    for (object, filament) in tests {
        let name = filaments
            .get(filament.max(1) - 1)
            .map(|preset| filament_name(preset))
            .unwrap_or_default();
        let mesh = test_mesh(params, &name, nozzle, first, fonts_dir)?;
        let (part, now) = model.mesh_of_part(object, None)?;
        if now.vertices == mesh.vertices {
            continue;
        }
        model.set_part_mesh(bed, object, part, mesh, Default::default())?;
        changed = true;
    }
    Ok(changed)
}
