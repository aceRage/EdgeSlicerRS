//! The idle nozzle's pre-cooling and pre-heating on a Bambu Lab printer of
//! two nozzles (the H2D), `enable_pre_heating` (GCode/PreCoolingInjector.cpp,
//! a port of Bambu Studio's GCodeProcessor `PreCoolingInjector` and the
//! usage blocks of `TimeProcessor::post_process`; GCodeProcessor.cpp
//! `plan_pre_cooling`).
//!
//! While one nozzle prints the other is idle: it is let cool right after
//! its last use (`M104 T<hotend> S<t> N0 C ;Multi extruder pre cooling`)
//! and heated back in time for its next one (`M632 S<filament> W`,
//! `M104 T<hotend> S<t> N0 W ;Multi extruder pre heating`, `M633`). Where
//! the lines go comes from the file's time, so this runs once the file is
//! written and timed, on the file as a whole; the lines are slipped in
//! after the line they belong after.
//!
//! Not done, as nothing here reaches it: the rack hotends of the H2C and
//! H3C (`handle_hotend_as_extruder`, refused before), and EdgeSlicer's
//! support interface temperature, which the reference pre-heats an idle
//! nozzle to (`Print::support_interface_temperatures`; not a setting this
//! engine follows).

use edge_config::{ConfigError, Settings};

use crate::NozzleMap;

/// What the pass reads of the settings (PreCoolingInjector.cpp `make_plan`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreHeating {
    /// Per filament: `nozzle_temperature`, `filament_pre_cooling_temperature`
    /// and `filament_preheat_temperature_delta`.
    pub temperatures: Vec<i32>,
    pub pre_cooling: Vec<i32>,
    pub preheat_delta: Vec<f64>,
    /// Per extruder, in degrees a second: `hotend_cooling_rate` and
    /// `hotend_heating_rate` at the extruder's variant; 2 for none or 0.
    pub cooling_rate: Vec<f64>,
    pub heating_rate: Vec<f64>,
    /// Per extruder: the hotend a temperature line names
    /// (`physical_extruder_map`), its nozzle count
    /// (`extruder_max_nozzle_count`) and whether it is a Bowden one
    /// (`extruder_type`).
    pub physical: Vec<usize>,
    pub nozzle_count: Vec<i64>,
    pub bowden: Vec<bool>,
    /// The first nozzle's diameter.
    pub first_nozzle: f64,
    /// `has_filament_switcher`.
    pub switcher: bool,
    /// Bambu Studio's time for a change: `machine_switch_extruder_time`,
    /// `machine_load_filament_time`, `machine_unload_filament_time`.
    pub switch_time: f64,
    pub load_time: f64,
    pub unload_time: f64,
}

