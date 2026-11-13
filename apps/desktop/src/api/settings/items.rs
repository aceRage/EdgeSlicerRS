//! The process settings of one object, one part or one height range of an
//! object: the reference's "Objects" side of the Process tab (ParamsPanel.cpp
//! `m_mode_region`, "Global" / "Objects"; Tab.cpp `TabPrintObject`,
//! `TabPrintPart`, `TabPrintLayer`), and the height ranges themselves
//! ("Height range Modifier": GUI_ObjectList.cpp `layers_editing`,
//! `add_layer_range_after_current`, `edit_layer_range`, `del_layer_range`;
//! GUI_ObjectLayers.cpp).
//!
//! - Which options an item may set is the reference's
//!   (`settings_pages.json`, `items`, made by tools/make_settings_pages.py).
//! - The value in force is the item's own, else its parent's: a part's and
//!   a height range's parent is its object, an object's the project's
//!   settings ("Global"), as the reference's tabs take their parent's.
//! - What is set is kept with the model ([`edge_engine::Model::set_object_settings`]
//!   and the rest), so it is saved with the project and every edit is one
//!   step of the undo history (`scene_edit`).
//! - What the engine does with an item's option is said by one function,
//!   [`item_followed`]; see there.
//!
//! The reference's words (the "Invalid numeric." of a height) go through
//! the translation layer (`tr`); the steps' names are its message ids,
//! put in the language where the page shows them, but "Change Option %s",
//! which the reference composes untranslated (`boost::format`). The words
//! this work adds that are not the reference's are all in [`words`] and
//! listed in `src/api/i18n/wording.txt`.

use std::collections::{BTreeMap, BTreeSet};

use edge_config::{Settings, Value as Setting};
use edge_engine::{Bed, LayerRange, Model, PartKind, SettingChange};
use serde_json::{Value, json};

use super::{Book, Described, address, checked, held, overrides, pages, rules, shown, tr};
use crate::api::{Session, lock, needed, whole};

/// The words of this work that are not the reference's, in one place.
pub(crate) mod words {
    /// An option the engine does not read set on that kind of item, where
    /// it gives no note of its own (see [`super::item_followed`]).
    pub const NOT_READ_THERE: &str = "not read by the engine set on";
    /// A key an item cannot set.
    pub const NOT_ON_THIS_ITEM: &str = "cannot be set on";
    /// A height range's top at or under its bottom.
    pub const RANGE_ENDS_LOW: &str = "The top of a height range must be above its bottom.";
    /// Two height ranges over the same heights.
    pub const RANGES_OVERLAP: &str = "Height ranges cannot overlap:";
    /// No room for another range after this one.
    pub const NO_ROOM: &str = "There is no room for another height range after this one.";
    /// The step of the undo history that edits a range's heights; the
    /// reference takes it unnamed (GUI_ObjectList.cpp `edit_layer_range`).
    pub const EDIT_RANGE: &str = "Edit height range";
    /// A part that has no settings of its own.
    pub const HELPER: &str = "A negative part, a support blocker, a support enforcer and a Precise Seam part have no settings of their own.";
}

/// The `scene_edit` operations made here.
pub(crate) const OPS: [&str; 9] = [
    "item_set",
    "item_reset",
    "range_add",
    "range_set",
    "range_delete",
    "profile_reset",
    "profile_adaptive",
    "profile_smooth",
    "profile_strokes",
];

/// Which kind of item, as the reference's tabs tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemKind {
    Object,
    Part,
    Range,
}

impl ItemKind {
    fn name(self) -> &'static str {
        match self {
            ItemKind::Object => "object",
            ItemKind::Part => "part",
            ItemKind::Range => "range",
        }
    }

    /// What the reference calls it in a sentence.
    fn said(self) -> &'static str {
        match self {
            ItemKind::Object => "an object",
            ItemKind::Part => "a part",
            ItemKind::Range => "a height range",
        }
    }
}

/// What the engine does with an option set on an item.
#[derive(Clone, Debug, PartialEq, Eq)]
// Noted: followed with a note of the engine's, which it gives none of on an
// item yet (its notes on items are of what it does not follow).
#[allow(dead_code)]
pub(crate) enum Following {
    /// Followed, as the global page shows nothing.
    Followed,
    /// Followed, with what the engine says of it.
    Noted(String),
    /// Not followed, and why.
    NotFollowed(String),
}

