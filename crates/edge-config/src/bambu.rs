//! "Export Bambu 3MF": a project's settings, its presets and its objects'
//! settings rewritten into the shape Bambu Studio reads (the reference's
//! `Format/BambuExport.cpp` and `BambuKeyAliases.cpp`, ported).
//!
//! Bambu Studio reads a project's settings only from a file whose
//! `Application` starts with "BambuStudio-", and then drops every key its
//! own options do not declare and refuses values that do not fit their
//! types. So, for every key:
//!
//! 1. renamed where Bambu spells the same setting otherwise
//!    (`infill_anchor` is `sparse_infill_anchor`), the value converted
//!    where the two write it differently;
//! 2. left out where Bambu has no such setting;
//! 3. reshaped to Bambu's type: a list to its first entry, one value to a
//!    list, a fraction rounded to a whole number, "mm or %" resolved
//!    against what the percentage is of;
//! 4. an enum value translated (`rectilinear` is `zig-zag`), or the key
//!    left out where Bambu has no such value, so Bambu keeps its own;
//! 5. checked against Bambu's type, and left out when it does not fit;
//! 6. lists kept per extruder variant laid out as Bambu indexes them.
//!
//! Nothing is made up: what cannot be said in Bambu's terms is left out
//! and reported. The tables are the reference's (`known.rs`, written by
//! tools/make_bambu_keys.py).

use std::collections::{BTreeMap, BTreeSet};

use crate::{Settings, Value};

#[rustfmt::skip]
mod known;

/// How Bambu sizes a list (`VariantClass`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// One entry per extruder or filament, or not per anything.
    None,
    /// One per `print_extruder_variant` entry.
    Print,
    /// One per `filament_extruder_variant` entry.
    Filament,
    /// One per `printer_extruder_variant` entry.
    Printer,
    /// A (normal, silent) pair per `printer_extruder_variant` entry.
    PrinterPair,
    /// One per extruder (`nozzle_diameter`'s length).
    Extruder,
}

/// One option Bambu Studio keeps (`BambuKeyDef`).
#[derive(Clone, Copy, Debug)]
pub struct Def {
    pub key: &'static str,
    /// Bambu's `ConfigOptionType`, such as "coFloats".
    pub kind: &'static str,
    pub nullable: bool,
    pub variant: Variant,
    /// The enum values it takes, joined by "|"; none for one not an enum.
    pub enums: Option<&'static str>,
}

/// Where a set of settings is written (`Scope`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// `Metadata/project_settings.config`, the whole settings.
    Project,
    /// A process preset inside the project.
    Print,
    /// A filament preset inside the project (one filament).
    Filament,
    /// A printer preset inside the project.
    Printer,
    /// An object's, a part's or a height range's own settings.
    Object,
}

/// One value in Bambu's shape, each entry as it is written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BambuValue {
    pub vector: bool,
    /// A text option (`coString`, `coStrings`): written escaped.
    pub strings: bool,
    pub values: Vec<String>,
}

/// Settings in Bambu's shape, by Bambu's names.
pub type Config = BTreeMap<String, BambuValue>;

/// What an export changed (`Report`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Our keys Bambu cannot take, by our names.
    pub dropped: BTreeSet<String>,
    /// Our key to Bambu's.
    pub renamed: BTreeMap<String, String>,
    /// Bambu keys whose value was reshaped.
    pub converted: BTreeSet<String>,
    /// One line for each change worth saying, for the log.
    pub notes: Vec<String>,
}

impl Report {
    pub fn merge(&mut self, other: &Report) {
        self.dropped.extend(other.dropped.iter().cloned());
        for (ours, theirs) in &other.renamed {
            self.renamed
                .entry(ours.clone())
                .or_insert_with(|| theirs.clone());
        }
        self.converted.extend(other.converted.iter().cloned());
        self.notes.extend(other.notes.iter().cloned());
    }

    /// The reference's log summary.
    pub fn summary(&self) -> String {
        format!(
            "{} setting(s) not supported by Bambu Studio left out, {} renamed, {} converted",
            self.dropped.len(),
            self.renamed.len(),
            self.converted.len()
        )
    }
}

/// A per-extruder-variant layout: slot i is `(names[i], ids[i])`, ids from
/// 1 (an extruder for print and printer lists, a filament for filament
/// lists).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub names: Vec<String>,
    pub ids: Vec<i64>,
}

impl Layout {
    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// What the conversions of one project need to know of it (`Context`).
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub nozzle_diameter: f64,
    pub extruder_count: usize,
    pub filament_count: usize,
    pub process_flow_support: Vec<String>,
    /// The project's options that are numbers.
    pub numbers: BTreeMap<String, Vec<f64>>,
    /// The raw values of the layout keys.
    pub lists: BTreeMap<String, Vec<String>>,
    /// Bambu's layouts of the converted project ([`convert_project`]).
    pub print: Layout,
    pub printer: Layout,
    pub filament: Layout,
}

// ---------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------

/// Keys Bambu reads differently present but empty than absent: absent, it
/// works the nozzles out from the printer (`OMIT_WHEN_EMPTY`).
const OMIT_WHEN_EMPTY: [&str; 2] = ["extruder_nozzle_stats", "extruder_nozzle_stats_new"];

/// The keys of the variant layout, written from the layout worked out
/// (`LAYOUT_KEYS`).
const LAYOUT_KEYS: [&str; 6] = [
    "print_extruder_variant",
    "print_extruder_id",
    "printer_extruder_variant",
    "printer_extruder_id",
    "filament_extruder_variant",
    "filament_self_index",
];

/// Options whose "%" is of something their own definition does not name
/// (`EXTRA_RATIO_OVER`).
const EXTRA_RATIO_OVER: [(&str, &str); 1] = [("seam_gap", "nozzle_diameter")];