impl PreHeating {
    /// The pass's settings on a Bambu Lab printer of two nozzles or more
    /// with `enable_pre_heating` on (`pre_cooling_active`); none otherwise.
    pub fn from_settings(settings: &Settings, bambu: bool) -> Result<Option<Self>, ConfigError> {
        let floats = |key: &str| -> Result<Vec<Option<f64>>, ConfigError> {
            Ok(settings.floats(key)?.unwrap_or_default())
        };
        let nozzles = floats("nozzle_diameter")?;
        if !bambu || settings.flag("enable_pre_heating")? != Some(true) || nozzles.len() < 2 {
            return Ok(None);
        }
        let filaments = settings
            .texts("filament_diameter")
            .map_or(1, <[String]>::len)
            .max(1);
        let at = |values: &[Option<f64>], index: usize, default: f64| {
            values
                .get(index)
                .or(values.last())
                .copied()
                .flatten()
                .unwrap_or(default)
        };
        let temperature = floats("nozzle_temperature")?;
        let cooling = floats("filament_pre_cooling_temperature")?;
        let delta = floats("filament_preheat_temperature_delta")?;
        let ids = floats("printer_extruder_id")?;
        let variants = settings
            .texts("printer_extruder_variant")
            .unwrap_or_default();
        let types = settings.texts("extruder_type").unwrap_or_default();
        let volumes = settings.texts("nozzle_volume_type").unwrap_or_default();
        // The entry of a per-variant printer list for extruder `e`
        // (PreCoolingInjector.cpp `extruder_variant_value`,
        // PrintConfig.cpp `printer_extruder_variant_slot`).
        let variant_value = |values: &[Option<f64>], e: usize| -> f64 {
            let fallback = 2.0;
            if values.is_empty() {
                return fallback;
            }
            if ids.len() == values.len() && variants.len() == ids.len() {
                let kind = if types.get(e).is_some_and(|t| t == "Bowden") {
                    "Bowden"
                } else {
                    "Direct Drive"
                };
                let volume = match volumes.get(e).map(String::as_str) {
                    None | Some("Hybrid") => "Standard",
                    Some(volume) => volume,
                };
                let wanted = format!("{kind} {volume}");
                let own = |i: &usize| ids[*i].is_some_and(|id| id as usize == e + 1);
                let slot = (0..ids.len())
                    .filter(own)
                    .find(|i| variants[*i] == wanted)
                    .or_else(|| (0..ids.len()).find(own));
                if let Some(slot) = slot {
                    return values[slot].unwrap_or(fallback);
                }
            }
            values[e.min(values.len() - 1)].unwrap_or(fallback)
        };
        let cooling_rates = floats("hotend_cooling_rate")?;
        let heating_rates = floats("hotend_heating_rate")?;
        let physical = floats("physical_extruder_map")?;
        let counts = floats("extruder_max_nozzle_count")?;
        let rate = |r: f64| if r > 0.0 { r } else { 2.0 };
        let single = |key: &str| -> Result<f64, ConfigError> {
            Ok(settings
                .floats(key)?
                .and_then(|values| values.first().copied().flatten())
                .unwrap_or(0.0))
        };
        Ok(Some(Self {
            temperatures: (0..filaments)
                .map(|f| at(&temperature, f, 0.0) as i32)
                .collect(),
            pre_cooling: (0..filaments)
                .map(|f| at(&cooling, f, 0.0) as i32)
                .collect(),
            preheat_delta: (0..filaments).map(|f| at(&delta, f, 0.0)).collect(),
            cooling_rate: (0..nozzles.len())
                .map(|e| rate(variant_value(&cooling_rates, e)))
                .collect(),
            heating_rate: (0..nozzles.len())
                .map(|e| rate(variant_value(&heating_rates, e)))
                .collect(),
            physical: (0..nozzles.len())
                .map(|e| {
                    physical
                        .get(e)
                        .copied()
                        .flatten()
                        .map_or(e, |p| p.max(0.0) as usize)
                })
                .collect(),
            nozzle_count: (0..nozzles.len())
                .map(|e| counts.get(e).copied().flatten().map_or(1, |c| c as i64))
                .collect(),
            bowden: (0..nozzles.len())
                .map(|e| types.get(e).is_some_and(|t| t == "Bowden"))
                .collect(),
            first_nozzle: nozzles.first().copied().flatten().unwrap_or(0.4),
            switcher: settings.flag("has_filament_switcher")? == Some(true),
            switch_time: single("machine_switch_extruder_time")?,
            load_time: single("machine_load_filament_time")?,
            unload_time: single("machine_unload_filament_time")?,
        }))
    }
}

/// The marker the pass starts from, written after the machine start
/// G-code, and the one it stops at, before the machine end G-code.
pub(crate) const START_END: &str = "; MACHINE_START_GCODE_END";
pub(crate) const END_START: &str = "; MACHINE_END_GCODE_START";

/// No line.
const NONE: usize = usize::MAX;

/// The nozzles of the grouping: on the H2D one to each extruder, so a
/// nozzle's number is its extruder's.
struct Group<'a> {
    map: &'a NozzleMap,
}

impl Group<'_> {
    fn nozzle_of(&self, filament: i32) -> i32 {
        if filament < 0 {
            return -1;
        }
        self.map
            .nozzles
            .get(filament as usize)
            .copied()
            .unwrap_or(-1)
    }
    fn extruder_of_nozzle(&self, nozzle: i32) -> Option<usize> {
        (nozzle >= 0).then_some(nozzle as usize)
    }
}

/// Where one filament printed: [lower, upper) by line.
#[derive(Clone, Copy, Debug)]
struct FilamentBlock {
    filament: i32,
    extruder: i32,
    nozzle: i32,
    lower: usize,
    upper: usize,
}

/// Where one extruder printed, between two nozzle changes
/// (`ExtruderUsageBlock`).
#[derive(Clone, Copy, Debug)]
struct ExtruderBlock {
    extruder: i32,
    start: usize,
    end: usize,
    start_filament: i32,
    end_filament: i32,
    start_nozzle: i32,
    end_nozzle: i32,
    post_start: usize,
    post_end: usize,
    ignore_cooling_before_tower: bool,
}

impl Default for ExtruderBlock {
    fn default() -> Self {
        Self {
            extruder: -1,
            start: NONE,
            end: NONE,
            start_filament: -1,
            end_filament: -1,
            start_nozzle: -1,
            end_nozzle: -1,
            post_start: NONE,
            post_end: NONE,
            ignore_cooling_before_tower: false,
        }
    }
}