/// What the engine does with option `key` set on an item of `kind`: the
/// one place the engine's answer plugs in (`edge_engine::settable`,
/// `edge_engine::unfollowed_note`, regions.rs). Where the engine notes
/// the option set there as not followed (a layer height on an object or a
/// range, an object's supports and seam, the feature filaments, an
/// object's option on a part), that note, the words "About this print"
/// gives after a slice; else, where it follows it on that kind of item,
/// followed; else not read there. `effective` is the item's settings as a
/// slice has them: whether a slice reads an option with these values is
/// the global page's mark (section 12), which the rules mostly hide or
/// grey for these pages too.
pub(crate) fn item_followed(kind: ItemKind, key: &str, effective: &Settings) -> Following {
    let _ = effective;
    let settable = edge_engine::settable(key);
    let followed = match kind {
        ItemKind::Object => settable.followed_on_object,
        ItemKind::Part => settable.followed_on_part,
        ItemKind::Range => settable.followed_on_range,
    };
    if let Some(note) = edge_engine::unfollowed_note(key, kind.said()) {
        return Following::NotFollowed(note);
    }
    if followed {
        Following::Followed
    } else {
        Following::NotFollowed(format!("{key}: {} {}", words::NOT_READ_THERE, kind.said()))
    }
}

/// The options an item of a kind may set, in the order of the pages.
pub(crate) fn allowed(kind: ItemKind) -> &'static [String] {
    use std::sync::OnceLock;
    static LISTS: OnceLock<[Vec<String>; 3]> = OnceLock::new();
    let lists = LISTS.get_or_init(|| {
        let items = &pages(Book::Process).json["items"];
        [ItemKind::Object, ItemKind::Part, ItemKind::Range].map(|kind| {
            items[kind.name()]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|key| key.as_str().map(str::to_owned))
                .collect()
        })
    });
    match kind {
        ItemKind::Object => &lists[0],
        ItemKind::Part => &lists[1],
        ItemKind::Range => &lists[2],
    }
}

/// Which item a call names: `object`, and `part` or `range` (its place
/// among the object's height ranges, from 0).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Named {
    object: u64,
    part: Option<u64>,
    range: Option<usize>,
}

impl Named {
    fn of(args: &Value) -> Result<Named, String> {
        let object = whole(args, "object")?;
        let part = args.get("part").and_then(Value::as_u64);
        let range = args
            .get("range")
            .and_then(Value::as_u64)
            .map(|range| range as usize);
        if part.is_some() && range.is_some() {
            return Err("a part or a height range, not both".into());
        }
        Ok(Named {
            object,
            part,
            range,
        })
    }

    fn kind(self) -> ItemKind {
        match (self.part, self.range) {
            (Some(_), _) => ItemKind::Part,
            (_, Some(_)) => ItemKind::Range,
            _ => ItemKind::Object,
        }
    }
}

/// An item's own settings as kept, and its object's.
struct Found {
    object_own: Vec<(String, String)>,
    own: Vec<(String, String)>,
    name: String,
    range: Option<LayerRange>,
}

fn found(model: &Model, named: Named) -> Result<Found, String> {
    let object = model
        .objects()
        .iter()
        .find(|object| object.id() == named.object)
        .ok_or_else(|| format!("there is no object {}", named.object))?;
    let object_own = object.settings();
    match (named.part, named.range) {
        (Some(part), _) => {
            let at = object
                .shape()
                .part_ids()
                .iter()
                .position(|id| *id == part)
                .ok_or_else(|| format!("there is no part {part}"))?;
            let held = &object.shape().parts()[at];
            // GUI_ObjectList.cpp `add_settings_item`: none for these.
            if !matches!(held.kind, PartKind::Normal | PartKind::Modifier) {
                return Err(words::HELPER.into());
            }
            Ok(Found {
                object_own,
                own: held.settings(),
                name: held.name.clone().unwrap_or_default(),
                range: None,
            })
        }
        (_, Some(range)) => {
            let ranges = object.layer_ranges();
            let held = ranges
                .get(range)
                .ok_or_else(|| format!("the object has no height range {}", range + 1))?;
            Ok(Found {
                object_own,
                own: held.settings.clone(),
                name: format!("{:.2}-{:.2}", held.min_z, held.max_z),
                range: Some(held.clone()),
            })
        }
        _ => Ok(Found {
            own: object_own.clone(),
            object_own,
            name: object.name().to_owned(),
            range: None,
        }),
    }
}

