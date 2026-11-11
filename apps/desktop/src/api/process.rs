//! The process settings of the sidebar: the reference's pages "Quality",
//! "Strength", "Speed", "Support", "Multimaterial" and "Others", every
//! option on them, what a project sets over its process preset, and what
//! the engine does with each.
//!
//! - The pages and the options are data, `settings_pages.json`, made from
//!   the reference's own source by `tools/make_settings_pages.py`
//!   (Tab.cpp `TabPrint::build`, PrintConfig.cpp `PrintConfigDef`). Nothing
//!   here names an option but [`rules`] and the four the page set before
//!   there were pages.
//! - A value set here is the project's own, over the preset
//!   ([`Choices::over`]); it goes into the settings a slice uses
//!   ([`overrides`]) and stays through a change of preset or printer, as
//!   the reference carries what was changed by hand (Tab.cpp
//!   `capture_print_settings_carry`).
//! - What the engine does with an option is the engine's own answer for
//!   the settings as they stand ([`edge_engine::followed`]): whether a
//!   slice reads it, and the notes it gives. There is no list of followed
//!   settings here.
//! - Which options are shown and which are greyed out is the reference's
//!   `toggle_print_fff_options`, ported in [`rules`].

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
}

/// What the reference says of one option, as far as a value is checked
/// against it.
struct Described {
    label: String,
    kind: Kind,
    /// One value for each nozzle or flow variant; set alike for all.
    list: bool,
    min: Option<f64>,
    max: Option<f64>,
    /// The most a length may be, where a percentage may be more.
    max_literal: Option<f64>,
    /// The values of a list to pick from.
    values: Vec<String>,
    readonly: bool,
}

struct Pages {
    json: Value,
    options: BTreeMap<String, Described>,
    /// Each page's title and its lines, a line being the options on it.
    pages: Vec<(String, Vec<Vec<String>>)>,
}

fn pages() -> &'static Pages {
    static PAGES_READ: OnceLock<Pages> = OnceLock::new();
    PAGES_READ.get_or_init(|| {
        let all: Value = serde_json::from_str(PAGES).expect("settings_pages.json is JSON");
        let json = all["process"].clone();
        let options = json["options"]
            .as_object()
            .expect("options")
            .iter()
            .map(|(key, option)| {
                let kind = match option["type"].as_str() {
                    Some("bool") => Kind::Bool,
                    Some("int") => Kind::Int,
                    Some("float") => Kind::Float,
                    Some("percent") => Kind::Percent,
                    Some("float_or_percent") => Kind::FloatOrPercent,
                    Some("enum") => Kind::Enum,
                    Some("strings") => Kind::Texts,
                    _ => Kind::Text,
                };
                let described = Described {
                    label: option["label"].as_str().unwrap_or(key).to_owned(),
                    kind,
                    list: option["list"].as_bool().unwrap_or(false),
                    min: option["min"].as_f64(),
                    max: option["max"].as_f64(),
                    max_literal: option["max_literal"].as_f64(),
                    values: option["values"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|value| value.as_str().map(str::to_owned))
                        .collect(),
                    readonly: option["readonly"].as_bool().unwrap_or(false),
                };
                (key.clone(), described)
            })
            .collect();
        let pages = json["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .map(|page| {
                let lines = page["groups"]
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
                    .collect();
                (page["title"].as_str().unwrap_or_default().to_owned(), lines)
            })
            .collect();
        Pages {
            json,
            options,
            pages,
        }
    })
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
        Kind::Text => Ok(given),
        Kind::Texts => Ok(given
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")),
    }
}

/// A setting as the pages show it: of a list with a value for each
/// variant the first, of several texts a line each, a number as short as
/// it can be written.
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
        Setting::Many(items) => items.first().map_or("", String::as_str),
    };
    let number = |text: &str| text.trim().parse::<f64>().ok().map(plain);
    let as_it_is = || first.to_owned();
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
        Kind::Enum | Kind::Text => as_it_is(),
        Kind::Texts => first
            .lines()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// A value of the pages as the settings hold it, in the shape `under`