/// The Bambu Studio line the tables were made from.
pub fn reference_version() -> &'static str {
    known::REFERENCE_VERSION
}

/// The version written into a Bambu export (`export_version`): the
/// major.minor line of the Bambu Studio the tables were made from, patch
/// 0 ("02.08.00.00"), so a Bambu Studio of the same line opens it without
/// a word and an older one says it is newer.
pub fn export_version() -> String {
    let parts: Vec<&str> = known::REFERENCE_VERSION.split('.').collect();
    if parts.len() < 2 {
        return known::REFERENCE_VERSION.to_owned();
    }
    format!("{}.{}.00.00", parts[0], parts[1])
}

/// The `Application` of a Bambu export (`application_tag`).
pub fn application_tag() -> String {
    format!("BambuStudio-{}", export_version())
}

/// Bambu's option of that name; none for one Bambu Studio drops when it
/// reads a project (`find_key`).
pub fn find_key(bambu_key: &str) -> Option<&'static Def> {
    known::KEYS
        .binary_search_by(|def| def.key.cmp(bambu_key))
        .ok()
        .map(|at| &known::KEYS[at])
}

/// The renamed setting of ours, if `our_key` is one.
fn alias_of(
    our_key: &str,
) -> Option<&'static (
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
)> {
    known::ALIASES.iter().find(|alias| alias.0 == our_key)
}

/// Bambu's name for one of our keys (`bambu_key_name`).
pub fn bambu_key_name(our_key: &str) -> &str {
    alias_of(our_key).map_or(our_key, |alias| alias.1)
}

/// The renamed settings: ours and Bambu's.
pub fn renamed() -> impl Iterator<Item = (&'static str, &'static str)> {
    known::ALIASES.iter().map(|alias| (alias.0, alias.1))
}

fn ratio_over(our_key: &str) -> Option<&'static str> {
    if let Some((_, over)) = EXTRA_RATIO_OVER.iter().find(|(key, _)| *key == our_key) {
        return Some(over);
    }
    known::RATIO_OVER
        .iter()
        .find(|(key, _)| *key == our_key)
        .map(|(_, over)| *over)
        .filter(|over| !over.is_empty())
}

// ---------------------------------------------------------------------
// Value conversions tied to a rename (BambuKeyAliases.cpp): one entry each
// ---------------------------------------------------------------------

/// Carries out the reference's conversion of one entry named `function`;
/// false where the value has no Bambu equivalent.
fn to_bambu(function: &str, value: &mut String) -> bool {
    match function {
        // only_one_wall_top -> top_one_wall_type.
        "one_wall_top_to_bambu" => {
            *value = if value == "1" { "all top" } else { "not apply" }.into();
            true
        }
        // reduce_infill_retraction -> reduce_infill_retraction_mode.
        "reduce_retraction_to_bambu" => {
            *value = if value == "1" { "Enabled" } else { "Disabled" }.into();
            true
        }
        // wipe_tower_wall_type -> prime_tower_rib_wall: Bambu has no cone.
        "tower_wall_to_bambu" => {
            *value = if value == "rib" { "1" } else { "0" }.into();
            true
        }
        // A negative angle is "the default" here; Bambu has no such value.
        "ironing_angle_to_bambu" => number(value).is_some_and(|angle| angle >= 0.0),
        // filament_colour_mode (0 split, 1 gradient) -> filament_colour_type
        // (0 gradient, 1 single or split).
        "colour_mode_to_bambu" => {
            *value = if value == "1" { "0" } else { "1" }.into();
            true
        }
        // "#RRGGBB|#RRGGBB" -> "#RRGGBB #RRGGBB".
        "multi_colours_to_bambu" => {
            *value = value
                .split('|')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            true
        }
        other => {
            unreachable!("no conversion {other}: tools/make_bambu_keys.py names one not ported")
        }
    }
}

// ---------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------

/// A number as `strtod` reads the whole of it; none for anything else.
fn number(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // strtod reads "inf" and "nan" too; neither is finite.
    let value: f64 = text.parse().ok()?;
    value.is_finite().then_some(value)
}

/// A number with a "%" after it allowed: the number, and whether it had.
fn number_or_percent(text: &str) -> Option<(f64, bool)> {
    let text = text.trim();
    match text.strip_suffix('%') {
        Some(rest) => number(rest).map(|value| (value, true)),
        None => number(text).map(|value| (value, false)),
    }
}

fn is_integer(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty() && text.parse::<i64>().is_ok()
}

/// `%.10g`: ten significant digits, without trailing zeros.
pub fn format_number(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.into();
    }
    let exponent = value.abs().log10().floor() as i32;
    // Rounding to ten digits may carry into the next power of ten.
    let scientific = format!("{value:.9e}");
    let (mantissa, power) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let power: i32 = power.parse().unwrap_or(exponent);
    if !(-4..10).contains(&power) {
        let mantissa = trim_zeros(mantissa);
        let sign = if power < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{:02}", power.abs());
    }
    let decimals = usize::try_from(9 - power).unwrap_or(0);
    trim_zeros(&format!("{value:.decimals$}"))
}

fn trim_zeros(text: &str) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text.to_owned()
    }
}

// ---------------------------------------------------------------------
// Reading our settings
// ---------------------------------------------------------------------

fn read_value(value: &Value) -> BambuValue {
    match value {
        Value::One(text) => BambuValue {
            vector: false,
            strings: false,
            values: vec![text.clone()],
        },
        Value::Many(items) => BambuValue {
            vector: true,
            strings: false,
            values: items.clone(),
        },
    }
}

