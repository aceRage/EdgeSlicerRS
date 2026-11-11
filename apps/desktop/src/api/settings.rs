//! The pages of settings of the three kinds of preset: the process's
//! ("Quality", "Strength", "Speed", "Support", "Multimaterial", "Others"),
//! each filament's ("Filament", "Cooling", "Setting Overrides",
//! "Advanced", "Multimaterial", "Dependencies", "Notes") and the
//! printer's ("Basic information", "Machine G-code", "Multimaterial", an
//! "Extruder n" for each extruder, "Motion ability", "Notes"); every
//! option on them, what a project sets over its presets, and what the
//! engine does with each.
//!
//! - The pages and the options are data, `settings_pages.json`, made from
//!   the reference's own source by `tools/make_settings_pages.py`
//!   (Tab.cpp `TabPrint::build`, `TabFilament::build`,
//!   `TabPrinter::build_fff` and the rest; PrintConfig.cpp
//!   `PrintConfigDef`). Nothing here names an option but the rules and
//!   the four the page set before there were pages.
//! - A value set here is the project's own, over the preset
//!   ([`Choices::over`] for the process, [`Choices::filament_over`] for
//!   each filament, [`Choices::printer_over`] for the printer); it goes
//!   into the settings a slice uses ([`overrides`]) and stays through a
//!   change of preset or printer, as the reference carries what was
//!   changed by hand (Tab.cpp `capture_print_settings_carry`).
//! - A filament's value is that filament's alone: the one entry of a list
//!   with a value for each filament. An extruder's likewise, its options
//!   named `key#n` (n from 0) as the reference names a field of one
//!   extruder. A list with a value for each nozzle flow (Standard, High
//!   Flow) or each mode (normal, silent) is shown and set as the process
//!   pages do it: the first value, set alike for every flow; of the
//!   machine limits the normal mode's alone.
//! - What the engine does with an option is the engine's own answer for
//!   the settings as they stand ([`edge_engine::followed_in`]): whether a
//!   slice reads it, and the notes it gives. There is no list of followed
//!   settings here.
//! - Which options are shown and which are greyed out is the reference's
//!   (`ConfigManipulation::toggle_print_fff_options` in [`rules`],
//!   `TabFilament::toggle_options` in [`filament_rules`],
//!   `TabPrinter::toggle_options` in [`printer_rules`]).

mod filament_rules;
mod printer_rules;
mod rules;

use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, Mutex, OnceLock},
};

use edge_config::{Settings, Value as Setting};
use edge_engine::{Chosen, Overrides};
use serde_json::{Value, json};

use super::{Choices, File, Session, lock, text};

/// The pages, as `tools/make_settings_pages.py` writes them.
const PAGES: &str = include_str!("settings_pages.json");

/// The three kinds of preset, each with its pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Book {
    Process,
    Filament,
    Printer,
}

impl Book {
    fn name(self) -> &'static str {
        match self {
            Book::Process => "process",
            Book::Filament => "filament",
            Book::Printer => "printer",
        }
    }

    /// The kind a call names (`kind`), the process's when it names none.
    pub(super) fn of(args: &Value) -> Result<Book, String> {
        match text(args, "kind") {
            None | Some("process") => Ok(Book::Process),
            Some("filament") => Ok(Book::Filament),
            Some("printer") => Ok(Book::Printer),
            Some(other) => Err(format!("there are no {other} settings")),
        }
    }
}

/// What kind of value an option takes (PrintConfig.cpp `coFloat` and the
/// rest, a list of them counting as what it is a list of).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Bool,
    Int,
    Float,
    Percent,
    FloatOrPercent,
    Enum,
    Text,
    /// Several texts, a line each.
    Texts,
    /// Points "XxY", several joined by commas.
    Points,
}

/// Where a filament's override takes its value from while it is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fallback {
    Printer,
    Process,
}

/// What the reference says of one option, as far as a value is checked
/// against it.
struct Described {
    label: String,
    kind: Kind,
    /// One value for each nozzle or flow variant; set alike for all.
    list: bool,
    /// Of a filament or a printer: a value for each filament or extruder.
    vector: bool,
    /// A value for the normal mode, then the silent: the first is set.
    modes: bool,
    min: Option<f64>,
    max: Option<f64>,
    /// The most a length may be, where a percentage may be more.
    max_literal: Option<f64>,
    /// The values of a list to pick from.
    values: Vec<String>,
    /// May be "nil", which takes the value from elsewhere.
    nullable: bool,
    /// A text of several lines (G-code, notes), which the settings hold
    /// with its line breaks, quotes and backslashes escaped.
    escaped: bool,
    /// A filament's override of a printer's or a process's option.
    fallback: Option<(Fallback, String)>,
    /// Why it cannot be set here, when it cannot.
    fixed: Option<&'static str>,
}