/// has it in: a list with a value for each variant keeps its length.
fn held(option: &Described, value: &str, under: Option<&Setting>) -> Setting {
    if option.kind == Kind::Texts {
        let lines: Vec<String> = value.lines().map(str::to_owned).collect();
        return if lines.is_empty() {
            Setting::Many(vec![String::new()])
        } else {
            Setting::Many(lines)
        };
    }
    match under {
        Some(Setting::Many(items)) if option.list => {
            Setting::Many(vec![value.to_owned(); items.len().max(1)])
        }
        _ => Setting::One(value.to_owned()),
    }
}

/// What a project is sliced with beside its file: the presets chosen and
/// the settings the file carries.
pub(super) struct Basis {
    pub(super) chosen: Chosen,
    pub(super) project: Option<Arc<Settings>>,
}

impl Basis {
    /// The settings a value set by hand lies over: the process preset's
    /// when presets are chosen, else the project's own.
    fn under(&self) -> Option<&Settings> {
        self.chosen
            .process
            .as_ref()
            .map(|(_, settings)| settings)
            .or(self.project.as_deref())
    }
}

/// What the person set over the presets and the project, for the engine:
/// the bed type, and every process option set here in the shape the
/// settings under it have it in.
pub(super) fn overrides(choices: &Choices, basis: &Basis) -> Overrides {
    let under = basis.under();
    let mut settings = Settings::default();
    for (key, value) in &choices.over {
        if let Some(option) = pages().options.get(key) {
            let below = under
                .and_then(|settings| settings.get(key))
                .or_else(|| edge_config::reference_defaults().get(key));
            settings.insert(key.clone(), held(option, value, below));
        }
    }
    Overrides {
        bed_type: choices.bed_type.clone(),
        settings,
        ..Overrides::default()
    }
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

    /// The pages and their options, as the reference has them.
    pub(super) fn process_pages() -> Value {
        pages().json.clone()
    }

    /// The process settings of a project: each option's value by the
    /// preset, what is set over it, and what the engine and the rules say
    /// of it with the settings as they stand.
    pub(super) fn process(&self, args: &Value) -> Result<Value, String> {
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
        let basis = self.basis(&choices, file.as_deref())?;
        Ok(described(id, &choices, &basis, filaments))
    }

    /// Sets a process option over the preset (`key`, `value`), gives it
    /// back to the preset (`value` null), or gives all of them back
    /// (`reset`). A value the option does not take, or one the engine
    /// would stop at, is refused and nothing is changed.
    pub(super) fn process_set(&self, args: &Value) -> Result<Value, String> {
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
        let was = choices.clone();
        let basis = self.basis(&choices, file.as_deref())?;
        if args.get("reset").and_then(Value::as_bool) == Some(true) {
            choices.over.clear();
        } else {
            let key = text(args, "key").ok_or("key was not given")?;
            let option = pages()
                .options
                .get(key)
                .ok_or_else(|| format!("there is no process setting named {key}"))?;
            if option.readonly {
                return Err(format!("{} cannot be changed", option.label));
            }
            match args.get("value") {
                None => return Err("value was not given".into()),
                Some(Value::Null) => {
                    choices.over.remove(key);
                }
                Some(value) => {
                    let value = checked(option, value)?;
                    // Set to what the preset has, it is the preset's again.
                    let preset = preset_values(&choices, &basis);
                    if preset.get(key).is_some_and(|preset| *preset == value) {
                        choices.over.remove(key);
                    } else {
                        choices.over.insert(key.to_owned(), value);
                    }
                }
            }
            // What the engine would stop at is not kept either, unless it
            // stopped at the settings as they were.
            if choices.over != was.over {
                let now = edge_engine::followed(
                    &basis.chosen,
                    &overrides(&choices, &basis),
                    basis.project.as_deref(),
                );
                if let Some(refused) = now.refused {
                    let before = edge_engine::followed(
                        &basis.chosen,
                        &overrides(&was, &basis),
                        basis.project.as_deref(),
                    );
                    if before.refused.is_none() {
                        return Err(format!("{}: {refused}", option.label));
                    }
                }
            }
        }
        {
            let mut state = lock(&self.state);
            let at = state
                .index_of(id)
                .ok_or_else(|| format!("there is no project {id}"))?;
            let project = &mut state.projects[at];
            // Only what this call changed is written, in case the project
            // was given other choices meanwhile.
            let mut current = project.choices.clone();
            current.over = choices.over.clone();
            if !current.settle_as(&project.choices) {
                project.forget();
            }
            project.choices = current;
        }
        Ok(described(id, &choices, &basis, filaments))
    }
}