/// Bambu's type name as its base kind and whether it is a list.
fn type_info(kind: &str) -> (&'static str, bool) {
    match kind {
        "coFloat" => ("Float", false),
        "coFloats" => ("Float", true),
        "coInt" => ("Int", false),
        "coInts" => ("Int", true),
        "coString" => ("String", false),
        "coStrings" => ("String", true),
        "coPercent" => ("Percent", false),
        "coPercents" => ("Percent", true),
        "coFloatOrPercent" => ("FloatOrPercent", false),
        "coFloatsOrPercents" => ("FloatOrPercent", true),
        "coPoint" => ("Point", false),
        "coPoints" => ("Point", true),
        "coPoint3" => ("Point3", false),
        "coBool" => ("Bool", false),
        "coBools" => ("Bool", true),
        "coEnum" => ("Enum", false),
        "coEnums" => ("Enum", true),
        // Group types (coPointsGroups, coIntsGroups...): lists of what
        // is not looked into.
        other => ("Opaque", other.ends_with('s')),
    }
}

/// The value a "%" of `our_key` is of, at entry `slot`: the settings
/// being converted first (an object's own outer wall speed), then the
/// project's (`ratio_base`).
fn ratio_base(our_key: &str, cfg: &Settings, ctx: &Context, slot: usize) -> Option<f64> {
    let over = ratio_over(our_key)?;
    let mut values: Vec<f64> = cfg
        .get(over)
        .map(|value| {
            read_value(value)
                .values
                .iter()
                .filter_map(|text| number(text))
                .collect()
        })
        .unwrap_or_default();
    if values.is_empty() {
        values = ctx.numbers.get(over).cloned().unwrap_or_default();
    }
    let base = *values.get(slot.min(values.len().checked_sub(1)?))?;
    (base > 0.0).then_some(base)
}

/// Our value of an enum in Bambu's spelling, or none where Bambu has no
/// such value (`translate_enum`).
fn translate_enum(bambu_key: &str, value: &str, accepted: &[&str]) -> Option<String> {
    if accepted.contains(&value) {
        return Some(value.to_owned());
    }
    for (key, ours, theirs) in known::ENUM_ALIASES {
        if *key == bambu_key && *ours == value {
            return Some((*theirs).to_owned());
        }
    }
    for (key, ours, theirs) in known::ENUMS {
        if *key == bambu_key && *ours == value {
            return Some((*theirs).to_owned()).filter(|theirs| !theirs.is_empty());
        }
    }
    None
}

/// Bambu's spelling of one of our values of an enum of Bambu's; empty
/// where Bambu has none (`translate_enum_value`).
pub fn translate_enum_value(bambu_key: &str, our_value: &str) -> String {
    let Some(def) = find_key(bambu_key) else {
        return our_value.to_owned();
    };
    let Some(enums) = def.enums else {
        return our_value.to_owned();
    };
    let accepted: Vec<&str> = enums.split('|').collect();
    translate_enum(bambu_key, our_value, &accepted).unwrap_or_default()
}

/// What happened to one key.
#[derive(Default)]
struct KeyResult {
    reason: String,
    changed: bool,
}

/// The conversion tied to a rename (`rename_value`).
fn rename_value(our_key: &str, value: &mut BambuValue, result: &mut KeyResult) -> bool {
    let Some((_, _, Some(function), reason)) = alias_of(our_key) else {
        return true;
    };
    if value.values.is_empty() {
        result.reason = "empty value".into();
        return false;
    }
    for entry in &mut value.values {
        let before = entry.clone();
        if !to_bambu(function, entry) {
            result.reason = reason.map_or_else(
                || format!("value \"{before}\" has no Bambu Studio equivalent"),
                str::to_owned,
            );
            return false;
        }
        result.changed |= *entry != before;
    }
    true
}

/// `value` reshaped to Bambu's type for `def` (`fit_type`); false, with
/// the reason, where it cannot be.
fn fit_type(
    our_key: &str,
    def: &Def,
    cfg: &Settings,
    ctx: &Context,
    value: &mut BambuValue,
    result: &mut KeyResult,
) -> bool {
    let (base, vector) = type_info(def.kind);
    if value.vector && !vector {
        let Some(first) = value.values.first().cloned() else {
            result.reason = "empty list".into();
            return false;
        };
        let uniform = value.values.iter().all(|entry| *entry == first);
        value.values = vec![first];
        value.vector = false;
        result.changed = true;
        if !uniform {
            result.reason =
                "per-variant list collapsed to its first slot (Bambu has a single value)".into();
        }
    } else if !value.vector && vector {
        value.vector = true;
        result.changed = true;
    }
    value.strings = base == "String";
    let accepted: Vec<&str> = def
        .enums
        .map(|enums| enums.split('|').collect())
        .unwrap_or_default();
    for (slot, entry) in value.values.iter_mut().enumerate() {
        if entry == "nil" {
            if def.nullable && vector {
                continue;
            }
            result.reason = "\"nil\" in an option Bambu does not allow to be empty".into();
            return false;
        }
        match base {
            "Float" => {
                let Some((number, percent)) = number_or_percent(entry) else {
                    result.reason = format!("value \"{entry}\" is not a number");
                    return false;
                };
                if percent {
                    let Some(of) = ratio_base(our_key, cfg, ctx, slot) else {
                        result.reason =
                            format!("relative value \"{entry}\" has nothing to resolve against");
                        return false;
                    };
                    *entry = format_number(number * of / 100.0);
                    result.changed = true;
                }
            }
            "Int" => {
                if !is_integer(entry) {
                    let Some(number) = number(entry) else {
                        result.reason = format!("value \"{entry}\" is not a number");
                        return false;
                    };
                    *entry = format_number(number.round());
                    result.changed = true;
                }
            }
            "Percent" => {
                let Some((number, percent)) = number_or_percent(entry) else {
                    result.reason = format!("value \"{entry}\" is not a number");
                    return false;
                };
                if !percent {
                    // Ours in mm; Bambu takes a percentage of the same base.
                    if number == 0.0 {
                        *entry = "0%".into();
                    } else if let Some(of) = ratio_base(our_key, cfg, ctx, slot) {
                        *entry = format!("{}%", format_number(number * 100.0 / of));
                    } else {
                        result.reason = format!(
                            "absolute value \"{entry}\" has nothing to express it relative to"
                        );
                        return false;
                    }
                    result.changed = true;
                }
            }
            "FloatOrPercent" => {
                if number_or_percent(entry).is_none() {
                    result.reason = format!("value \"{entry}\" is not a number");
                    return false;
                }
            }
            "Bool" => {
                if entry == "true" {
                    *entry = "1".into();
                    result.changed = true;
                }
                if entry == "false" {
                    *entry = "0".into();
                    result.changed = true;
                }
                if entry != "0" && entry != "1" {
                    result.reason = format!("value \"{entry}\" is not a boolean");
                    return false;
                }
            }
            "Enum" if !accepted.is_empty() => match translate_enum(def.key, entry, &accepted) {
                None => {
                    result.reason = format!("value \"{entry}\" does not exist in Bambu Studio");
                    return false;
                }
                Some(theirs) => {
                    if theirs != *entry {
                        *entry = theirs;
                        result.changed = true;
                    }
                }
            },
            _ => {}
        }
    }
    true
}

