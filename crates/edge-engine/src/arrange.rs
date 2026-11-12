//! Arranging a model's copies with the reference's arranger
//! ([`edge_arrange`]), gathering what it needs as the reference's
//! `ArrangeJob` does (Jobs/ArrangeJob.cpp `prepare_all`,
//! `prepare_partplate`, `prepare_selected`, `prepare_wipe_tower`,
//! `init_arrange_params`, `process`, `finalize`; ModelArrange.cpp
//! `get_instance_arrange_poly`; PartPlate.cpp `preprocess_*` and
//! `postprocess_*`) and putting the result back as moves of the copies.

use std::collections::{BTreeMap, BTreeSet};

use edge_3mf::{PaintKind, PartKind};
use edge_arrange::{self as ea, Params, Piece, SupportKind};
use edge_config::Settings;
use edge_geom::{Point2, Polygon, scaled};
use edge_mesh::Transform;

use super::{Bed, MAX_PLATES, Model, ModelPlate, filaments_of, refused};
use crate::Failure;

/// The options the reference offers beside its Arrange button
/// (GLCanvas3D.cpp `_render_arrange_menu`, `ArrangeSettings`).
#[derive(Clone, Debug, PartialEq)]
pub struct ArrangeOptions {
    /// "Spacing" in mm; 0 is "auto".
    pub spacing: f64,
    /// "Auto rotate for arrangement".
    pub rotate: bool,
    /// "Allow multiple materials on same plate".
    pub multi_materials: bool,
    /// "Avoid extrusion calibration region" (offered for Bambu Lab
    /// printers that scan the first layer).
    pub avoid_calibration: bool,
    /// "Align to Y axis"; none for the printer's own (an i3's: on). Off
    /// whenever turning is on, as the reference disables it then.
    pub align_to_y: Option<bool>,
}

impl Default for ArrangeOptions {
    fn default() -> Self {
        Self {
            spacing: 0.0,
            rotate: false,
            multi_materials: true,
            avoid_calibration: true,
            align_to_y: None,
        }
    }
}

/// What is arranged.
#[derive(Clone, Copy, Debug)]
pub enum ArrangeScope<'a> {
    /// Every plate but the locked ones and those named
    /// (ArrangeJob.cpp `prepare_all`).
    All { leave: &'a [u32] },
    /// One plate, by number (`prepare_partplate`).
    Plate(u32),
    /// The copies named, around what else stands on their plates, which
    /// stays where it is (`prepare_selected`).
    Copies(&'a [u64]),
}

/// What came of arranging.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arranged {
    /// The names of the objects no plate could hold, put beside the plates.
    pub unplaced: Vec<String>,
    /// Objects not arranged because they have no size or are taller than
    /// the printer prints, left where they were.
    pub zero_size: Vec<String>,
    pub too_tall: Vec<String>,
    /// How many plates were added for what the others could not hold.
    pub plates_added: usize,
}

/// A print setting as the project or the presets have it, else the
/// reference's default.
struct Config<'a> {
    own: Option<&'a Settings>,
}

impl Config<'_> {
    fn texts(&self, key: &str) -> Vec<String> {
        self.own
            .and_then(|own| own.texts(key))
            .or_else(|| edge_config::reference_defaults().texts(key))
            .map(<[String]>::to_vec)
            .unwrap_or_default()
    }

    fn text(&self, key: &str) -> String {
        self.texts(key).into_iter().next().unwrap_or_default()
    }

    fn numbers(&self, key: &str) -> Vec<f64> {
        self.texts(key)
            .iter()
            .map(|text| {
                text.trim()
                    .trim_end_matches('%')
                    .parse::<f64>()
                    .unwrap_or(0.0)
            })
            .collect()
    }

    fn number(&self, key: &str) -> f64 {
        self.numbers(key).first().copied().unwrap_or(0.0)
    }

    fn number_at(&self, key: &str, at: usize) -> f64 {
        let values = self.numbers(key);
        values.get(at).or(values.first()).copied().unwrap_or(0.0)
    }

    fn flag(&self, key: &str) -> bool {
        matches!(self.text(key).trim(), "1" | "true")
    }

    fn is_percent(&self, key: &str) -> bool {
        self.text(key).trim().ends_with('%')
    }
}