/// `under` with `own` laid over it, each kept text read as the reference
/// writes it.
fn laid_over(under: &Settings, own: &[(String, String)]) -> Settings {
    let mut settings = under.clone();
    for (key, value) in own {
        settings.insert(key.clone(), edge_config::deserialized(value));
    }
    settings
}

/// The settings of the project as a slice would have them: the item
/// pages' "Global".
fn global(session: &Session, id: u32) -> Result<(Settings, usize), String> {
    let (choices, file, filaments) = {
        let state = lock(&session.state);
        let project = &state.projects[state.target(&json!({ "project": id }))?];
        (
            project.choices.clone(),
            project.file.clone(),
            project.filament_count(),
        )
    };
    let basis = session.basis(&choices, file.as_deref())?;
    let settled = edge_engine::settled(
        &basis.chosen,
        &overrides(&choices, &basis),
        basis.project.as_deref(),
    )
    .unwrap_or_else(|_| edge_config::reference_defaults().clone());
    let mut complete = edge_config::reference_defaults().clone();
    complete.overlay(&settled);
    Ok((complete, filaments))
}

impl Session {
    pub(crate) fn items_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "item_settings" => self.item_settings(args),
            "layer_profile" => self.layer_profile(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// The settings of one item: its own and those in force from over it,
    /// which options it may set, and what the rules and the engine say of
    /// each.
    fn item_settings(&self, args: &Value) -> Result<Value, String> {
        let named = Named::of(args)?;
        let (id, file) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (project.id, project.file.clone())
        };
        let file = file.ok_or("the project has no objects")?;
        let item = found(&file.model, named)?;
        let (global, filaments) = global(self, id)?;
        Ok(described(id, named, &item, &global, filaments))
    }

    /// An object's layers for the "Variable layer height" tool: its layer
    /// height and the limits of its layer heights, its profile, the layers
    /// it lays and its height ranges, in its own heights.
    fn layer_profile(&self, args: &Value) -> Result<Value, String> {
        let object = args
            .get("object")
            .and_then(Value::as_u64)
            .ok_or("object was not given")?;
        let (id, file) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (project.id, project.file.clone())
        };
        let file = file.ok_or("the project has no objects")?;
        let (global, _) = global(self, id)?;
        let layers = file
            .model
            .object_layers(&global, object)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "project": id,
            "object": object,
            "layer_height": layers.layer_height,
            "first_layer_height": layers.first_layer_height,
            "min_layer_height": layers.min_layer_height,
            "max_layer_height": layers.max_layer_height,
            "height": layers.height,
            "profile": layers.profile,
            "own_profile": layers.own_profile,
            "layers": layers.layers,
            "ranges": layers.ranges,
        }))
    }

    /// The project's settings as the items' pages lay theirs over, for
    /// `scene_edit` to give [`apply`].
    pub(crate) fn items_global(&self, id: u32) -> Result<Settings, String> {
        global(self, id).map(|(settings, _)| settings)
    }
}