/// One page: its title, the options on each of its lines, and when it is
/// there.
struct Page {
    title: String,
    /// One page for each extruder ("Extruder %d"), its options `key#n`.
    per_extruder: bool,
    /// The G-code flavors it is shown for, when not for every one.
    flavors: Option<Vec<String>>,
    lines: Vec<Vec<String>>,
}

struct Pages {
    json: Value,
    options: BTreeMap<String, Described>,
    pages: Vec<Page>,
}

/// What the page says of an option that is set in a dialog of the
/// reference's own.
const IN_A_DIALOG: &str = "Set in a dialog of its own in the reference (the bed shape, the ramming, the compatible printers and processes), which is not built here yet; shown as the preset has it.";
/// Of `extruders_count`.
const EXTRUDER_COUNT: &str = "The printer preset's own number of extruders (one for each Nozzle diameter); another printer is picked in the Printer section.";
/// Of a filament preset's setting that is not a filament's (the
/// conditions on the printers and processes it is offered with).
const THE_PRESETS_OWN: &str = "Says which printers and processes the preset is offered with: kept with a saved preset, and presets are not saved yet.";

fn read(book: Book, all: &Value) -> Pages {
    let json = all[book.name()].clone();
    let options = json["options"]
        .as_object()
        .expect("options")
        .iter()
        .map(|(key, option)| {
            let vector = option["vector"].as_bool().unwrap_or(false);
            let kind = match option["type"].as_str() {
                Some("bool") => Kind::Bool,
                Some("int") => Kind::Int,
                Some("float") => Kind::Float,
                Some("percent") => Kind::Percent,
                Some("float_or_percent") => Kind::FloatOrPercent,
                Some("enum") => Kind::Enum,
                // A filament's or an extruder's one text.
                Some("strings") if book != Book::Process => Kind::Text,
                Some("strings") => Kind::Texts,
                Some("point" | "points") => Kind::Points,
                _ => Kind::Text,
            };
            let fixed = if option["widget"].as_bool() == Some(true) {
                Some(IN_A_DIALOG)
            } else if option["own"].as_bool() == Some(true) {
                Some(EXTRUDER_COUNT)
            } else if option["readonly"].as_bool() == Some(true) {
                Some("It is not changed by hand.")
            } else if book == Book::Filament && !vector {
                Some(THE_PRESETS_OWN)
            } else {
                None
            };
            let fallback = match (option["override"].as_str(), option["of"].as_str()) {
                (Some("printer"), Some(of)) => Some((Fallback::Printer, of.to_owned())),
                (Some("process"), Some(of)) => Some((Fallback::Process, of.to_owned())),
                _ => None,
            };
            let described = Described {
                label: option["label"].as_str().unwrap_or(key).to_owned(),
                kind,
                list: option["list"].as_bool().unwrap_or(false),
                vector,
                modes: option["modes"].as_bool().unwrap_or(false),
                min: option["min"].as_f64(),
                max: option["max"].as_f64(),
                max_literal: option["max_literal"].as_f64(),
                values: option["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect(),
                nullable: option["nullable"].as_bool().unwrap_or(false) || fallback.is_some(),
                escaped: kind == Kind::Text
                    && (option["code"].as_bool() == Some(true)
                        || option["multiline"].as_bool() == Some(true)),
                fallback,
                fixed,
            };
            (key.clone(), described)
        })
        .collect();
    let pages = json["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .map(|page| Page {
            title: page["title"].as_str().unwrap_or_default().to_owned(),
            per_extruder: page["per"].as_str() == Some("extruder"),
            flavors: page["flavors"].as_array().map(|flavors| {
                flavors
                    .iter()
                    .filter_map(|flavor| flavor.as_str().map(str::to_owned))
                    .collect()
            }),
            lines: page["groups"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|group| group["lines"].as_array().into_iter().flatten())
                .map(|line| {
                    line["keys"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|key| key.as_str().map(str::to_owned))
                        .collect()
                })
                .collect(),
        })
        .collect();
    Pages {
        json,
        options,
        pages,
    }
}

fn pages(book: Book) -> &'static Pages {
    static PAGES_READ: OnceLock<[Pages; 3]> = OnceLock::new();
    let all = PAGES_READ.get_or_init(|| {
        let all: Value = serde_json::from_str(PAGES).expect("settings_pages.json is JSON");
        [Book::Process, Book::Filament, Book::Printer].map(|book| read(book, &all))
    });
    match book {
        Book::Process => &all[0],
        Book::Filament => &all[1],
        Book::Printer => &all[2],
    }
}