/// The object's own setting over the print's (`get_config_value`).
fn own_or(object: &[(String, String)], config: &Config<'_>, key: &str) -> String {
    object
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| config.text(key))
}

fn flag_of(text: &str) -> bool {
    matches!(text.trim(), "1" | "true")
}

/// The first-layer bed temperature key for a bed type
/// (PrintConfig.cpp `get_bed_temp_1st_layer_key`).
fn first_layer_bed_key(bed_type: &str) -> Option<&'static str> {
    Some(match bed_type {
        "Cool Plate" => "cool_plate_temp_initial_layer",
        "Engineering Plate" => "eng_plate_temp_initial_layer",
        "High Temp Plate" => "hot_plate_temp_initial_layer",
        "Textured PEI Plate" => "textured_plate_temp_initial_layer",
        "Textured Cool Plate" => "textured_cool_plate_temp_initial_layer",
        "Supertack Plate" => "supertack_plate_temp_initial_layer",
        _ => return None,
    })
}

/// The skirt's distance from the objects as the arranger keeps it
/// (PrintConfig.cpp `get_real_skirt_dist`): its distance and its loops.
fn skirt_distance(config: &Config<'_>) -> f64 {
    let draft_shield = config.text("draft_shield");
    let shield = !draft_shield.is_empty() && draft_shield != "disabled";
    let (height, mut loops) = (config.number("skirt_height"), config.number("skirt_loops"));
    if !((height > 0.0 && loops > 0.0) || shield) {
        return 0.0;
    }
    if shield && loops == 0.0 {
        loops = 1.0;
    }
    let mut width = if config.is_percent("initial_layer_line_width") {
        0.0
    } else {
        config.number("initial_layer_line_width")
    };
    if width <= 0.0 {
        width = config.number("line_width");
    }
    if width <= 0.0 {
        width = config
            .numbers("nozzle_diameter")
            .into_iter()
            .fold(0.0, f64::max);
    }
    if width <= 0.0 {
        width = 0.4;
    }
    config.number("skirt_distance") + loops * width
}

/// The extruder's clearance radius printing by object
/// (PrintConfig.cpp `sequential_clearance_radius`).
fn clearance_radius(config: &Config<'_>) -> f64 {
    let max = config.number("extruder_clearance_max_radius");
    if config.text("printer_model").starts_with("Bambu Lab") && max > 0.0 {
        return max;
    }
    config.number("extruder_clearance_radius")
}

/// How far a by-object skirt reaches past the objects beyond the
/// clearance (Print.cpp `Print::object_skirt_offset`), its line width
/// taken as the first layer's.
fn object_skirt_offset(config: &Config<'_>, all_short: bool) -> f64 {
    let loops = config.number("skirt_loops");
    if loops == 0.0 || config.text("skirt_type") != "perobject" {
        return 0.0;
    }
    let nozzle = config
        .numbers("nozzle_diameter")
        .into_iter()
        .fold(0.0, f64::max);
    let width = if config.is_percent("initial_layer_line_width") {
        nozzle * config.number("initial_layer_line_width") / 100.0
    } else {
        config.number("initial_layer_line_width")
    }
    .max(if nozzle > 0.0 { nozzle } else { 0.4 });
    let skirt_width = loops * width;
    let distance = config.number("skirt_distance");
    let radius = clearance_radius(config);
    let max_layer = config
        .numbers("max_layer_height")
        .into_iter()
        .fold(0.0, f64::max);
    if all_short {
        distance + skirt_width
    } else if config.text("draft_shield") == "enabled"
        || config.number("skirt_height") * max_layer > config.number("nozzle_height")
    {
        distance + width
    } else if distance + skirt_width > radius / 2.0 {
        distance + skirt_width - radius / 2.0
    } else {
        0.0
    }
}