/// The answer of `item_settings`.
fn described(id: u32, named: Named, item: &Found, global: &Settings, filaments: usize) -> Value {
    let kind = named.kind();
    let book = pages(Book::Process);
    let keys = allowed(kind);
    let offered: BTreeSet<&str> = keys.iter().map(String::as_str).collect();
    // The parent's settings and the item's own over them.
    let parent = match kind {
        ItemKind::Object => global.clone(),
        _ => laid_over(global, &item.object_own),
    };
    let effective = laid_over(&parent, &item.own);
    let shown_in = |settings: &Settings, key: &str| -> Option<String> {
        let option = book.options.get(key)?;
        settings.get(key).map(|value| shown(option, value))
    };
    let values: BTreeMap<&str, String> = keys
        .iter()
        .filter_map(|key| Some((key.as_str(), shown_in(&parent, key)?)))
        .collect();
    let own: BTreeMap<&str, String> = item
        .own
        .iter()
        .filter(|(key, _)| offered.contains(key.as_str()))
        .filter_map(|(key, value)| {
            let option = book.options.get(key.as_str())?;
            Some((
                key.as_str(),
                shown(option, &edge_config::deserialized(value)),
            ))
        })
        .collect();
    let toggles = rules::toggles(&effective, filaments);
    let hidden: Vec<&str> = keys
        .iter()
        .map(String::as_str)
        .filter(|key| toggles.hidden(key))
        .collect();
    let off: Vec<&str> = keys
        .iter()
        .map(String::as_str)
        .filter(|key| toggles.off(key))
        .collect();
    let mut unread = Vec::new();
    let mut notes: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut why: BTreeMap<&str, String> = BTreeMap::new();
    for key in keys {
        match item_followed(kind, key, &effective) {
            Following::Followed => {}
            Following::Noted(note) => {
                notes.insert(key, vec![note]);
            }
            Following::NotFollowed(reason) => {
                unread.push(key.as_str());
                why.insert(key, reason);
            }
        }
    }
    // One sentence for all of them, when they all say the same.
    let reasons: BTreeSet<&String> = why.values().collect();
    let unread_note = (reasons.len() == 1).then(|| reasons.into_iter().next().cloned());
    // How many each page has set, the reference's "Frequent" page first.
    let frequent = &book.json["items"]["frequent"];
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(title) = frequent["title"].as_str() {
        let on: usize = frequent["keys"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|key| own.contains_key(key))
            .count();
        counts.insert(title.to_owned(), on);
    }
    for page in &book.pages {
        let on = page
            .lines
            .iter()
            .flatten()
            .filter(|key| own.contains_key(key.as_str()))
            .count();
        counts.insert(page.title.clone(), on);
    }
    let range = item
        .range
        .as_ref()
        .map(|range| json!({ "min_z": range.min_z, "max_z": range.max_z }));
    json!({
        "project": id,
        "kind": "process",
        "item": {
            "kind": kind.name(),
            "object": named.object,
            "part": named.part,
            "range": named.range,
            "name": item.name,
            "heights": range,
        },
        "keys": keys,
        "values": values,
        "set": own,
        "changed": own.len(),
        "hidden": hidden,
        "off": off,
        "unread": unread,
        "unread_note": unread_note.flatten(),
        "why": if reasons_differ(&why) { json!(why) } else { json!({}) },
        "notes": notes,
        "counts": counts,
        "refused": null,
    })
}

fn reasons_differ(why: &BTreeMap<&str, String>) -> bool {
    why.values().collect::<BTreeSet<_>>().len() > 1
}

/// A height range as a new one starts (GUI_ObjectList.cpp
/// `get_default_layer_config`): the object's layer height, else the
/// project's, and filament 0, "default".
fn new_range(
    min_z: f64,
    max_z: f64,
    object_own: &[(String, String)],
    global: &Settings,
) -> LayerRange {
    let layer_height = object_own
        .iter()
        .find(|(key, _)| key == "layer_height")
        .map(|(_, value)| value.clone())
        .or_else(|| global.text("layer_height").map(str::to_owned))
        .unwrap_or_else(|| "0.2".into());
    LayerRange {
        min_z,
        max_z,
        settings: vec![
            ("extruder".into(), "0".into()),
            ("layer_height".into(), layer_height),
        ],
    }
}

/// The least layer the printer makes (GUI_ObjectList.cpp
/// `get_min_layer_height`: the printer's `min_layer_height` of its first
/// extruder).
fn least_layer(global: &Settings) -> f64 {
    global
        .floats("min_layer_height")
        .ok()
        .flatten()
        .and_then(|values| values.first().copied().flatten())
        .unwrap_or(0.07)
}