impl ExtruderBlock {
    fn step_1(&mut self, extruder: i32, start: usize, filament: i32, nozzle: i32) {
        self.extruder = extruder;
        self.start = start;
        self.start_filament = filament;
        self.start_nozzle = nozzle;
    }
    fn step_3(&mut self, end: usize, filament: i32, post_end: usize, nozzle: i32) {
        self.end = end;
        self.end_filament = filament;
        self.post_end = post_end;
        self.end_nozzle = nozzle;
    }
}

/// An idle stretch of one extruder (`ExtruderFreeBlock`).
#[derive(Clone, Copy, Debug)]
struct FreeBlock {
    lower: usize,
    upper: usize,
    partial_lower: usize,
    partial_upper: usize,
    last_filament: i32,
    next_filament: i32,
    last_nozzle: i32,
    extruder: i32,
    ignore_cooling_before_tower: bool,
}

/// The filament and the H parameter of a `T` line's rest ("1 H-1"):
/// none for a filament past 254 (PreCoolingInjector.cpp
/// `parse_filament_and_nozzle`).
fn filament_and_nozzle(text: &str) -> Option<(i32, i32)> {
    let text = text.trim_start();
    let digits = text
        .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '+'))
        .unwrap_or(text.len());
    let filament: i32 = text[..digits].parse().ok()?;
    if !(0..255).contains(&filament) {
        return None;
    }
    let mut nozzle = -1;
    for word in text[digits..].split_whitespace() {
        if word.starts_with(';') {
            break;
        }
        if let Some(value) = word.strip_prefix('H') {
            let end = value
                .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '+'))
                .unwrap_or(value.len());
            nozzle = value[..end].parse().unwrap_or(-1);
            break;
        }
    }
    Some((filament, nozzle))
}