/// An option's name on the page, and the option and extruder it is of:
/// `retraction_length#1` is extruder 2's retraction length.
fn address(key: &str) -> (&str, Option<usize>) {
    match key.split_once('#') {
        Some((key, extruder)) => (key, extruder.parse().ok()),
        None => (key, None),
    }
}

/// A text as the reference writes it in its settings
/// (`escape_string_cstyle`): line breaks, quotes and backslashes escaped.
fn escaped(text: &str) -> String {
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

/// The text an escaped one stands for (`unescape_string_cstyle`).
fn unescaped(text: &str) -> String {
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
            None => out.push('\\'),
        }
    }
    out
}

/// A value of the pages as the settings hold the text of it.
fn stored(option: &Described, value: &str) -> String {
    if option.escaped {
        escaped(value)
    } else {
        value.to_owned()
    }
}

/// A number as short as it can be written: 0.30 is "0.3", 3.0 is "3".
fn plain(number: f64) -> String {
    format!("{number}")
}

/// What is said when a value is outside what an option takes, in the
/// reference's words (Field.cpp) with the limit after them.
fn out_of_range(option: &Described, shown: &str, least: Option<f64>, most: Option<f64>) -> String {
    let label = &option.label;
    match (least, most) {
        (Some(least), Some(most)) => format!(
            "{label}: Value {shown} is out of range. The valid range is from {} to {}.",
            plain(least),
            plain(most)
        ),
        (Some(least), None) => format!(
            "{label}: Value {shown} is out of range. It cannot be less than {}.",
            plain(least)
        ),
        (None, Some(most)) => format!(
            "{label}: Value {shown} is out of range. It cannot be more than {}.",
            plain(most)
        ),
        (None, None) => format!("{label}: Value is out of range."),
    }
}

/// Whether a text is a point, "XxY" (PrintConfig.cpp `ConfigOptionPoint`).
fn is_point(text: &str) -> bool {
    text.split_once('x').is_some_and(|(x, y)| {
        [x, y]
            .iter()
            .all(|part| part.trim().parse::<f64>().is_ok_and(f64::is_finite))
    })
}