/// A height as typed: a number, not under the bed (GUI_ObjectLayers.cpp
/// `LayerRangeEditor::get_value`: "Invalid numeric.", which the reference
/// says and then takes the value all the same; here it is refused).
fn height(args: &Value, key: &str) -> Result<Option<f64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let number = match value {
                Value::Number(number) => number.as_f64(),
                Value::String(text) => text.trim().parse::<f64>().ok(),
                _ => None,
            };
            match number {
                Some(number) if number.is_finite() && number >= 0.0 => Ok(Some(number)),
                _ => Err(tr("Invalid numeric.")),
            }
        }
    }
}

/// Sets `key` among a list of settings as kept, or takes it away.
fn set_in(list: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    match (list.iter().position(|(held, _)| held == key), value) {
        (Some(at), Some(value)) => list[at].1 = value,
        (Some(at), None) => {
            list.remove(at);
        }
        (None, Some(value)) => {
            list.push((key.to_owned(), value));
            // The reference's config keeps its keys in order.
            list.sort_by(|a, b| a.0.cmp(&b.0));
        }
        (None, None) => {}
    }
}

/// Makes one of the [`OPS`] to a model, with the project's settings as
/// `global`. Answers the step's name in the history, or none when nothing
/// changed.
pub(crate) fn apply(
    model: &mut Model,
    bed: Option<&Bed>,
    op: &str,
    args: &Value,
    global: &Settings,
) -> Result<Option<String>, String> {
    let failed = |error: edge_engine::Failure| error.to_string();
    if op.starts_with("profile_") {
        return profile_edit(model, bed, op, args, global);
    }
    let named = Named::of(args)?;
    let kind = named.kind();
    let item = found(model, named)?;
    let object = named.object;
    // Each edit of an item's settings, as kept.
    let mut changes: Vec<SettingChange> = Vec::new();
    let name = match op {
        "item_set" => {
            let key = needed(args, "key")?;
            if !allowed(kind).iter().any(|held| held == key) {
                return Err(format!("{key} {} {}", words::NOT_ON_THIS_ITEM, kind.said()));
            }
            let option: &Described = pages(Book::Process)
                .options
                .get(key)
                .ok_or_else(|| format!("there is no process setting named {key}"))?;
            if let Some(why) = option.fixed {
                return Err(format!("{} cannot be changed here. {why}", option.label));
            }
            match args.get("value") {
                None => return Err("value was not given".into()),
                // The arrow: the value goes back to the one over it.
                Some(Value::Null) => {
                    if !item.own.iter().any(|(held, _)| held == key) {
                        return Ok(None);
                    }
                    changes.push((key.to_owned(), None));
                }
                Some(value) => {
                    let value = checked(option, value)?;
                    // In the shape the value over it has (a list with a
                    // value for each nozzle flow stays one), as the
                    // reference's config would write it.
                    let parent = match kind {
                        ItemKind::Object => global.clone(),
                        _ => laid_over(global, &item.object_own),
                    };
                    let setting: Setting = held(option, &value, parent.get(address(key).0));
                    let text = edge_config::serialized(key, &setting);
                    if item
                        .own
                        .iter()
                        .any(|(held, now)| held == key && *now == text)
                    {
                        return Ok(None);
                    }
                    changes.push((key.to_owned(), Some(text)));
                }
            }
            // Tab.cpp `TabPrintModel::on_value_change`, untranslated.
            format!("Change Option {key}")
        }
        "item_reset" => {
            let offered = allowed(kind);
            changes = item
                .own
                .iter()
                .filter(|(key, _)| offered.contains(key))
                .map(|(key, _)| (key.clone(), None))
                .collect();
            if changes.is_empty() {
                return Ok(None);
            }
            // Tab.cpp `TabPrintModel::reset_model_config`.
            "Reset Options".to_owned()
        }
        "range_add" | "range_set" | "range_delete" => {
            let mut ranges = model
                .objects()
                .iter()
                .find(|held| held.id() == object)
                .map(|held| held.layer_ranges())
                .unwrap_or_default();
            let at = args
                .get("range")
                .and_then(Value::as_u64)
                .map(|at| at as usize);
            let name = match op {
                "range_add" => match at {
                    // "Height range Modifier" on an object without ranges
                    // (`layers_editing`): 0 to 2 mm.
                    None if ranges.is_empty() => {
                        ranges.push(new_range(0.0, 2.0, &item.object_own, global));
                        "Add layers"
                    }
                    None => return Ok(None),
                    Some(at) => {
                        add_after(&mut ranges, at, &item.object_own, global)?;
                        "Add height range"
                    }
                },
                "range_set" => {
                    let at = at.ok_or("range was not given")?;
                    let held = ranges
                        .get(at)
                        .cloned()
                        .ok_or_else(|| format!("the object has no height range {}", at + 1))?;
                    let (mut min_z, mut max_z) = (held.min_z, held.max_z);
                    if let Some(min) = height(args, "min_z")? {
                        min_z = min;
                        // GUI_ObjectLayers.cpp: a bottom typed at or over the
                        // top takes the top half a millimetre over it.
                        if min_z >= max_z {
                            max_z = min_z + 0.5;
                        }
                    }
                    if let Some(max) = height(args, "max_z")? {
                        if max <= min_z {
                            return Err(words::RANGE_ENDS_LOW.into());
                        }
                        max_z = max;
                    }
                    if (min_z - held.min_z).abs() < 1e-9 && (max_z - held.max_z).abs() < 1e-9 {
                        return Ok(None);
                    }
                    ranges[at].min_z = min_z;
                    ranges[at].max_z = max_z;
                    words::EDIT_RANGE
                }
                _ => {
                    let at = at.ok_or("range was not given")?;
                    if at >= ranges.len() {
                        return Err(format!("the object has no height range {}", at + 1));
                    }
                    ranges.remove(at);
                    // GUI_ObjectList.cpp `del_layer_range`.
                    "Remove height range"
                }
            };
            check_ranges(&ranges)?;
            model
                .set_height_ranges(bed, object, ranges)
                .map_err(failed)?;
            return Ok(Some(name.to_owned()));
        }
        other => return Err(format!("there is no edit named {other}")),
    };
    match (named.part, named.range) {
        (Some(part), _) => model
            .set_part_settings(bed, object, part, &changes)
            .map_err(failed)?,
        (_, Some(range)) => {
            let mut ranges = model
                .objects()
                .iter()
                .find(|held| held.id() == object)
                .map(|held| held.layer_ranges())
                .unwrap_or_default();
            let held = ranges
                .get_mut(range)
                .ok_or_else(|| format!("the object has no height range {}", range + 1))?;
            for (key, value) in changes {
                set_in(&mut held.settings, &key, value);
            }
            model
                .set_height_ranges(bed, object, ranges)
                .map_err(failed)?;
        }
        _ => model
            .set_object_settings(bed, object, &changes)
            .map_err(failed)?,
    }
    Ok(Some(name))
}