/// The least depth a tower as tall as `height` mm should have to stand
/// (WipeTower.cpp `get_limit_depth_by_height`).
fn least_tower_depth(height: f64) -> f64 {
    if height <= 100.0 {
        20.0
    } else if height >= 250.0 {
        40.0
    } else {
        20.0 + (height - 100.0) / 150.0 * 20.0
    }
}

/// The ground a prime tower takes on a plate, for `filaments` filaments
/// and objects up to `height` mm tall, as `[min_x, min_y, max_x, max_y]`
/// in mm on the plate (PartPlate.cpp `estimate_wipe_tower_polygon`, with
/// the size of WipeTowerEstimate.cpp `estimate_wipe_tower_footprint` for a
/// tower that purges the prime volume at each change: a box wall `width`
/// wide and as deep as the purges need, or a rib wall squared to them).
fn tower_box(
    config: &Config<'_>,
    plate: usize,
    filaments: usize,
    height: f64,
    bed: &Bed,
) -> [f64; 4] {
    let width = config.number("prime_tower_width").max(1.0);
    let mut layer = config.number("layer_height");
    let first = config.number("initial_layer_print_height");
    if first > 0.0 {
        layer = layer.min(first);
    }
    let layer = layer.max(0.01);
    let spacing = config.number("wipe_tower_extra_spacing").max(50.0) / 100.0;
    let volume = config.number("prime_volume") * filaments.saturating_sub(1) as f64;
    let least = least_tower_depth(height);
    let (wide, deep) = if config.text("wipe_tower_wall_type") == "rib" {
        // `WipeTower::rib_footprint_side`.
        let square = (volume / layer * spacing).sqrt();
        if square <= 0.0 {
            (least, least)
        } else {
            let diagonal = (2.0 * square * square).sqrt();
            let mut rib = diagonal;
            if square < least {
                rib = rib.max(least * 2f64.sqrt());
            }
            rib = diagonal.max(rib + config.number("wipe_tower_extra_rib_length"));
            let rib_width = config.number("wipe_tower_rib_width").min(square / 2.0);
            let side = square + 2.0 * (((rib - diagonal) / 2.0 + rib_width / 2.0) / 2f64.sqrt());
            (side, side)
        }
    } else {
        (width, (volume / (layer * width) * spacing).max(least))
    };
    // The brim as laid (`WipeTower::estimate_brim_real_width`, a box
    // tower's lines).
    let mut brim = config.number("prime_tower_brim_width");
    if brim < 0.0 {
        brim = if height < 100.0 {
            height / 100.0 * 8.0
        } else {
            8.0
        };
    }
    let nozzle = config.number("nozzle_diameter").max(0.1);
    let line = nozzle * 1.25 - first.max(layer) * (1.0 - std::f64::consts::FRAC_PI_4);
    if brim > 0.0 && line > 0.0 {
        brim = ((brim + line / 2.0) / line).floor() * line;
    }
    let brim = brim.max(0.0);
    let (size_x, size_y) = (bed.max[0] - bed.min[0], bed.max[1] - bed.min[1]);
    // Kept on the bed, a millimetre in, as the reference keeps a tower
    // the person has not placed (WIPE_TOWER_MARGIN).
    let margin = 1.0 + brim;
    let place = |at: f64, size: f64, room: f64| {
        let hi = (room - size - margin).max(margin);
        at.clamp(margin, hi)
    };
    let x = place(config.number_at("wipe_tower_x", plate), wide, size_x);
    let y = place(config.number_at("wipe_tower_y", plate), deep, size_y);
    [
        bed.min[0] + x - brim,
        bed.min[1] + y - brim,
        bed.min[0] + x + wide + brim,
        bed.min[1] + y + deep + brim,
    ]
}

fn rectangle(b: [f64; 4]) -> Polygon {
    Polygon {
        points: vec![
            Point2::from_mm(b[0], b[1]),
            Point2::from_mm(b[2], b[1]),
            Point2::from_mm(b[2], b[3]),
            Point2::from_mm(b[0], b[3]),
        ],
    }
}