// ---------------------------------------------------------------------
// The variant layout
// ---------------------------------------------------------------------

fn list_of(cfg: &Settings, ctx: Option<&Context>, key: &str) -> Vec<String> {
    if let Some(value) = cfg.get(key) {
        return read_value(value).values;
    }
    ctx.and_then(|ctx| ctx.lists.get(key).cloned())
        .unwrap_or_default()
}

fn make_layout(names: Vec<String>, ids: &[String]) -> Layout {
    let ids = (0..names.len())
        .map(|at| {
            ids.get(at)
                .and_then(|text| number(text))
                .map_or(1, |id| id as i64)
        })
        .collect();
    Layout { names, ids }
}

fn covers_extruders(layout: &Layout, extruders: usize) -> bool {
    (1..=extruders as i64).all(|extruder| layout.ids.contains(&extruder))
}

/// Which entry of our list (`n` long, laid out as `ours`) feeds slot `j`
/// of Bambu's layout (`source_slot`).
fn source_slot(
    j: usize,
    n: usize,
    ours: &Layout,
    target: &Layout,
    per_group: usize,
    ctx: &Context,
    print_flow: bool,
) -> usize {
    if n <= 1 {
        return 0;
    }
    let name = &target.names[j];
    let id = target.ids[j];
    if n == ours.len() {
        if let Some(k) = (0..n).find(|&k| ours.names[k] == *name && ours.ids[k] == id) {
            return k;
        }
        if let Some(k) = (0..n).find(|&k| ours.ids[k] == id) {
            return k;
        }
        if let Some(k) = (0..n).find(|&k| ours.names[k] == *name) {
            return k;
        }
    }
    if per_group > 1 && n == per_group && id >= 1 {
        return ((id - 1) as usize).min(n - 1);
    }
    if print_flow && n == ctx.process_flow_support.len() {
        // Ours: one per flow kind; Bambu's: one per extruder variant.
        let high = name.contains("High Flow");
        return (0..n)
            .find(|&k| high == (ctx.process_flow_support[k] == "high_flow"))
            .unwrap_or(0);
    }
    j.min(n - 1)
}

/// Bambu's variants that take our high-flow column: "... High Flow" and
/// "... E3D High Flow"; "... TPU High Flow" takes the standard one
/// (`slices_high_flow_column`).
fn slices_high_flow(name: &str) -> bool {
    name.contains("High Flow") && !name.contains("TPU High Flow")
}

/// Where each filament's entries start in a list of our full settings
/// kept per flow variant, and how many it has
/// (`packed_filament_segments`).
fn packed_segments(ctx: &Context, n: usize, filaments: usize) -> Option<(Vec<usize>, Vec<usize>)> {
    let steps = ctx.numbers.get("filament_flow_step_size")?;
    if steps.len() != filaments {
        return None;
    }
    let mut total = 0;
    let (mut starts, mut sizes) = (Vec::new(), Vec::new());
    for step in steps {
        let size = step.max(1.0) as usize;
        starts.push(total);
        sizes.push(size);
        total += size;
    }
    (total == n).then_some((starts, sizes))
}

struct Layouts<'a> {
    ours_print: &'a Layout,
    ours_printer: &'a Layout,
    ours_filament: &'a Layout,
    target_filament: &'a Layout,
    filaments: usize,
}