/// A value for an option as the settings hold it, or why it cannot be
/// one: not a number, outside the option's range, not one of its list.
fn checked(option: &Described, value: &Value) -> Result<String, String> {
    let label = &option.label;
    let given = match value {
        Value::Bool(on) => if *on { "1" } else { "0" }.to_owned(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        _ => return Err(format!("{label} cannot be {value}")),
    };
    let entered = given.trim();
    // An override turned off, which takes its value from elsewhere.
    if option.nullable && entered == "nil" {
        return Ok("nil".into());
    }
    let number = |entered: &str| -> Result<f64, String> {
        match entered.trim().parse::<f64>() {
            Ok(number) if number.is_finite() => Ok(number),
            _ => Err(format!("{label}: Invalid numeric.")),
        }
    };
    let within = |number: f64, shown: &str, most: Option<f64>| -> Result<(), String> {
        let under = option.min.is_some_and(|least| number < least);
        let over = most.is_some_and(|most| number > most);
        if under || over {
            return Err(out_of_range(option, shown, option.min, most));
        }
        Ok(())
    };
    match option.kind {
        Kind::Bool => match entered {
            "1" | "true" => Ok("1".into()),
            "0" | "false" => Ok("0".into()),
            _ => Err(format!("{label} is on or off, not \"{entered}\"")),
        },
        Kind::Int => {
            let number = number(entered)?;
            if number.fract() != 0.0 {
                return Err(format!("{label}: Invalid numeric. It is a whole number."));
            }
            within(number, entered, option.max)?;
            Ok(plain(number))
        }
        Kind::Float => {
            if entered.ends_with('%') {
                return Err(format!("{label} can't be a percentage"));
            }
            let number = number(entered)?;
            within(number, entered, option.max)?;
            Ok(plain(number))
        }
        Kind::Percent => {
            let number = number(entered.trim_end_matches('%'))?;
            let shown = format!("{}%", plain(number));
            within(number, &shown, option.max)?;
            Ok(shown)
        }
        Kind::FloatOrPercent => match entered.strip_suffix('%') {
            Some(percent) => {
                let number = number(percent)?;
                let shown = format!("{}%", plain(number));
                within(number, &shown, option.max)?;
                Ok(shown)
            }
            None => {
                let number = number(entered)?;
                within(number, entered, option.max_literal.or(option.max))?;
                Ok(plain(number))
            }
        },
        Kind::Enum => {
            if option.values.iter().any(|known| known == entered) {
                Ok(entered.to_owned())
            } else {
                Err(format!(
                    "{label} cannot be \"{entered}\"; it is one of {}",
                    option.values.join(", ")
                ))
            }
        }
        Kind::Points => {
            let points: Vec<&str> = entered.split(',').map(str::trim).collect();
            if points.iter().all(|point| is_point(point)) {
                Ok(points.join(","))
            } else {
                Err(format!(
                    "{label}: \"{entered}\" is not points in the form XxY, XxY, ..."
                ))
            }
        }
        Kind::Text => Ok(given),
        Kind::Texts => Ok(given
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")),
    }
}

/// One text of a setting as the pages show it: a number as short as it
/// can be written, a tick as 1 or 0.
fn shown_text(option: &Described, first: &str) -> String {
    let number = |text: &str| text.trim().parse::<f64>().ok().map(plain);
    let as_it_is = || first.to_owned();
    if first.trim() == "nil" {
        return "nil".into();
    }
    match option.kind {
        Kind::Bool => match first.trim() {
            "1" | "true" => "1".into(),
            "0" | "false" => "0".into(),
            _ => as_it_is(),
        },
        Kind::Int | Kind::Float => number(first).unwrap_or_else(as_it_is),
        Kind::Percent => number(first.trim().trim_end_matches('%'))
            .map_or_else(as_it_is, |number| format!("{number}%")),
        Kind::FloatOrPercent => match first.trim().strip_suffix('%') {
            Some(percent) => number(percent).map_or_else(as_it_is, |number| format!("{number}%")),
            None => number(first).unwrap_or_else(as_it_is),
        },
        Kind::Text if option.escaped => unescaped(first),
        Kind::Enum | Kind::Text | Kind::Points => as_it_is(),
        Kind::Texts => first
            .lines()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// A setting as the pages show it: of a list with a value for each
/// variant the first, of several texts a line each, of points all of
/// them, a number as short as it can be written.
fn shown(option: &Described, value: &Setting) -> String {
    let first = match value {
        Setting::One(text) => text.as_str(),
        Setting::Many(items) if option.kind == Kind::Texts => {
            return items
                .iter()
                .map(|item| item.trim_matches(['\n', '\r']))
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
        }
        Setting::Many(items) if option.kind == Kind::Points => {
            return items.join(",");
        }
        Setting::Many(items) => items.first().map_or("", String::as_str),
    };
    shown_text(option, first)
}

/// Entry `index` of a setting with a value for each filament or
/// extruder, as the pages show it: the last where the list is shorter,
/// as the engine reads it.
fn shown_at(option: &Described, value: &Setting, index: usize) -> String {
    match value {
        Setting::One(text) => shown_text(option, text),
        Setting::Many(items) => items
            .get(index)
            .or(items.last())
            .map_or_else(String::new, |text| shown_text(option, text)),
    }
}

/// A value of the pages as the settings hold it, in the shape `under`
/// has it in: a list with a value for each variant keeps its length; of
/// one with a value for each mode only the first is set.
fn held(option: &Described, value: &str, under: Option<&Setting>) -> Setting {
    if option.kind == Kind::Texts {
        let lines: Vec<String> = value.lines().map(str::to_owned).collect();
        return if lines.is_empty() {
            Setting::Many(vec![String::new()])
        } else {
            Setting::Many(lines)
        };
    }
    if option.kind == Kind::Points {
        return Setting::Many(value.split(',').map(str::to_owned).collect());
    }
    match under {
        Some(Setting::Many(items)) if option.modes && !items.is_empty() => {
            let mut items = items.clone();
            items[0] = value.to_owned();
            Setting::Many(items)
        }
        Some(Setting::Many(items)) if option.list || option.vector => {
            Setting::Many(vec![stored(option, value); items.len().max(1)])
        }
        _ if option.vector => Setting::Many(vec![stored(option, value)]),
        _ => Setting::One(stored(option, value)),
    }
}

/// What a project is sliced with beside its file: the presets chosen and
/// the settings the file carries.
pub(super) struct Basis {
    pub(super) chosen: Chosen,
    pub(super) project: Option<Arc<Settings>>,
}

impl Basis {
    /// The settings a value set by hand lies over: the preset's when
    /// presets are chosen, else the project's own.
    fn under(&self, book: Book) -> Option<&Settings> {
        let preset = match book {
            Book::Process => self.chosen.process.as_ref(),
            Book::Printer => self.chosen.machine.as_ref(),
            Book::Filament => self.chosen.filaments.first(),
        };
        preset
            .map(|(_, settings)| settings)
            .or(self.project.as_deref())
    }
}

/// What the person set over the presets and the project, for the engine:
/// the bed type, every process and printer option set here in the shape
/// the settings under it have it in, and each filament's and extruder's
/// own values.
pub(super) fn overrides(choices: &Choices, basis: &Basis) -> Overrides {
    let mut settings = Settings::default();
    let mut extruders: Vec<Settings> = Vec::new();
    for (book, over) in [
        (Book::Process, &choices.over),
        (Book::Printer, &choices.printer_over),
    ] {
        let under = basis.under(book);
        for (name, value) in over {
            let (key, extruder) = address(name);
            let Some(option) = pages(book).options.get(key) else {
                continue;
            };
            match extruder {
                Some(extruder) => {
                    if extruders.len() <= extruder {
                        extruders.resize_with(extruder + 1, Settings::default);
                    }
                    extruders[extruder].insert(key, Setting::One(stored(option, value)));
                }
                None => {
                    let below = under
                        .and_then(|settings| settings.get(key))
                        .or_else(|| edge_config::reference_defaults().get(key));
                    settings.insert(key, held(option, value, below));
                }
            }
        }
    }
    let filaments = choices
        .filament_over
        .iter()
        .map(|own| {
            let mut settings = Settings::default();
            for (key, value) in own {
                if let Some(option) = pages(Book::Filament).options.get(key) {
                    settings.insert(key.clone(), Setting::One(stored(option, value)));
                }
            }
            settings
        })
        .collect();
    Overrides {
        bed_type: choices.bed_type.clone(),
        settings,
        filaments,
        extruders,
        ..Overrides::default()
    }
}

/// What is set by hand for one book (and filament, from 0).
fn over_of(choices: &Choices, book: Book, filament: usize) -> Option<&BTreeMap<String, String>> {
    match book {
        Book::Process => Some(&choices.over),
        Book::Printer => Some(&choices.printer_over),
        Book::Filament => choices.filament_over.get(filament),
    }
}

fn over_mut(choices: &mut Choices, book: Book, filament: usize) -> &mut BTreeMap<String, String> {
    match book {
        Book::Process => &mut choices.over,
        Book::Printer => &mut choices.printer_over,
        Book::Filament => {
            if choices.filament_over.len() <= filament {
                choices
                    .filament_over
                    .resize_with(filament + 1, BTreeMap::new);
            }
            &mut choices.filament_over[filament]
        }
    }
}

/// Leaves out what is no longer set: an empty filament at the end.
fn tidy(choices: &mut Choices) {
    while choices.filament_over.last().is_some_and(BTreeMap::is_empty) {
        choices.filament_over.pop();
    }
}

/// Which pages are asked about: a book, and of the filaments the one
/// (from 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Asked {
    book: Book,
    filament: usize,
}

impl Session {
    /// The presets a project's choices name, found, and the settings its
    /// file carries.
    pub(super) fn basis(&self, choices: &Choices, file: Option<&File>) -> Result<Basis, String> {
        let chosen = match (choices.from_presets, &choices.vendor) {
            (true, Some(vendor)) => {
                let filaments: Vec<&str> = choices.filaments.iter().map(String::as_str).collect();
                self.shelf(vendor)?
                    .choose(
                        choices.machine.as_deref(),
                        choices.process.as_deref(),
                        &filaments,
                    )
                    .map_err(|e| e.to_string())?
            }
            _ => Chosen::default(),
        };
        Ok(Basis {
            chosen,
            project: file.and_then(|file| file.settings.clone()),
        })
    }

    /// The pages of a kind of preset and their options, as the reference
    /// has them.
    pub(super) fn settings_pages(book: Book) -> Value {
        pages(book).json.clone()
    }

    /// Which book, and which filament: `filament` counts from 1 and is one
    /// the project has.
    fn asked(args: &Value, book: Book, filaments: usize) -> Result<Asked, String> {
        let filament = match (book, args.get("filament").and_then(Value::as_u64)) {
            (Book::Filament, Some(number)) => {
                let number = usize::try_from(number).unwrap_or(usize::MAX);
                if number == 0 || number > filaments {
                    return Err(format!(
                        "the project has {filaments} filament(s); there is no filament {number}"
                    ));
                }
                number - 1
            }
            (Book::Filament, None) => return Err("filament was not given".into()),
            _ => 0,
        };
        Ok(Asked { book, filament })
    }

    /// The settings of a project of one kind (and filament): each option's
    /// value by the preset, what is set over it, and what the engine and
    /// the rules say of it with the settings as they stand.
    pub(super) fn settings(&self, args: &Value, book: Book) -> Result<Value, String> {
        let (id, choices, file, filaments) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (
                project.id,
                project.choices.clone(),
                project.file.clone(),
                project.filament_count(),
            )
        };
        let asked = Self::asked(args, book, filaments)?;
        let basis = self.basis(&choices, file.as_deref())?;
        Ok(described(id, asked, &choices, &basis, filaments))
    }

    /// Sets an option over the preset (`key`, `value`), gives it back to
    /// the preset (`value` null), or gives all of the book's (and the
    /// filament's) back (`reset`). A value the option does not take, or
    /// one the engine would stop at, is refused and nothing is changed.
    pub(super) fn settings_set(&self, args: &Value, book: Book) -> Result<Value, String> {
        let (id, mut choices, file, filaments) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (
                project.id,
                project.choices.clone(),
                project.file.clone(),
                project.filament_count(),
            )
        };
        let asked = Self::asked(args, book, filaments)?;
        let was = choices.clone();
        let basis = self.basis(&choices, file.as_deref())?;
        if args.get("reset").and_then(Value::as_bool) == Some(true) {
            over_mut(&mut choices, book, asked.filament).clear();
        } else {
            let name = text(args, "key").ok_or("key was not given")?;
            let (key, extruder) = address(name);
            let option = pages(book)
                .options
                .get(key)
                .ok_or_else(|| format!("there is no {} setting named {key}", book.name()))?;
            let per_extruder = pages(book).pages.iter().any(|page| {
                page.per_extruder && page.lines.iter().flatten().any(|named| named == key)
            });
            if name.contains('#') && (extruder.is_none() || !per_extruder) {
                return Err(format!("{} is not an extruder's own", option.label));
            }
            if per_extruder && extruder.is_none() {
                return Err(format!(
                    "{} is an extruder's own: name it {key}#n",
                    option.label
                ));
            }
            if let Some(extruder) = extruder {
                let now = edge_engine::settled(
                    &basis.chosen,
                    &overrides(&choices, &basis),
                    basis.project.as_deref(),
                )
                .map(|settled| printer_rules::extruders(&settled))
                .unwrap_or(1);
                if extruder >= now {
                    return Err(format!(
                        "the printer has {now} extruder(s); there is no extruder {}",
                        extruder + 1
                    ));
                }
            }
            if let Some(why) = option.fixed {
                return Err(format!("{} cannot be changed here. {why}", option.label));
            }
            match args.get("value") {
                None => return Err("value was not given".into()),
                Some(Value::Null) => {
                    over_mut(&mut choices, book, asked.filament).remove(name);
                }
                Some(value) => {
                    let value = checked(option, value)?;
                    // Set to what the preset has, it is the preset's again.
                    let preset = preset_values(asked, &choices, &basis);
                    let own = over_mut(&mut choices, book, asked.filament);
                    if preset.get(name).is_some_and(|preset| *preset == value) {
                        own.remove(name);
                    } else {
                        own.insert(name.to_owned(), value);
                    }
                }
            }
            tidy(&mut choices);
            // What the engine would stop at is not kept either, unless it
            // stopped at the settings as they were.
            if choices != was {
                let in_filament = asked.filament + 1;
                let now = edge_engine::followed_in(
                    &basis.chosen,
                    &overrides(&choices, &basis),
                    basis.project.as_deref(),
                    in_filament,
                );
                if let Some(refused) = now.refused {
                    let before = edge_engine::followed_in(
                        &basis.chosen,
                        &overrides(&was, &basis),
                        basis.project.as_deref(),
                        in_filament,
                    );
                    if before.refused.is_none() {
                        return Err(format!("{}: {refused}", option.label));
                    }
                }
            }
        }
        tidy(&mut choices);
        {
            let mut state = lock(&self.state);
            let at = state
                .index_of(id)
                .ok_or_else(|| format!("there is no project {id}"))?;
            let project = &mut state.projects[at];
            // Only what this call changed is written, in case the project
            // was given other choices meanwhile.
            let mut current = project.choices.clone();
            *over_mut(&mut current, book, asked.filament) = over_of(&choices, book, asked.filament)
                .cloned()
                .unwrap_or_default();
            tidy(&mut current);
            if !current.settle_as(&project.choices) {
                project.forget();
            }
            project.choices = current;
        }
        Ok(described(id, asked, &choices, &basis, filaments))
    }
}

/// The names the pages give a book's options: an extruder's own once for
/// each extruder (`key#n`).
fn names(book: Book, extruders: usize) -> Vec<(String, &'static str)> {
    let mut found = Vec::new();
    for page in &pages(book).pages {
        for key in page.lines.iter().flatten() {
            if page.per_extruder {
                for extruder in 0..extruders {
                    found.push((
                        format!("{key}#{extruder}"),
                        pages(book)
                            .options
                            .get_key_value(key)
                            .expect("described")
                            .0
                            .as_str(),
                    ));
                }
            } else {
                found.push((
                    key.clone(),
                    pages(book)
                        .options
                        .get_key_value(key)
                        .expect("described")
                        .0
                        .as_str(),
                ));
            }
        }
    }
    found
}

/// Each option's value with nothing set by hand: the preset's, or the
/// project's own, or the reference's default. A filament's of that
/// filament, an extruder's of that extruder.
fn preset_values(asked: Asked, choices: &Choices, basis: &Basis) -> BTreeMap<String, String> {
    let unset = Overrides {
        bed_type: choices.bed_type.clone(),
        ..Overrides::default()
    };
    let settled = edge_engine::settled(&basis.chosen, &unset, basis.project.as_deref()).ok();
    values_in(asked, settled.as_ref())
}

/// Each option's value in `settled`, as the pages show it.
fn values_in(asked: Asked, settled: Option<&Settings>) -> BTreeMap<String, String> {
    let defaults = edge_config::reference_defaults();
    let extruders = settled.map_or(1, printer_rules::extruders);
    let book = pages(asked.book);
    names(asked.book, extruders)
        .into_iter()
        .filter_map(|(name, key)| {
            let option = &book.options[key];
            let value = settled
                .and_then(|settled| settled.get(key))
                .or_else(|| defaults.get(key));
            let shown = match (value, address(&name).1) {
                (Some(value), Some(extruder)) => shown_at(option, value, extruder),
                (Some(value), None) if asked.book == Book::Filament && option.vector => {
                    shown_at(option, value, asked.filament)
                }
                (Some(value), None) => shown(option, value),
                (None, _) if key == "extruders_count" => extruders.to_string(),
                (None, _) => return None,
            };
            Some((name, shown))
        })
        .collect()
}

/// What tells one set of settings to answer for from another: the
/// project, the book and filament asked about, what is set by hand, and
/// every value of every preset and of the project's own settings under it.
fn fingerprint(id: u32, asked: Asked, choices: &Choices, basis: &Basis, filaments: usize) -> u64 {
    let mut hasher = DefaultHasher::new();
    (
        id,
        asked,
        filaments,
        choices.from_presets,
        &choices.bed_type,
    )
        .hash(&mut hasher);
    choices.over.hash(&mut hasher);
    choices.filament_over.hash(&mut hasher);
    choices.printer_over.hash(&mut hasher);
    let presets = [&basis.chosen.machine, &basis.chosen.process]
        .into_iter()
        .flatten()
        .chain(&basis.chosen.filaments);
    for (name, settings) in presets {
        name.hash(&mut hasher);
        for (key, value) in settings.iter() {
            (key, edge_config::serialized(key, value)).hash(&mut hasher);
        }
    }
    if let Some(project) = &basis.project {
        for (key, value) in project.iter() {
            (key, edge_config::serialized(key, value)).hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// The answer of `settings` and `settings_set`. Working it out slices the
/// engine's block, so the last few answers are kept: the sidebar asks
/// again whenever anything about the project is shown anew.
fn described(id: u32, asked: Asked, choices: &Choices, basis: &Basis, filaments: usize) -> Value {
    static KEPT: Mutex<Vec<(u64, Value)>> = Mutex::new(Vec::new());
    let key = fingerprint(id, asked, choices, basis, filaments);
    if let Some((_, answer)) = lock(&KEPT).iter().find(|(kept, _)| *kept == key) {
        return answer.clone();
    }
    let answer = worked_out(id, asked, choices, basis, filaments);
    let mut kept = lock(&KEPT);
    if kept.len() >= 24 {
        kept.remove(0);
    }
    kept.push((key, answer.clone()));
    answer
}

fn worked_out(id: u32, asked: Asked, choices: &Choices, basis: &Basis, filaments: usize) -> Value {
    let book = pages(asked.book);
    let values = preset_values(asked, choices, basis);
    let set = overrides(choices, basis);
    let followed = edge_engine::followed_in(
        &basis.chosen,
        &set,
        basis.project.as_deref(),
        asked.filament + 1,
    );
    // The rules go by the settings a slice would get, what is set by
    // hand among them.
    let settled = edge_engine::settled(&basis.chosen, &set, basis.project.as_deref())
        .unwrap_or_else(|_| set.settings.clone());
    let extruders = printer_rules::extruders(&settled);
    let (toggles, labels) = match asked.book {
        Book::Process => (rules::toggles(&settled, filaments), BTreeMap::new()),
        Book::Filament => filament_rules::toggles(&settled, asked.filament),
        Book::Printer => (printer_rules::toggles(&settled), BTreeMap::new()),
    };
    let own = over_of(choices, asked.book, asked.filament)
        .cloned()
        .unwrap_or_default();
    // The pages as they stand: an extruder's page once for each extruder,
    // its options named for it.
    let mut shown_pages: Vec<(String, Vec<Vec<String>>, bool)> = Vec::new();
    let flavor = settled.text("gcode_flavor").unwrap_or("marlin");
    for page in &book.pages {
        let away = page
            .flavors
            .as_ref()
            .is_some_and(|flavors| !flavors.iter().any(|known| known == flavor));
        if page.per_extruder {
            for extruder in 0..extruders {
                let lines = page
                    .lines
                    .iter()
                    .map(|line| line.iter().map(|key| format!("{key}#{extruder}")).collect())
                    .collect();
                shown_pages.push((
                    page.title.replace("%d", &(extruder + 1).to_string()),
                    lines,
                    away,
                ));
            }
        } else {
            shown_pages.push((page.title.clone(), page.lines.clone(), away));
        }
    }
    // A line is hidden with any option on it, as the reference hides the
    // line an option is on.
    let mut hidden: BTreeSet<&str> = BTreeSet::new();
    for (_, lines, _) in &shown_pages {
        for line in lines {
            if line
                .iter()
                .any(|key| toggles.hidden(key) || toggles.hidden(address(key).0))
            {
                hidden.extend(line.iter().map(String::as_str));
            }
        }
    }
    let all: Vec<&str> = shown_pages
        .iter()
        .flat_map(|(_, lines, _)| lines.iter().flatten())
        .map(String::as_str)
        .collect();
    let off: Vec<&str> = all
        .iter()
        .copied()
        .filter(|key| toggles.off(key) || toggles.off(address(key).0))
        .collect();
    let unread: Vec<&str> = all
        .iter()
        .copied()
        .filter(|key| {
            let key = address(key).0;
            book.options[key].fixed.is_none() && !followed.read.contains(key)
        })
        .collect();
    let notes: BTreeMap<&str, Vec<&str>> = all
        .iter()
        .filter_map(|key| {
            let said: Vec<&str> = followed.notes_on(address(key).0).collect();
            (!said.is_empty()).then_some((*key, said))
        })
        .collect();
    let counts: BTreeMap<&str, usize> = shown_pages
        .iter()
        .map(|(title, lines, _)| {
            let changed = lines
                .iter()
                .flatten()
                .filter(|key| own.contains_key(*key))
                .count();
            (title.as_str(), changed)
        })
        .collect();
    let away: Vec<&str> = shown_pages
        .iter()
        .filter(|(_, _, away)| *away)
        .map(|(title, _, _)| title.as_str())
        .collect();
    // What an override that is off stands for: the printer's value for
    // this filament's extruder, or the process's, as the slice has them.
    let fallback: BTreeMap<&str, String> = if asked.book == Book::Filament {
        book.options
            .iter()
            .filter_map(|(key, option)| {
                let (from, of) = option.fallback.as_ref()?;
                let value = settled.get(of)?;
                let shown = match from {
                    Fallback::Printer => shown_at(option, value, asked.filament),
                    Fallback::Process => shown(option, value),
                };
                Some((key.as_str(), shown))
            })
            .collect()
    } else {
        BTreeMap::new()
    };
    let named = |key: &str| {
        basis
            .project
            .as_ref()
            .and_then(|settings| settings.texts(key).map(<[String]>::to_vec))
    };
    let name = match (asked.book, choices.from_presets) {
        (Book::Process, true) => choices.process.clone(),
        (Book::Printer, true) => choices.machine.clone(),
        (Book::Filament, true) => choices
            .filaments
            .get(asked.filament)
            .or(choices.filaments.last())
            .cloned(),
        (Book::Process, false) => {
            named("print_settings_id").and_then(|names| names.first().cloned())
        }
        (Book::Printer, false) => {
            named("printer_settings_id").and_then(|names| names.first().cloned())
        }
        (Book::Filament, false) => named("filament_settings_id")
            .and_then(|names| names.get(asked.filament).or(names.last()).cloned()),
    };
    let mut answer = json!({
        "project": id,
        "kind": asked.book.name(),
        "source": if choices.from_presets { "presets" } else { "project" },
        "name": name,
        "values": values,
        "set": own,
        "hidden": hidden,
        "off": off,
        "unread": unread,
        "notes": notes,
        "refused": followed.refused,
        "counts": counts,
        "changed": own.len(),
    });
    if asked.book != Book::Process {
        let extra = json!({
            "filament": asked.filament + 1,
            "filaments": filaments,
            "extruders": extruders,
            "pages": shown_pages.iter().map(|(title, _, _)| title.as_str()).collect::<Vec<_>>(),
            "away": away,
            "labels": labels,
            "fallback": fallback,
            "fixed": book.options.iter().filter_map(|(key, option)| Some((key.as_str(), option.fixed?))).collect::<BTreeMap<_, _>>(),
            "gcode_flavor": flavor,
        });
        if let (Some(answer), Value::Object(extra)) = (answer.as_object_mut(), extra) {
            answer.extend(extra);
        }
    }
    answer
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod preset_tests;