/// Each option's value with nothing set by hand: the preset's, or the
/// project's own, or the reference's default.
fn preset_values(choices: &Choices, basis: &Basis) -> BTreeMap<String, String> {
    let unset = Overrides {
        bed_type: choices.bed_type.clone(),
        ..Overrides::default()
    };
    let settled = edge_engine::settled(&basis.chosen, &unset, basis.project.as_deref()).ok();
    let defaults = edge_config::reference_defaults();
    pages()
        .options
        .iter()
        .filter_map(|(key, option)| {
            let value = settled
                .as_ref()
                .and_then(|settled| settled.get(key))
                .or_else(|| defaults.get(key))?;
            Some((key.clone(), shown(option, value)))
        })
        .collect()
}

/// What tells one set of settings to answer for from another: the
/// project, what is set by hand, and every value of every preset and of
/// the project's own settings under it.
fn fingerprint(id: u32, choices: &Choices, basis: &Basis, filaments: usize) -> u64 {
    let mut hasher = DefaultHasher::new();
    (id, filaments, choices.from_presets, &choices.bed_type).hash(&mut hasher);
    choices.over.hash(&mut hasher);
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

/// The answer of `process` and `process_set`. Working it out slices the
/// engine's block, so the last few answers are kept: the sidebar asks
/// again whenever anything about the project is shown anew.
fn described(id: u32, choices: &Choices, basis: &Basis, filaments: usize) -> Value {
    static KEPT: Mutex<Vec<(u64, Value)>> = Mutex::new(Vec::new());
    let key = fingerprint(id, choices, basis, filaments);
    if let Some((_, answer)) = lock(&KEPT).iter().find(|(kept, _)| *kept == key) {
        return answer.clone();
    }
    let answer = worked_out(id, choices, basis, filaments);
    let mut kept = lock(&KEPT);
    if kept.len() >= 16 {
        kept.remove(0);
    }
    kept.push((key, answer.clone()));
    answer
}

fn worked_out(id: u32, choices: &Choices, basis: &Basis, filaments: usize) -> Value {
    let pages = pages();
    let values = preset_values(choices, basis);
    let set = overrides(choices, basis);
    let followed = edge_engine::followed(&basis.chosen, &set, basis.project.as_deref());
    // The rules go by the settings a slice would get, what is set by
    // hand among them.
    let settled = edge_engine::settled(&basis.chosen, &set, basis.project.as_deref())
        .unwrap_or_else(|_| set.settings.clone());
    let toggles = rules::toggles(&settled, filaments);
    // A line is hidden with any option on it, as the reference hides the
    // line an option is on.
    let mut hidden: BTreeSet<&str> = BTreeSet::new();
    for (_, lines) in &pages.pages {
        for line in lines {
            if line.iter().any(|key| toggles.hidden(key)) {
                hidden.extend(line.iter().map(String::as_str));
            }
        }
    }
    let off: Vec<&str> = pages
        .options
        .keys()
        .map(String::as_str)
        .filter(|key| toggles.off(key))
        .collect();
    let unread: Vec<&str> = pages
        .options
        .keys()
        .map(String::as_str)
        .filter(|key| !followed.read.contains(*key))
        .collect();
    let notes: BTreeMap<&str, Vec<&str>> = pages
        .options
        .keys()
        .filter_map(|key| {
            let said: Vec<&str> = followed.notes_on(key).collect();
            (!said.is_empty()).then_some((key.as_str(), said))
        })
        .collect();
    let counts: BTreeMap<&str, usize> = pages
        .pages
        .iter()
        .map(|(title, lines)| {
            let changed = lines
                .iter()
                .flatten()
                .filter(|key| choices.over.contains_key(*key))
                .count();
            (title.as_str(), changed)
        })
        .collect();
    let name = if choices.from_presets {
        choices.process.clone()
    } else {
        basis
            .project
            .as_ref()
            .and_then(|settings| settings.text("print_settings_id").map(str::to_owned))
    };
    json!({
        "project": id,
        "source": if choices.from_presets { "presets" } else { "project" },
        "name": name,
        "values": values,
        "set": choices.over,
        "hidden": hidden,
        "off": off,
        "unread": unread,
        "notes": notes,
        "refused": followed.refused,
        "counts": counts,
        "changed": choices.over.len(),
    })
}

#[cfg(test)]
mod tests;