/// A list laid out as Bambu indexes it (`fit_layout`); whether it moved.
fn fit_layout(
    value: &mut BambuValue,
    variant: Variant,
    ctx: &Context,
    layouts: &Layouts<'_>,
    packed_flow: bool,
) -> bool {
    if !value.vector || value.values.is_empty() {
        return false;
    }
    let input = value.values.clone();
    let n = input.len();
    let mut out = Vec::new();
    match variant {
        Variant::Print => {
            for j in 0..ctx.print.len() {
                out.push(
                    input[source_slot(j, n, layouts.ours_print, &ctx.print, 0, ctx, true)].clone(),
                );
            }
        }
        Variant::Printer => {
            for j in 0..ctx.printer.len() {
                out.push(
                    input[source_slot(
                        j,
                        n,
                        layouts.ours_printer,
                        &ctx.printer,
                        ctx.extruder_count,
                        ctx,
                        false,
                    )]
                    .clone(),
                );
            }
        }
        Variant::PrinterPair => {
            // (normal, silent) pairs, variant first.
            let pairs = (n / 2).max(1);
            for j in 0..ctx.printer.len() {
                let pair = source_slot(
                    j,
                    pairs,
                    layouts.ours_printer,
                    &ctx.printer,
                    ctx.extruder_count,
                    ctx,
                    false,
                );
                for sub in 0..2 {
                    out.push(input[(pair * 2 + sub).min(n - 1)].clone());
                }
            }
        }
        Variant::Filament => {
            let target = layouts.target_filament;
            if packed_flow && let Some((starts, sizes)) = packed_segments(ctx, n, layouts.filaments)
            {
                // Each filament's standard entry, and its high-flow one for
                // Bambu's high-flow variants where it has one.
                let support = ctx.lists.get("filament_flow_support");
                for j in 0..target.len() {
                    let f = ((target.ids[j].max(1) - 1) as usize).min(starts.len() - 1);
                    let mut source = starts[f];
                    if slices_high_flow(&target.names[j])
                        && let Some(support) = support
                        && let Some(k) = (starts[f]..starts[f] + sizes[f])
                            .take_while(|k| *k < support.len())
                            .find(|k| support[*k] == "high_flow")
                    {
                        source = k;
                    }
                    out.push(input[source.min(n - 1)].clone());
                }
            } else {
                // Bambu: one per (filament, variant), filament_self_index
                // naming the filament; ours one per filament (or a group per
                // filament for a preset of several variants). Our own
                // filament_self_index is not trusted.
                let filaments = layouts.filaments;
                let g = if filaments > 0 && n.is_multiple_of(filaments) {
                    n / filaments
                } else {
                    0
                };
                for j in 0..target.len() {
                    let f = (target.ids[j].max(1) - 1) as usize;
                    let mut source = j.min(n - 1);
                    if g >= 1 {
                        source = (f * g).min(n - 1);
                        if layouts.ours_filament.len() == n
                            && let Some(k) = (f * g..(f * g + g).min(n))
                                .find(|k| layouts.ours_filament.names[*k] == target.names[j])
                        {
                            source = k;
                        }
                    }
                    out.push(input[source].clone());
                }
            }
        }
        Variant::Extruder => {
            for e in 0..ctx.extruder_count {
                out.push(input[e.min(n - 1)].clone());
            }
        }
        Variant::None => return false,
    }
    if out.is_empty() {
        return false;
    }
    let moved = out != input;
    value.values = out;
    moved
}

fn strings_value(values: &[String]) -> BambuValue {
    BambuValue {
        vector: true,
        strings: true,
        values: values.to_vec(),
    }
}

fn ints_value(ids: &[i64]) -> BambuValue {
    BambuValue {
        vector: true,
        strings: false,
        values: ids.iter().map(i64::to_string).collect(),
    }
}

/// One slot per (filament, distinct printer variant), as Bambu's own
/// whole settings have it (`filament_layout`).
fn filament_layout(printer: &Layout, filaments: usize) -> Layout {
    let mut distinct: Vec<String> = Vec::new();
    for name in &printer.names {
        if !distinct.contains(name) {
            distinct.push(name.clone());
        }
    }
    if distinct.is_empty() {
        distinct.push("Direct Drive Standard".into());
    }
    let mut layout = Layout::default();
    for f in 0..filaments.max(1) {
        for name in &distinct {
            layout.names.push(name.clone());
            layout.ids.push(f as i64 + 1);
        }
    }
    layout
}

fn number_of(cfg: &Settings, ctx: &Context, key: &str) -> f64 {
    cfg.get(key)
        .and_then(|value| {
            read_value(value)
                .values
                .first()
                .and_then(|text| number(text))
        })
        .or_else(|| {
            ctx.numbers
                .get(key)
                .and_then(|values| values.first().copied())
        })
        .unwrap_or(0.0)
}

/// The depth of paint in Bambu's terms (`write_paint_penetration`): ours
/// has 0, "follow the shell", which Bambu cannot say, so it is written as
/// the depth the shell settings give, at least 1.
fn paint_penetration(
    cfg: &Settings,
    ctx: &Context,
    scope: Scope,
    out: &mut Config,
    local: &mut Report,
    at: &str,
) {
    if !matches!(scope, Scope::Project | Scope::Print | Scope::Object) {
        return;
    }
    for top in [true, false] {
        let (key, layers_key, thick_key) = if top {
            (
                "top_color_penetration_layers",
                "top_shell_layers",
                "top_shell_thickness",
            )
        } else {
            (
                "bottom_color_penetration_layers",
                "bottom_shell_layers",
                "bottom_shell_thickness",
            )
        };
        if cfg.get(key).is_some() {
            if number_of(cfg, ctx, key) >= 1.0 {
                continue;
            }
        } else {
            let shell = cfg.get(layers_key).is_some()
                || cfg.get(thick_key).is_some()
                || cfg.get("layer_height").is_some();
            if scope != Scope::Object || !shell || number_of(cfg, ctx, key) >= 1.0 {
                continue;
            }
        }
        let layers = number_of(cfg, ctx, layers_key).round() as i64;
        let thickness = number_of(cfg, ctx, thick_key);
        let height = number_of(cfg, ctx, "layer_height");
        let mut depth = layers;
        if layers > 0 && thickness > 0.0 && height > 0.0 {
            depth = depth.max(((thickness - 1e-4) / height).ceil() as i64);
        }
        depth = depth.max(1);
        out.insert(
            key.to_owned(),
            BambuValue {
                vector: false,
                strings: false,
                values: vec![depth.to_string()],
            },
        );
        local.converted.insert(key.to_owned());
        local.notes.push(format!(
            "{at}: {key} 0 (follow the shell) written as {depth}, the depth the {} shell settings give",
            if top { "top" } else { "bottom" }
        ));
    }
}