/// `OF<n> NF<n> ON<n> NN<n>` of a nozzle change's tag.
fn nozzle_change(tag: &str) -> Option<[i32; 4]> {
    let mut out = [-1; 4];
    for (slot, name) in ["OF", "NF", "ON", "NN"].iter().enumerate() {
        let at = tag.find(name)?;
        let rest = &tag[at + name.len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        out[slot] = rest[..end].parse().ok()?;
    }
    Some(out)
}

/// The command of a line: its first word, without leading blanks.
fn command(line: &str) -> &str {
    line.trim_start()
        .split(|c: char| c.is_whitespace() || c == ';')
        .next()
        .unwrap_or_default()
}

/// The usage blocks of the file (`UsageBlockBuilder`).
#[derive(Default)]
struct Usage {
    filament_blocks: Vec<FilamentBlock>,
    extruder_blocks: Vec<ExtruderBlock>,
    skippable: Vec<(usize, usize)>,
    start_end: usize,
    end_start: usize,
    pending: ExtruderBlock,
    layer: usize,
}

impl Usage {
    fn new() -> Self {
        Self {
            // The first use of an extruder has no nozzle change.
            extruder_blocks: vec![ExtruderBlock::default()],
            start_end: NONE,
            end_start: NONE,
            ..Self::default()
        }
    }

    fn filament_change(&mut self, group: &Group<'_>, filament: i32, line: usize, nozzle: i32) {
        if self.start_end == NONE || (self.end_start != NONE && line > self.end_start) {
            return;
        }
        if let Some(last) = self.filament_blocks.last_mut() {
            last.upper = line;
        }
        let nozzle = if nozzle == -1 {
            group.nozzle_of(filament)
        } else {
            nozzle
        };
        let extruder = group.extruder_of_nozzle(nozzle).map_or(0, |e| e as i32);
        self.filament_blocks.push(FilamentBlock {
            filament,
            extruder,
            nozzle,
            lower: line,
            upper: NONE,
        });
    }

    fn line(&mut self, group: &Group<'_>, raw: &str, line: usize) {
        let text = raw.trim_end_matches(['\n', '\r']).trim_start();
        if text.is_empty() {
            return;
        }
        if let Some(tag) = text.strip_prefix(';') {
            match tag {
                " MACHINE_START_GCODE_END" => self.start_end = line,
                " MACHINE_END_GCODE_START" => self.end_start = line,
                " SKIPPABLE_START" => self.skippable.push((line, 0)),
                " SKIPPABLE_END" => {
                    if let Some(last) = self.skippable.last_mut() {
                        last.1 = line;
                    }
                }
                _ => {
                    if let Some(rest) = tag.strip_prefix("VT") {
                        if let Some((filament, nozzle)) = filament_and_nozzle(rest) {
                            self.filament_change(group, filament, line, nozzle);
                        }
                    } else if tag.starts_with(" NOZZLE_CHANGE_START") {
                        if let Some(last) = self.extruder_blocks.last_mut() {
                            last.post_start = line;
                        }
                    } else if tag.starts_with(" NOZZLE_CHANGE_END") {
                        let [of, nf, on, nn] = nozzle_change(tag).unwrap_or([-1; 4]);
                        let extruder = group.extruder_of_nozzle(nn).map_or(-1, |e| e as i32);
                        if let Some(last) = self.extruder_blocks.last_mut() {
                            last.step_3(line, of, line, on);
                        }
                        let mut block = std::mem::take(&mut self.pending);
                        block.step_1(extruder, line, nf, nn);
                        // No pre-heat delta on the first layer (Bambu reads
                        // it from the tower's `CP_TOOLCHANGE_WIPE`, which is
                        // not written).
                        block.ignore_cooling_before_tower = self.layer <= 1;
                        self.extruder_blocks.push(block);
                    } else if let Some(rest) = tag.strip_prefix(" CP_TOOLCHANGE_WIPE") {
                        let value = |name: &str| {
                            rest.split_whitespace()
                                .find_map(|word| word.strip_prefix(name))
                                .and_then(|v| v.chars().next())
                        };
                        if let (Some(contact), Some(last)) =
                            (value("CT"), self.extruder_blocks.last_mut())
                        {
                            last.ignore_cooling_before_tower =
                                contact != '0' || value("FL").is_some_and(|first| first != '0');
                        }
                    } else if tag.starts_with(" CHANGE_LAYER") {
                        self.layer += 1;
                    }
                }
            }
            return;
        }
        if let Some(rest) = text.strip_prefix('T') {
            if let Some((filament, nozzle)) = filament_and_nozzle(rest) {
                self.filament_change(group, filament, line, nozzle);
            }
            return;
        }
        if command(text) == "M1020"
            && let Some(at) = text.find('S')
            && let Some((filament, nozzle)) = filament_and_nozzle(&text[at + 1..])
        {
            self.filament_change(group, filament, line, nozzle);
        }
    }

    fn finish(&mut self, group: &Group<'_>) {
        if let Some(last) = self.filament_blocks.last_mut() {
            last.upper = self.end_start;
        }
        let first = self.filament_blocks.first().map_or(0, |b| b.filament);
        let last = self.filament_blocks.last().map_or(0, |b| b.filament);
        let nozzle = group.nozzle_of(first);
        let extruder = group.extruder_of_nozzle(nozzle).map_or(-1, |e| e as i32);
        let start_end = self.start_end;
        let end_start = self.end_start;
        if let Some(front) = self.extruder_blocks.first_mut() {
            front.step_1(extruder, start_end, first, nozzle);
        }
        let last_nozzle = self.filament_blocks.last().map_or(-1, |b| b.nozzle);
        if let Some(back) = self.extruder_blocks.last_mut() {
            back.post_start = end_start;
            back.step_3(end_start, last, end_start, last_nozzle);
        }
    }
}

/// The idle stretches from the filament blocks, where no nozzle changes
/// (`build_by_filament_blocks`).
fn by_filament_blocks(usage: &Usage) -> Vec<FreeBlock> {
    let mut per: std::collections::BTreeMap<i32, Vec<FilamentBlock>> = Default::default();
    for block in &usage.filament_blocks {
        per.entry(block.extruder).or_default().push(*block);
    }
    let start = FilamentBlock {
        filament: -1,
        extruder: -1,
        nozzle: -1,
        lower: 0,
        upper: usage.start_end,
    };
    let end = FilamentBlock {
        filament: -1,
        extruder: -1,
        nozzle: -1,
        lower: usage.end_start,
        upper: NONE,
    };
    let mut free = Vec::new();
    for (extruder, blocks) in &mut per {
        blocks.insert(0, start);
        blocks.push(end);
        for pair in blocks.windows(2) {
            let (it, next) = (pair[0], pair[1]);
            free.push(FreeBlock {
                lower: it.upper,
                upper: next.lower,
                partial_lower: it.upper,
                partial_upper: it.upper,
                last_filament: it.filament,
                next_filament: next.filament,
                last_nozzle: if it.nozzle == -1 {
                    next.nozzle
                } else {
                    it.nozzle
                },
                extruder: *extruder,
                ignore_cooling_before_tower: true,
            });
        }
    }
    free.sort_by_key(|b| (b.lower, b.upper));
    free
}

/// The idle stretches between one extruder's uses (`build_by_extruder_blocks`).
fn by_extruder_blocks(usage: &Usage) -> Vec<FreeBlock> {
    let mut per: std::collections::BTreeMap<i32, Vec<ExtruderBlock>> = Default::default();
    for block in &usage.extruder_blocks {
        per.entry(block.extruder).or_default().push(*block);
    }
    let far = i32::MAX as usize;
    let mut free = Vec::new();
    for (extruder, blocks) in &mut per {
        let mut start = ExtruderBlock::default();
        start.step_1(*extruder, 0, -1, -1);
        start.post_start = usage.start_end;
        start.step_3(usage.start_end, -1, usage.start_end, -1);
        let mut end = ExtruderBlock::default();
        end.step_1(*extruder, usage.end_start, -1, -1);
        end.post_start = far;
        end.step_3(far, -1, far, -1);
        blocks.insert(0, start);
        blocks.push(end);
        for pair in blocks.windows(2) {
            let (it, next) = (pair[0], pair[1]);
            free.push(FreeBlock {
                lower: it.end,
                upper: next.start,
                partial_lower: it.post_start,
                partial_upper: it.post_end,
                last_filament: it.end_filament,
                next_filament: next.start_filament,
                last_nozzle: if it.end_nozzle == -1 {
                    next.start_nozzle
                } else {
                    it.end_nozzle
                },
                extruder: *extruder,
                ignore_cooling_before_tower: next.ignore_cooling_before_tower,
            });
        }
    }
    free.sort_by_key(|b| (b.lower, b.upper));
    free
}

/// The lines to slip in, each after the line it is keyed by (counting
/// from 0), for the file `text` whose lines took `by_line` seconds each
/// as its estimate has it (PreCoolingInjector.cpp, GCodeProcessor.cpp
/// `plan_pre_cooling`).
pub(crate) fn plan(
    text: &str,
    by_line: &[f64],
    params: &PreHeating,
    map: &NozzleMap,
) -> std::collections::BTreeMap<usize, Vec<String>> {
    let group = Group { map };
    let mut usage = Usage::new();
    // Each move's line and the seconds gone at its end. The estimate puts
    // a filament change's time on its `T` line as unload, load and tool
    // change; Bambu Studio books it as its own model has it (the extruder
    // switch on a change of extruder, unload and load when the nozzle
    // changes filament), on the same line, which is put in its place.
    let mut moves: Vec<(usize, f64)> = Vec::new();
    let mut elapsed = 0.0;
    let mut current_filament = -1;
    let mut current_extruder = -1;
    let mut unloaded = true;
    let mut nozzle_in_extruder: std::collections::BTreeMap<i32, i32> = Default::default();
    let mut filament_in_nozzle: std::collections::BTreeMap<i32, i32> = Default::default();
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let spent = by_line.get(index).copied().unwrap_or(0.0);
        let code = command(line);
        if matches!(code, "G0" | "G1" | "G2" | "G3") {
            elapsed += spent;
            moves.push((index, elapsed));
            continue;
        }
        usage.line(&group, line, index);
        let trimmed = line.trim_start();
        let booked = trimmed
            .strip_prefix('T')
            .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
            .and_then(filament_and_nozzle);
        let Some((filament, nozzle_id)) = booked else {
            elapsed += spent;
            continue;
        };
        // The estimate's own time for the change is left out.
        if filament == current_filament && nozzle_id == -1 {
            continue;
        }
        let nozzle = if nozzle_id >= 0 {
            nozzle_id
        } else {
            group.nozzle_of(filament)
        };
        let Some(extruder) = group.extruder_of_nozzle(nozzle).map(|e| e as i32) else {
            continue;
        };
        let mut seconds = 0.0;
        if extruder != current_extruder && current_extruder != -1 {
            seconds += params.switch_time;
        }
        let mounted = nozzle_in_extruder.get(&extruder).copied().unwrap_or(-1);
        let in_nozzle = filament_in_nozzle.get(&nozzle).copied().unwrap_or(-1);
        let in_extruder = filament_in_nozzle.get(&mounted).copied().unwrap_or(-1);
        if mounted != nozzle || in_nozzle != filament {
            if in_extruder >= 0 && !unloaded {
                seconds += params.unload_time;
            }
            unloaded = false;
            seconds += params.load_time;
        }
        current_filament = filament;
        current_extruder = extruder;
        nozzle_in_extruder.insert(extruder, nozzle);
        filament_in_nozzle.insert(nozzle, filament);
        elapsed += seconds;
    }
    usage.finish(&group);
    let mut out: std::collections::BTreeMap<usize, Vec<String>> = Default::default();
    if usage.start_end == NONE {
        return out;
    }
    let free = if usage.extruder_blocks.len() <= 1 {
        by_filament_blocks(&usage)
    } else {
        by_extruder_blocks(&usage)
    };
    let injector = Injector {
        moves: &moves,
        params,
        skippable: &usage.skippable,
        group: &group,
    };
    injector.process(&free, &mut out);
    out
}