/// One change of the "Variable layer height" tool (GLCanvas3D.cpp
/// `LayersEditing`), one step of the history each, named as the reference
/// names its snapshots (untranslated, as `take_snapshot` takes them).
fn profile_edit(
    model: &mut Model,
    bed: Option<&Bed>,
    op: &str,
    args: &Value,
    global: &Settings,
) -> Result<Option<String>, String> {
    use edge_engine::{EditAction, ProfileEdit};
    let object = args
        .get("object")
        .and_then(Value::as_u64)
        .ok_or("object was not given")?;
    let number = |key: &str, otherwise: f64| -> Result<f64, String> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(otherwise),
            Some(value) => value
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| tr("Invalid numeric.")),
        }
    };
    let (edit, name) = match op {
        "profile_reset" => (ProfileEdit::Reset, "Variable layer height - Reset"),
        "profile_adaptive" => (
            // "Quality / Speed", 0.5 at first (GLCanvas3D.hpp).
            ProfileEdit::Adaptive {
                quality: number("quality", 0.5)?.clamp(0.0, 1.0) as f32,
            },
            "Variable layer height - Adaptive",
        ),
        "profile_smooth" => (
            // "Radius" 1 to 10, 5 at first; "Keep min" off
            // (Slicing.hpp `HeightProfileSmoothingParams`).
            ProfileEdit::Smooth {
                radius: number("radius", 5.0)?.round().clamp(1.0, 10.0) as u32,
                keep_min: args
                    .get("keep_min")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
            "Variable layer height - Smooth all",
        ),
        _ => {
            let strokes = args
                .get("strokes")
                .and_then(Value::as_array)
                .ok_or("strokes was not given")?
                .iter()
                .map(|stroke| -> Result<(f64, EditAction), String> {
                    let z = stroke
                        .get("z")
                        .and_then(Value::as_f64)
                        .filter(|z| z.is_finite())
                        .ok_or_else(|| tr("Invalid numeric."))?;
                    let action = match stroke.get("action").and_then(Value::as_str) {
                        Some("increase") => EditAction::Increase,
                        Some("decrease") => EditAction::Decrease,
                        Some("reduce") => EditAction::Reduce,
                        Some("smooth") => EditAction::Smooth,
                        other => return Err(format!("there is no stroke named {other:?}")),
                    };
                    Ok((z, action))
                })
                .collect::<Result<Vec<_>, String>>()?;
            if strokes.is_empty() {
                return Ok(None);
            }
            (
                // 2 mm wide (1.5 to 10 by the wheel), 0.005 mm a stroke
                // (GLCanvas3D.hpp `LayersEditing`).
                ProfileEdit::Strokes {
                    band_width: number("band_width", 2.0)?.clamp(1.5, 10.0),
                    strength: number("strength", 0.005)?.max(0.0),
                    strokes,
                },
                "Variable layer height - Manual edit",
            )
        }
    };
    let changed = model
        .edit_layer_profile(bed, global, object, &edit)
        .map_err(|error| error.to_string())?;
    Ok(changed.then(|| name.to_owned()))
}