/// Every key of `cfg` converted (`convert_impl`).
fn convert_settings(
    cfg: &Settings,
    ctx: &Context,
    scope: Scope,
    report: &mut Report,
    at: &str,
) -> Config {
    let mut out = Config::new();
    let mut local = Report::default();
    let ours_print = make_layout(
        list_of(cfg, Some(ctx), "print_extruder_variant"),
        &list_of(cfg, Some(ctx), "print_extruder_id"),
    );
    let ours_printer = make_layout(
        list_of(cfg, Some(ctx), "printer_extruder_variant"),
        &list_of(cfg, Some(ctx), "printer_extruder_id"),
    );
    let ours_filament = make_layout(list_of(cfg, Some(ctx), "filament_extruder_variant"), &[]);
    let one_filament = filament_layout(&ctx.printer, 1);
    let (filaments, target_filament) = if scope == Scope::Filament {
        (1, &one_filament)
    } else {
        (ctx.filament_count, &ctx.filament)
    };
    let mut dropped: Vec<(String, String)> = Vec::new();
    const NOT_BAMBUS: &str = "not a Bambu Studio setting";
    for (our_key, value) in cfg.iter() {
        if LAYOUT_KEYS.contains(&our_key) && scope != Scope::Object {
            continue;
        }
        let mut bambu_key = bambu_key_name(our_key);
        // Both spellings there: the one Bambu's name is wins, this one goes.
        if bambu_key != our_key && cfg.get(bambu_key).is_some() {
            bambu_key = our_key;
        }
        let def = if bambu_key != our_key || alias_of(our_key).is_none() {
            find_key(bambu_key)
        } else {
            None
        };
        let Some(def) = def else {
            dropped.push((our_key.to_owned(), NOT_BAMBUS.into()));
            continue;
        };
        let mut value = read_value(value);
        if value.vector && value.values.is_empty() && OMIT_WHEN_EMPTY.contains(&bambu_key) {
            local.notes.push(format!(
                "{at}: {bambu_key} is empty; left out so Bambu Studio derives it from the printer"
            ));
            continue;
        }
        let mut result = KeyResult::default();
        if bambu_key != our_key && !rename_value(our_key, &mut value, &mut result) {
            dropped.push((our_key.to_owned(), result.reason));
            continue;
        }
        if !fit_type(our_key, def, cfg, ctx, &mut value, &mut result) {
            dropped.push((our_key.to_owned(), result.reason));
            continue;
        }
        let moved = if scope != Scope::Object || def.variant != Variant::Filament {
            let layouts = Layouts {
                ours_print: &ours_print,
                ours_printer: &ours_printer,
                ours_filament: &ours_filament,
                target_filament,
                filaments,
            };
            let packed = scope == Scope::Project && known::FILAMENT_FLOW_VARIANT.contains(&our_key);
            fit_layout(&mut value, def.variant, ctx, &layouts, packed)
        } else {
            // An object's own filament option is one value.
            let layouts = Layouts {
                ours_print: &ours_print,
                ours_printer: &ours_printer,
                ours_filament: &ours_filament,
                target_filament: &one_filament,
                filaments: 1,
            };
            fit_layout(&mut value, def.variant, ctx, &layouts, false)
        };
        if bambu_key != our_key {
            local
                .renamed
                .insert(our_key.to_owned(), bambu_key.to_owned());
            local.notes.push(format!("{at}: {our_key} -> {bambu_key}"));
        }
        if result.changed || moved {
            local.converted.insert(bambu_key.to_owned());
            if !result.reason.is_empty() {
                local
                    .notes
                    .push(format!("{at}: {bambu_key}: {}", result.reason));
            }
        }
        out.insert(bambu_key.to_owned(), value);
    }
    paint_penetration(cfg, ctx, scope, &mut out, &mut local, at);
    if matches!(scope, Scope::Project | Scope::Print) {
        out.insert(
            "print_extruder_variant".into(),
            strings_value(&ctx.print.names),
        );
        out.insert("print_extruder_id".into(), ints_value(&ctx.print.ids));
    }
    if matches!(scope, Scope::Project | Scope::Printer) {
        out.insert(
            "printer_extruder_variant".into(),
            strings_value(&ctx.printer.names),
        );
        out.insert("printer_extruder_id".into(), ints_value(&ctx.printer.ids));
    }
    if matches!(scope, Scope::Project | Scope::Filament) {
        out.insert(
            "filament_extruder_variant".into(),
            strings_value(&target_filament.names),
        );
        out.insert(
            "filament_self_index".into(),
            ints_value(&target_filament.ids),
        );
    }
    // Bambu's one switch for the bundle our three tower interface options
    // split: on when any of them is.
    if matches!(scope, Scope::Project | Scope::Print) {
        let keys = [
            "wipe_tower_interface_temp",
            "wipe_tower_interface_run_in",
            "wipe_tower_interface_extra_prime",
        ];
        let present: Vec<&str> = keys
            .into_iter()
            .filter(|key| cfg.get(key).is_some())
            .collect();
        if present.is_empty() {
            out.remove("enable_tower_interface_features");
        } else {
            let on: Vec<&str> = present
                .iter()
                .copied()
                .filter(|key| matches!(cfg.text(key), Some("1") | Some("true")))
                .collect();
            out.insert(
                "enable_tower_interface_features".into(),
                BambuValue {
                    vector: false,
                    strings: false,
                    values: vec![if on.is_empty() { "0" } else { "1" }.into()],
                },
            );
            if !on.is_empty() && on.len() < 3 {
                local.notes.push(format!(
                    "{at}: enable_tower_interface_features written as on although only {} is on: Bambu Studio turns on its whole tower interface bundle (interface temperature, run-in, extra prime, interface wall gaps and, on the H2 series, a firmware purge)",
                    on.join(", ")
                ));
            }
        }
    }
    // What was set by hand over each preset, in Bambu's names.
    if let Some(value) = out.get_mut("different_settings_to_system") {
        for list in &mut value.values {
            let mut kept: Vec<&str> = Vec::new();
            for key in list.split(';').filter(|key| !key.is_empty()) {
                let theirs = bambu_key_name(key);
                if find_key(theirs).is_some() && !kept.contains(&theirs) {
                    kept.push(theirs);
                }
            }
            *list = kept.join(";");
        }
    }
    for (key, reason) in &dropped {
        local.dropped.insert(key.clone());
        if reason != NOT_BAMBUS {
            local.notes.push(format!("{at}: left out {key}: {reason}"));
        }
    }
    if !dropped.is_empty() {
        let names: Vec<&str> = dropped.iter().map(|(key, _)| key.as_str()).collect();
        local.notes.insert(
            0,
            format!(
                "{at}: {} setting(s) left out: {}",
                dropped.len(),
                names.join(", ")
            ),
        );
    }
    report.merge(&local);
    out
}