struct Injector<'a> {
    moves: &'a [(usize, f64)],
    params: &'a PreHeating,
    skippable: &'a [(usize, usize)],
    group: &'a Group<'a>,
}

const ROOM_TEMPERATURE: f64 = 25.0;

impl Injector<'_> {
    /// The filament's temperature, less its pre-heat delta where asked;
    /// 140 or 0 for none (`get_nozzle_temp`).
    fn temperature(&self, filament: i32, from: bool, delta: bool) -> i32 {
        let p = self.params;
        match usize::try_from(filament)
            .ok()
            .and_then(|f| p.temperatures.get(f).map(|t| (f, *t)))
        {
            None => {
                if from {
                    140
                } else {
                    0
                }
            }
            Some((f, t)) if delta => {
                (f64::from(t) - p.preheat_delta.get(f).copied().unwrap_or(0.0)) as i32
            }
            Some((_, t)) => t,
        }
    }

    /// `process_pre_cooling_and_heating`.
    fn process(
        &self,
        free: &[FreeBlock],
        out: &mut std::collections::BTreeMap<usize, Vec<String>>,
    ) {
        let p = self.params;
        let mixed = p.bowden.windows(2).any(|pair| pair[0] != pair[1]);
        let offset = if p.first_nozzle >= 0.6 - 1e-4 {
            40.0
        } else {
            20.0
        };
        let mut per: std::collections::BTreeMap<i32, Vec<FreeBlock>> = Default::default();
        for block in free {
            per.entry(block.extruder).or_default().push(*block);
        }
        for blocks in per.values() {
            for (index, block) in blocks.iter().enumerate() {
                let is_end = index + 1 == blocks.len();
                let pre_cooling = true;
                let pre_heating = !is_end;
                let current = f64::from(self.temperature(block.last_filament, true, false));
                let mut target = f64::from(self.temperature(
                    block.next_filament,
                    false,
                    !block.ignore_cooling_before_tower,
                ));
                if p.switcher && mixed && pre_heating {
                    let print = f64::from(self.temperature(block.next_filament, false, false));
                    target = target.min(print - offset);
                }
                self.inject(out, block, current, target, pre_cooling, pre_heating);
            }
        }
    }

    /// Where a skippable block (the timelapse's) that holds `line` starts;
    /// 0 for none.
    fn skip_block_start(&self, line: usize) -> usize {
        let after = self.skippable.partition_point(|(start, _)| *start <= line);
        if after > 0 {
            let (start, end) = self.skippable[after - 1];
            if line >= start && line <= end {
                return start;
            }
        }
        0
    }

    /// The first move whose line is not before `line`.
    fn move_at(&self, line: usize) -> usize {
        self.moves.partition_point(|(id, _)| *id < line)
    }

    /// `inject_cooling_heating_command`.
    fn inject(
        &self,
        out: &mut std::collections::BTreeMap<usize, Vec<String>>,
        block: &FreeBlock,
        mut current: f64,
        target: f64,
        pre_cooling: bool,
        pre_heating: bool,
    ) {
        let p = self.params;
        let moves = self.moves;
        if !pre_cooling && !pre_heating && block.upper <= block.lower {
            return;
        }
        let lower = self.move_at(block.lower);
        let upper = self.move_at(block.upper);
        if lower == moves.len() || upper == 0 {
            return;
        }
        let upper = upper - 1;
        let gap = |lower: usize, upper: usize| {
            if lower == 0 {
                moves[upper].1
            } else {
                moves[upper].1 - moves[lower - 1].1
            }
        };
        let complete_gap = gap(lower, upper);
        let partial_lower = self.move_at(block.partial_lower);
        let partial_upper = self.move_at(block.partial_upper);
        if partial_lower == moves.len() || partial_upper == 0 {
            return;
        }
        let partial_gap = gap(partial_lower, partial_upper - 1);
        if lower >= upper {
            return;
        }
        let cooling_valid = usize::try_from(block.last_filament).is_ok_and(|f| {
            let cool = p.pre_cooling.get(f).copied().unwrap_or(0);
            cool > 0 && cool < p.temperatures.get(f).copied().unwrap_or(0)
        });
        let cool_partial = cooling_valid && pre_cooling;
        // `inject_time_threshold` is 0 (GCodeProcessor.cpp l.2750).
        let threshold = 0.0;
        if (cool_partial && partial_gap + complete_gap < threshold)
            || (!cool_partial && complete_gap < threshold)
        {
            return;
        }
        let extruder = (self.group)
            .extruder_of_nozzle(block.last_nozzle)
            .unwrap_or(0);
        let heating_rate = p.heating_rate.get(extruder).copied().unwrap_or(2.0);
        let cooling_rate = p.cooling_rate.get(extruder).copied().unwrap_or(2.0);
        let mut add =
            |line: usize, temperature: i32, skippable: bool, heating: bool, comment: &str| {
                let buffer = out.entry(line).or_default();
                if skippable {
                    let mut m632 = format!("M632 S{}", block.next_filament);
                    if p.nozzle_count.get(extruder).is_some_and(|count| *count > 1) {
                        m632.push_str(" N R");
                    }
                    m632.push_str(" W\n");
                    buffer.push(m632);
                }
                // Only a cooling waits for the moves before it.
                if !heating {
                    buffer.push("M400\n".to_owned());
                }
                let physical = p.physical.get(extruder).copied().unwrap_or(extruder);
                buffer.push(format!(
                    "M104 T{physical} S{temperature} N0 {} ;{comment}\n",
                    if heating { "W" } else { "C" }
                ));
                if skippable {
                    buffer.push("M633\n".to_owned());
                }
            };
        if cool_partial {
            let f = block.last_filament as usize;
            let threshold = f64::from(p.temperatures[f]) - f64::from(p.pre_cooling[f]);
            let most = current.min(threshold.min(partial_gap * cooling_rate));
            current = ROOM_TEMPERATURE.max(current - most);
            add(
                block.partial_lower,
                current as i32,
                false,
                false,
                "Multi extruder pre cooling in post extrusion",
            );
        }
        if pre_cooling && !pre_heating {
            if target >= current {
                return;
            }
            let clamped = (ROOM_TEMPERATURE as i32).max(target as i32);
            add(
                block.lower,
                clamped,
                false,
                false,
                "Multi extruder pre cooling",
            );
            return;
        }
        // The first move ending after `time`, among lower..=upper.
        let first_after =
            |time: f64| lower + moves[lower..=upper].partition_point(|(_, t)| *t <= time);
        // Back out of a skippable block: a heating never goes inside one.
        let back = |mut at: usize| -> usize {
            while at != lower {
                let start = self.skip_block_start(moves[at].0);
                if start == 0 {
                    break;
                }
                let new = lower + moves[lower..at].partition_point(|(id, _)| *id < start);
                if new == lower {
                    break;
                }
                at = new - 1;
            }
            at
        };
        if !pre_cooling && pre_heating {
            if target <= current {
                return;
            }
            let start = moves[upper].1 - (target - current) / heating_rate;
            let at = first_after(start);
            let line = if at == lower {
                block.lower
            } else {
                moves[back(at - 1)].0
            };
            add(
                line,
                target as i32,
                true,
                true,
                "Multi extruder pre heating",
            );
            return;
        }
        // Cool first, then heat in time for the next use.
        let middle = ROOM_TEMPERATURE.max(
            (current * heating_rate + target * cooling_rate
                - complete_gap * cooling_rate * heating_rate)
                / (cooling_rate + heating_rate),
        );
        let heating = target - middle;
        let start = moves[upper].1 - heating / heating_rate;
        let at = first_after(start);
        if at == lower {
            return;
        }
        let at = back(at - 1);
        // Where the heating really goes decides how long the nozzle cools.
        let cooling_time = moves[at].1 - moves[lower].1;
        let delta = ((cooling_time * cooling_rate) as i32).min(current as i32);
        if delta == 0 {
            return;
        }
        let cooled = (ROOM_TEMPERATURE as i32).max(current as i32 - delta);
        add(
            block.lower,
            cooled,
            false,
            false,
            "Multi extruder pre cooling",
        );
        add(
            moves[at].0,
            target as i32,
            true,
            true,
            "Multi extruder pre heating",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> PreHeating {
        PreHeating {
            temperatures: vec![220, 220],
            pre_cooling: vec![0, 0],
            preheat_delta: vec![10.0, 10.0],
            cooling_rate: vec![2.0, 2.0],
            heating_rate: vec![3.6, 3.6],
            physical: vec![1, 0],
            nozzle_count: vec![1, 1],
            bowden: vec![false, false],
            first_nozzle: 0.4,
            switcher: false,
            switch_time: 5.6,
            load_time: 26.0,
            unload_time: 26.0,
        }
    }

    #[test]
    fn settings_are_read_only_where_pre_heating_is_on() {
        let on = Settings::from_json(
            r#"{"enable_pre_heating": "1", "nozzle_diameter": ["0.4", "0.4"],
                "filament_diameter": ["1.75", "1.75"], "nozzle_temperature": ["220", "230"],
                "hotend_heating_rate": ["3.6", "3.6", "4", "4"], "printer_extruder_id": ["1", "1", "2", "2"],
                "printer_extruder_variant": ["Direct Drive Standard", "Direct Drive High Flow",
                    "Direct Drive Standard", "Direct Drive High Flow"],
                "nozzle_volume_type": ["Standard", "High Flow"], "physical_extruder_map": ["1", "0"]}"#,
        )
        .unwrap();
        let read = PreHeating::from_settings(&on, true).unwrap().unwrap();
        assert_eq!(read.temperatures, [220, 230]);
        // Each extruder's own variant: the right one is High Flow.
        assert_eq!(read.heating_rate, [3.6, 4.0]);
        assert_eq!(read.cooling_rate, [2.0, 2.0]);
        assert_eq!(read.physical, [1, 0]);
        assert!(PreHeating::from_settings(&on, false).unwrap().is_none());
        let one = Settings::from_json(r#"{"enable_pre_heating": "1", "nozzle_diameter": ["0.4"]}"#)
            .unwrap();
        assert!(PreHeating::from_settings(&one, true).unwrap().is_none());
    }

    #[test]
    fn tags_are_read() {
        assert_eq!(filament_and_nozzle("1 H-1"), Some((1, -1)));
        assert_eq!(filament_and_nozzle("0"), Some((0, -1)));
        assert_eq!(filament_and_nozzle("1001"), None);
        assert_eq!(
            nozzle_change(" NOZZLE_CHANGE_END OF1 NF2 ON1 NN0"),
            Some([1, 2, 1, 0])
        );
    }

    /// Filament 0 on the right nozzle (1), filament 1 on the left (0):
    /// the right nozzle idles between two long stretches of the left, is
    /// cooled as it is put away and heated back in time.
    #[test]
    fn an_idle_nozzle_is_cooled_and_heated_back_in_time() {
        let map = NozzleMap {
            extruders: vec![2, 1],
            nozzles: vec![1, 0],
            volumes: vec![0, 0],
        };
        let mut text =
            String::from("; MACHINE_START_GCODE_END\nT0 H-1\nG1 X1 E1\n; CHANGE_LAYER\n");
        text.push_str("; CHANGE_LAYER\n; NOZZLE_CHANGE_START OF0 NF1 ON1 NN0\nG1 X2 E1\n");
        text.push_str("; NOZZLE_CHANGE_END OF0 NF1 ON1 NN0\nT1 H-1\n");
        for _ in 0..100 {
            text.push_str("G1 X3 E1\n");
        }
        text.push_str("; NOZZLE_CHANGE_START OF1 NF0 ON0 NN1\nG1 X4 E1\n");
        text.push_str("; NOZZLE_CHANGE_END OF1 NF0 ON0 NN1\nT0 H-1\nG1 X5 E1\n");
        text.push_str("; MACHINE_END_GCODE_START\nG1 X6\n");
        let by_line: Vec<f64> = text
            .lines()
            .map(|line| if line.starts_with("G1") { 1.0 } else { 0.0 })
            .collect();
        let out = plan(&text, &by_line, &params(), &map);
        let lines: Vec<String> = out.values().flatten().cloned().collect();
        let cooling: Vec<&String> = lines.iter().filter(|l| l.contains("pre cooling")).collect();
        let heating: Vec<&String> = lines.iter().filter(|l| l.contains("pre heating")).collect();
        // The right nozzle (physical 0) cooled at the first change...
        assert!(cooling[0].starts_with("M104 T0 S"), "{lines:?}");
        // ...and heated back to 210 (220 less the delta, past the first
        // layer) inside its idle stretch, with the interlock round it.
        let at = lines
            .iter()
            .position(|l| l.contains("pre heating"))
            .unwrap();
        assert_eq!(
            heating[0],
            "M104 T0 S210 N0 W ;Multi extruder pre heating\n"
        );
        assert_eq!(lines[at - 1], "M632 S0 W\n");
        assert_eq!(lines[at + 1], "M633\n");
    }
}