/// The boxes round the bed's excluded areas (`bed_exclude_area`, as
/// PartPlate.cpp keeps `m_exclude_bounding_box`), in mm.
fn excluded_boxes(config: &Config<'_>) -> Vec<[f64; 4]> {
    let points: Vec<(f64, f64)> = config
        .texts("bed_exclude_area")
        .iter()
        .flat_map(|text| text.split(','))
        .filter_map(|point| {
            let (x, y) = point.trim().split_once('x')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect();
    if points.len() < 3 {
        return Vec::new();
    }
    let (mut b, mut any) = ([f64::MAX, f64::MAX, f64::MIN, f64::MIN], false);
    for (x, y) in points {
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        any = true;
    }
    if any && b[2] > b[0] && b[3] > b[1] {
        vec![b]
    } else {
        Vec::new()
    }
}

/// One copy as the arranger takes it, with where it is in the model.
struct Gathered {
    at: (usize, usize),
    copy: u64,
    /// Where it stood when its outline was taken: plates added for the
    /// arrangement may take a copy that stood beside the plates onto
    /// themselves, which changes how it is measured, not where it is.
    local: Transform,
    piece: Piece,
}

impl Model {
    /// The copy at `at` as the arranger takes it (`get_instance_arrange_poly`):
    /// its outline on its plate's bed, what it prints with and the room
    /// kept round it for supports.
    fn piece_of(&self, at: (usize, usize), config: &Config<'_>, bed_type: &str) -> Piece {
        let object = &self.objects[at.0];
        let copy = &object.copies[at.1];
        let local = copy.local;
        let printed: Vec<_> = object.shape.printed().collect();
        let outline = ea::outline(printed.iter().flat_map(|part| {
            let placed = part.transform.then(&local);
            part.mesh
                .vertices
                .iter()
                .map(move |v| placed.apply(v.map(f64::from)))
        }));
        let own = object.settings();
        // Filaments: the parts', what is painted on them, then supports'.
        let mut filaments: Vec<i32> = filaments_of(std::iter::once(&*object.shape))
            .into_iter()
            .map(|f| f as i32)
            .collect();
        let support = flag_of(&own_or(&own, config, "enable_support"));
        let number = |key: &str| {
            own_or(&own, config, key)
                .trim()
                .parse::<f64>()
                .unwrap_or(0.0)
        };
        if support {
            for key in ["support_filament", "support_interface_filament"] {
                let filament = number(key) as i32;
                if filament > 0 {
                    filaments.push(filament);
                }
            }
        } else {
            let stabilizer = own_or(&own, config, "stabilizer_supports");
            if !stabilizer.is_empty() && stabilizer != "off" && stabilizer != "0" {
                let filament = number("support_filament") as i32;
                if filament > 0 {
                    filaments.push(filament);
                }
            }
        }
        if filaments.is_empty() {
            filaments.push(1);
        }
        let first = filaments[0].max(1) as usize - 1;
        let first_bed_temp = first_layer_bed_key(bed_type)
            .map(|key| config.number_at(key, first) as i32)
            .unwrap_or(0);
        let types = config.texts("filament_type");
        let filament_temp_type = ea::compatible_filament_type(filaments.iter().map(|f| {
            let at = (*f).max(1) as usize - 1;
            ea::filament_temp_type(types.get(at).or(types.first()).map_or("", String::as_str))
        }));
        // The room for supports, where supports would be printed.
        let kind = SupportKind::of(&own_or(&own, config, "support_type"));
        let supported = support && {
            let painted = printed
                .iter()
                .any(|part| part.has_paint(PaintKind::Supports))
                || object
                    .shape
                    .parts
                    .iter()
                    .any(|part| part.kind == PartKind::SupportEnforcer);
            ea::may_get_support(
                printed.iter().flat_map(|part| {
                    let placed = part.transform.then(&local);
                    part.mesh.triangles.iter().map(move |t| {
                        t.map(|v| placed.apply(part.mesh.vertices[v as usize].map(f64::from)))
                    })
                }),
                kind,
                number("support_threshold_angle"),
                painted,
            )
        };
        let (brim_width, has_tree_support) = ea::support_allowance(supported.then_some(kind));
        let height = copy.reach.map_or(0.0, |[low, high]| high[2] - low[2]);
        Piece {
            outline,
            filaments,
            filament_temp_type,
            first_bed_temp,
            bed_temp: first_bed_temp,
            height,
            brim_width,
            has_tree_support,
            name: object.name.clone(),
            ..Piece::default()
        }
    }

    /// The plates' own print sequence, else the project's.
    fn sequence_of(&self, plate: usize, config: &Config<'_>) -> String {
        self.plates[plate]
            .settings()
            .print_sequence
            .unwrap_or_else(|| config.text("print_sequence"))
    }

    /// Arranges what `scope` names with the reference's arranger, with
    /// the print's settings (`settings`: the project's and the presets'
    /// as they would be sliced; none for the model's own) and the
    /// options beside the Arrange button. Each copy is turned only about
    /// the vertical; what does not fit goes on the next plate, plates
    /// being added as needed, and what fits no plate is put beside them.
    pub fn arrange_with(
        &mut self,
        bed: Option<&Bed>,
        settings: Option<&Settings>,
        options: &ArrangeOptions,
        scope: ArrangeScope<'_>,
    ) -> Result<Arranged, Failure> {
        let bed =
            *bed.ok_or_else(|| refused("no printer is chosen: there is no bed to arrange on"))?;
        let own = self.underlying().cloned();
        let config = Config {
            own: settings.or(own.as_deref()),
        };
        let only = match scope {
            ArrangeScope::Plate(id) => {
                let at = self
                    .plate_index(id)
                    .ok_or_else(|| refused(format!("there is no plate {id}")))?;
                if self.plates[at].locked() {
                    return Err(refused(
                        "This plate is locked.\nCannot auto-arrange on this plate.",
                    ));
                }
                Some(at)
            }
            _ => None,
        };
        let chosen: BTreeSet<u64> = match scope {
            ArrangeScope::Copies(copies) => {
                for copy in copies {
                    self.copy_at(*copy)?;
                }
                copies.iter().copied().collect()
            }
            _ => BTreeSet::new(),
        };
        self.settle(Some(&bed));
        let global_sequence = config.text("print_sequence");

        // The plates arranged onto, in order (the reference's beds): every
        // plate but the locked ones and those left, or the one named.
        let leave: &[u32] = match scope {
            ArrangeScope::All { leave } => leave,
            _ => &[],
        };
        let bins: Vec<usize> = match only {
            Some(at) => vec![at],
            None => (0..self.plates.len())
                .filter(|&p| !self.plates[p].locked() && !leave.contains(&self.plates[p].id))
                .collect(),
        };
        let bin_of = |plate: Option<usize>| plate.and_then(|p| bins.iter().position(|&b| b == p));

        let is_seq = match only {
            Some(at) => self.sequence_of(at, &config) == "by object",
            None => global_sequence == "by object",
        };
        let curr_bed_type = config.text("curr_bed_type");

        let mut selected: Vec<Gathered> = Vec::new();
        let mut fixed: Vec<Piece> = Vec::new();
        let mut outcome = Arranged::default();
        let mut locked_chosen = false;
        for o in 0..self.objects.len() {
            for c in 0..self.objects[o].copies.len() {
                let copy = &self.objects[o].copies[c];
                let (id, printable) = (copy.id, copy.printable);
                let plate = self.plate_of(id);
                let bed_type = plate
                    .and_then(|p| self.plates[p].settings().bed_type)
                    .unwrap_or_else(|| curr_bed_type.clone());
                let is_selected = match scope {
                    ArrangeScope::All { .. } => plate.is_none() || bin_of(plate).is_some(),
                    ArrangeScope::Plate(_) => plate == only,
                    ArrangeScope::Copies(_) => {
                        if !chosen.contains(&id) {
                            false
                        } else if plate.is_some() && bin_of(plate).is_none() {
                            locked_chosen = true;
                            false
                        } else {
                            true
                        }
                    }
                };
                if !printable {
                    // Set not to print: left where it is.
                    continue;
                }
                if is_selected {
                    let piece = self.piece_of((o, c), &config, &bed_type);
                    selected.push(Gathered {
                        at: (o, c),
                        copy: id,
                        local: self.objects[o].copies[c].local,
                        piece,
                    });
                } else if matches!(scope, ArrangeScope::Copies(_))
                    && let Some(bin) = bin_of(plate)
                {
                    // What else stands on a plate arranged onto stays.
                    let mut piece = self.piece_of((o, c), &config, &bed_type);
                    piece.bed = bin as i32;
                    fixed.push(piece);
                }
            }
        }
        if selected.is_empty() {
            return Err(refused(
                if locked_chosen
                    || (matches!(scope, ArrangeScope::All { .. }) && bins.len() < self.plates.len())
                {
                    "All the selected objects are on a locked plate.\nCannot auto-arrange these objects."
                } else {
                    "No arrangeable objects are selected."
                },
            ));
        }
        // What has no size or is taller than the printer prints is not
        // arranged (`check_unprintable`).
        let printable_height = config.number("printable_height");
        selected.retain(|g| {
            let area = g.piece.outline.signed_area_mm2().abs();
            if area < 0.001 {
                outcome.zero_size.push(g.piece.name.clone());
                false
            } else if g.piece.height > printable_height && printable_height > 0.0 {
                outcome.too_tall.push(g.piece.name.clone());
                false
            } else {
                true
            }
        });
        if selected.is_empty() {
            return Ok(outcome);
        }

        // The parameters (`init_arrange_params`).
        let mut params = Params {
            clearance_height_to_rod: config.number("extruder_clearance_height_to_rod"),
            clearance_height_to_lid: config.number("extruder_clearance_height_to_lid"),
            printable_height,
            allow_rotations: options.rotate,
            nozzle_height: config.number("nozzle_height"),
            allow_multi_materials_on_same_plate: options.multi_materials,
            avoid_extrusion_cali_region: options.avoid_calibration,
            is_seq_print: is_seq,
            min_obj_distance: scaled(options.spacing.max(0.0)),
            align_to_y_axis: !options.rotate
                && options
                    .align_to_y
                    .unwrap_or_else(|| config.text("printer_structure") == "i3"),
            ..Params::default()
        };
        let pieces: Vec<Piece> = selected.iter().map(|g| g.piece.clone()).collect();
        let all_short = pieces.iter().all(|p| p.height < params.nozzle_height);
        params.object_skirt_offset = object_skirt_offset(&config, all_short);
        params.clearance_radius = clearance_radius(&config) + params.object_skirt_offset * 2.0;
        let best = config.texts("best_object_pos");
        let best: Vec<f64> = best
            .iter()
            .flat_map(|t| t.split(','))
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        if best.len() == 2 {
            params.align_center = [best[0], best[1]];
        }
        if let Some(at) = only
            && self.sequence_of(at, &config) != global_sequence
        {
            // A plate printed its own way: the spacing set for the
            // project's way no longer holds, so it is "auto".
            params.min_obj_distance = 0;
        }
        if is_seq {
            params.bed_shrink_x = ea::BED_SHRINK_SEQ_PRINT;
            params.bed_shrink_y = ea::BED_SHRINK_SEQ_PRINT;
        }

        // The prime tower's ground on each bed (`prepare_wipe_tower`; for
        // one plate, its tower when it has one).
        let tower_enabled = config.flag("enable_prime_tower");
        let smooth = config.text("timelapse_type") == "1";
        let filaments_on = |plate: usize| -> Vec<usize> {
            filaments_of(self.plates[plate].order.iter().filter_map(|copy| {
                let (o, c) = self.find(*copy)?;
                self.objects[o].copies[c]
                    .printable
                    .then_some(&*self.objects[o].shape)
            }))
        };
        let tallest = pieces.iter().map(|p| p.height).fold(0.0, f64::max);
        if tower_enabled && !is_seq {
            let need = smooth
                || pieces
                    .iter()
                    .any(|p| p.filaments.iter().collect::<BTreeSet<_>>().len() > 1)
                || (params.allow_multi_materials_on_same_plate && {
                    let mut by_temp: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
                    for p in &pieces {
                        by_temp
                            .entry(p.bed_temp)
                            .or_default()
                            .extend(p.filaments.iter().copied());
                    }
                    by_temp.values().any(|ids| ids.len() > 1)
                });
            let all_filaments = self.filaments().len();
            let beds = if only.is_some() {
                1
            } else {
                ea::MAX_NUM_PLATES
            };
            for b in 0..beds {
                let plate = bins.get(b).copied();
                let there = plate.map(filaments_on).unwrap_or_default().len();
                // A plate with a tower already keeps it; another gets one
                // where the print will need one.
                let count = if there > 1 {
                    Some(there)
                } else if need && only.is_none() {
                    Some(all_filaments.max(2))
                } else {
                    None
                };
                let Some(count) = count else { continue };
                let index = plate.unwrap_or(self.plates.len() + b);
                let b_box = tower_box(&config, index, count, tallest, &bed);
                fixed.push(Piece {
                    outline: rectangle(b_box),
                    bed: b as i32,
                    is_virtual: true,
                    is_wipe_tower: true,
                    priority: 1,
                    name: "WipeTower".into(),
                    ..Piece::default()
                });
            }
        }
        // The bed's excluded areas on every bed, and once more for the
        // selection to try a large item against.
        for (k, area) in excluded_boxes(&config).iter().enumerate() {
            let beds = if only.is_some() {
                1
            } else {
                ea::MAX_NUM_PLATES
            };
            for b in 0..beds {
                fixed.push(Piece {
                    outline: rectangle(*area),
                    bed: b as i32,
                    is_virtual: true,
                    height: 1.0,
                    name: format!("ExcludedRegion{k}"),
                    ..Piece::default()
                });
            }
            params.excluded_regions.push(Piece {
                outline: rectangle(*area),
                is_virtual: true,
                height: 1.0,
                inflation: scaled(1.0),
                name: format!("ExcludedRegion{k}"),
                ..Piece::default()
            });
        }
        // The extrusion calibration strip of Bambu Lab printers that scan
        // the first layer (`preprocess_nonprefered_areas`).
        if options.avoid_calibration
            && config.text("printer_model").starts_with("Bambu Lab")
            && config.flag("scan_first_layer")
        {
            let beds = if only.is_some() {
                1
            } else {
                ea::MAX_NUM_PLATES
            };
            for b in 0..beds {
                fixed.push(Piece {
                    outline: rectangle([18.0, 0.0, 240.0, 15.0]),
                    bed: b as i32,
                    is_virtual: true,
                    is_extrusion_cali: true,
                    height: 1.0,
                    name: "NonpreferedRegion0".into(),
                    ..Piece::default()
                });
            }
        }

        let mut pieces = pieces;
        ea::update_arrange_params(&mut params, skirt_distance(&config), &pieces);
        let shrunk = ea::shrunk_bed([bed.min[0], bed.min[1], bed.max[0], bed.max[1]], &params);
        ea::update_selected_items_inflation(&mut pieces, shrunk, &mut params);
        ea::update_unselected_items_inflation(&mut fixed, &params);
        ea::update_selected_items_axis_align(&mut pieces, &params);
        ea::arrange(&mut pieces, &fixed, shrunk, &params);

        // Each bed's plate: the plates arranged onto, then plates added
        // for the beds beyond them (the reference, arranging one plate,
        // puts every bed beyond the first on one plate's place after the
        // last, PartPlate.cpp l.5646, where they overlap).
        let mut plate_of_bin: Vec<usize> = bins.clone();
        let beds_used = pieces.iter().map(|p| p.bed + 1).max().unwrap_or(0).max(0) as usize;
        while plate_of_bin.len() < beds_used && self.plates.len() < MAX_PLATES {
            let id = self.add_plate(Some(&bed))?;
            outcome.plates_added += 1;
            plate_of_bin.push(self.plate_index(id).unwrap_or(self.plates.len() - 1));
        }
        let mut placed: Vec<Vec<(i32, u64)>> = vec![Vec::new(); self.plates.len()];
        let mut unplaced: Vec<usize> = Vec::new();
        for (k, (g, piece)) in selected.iter().zip(&pieces).enumerate() {
            let Some(&plate) = usize::try_from(piece.bed)
                .ok()
                .and_then(|bin| plate_of_bin.get(bin))
            else {
                outcome.unplaced.push(piece.name.clone());
                unplaced.push(k);
                continue;
            };
            let (s, c) = piece.rotation.sin_cos();
            let turn = Transform {
                rows: [
                    [c, s, 0.0],
                    [-s, c, 0.0],
                    [0.0, 0.0, 1.0],
                    [
                        piece.translation.x as f64 / 1e6,
                        piece.translation.y as f64 / 1e6,
                        0.0,
                    ],
                ],
            };
            let local = g.local.then(&turn);
            self.assign(g.copy, Some(plate));
            self.stand(g.at, Some(plate), local);
            placed[plate].push((piece.item_id, g.copy));
        }
        // What no plate holds goes beside the plates, one after another.
        let (ax, ay) = bed.aside(self.plates.len());
        let mut along = 0.0;
        for k in unplaced {
            let g = &selected[k];
            let copy = &self.objects[g.at.0].copies[g.at.1];
            let [low, high] = copy.reach.unwrap_or([[0.0; 3]; 2]);
            let local = copy.local.then(&Transform::translation([
                ax + bed.min[0] + along - low[0],
                ay + bed.min[1] - low[1],
                0.0,
            ]));
            along += high[0] - low[0] + super::APART;
            self.assign(g.copy, None);
            self.stand(g.at, None, local);
        }
        // Each plate lists what was placed on it in the order it was
        // placed, after what stayed (the reference sorts its objects by
        // `arrange_order`).
        for (plate, mut list) in placed.into_iter().enumerate() {
            if list.is_empty() || plate >= self.plates.len() {
                continue;
            }
            list.sort_by_key(|(order, _)| *order);
            let moved: BTreeSet<u64> = list.iter().map(|(_, copy)| *copy).collect();
            let held: &mut ModelPlate = &mut self.plates[plate];
            held.order.retain(|copy| !moved.contains(copy));
            held.order.extend(list.into_iter().map(|(_, copy)| copy));
        }
        // Arranging every plate takes away the empty plates at the end
        // but the first, passing over locked ones
        // (`rebuild_plates_after_arrangement`). The reference takes away a
        // locked plate that is empty too; here the person's lock keeps it.
        if matches!(scope, ArrangeScope::All { .. }) {
            let mut at = self.plates.len();
            while at > 1 {
                at -= 1;
                if self.plates[at].locked() {
                    continue;
                }
                if !self.plates[at].order.is_empty() {
                    break;
                }
                let id = self.plates[at].id;
                self.remove_plate(Some(&bed), id)?;
            }
        }
        Ok(outcome)
    }
}

impl Model {
    /// A copy's outline on the bed, as the arranger takes it: the convex
    /// hull of its printed parts on the ground, counter-clockwise, in mm,
    /// in its plate's coordinates (for a copy on no plate, the scene's).
    pub fn outline(&self, copy: u64) -> Option<Vec<[f64; 2]>> {
        let (o, c) = self.find(copy)?;
        let object = &self.objects[o];
        let local = object.copies[c].local;
        let hull = ea::outline(object.shape.printed().flat_map(|part| {
            let placed = part.transform.then(&local);
            part.mesh
                .vertices
                .iter()
                .map(move |v| placed.apply(v.map(f64::from)))
        }));
        Some(
            hull.points
                .iter()
                .map(|p| [p.x as f64 / 1e6, p.y as f64 / 1e6])
                .collect(),
        )
    }
}