impl Context {
    /// What the conversions need to know of a project's whole settings
    /// (`Context::from_project`).
    pub fn of(project: &Settings) -> Context {
        let mut ctx = Context {
            extruder_count: 1,
            filament_count: 1,
            ..Context::default()
        };
        for (key, value) in project.iter() {
            let values = read_value(value).values;
            let numbers: Vec<f64> = values
                .iter()
                .filter_map(|text| number_or_percent(text).map(|(number, _)| number))
                .collect();
            if !numbers.is_empty() && numbers.len() == values.len() {
                ctx.numbers.insert(key.to_owned(), numbers);
            }
        }
        for key in [
            "print_extruder_variant",
            "print_extruder_id",
            "printer_extruder_variant",
            "printer_extruder_id",
            "filament_extruder_variant",
            "filament_self_index",
            "extruder_type",
            "nozzle_volume_type",
            "filament_flow_support",
        ] {
            if let Some(value) = project.get(key) {
                ctx.lists.insert(key.into(), read_value(value).values);
            }
        }
        if let Some(diameters) = ctx.numbers.get("nozzle_diameter")
            && let Some(first) = diameters.first()
        {
            ctx.nozzle_diameter = *first;
            ctx.extruder_count = diameters.len();
        }
        // One entry per filament: the presets' names, else the colours.
        for key in ["filament_colour", "filament_settings_id"] {
            if let Some(value) = project.get(key) {
                ctx.filament_count = read_value(value).values.len().max(1);
            }
        }
        if let Some(value) = project.get("process_flow_support") {
            ctx.process_flow_support = read_value(value).values;
        }
        ctx
    }
}

/// A project's whole settings converted, and the layouts worked out into
/// `ctx` for the conversions after it (`convert_project`).
pub fn convert_project(project: &Settings, ctx: &mut Context, report: &mut Report) -> Config {
    // The printer's layout: ours when it names every extruder, else one
    // variant per extruder of its type and nozzle volume, as Bambu names
    // them (`get_extruder_variant_string`).
    let ours_printer = make_layout(
        list_of(project, Some(ctx), "printer_extruder_variant"),
        &list_of(project, Some(ctx), "printer_extruder_id"),
    );
    if !ours_printer.is_empty() && covers_extruders(&ours_printer, ctx.extruder_count) {
        ctx.printer = ours_printer;
    } else {
        let types = list_of(project, Some(ctx), "extruder_type");
        let volumes = list_of(project, Some(ctx), "nozzle_volume_type");
        let pick = |list: &[String], e: usize, default: &str| {
            list.get(e.min(list.len().saturating_sub(1)))
                .cloned()
                .unwrap_or_else(|| default.to_owned())
        };
        let mut layout = Layout::default();
        for e in 0..ctx.extruder_count {
            let kind = pick(&types, e, "Direct Drive");
            let mut volume = pick(&volumes, e, "Standard");
            // Bambu: "nvtHybrid not supported in presets, switch to nvtStandard".
            if volume == "Hybrid" {
                volume = "Standard".into();
            }
            layout.names.push(format!("{kind} {volume}"));
            layout.ids.push(e as i64 + 1);
        }
        ctx.printer = layout;
    }
    // The print layout: ours when it has a slot for every printer variant.
    let ours_print = make_layout(
        list_of(project, Some(ctx), "print_extruder_variant"),
        &list_of(project, Some(ctx), "print_extruder_id"),
    );
    let print_fits = !ours_print.is_empty()
        && (0..ctx.printer.len()).all(|j| {
            (0..ours_print.len()).any(|k| {
                ours_print.names[k] == ctx.printer.names[j]
                    && ours_print.ids[k] == ctx.printer.ids[j]
            })
        });
    ctx.print = if print_fits {
        ours_print
    } else {
        ctx.printer.clone()
    };
    ctx.filament = filament_layout(&ctx.printer, ctx.filament_count);

    let mut out = convert_settings(project, ctx, Scope::Project, report, "project");

    // Bambu writes the nozzle counts twice, `extruder_nozzle_stats_new`
    // with the real volume types. **Not ported**: the folding of volume
    // types newer than "TPU High Flow" into Standard for the legacy key
    // (`split_nozzle_stats_for_export`): this app never fills the counts,
    // and an empty list is left out above.
    if let Some(stats) = out
        .get("extruder_nozzle_stats")
        .filter(|stats| !stats.values.is_empty())
        .cloned()
    {
        out.insert("extruder_nozzle_stats_new".into(), stats);
    }
    // Bambu refuses ooze prevention with a prime tower ("Ooze prevention is
    // currently not supported with the prime tower enabled.").
    let on = |out: &Config, key: &str| {
        out.get(key)
            .and_then(|value| value.values.first())
            .is_some_and(|value| value == "1")
    };
    if on(&out, "ooze_prevention") && on(&out, "enable_prime_tower") {
        out.remove("ooze_prevention");
        report.dropped.insert("ooze_prevention".into());
        report.notes.push(
            "project: left out ooze_prevention: Bambu Studio cannot combine it with a prime tower"
                .into(),
        );
    }
    // Flushing per extruder in Bambu: the one matrix repeated for each.
    if ctx.extruder_count > 1 {
        let square = ctx.filament_count * ctx.filament_count;
        if let Some(matrix) = out.get_mut("flush_volumes_matrix")
            && matrix.values.len() == square
        {
            let once = matrix.values.clone();
            matrix.values = (0..ctx.extruder_count)
                .flat_map(|_| once.iter().cloned())
                .collect();
            report.converted.insert("flush_volumes_matrix".into());
        }
    }
    // Lists Bambu checks against the extruders when it opens a project.
    for key in ["extruder_type", "nozzle_volume_type", "flush_multiplier"] {
        let Some(value) = out.get_mut(key) else {
            continue;
        };
        if !value.vector || value.values.is_empty() || value.values.len() == ctx.extruder_count {
            continue;
        }
        let n = value.values.len();
        value.values = (0..ctx.extruder_count)
            .map(|e| value.values[e.min(n - 1)].clone())
            .collect();
        report.converted.insert(key.into());
    }
    out
}