/// Ranges as they are to be kept: above the bed, each with its top over
/// its bottom, none over another.
fn check_ranges(ranges: &[LayerRange]) -> Result<(), String> {
    let mut sorted: Vec<&LayerRange> = ranges.iter().collect();
    sorted.sort_by(|a, b| a.min_z.total_cmp(&b.min_z));
    for range in &sorted {
        if range.max_z <= range.min_z {
            return Err(words::RANGE_ENDS_LOW.into());
        }
    }
    if let Some(pair) = sorted
        .windows(2)
        .find(|pair| pair[1].min_z < pair[0].max_z - 1e-9)
    {
        return Err(format!(
            "{} {:.2}-{:.2} mm, {:.2}-{:.2} mm.",
            words::RANGES_OVERLAP,
            pair[0].min_z,
            pair[0].max_z,
            pair[1].min_z,
            pair[1].max_z
        ));
    }
    Ok(())
}

/// "Add height range" after range `at` (GUI_ObjectList.cpp
/// `add_layer_range_after_current`): after the last, a range of 2 mm on
/// top of it; where the next starts at its top, the next split in two, the
/// new one under; where there is a gap of a layer or more, the gap.
fn add_after(
    ranges: &mut Vec<LayerRange>,
    at: usize,
    object_own: &[(String, String)],
    global: &Settings,
) -> Result<(), String> {
    let current = ranges
        .get(at)
        .cloned()
        .ok_or_else(|| format!("the object has no height range {}", at + 1))?;
    let least = least_layer(global);
    const EPSILON: f64 = 1e-4;
    match ranges.get(at + 1).cloned() {
        None => {
            ranges.push(new_range(
                current.max_z,
                current.max_z + 2.0,
                object_own,
                global,
            ));
        }
        Some(next) if (next.min_z - current.max_z).abs() < 1e-9 => {
            let delta = next.max_z - next.min_z;
            // Both the range split and the new one take a layer at least.
            if delta < 2.0 * least - EPSILON {
                return Err(words::NO_ROOM.into());
            }
            let middle = if least > 0.5 * delta {
                next.max_z - least
            } else {
                next.min_z + least.max(0.5 * delta)
            };
            ranges[at + 1].min_z = middle;
            ranges.insert(at + 1, new_range(current.max_z, middle, object_own, global));
        }
        Some(next) if next.min_z - current.max_z >= least - EPSILON => {
            ranges.insert(
                at + 1,
                new_range(current.max_z, next.min_z, object_own, global),
            );
        }
        Some(_) => return Err(words::NO_ROOM.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests;