/// A preset inside the project, or an object's, a part's or a height
/// range's own settings, converted with the project's context
/// (`convert`).
pub fn convert(
    cfg: &Settings,
    ctx: &Context,
    scope: Scope,
    report: &mut Report,
    at: &str,
) -> Config {
    convert_settings(cfg, ctx, scope, report, at)
}

/// Settings as `Metadata/*.config` holds them (`to_json`, as
/// `save_to_json` lays it out): one JSON object, keys in order, four
/// spaces to a level, with `version`, `name` and `from`.
pub fn to_json(cfg: &Config, name: &str, from: &str, version: &str) -> String {
    let mut object = serde_json::Map::new();
    for (key, value) in cfg {
        let json = if value.vector {
            serde_json::Value::from(value.values.as_slice())
        } else {
            serde_json::Value::from(value.values.first().map_or("", String::as_str))
        };
        object.insert(key.clone(), json);
    }
    object.insert("version".into(), version.into());
    object.insert("name".into(), name.into());
    object.insert("from".into(), from.into());
    let pretty = serde_json::to_string_pretty(&serde_json::Value::Object(object))
        .expect("strings always serialise");
    let mut text = String::with_capacity(pretty.len() * 5 / 4);
    for line in pretty.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        text.extend(std::iter::repeat_n(' ', indent * 2));
        text.push_str(&line[indent..]);
        text.push('\n');
    }
    text
}

/// `escape_string_cstyle`.
fn escape_cstyle(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// `escape_strings_cstyle`: each in quotes where it needs them, joined by
/// ";". **Differs**: one holding a ";" is quoted too; the reference's
/// (Config.cpp l.95–101) is not, and is then read back as two.
fn escape_strings_cstyle(items: &[String]) -> String {
    let alone = items.len() == 1;
    items
        .iter()
        .map(|item| {
            let quoted = (alone && item.is_empty())
                || item
                    .chars()
                    .any(|c| matches!(c, ' ' | '\t' | '\\' | '"' | '\r' | '\n' | ';'));
            if !quoted {
                return item.clone();
            }
            let mut out = String::from('"');
            for c in item.chars() {
                match c {
                    '\\' | '"' => {
                        out.push('\\');
                        out.push(c);
                    }
                    '\r' => out.push_str("\\r"),
                    '\n' => out.push_str("\\n"),
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// One value as a `model_settings.config` attribute holds it
/// (`serialize`).
pub fn serialize(value: &BambuValue) -> String {
    if !value.vector {
        let text = value.values.first().map_or("", String::as_str);
        return if value.strings {
            escape_cstyle(text)
        } else {
            text.to_owned()
        };
    }
    if value.strings {
        escape_strings_cstyle(&value.values)
    } else {
        value.values.join(",")
    }
}

/// Text as `unescape_string_cstyle` reads it.
fn unescape_cstyle(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// A list of texts as `unescape_strings_cstyle` reads it.
fn unescape_strings_cstyle(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut rest = text;
    if rest.is_empty() {
        return items;
    }
    loop {
        let item;
        if let Some(quoted) = rest.strip_prefix('"') {
            let mut end = quoted.len();
            let mut escape = false;
            for (at, c) in quoted.char_indices() {
                if escape {
                    escape = false;
                } else if c == '\\' {
                    escape = true;
                } else if c == '"' {
                    end = at;
                    break;
                }
            }
            item = unescape_cstyle(&quoted[..end]);
            rest = quoted.get(end + 1..).unwrap_or("");
        } else {
            let end = rest.find(';').unwrap_or(rest.len());
            item = rest[..end].to_owned();
            rest = &rest[end..];
        }
        items.push(item);
        match rest.strip_prefix(';') {
            Some(after) => rest = after,
            None => break,
        }
    }
    items
}

/// An object's, a part's or a height range's own settings as their
/// entries hold them (each `opt_serialize`d), read back by the shape
/// Bambu's option has: a list split as it was joined, a text unescaped.
pub fn object_settings<'a>(entries: impl IntoIterator<Item = (&'a str, &'a str)>) -> Settings {
    let mut settings = Settings::default();
    for (key, text) in entries {
        let def = find_key(bambu_key_name(key)).or_else(|| find_key(key));
        let value = match def.map(|def| type_info(def.kind)) {
            Some(("String", true)) => Value::Many(unescape_strings_cstyle(text)),
            Some(("String", false)) => Value::One(unescape_cstyle(text)),
            Some((_, true)) => Value::Many(text.split(',').map(str::to_owned).collect()),
            _ => Value::One(text.to_owned()),
        };
        settings.insert(key, value);
    }
    settings
}

#[cfg(test)]
mod tests;
