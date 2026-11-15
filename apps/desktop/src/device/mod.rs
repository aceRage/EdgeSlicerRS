//! The Device page of every printer family, on the Bambu monitor's layout
//! (StatusPanel.cpp): what a printer can show and do (`Caps`), what it
//! shows now (`Reading`), and what it is told (`Command`), behind one trait
//! with an adapter for each family.
//!
//! The owner's decision of 2026-11-15: every family gets the native page,
//! not only Bambu (the reference shows a print host its own web page). An
//! adapter draws only what its family's API has; nothing is drawn dead.
//!
//! Safety: every command is refused here unless it was confirmed by the
//! person (`gate` answers the question to ask), and in a state where it
//! cannot act (pause when not printing, the axes while printing, a printer
//! that does not answer). Nothing here looks for printers on the network;
//! an adapter asks only the address it was given. A key or an access code
//! never goes into a `Reading`, a `Caps` or an error.

use serde_json::{Value, json};

use crate::api::i18n::{fill, tr};

pub mod bambu;
pub mod klipper;
pub mod octo;
pub mod prusalink;
pub mod sdcp;
#[cfg(test)]
mod tests;

/// One heater: a toolhead, the bed, the chamber.
#[derive(Clone, Debug, PartialEq)]
pub struct HeaterCap {
    /// What the adapter calls it ("extruder1", "heater_bed", "nozzle").
    pub id: String,
    /// Already in the page's language.
    pub label: String,
    /// The highest target the printer takes; a higher one is brought down
    /// to it, as the reference's `TempInput` does.
    pub max: f64,
    pub settable: bool,
}

/// One fan, 0..100 %.
#[derive(Clone, Debug, PartialEq)]
pub struct FanCap {
    pub id: String,
    pub label: String,
    pub settable: bool,
}

/// The printing speed: Bambu's four levels, or a percent.
#[derive(Clone, Debug, PartialEq)]
pub enum SpeedCap {
    /// (value sent, label).
    Levels(Vec<(i64, String)>),
    Percent {
        min: i64,
        max: i64,
    },
}

/// The axes: jog steps of 1 and 10 mm (StatusPanel's ring), home, and the
/// extruder (10 mm up or down).
#[derive(Clone, Debug, PartialEq)]
pub struct JogCap {
    /// "Bed" on a CoreXY Bambu (the bed moves), else "Z".
    pub z_label: String,
    pub extrude: bool,
}

/// What a printer can show and do: what the page draws.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Caps {
    /// "bambu", "u1", "klipper", "octoprint", "elegoo", "prusalink".
    pub family: &'static str,
    pub pause: bool,
    pub resume: bool,
    pub stop: bool,
    pub heaters: Vec<HeaterCap>,
    pub fans: Vec<FanCap>,
    pub speed: Option<SpeedCap>,
    /// The lamp: its id.
    pub light: Option<String>,
    pub jog: Option<JogCap>,
    /// "ams" or "toolheads".
    pub slots: Option<&'static str>,
    pub camera: bool,
    /// Bambu's "HMS" and "Update" tabs.
    pub hms: bool,
    pub versions: bool,
}

/// A heater now.
#[derive(Clone, Debug, PartialEq)]
pub struct Heater {
    pub id: String,
    pub now: f64,
    pub target: f64,
}

/// One slot of an AMS or one toolhead, and what it holds.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Slot {
    pub name: String,
    pub kind: String,
    /// `#RRGGBB`, or empty.
    pub color: String,
    pub loaded: bool,
    pub active: bool,
    /// 0..100, -1 unknown.
    pub remain: i64,
    /// More about it (vendor, sub-type, nozzle).
    pub detail: String,
}

/// One HMS message of a Bambu printer.
#[derive(Clone, Debug, PartialEq)]
pub struct Hms {
    pub code: String,
    pub level: String,
    pub module: String,
    pub text: String,
}

/// What a printer shows now.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Reading {
    /// "idle", "preparing", "printing", "paused", "finished", "cancelled",
    /// "error", "busy".
    pub state: String,
    /// The state in the page's language, or the stage the printer is on.
    pub said: String,
    pub file: String,
    pub percent: Option<u32>,
    /// Seconds.
    pub left: Option<u32>,
    pub layer: Option<u32>,
    pub total_layers: Option<u32>,
    /// The printer's own message (an error, a stage).
    pub message: String,
    pub heaters: Vec<Heater>,
    /// (fan id, percent).
    pub fans: Vec<(String, u32)>,
    /// The level or the percent in force.
    pub speed: Option<i64>,
    pub light: Option<bool>,
    /// The axes homed ("xyz", "xy", ""), when the printer says.
    pub homed: Option<String>,
    /// Groups of slots (an AMS each, or the toolheads), with their names.
    pub slots: Vec<(String, Vec<Slot>)>,
    pub camera: Option<String>,
    pub hms: Vec<Hms>,
    /// (module, software version, hardware version, serial).
    pub versions: Vec<(String, String, String, String)>,
}

impl Reading {
    pub fn heater(&self, id: &str) -> Option<&Heater> {
        self.heaters.iter().find(|heater| heater.id == id)
    }
}

/// What a printer is told.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Pause,
    Resume,
    Stop,
    Temperature {
        heater: String,
        target: f64,
    },
    Fan {
        fan: String,
        percent: u32,
    },
    Speed(i64),
    Light(bool),
    /// An axis ('x', 'y', 'z') and millimetres.
    Jog {
        axis: char,
        mm: f64,
    },
    Home,
    /// Millimetres of filament, negative to draw it back.
    Extrude(f64),
}

/// One family's printers.
pub trait Device: Send + Sync {
    /// What the page draws. May ask the printer (which objects a Klipper
    /// has); the session keeps the answer while Device is looked at.
    fn caps(&self) -> Result<Caps, String>;
    /// What the printer shows now.
    fn read(&self, caps: &Caps) -> Result<Reading, String>;
    /// Tells the printer. Called only once `gate` let it through and the
    /// person confirmed it.
    fn command(&self, caps: &Caps, command: &Command) -> Result<(), String>;
}

/// The question asked before a command, in the page's language.
#[derive(Clone, Debug, PartialEq)]
pub struct Confirm {
    pub title: String,
    pub lines: Vec<String>,
    /// The confirming button's words.
    pub ok: String,
}

impl Confirm {
    pub fn json(&self) -> Value {
        json!({ "title": self.title, "lines": self.lines, "ok": self.ok, "blocking": false })
    }
}

/// The reference's lowest temperature a target is set to (StatusPanel.cpp
/// `bed_temp_range`, `nozzle_temp_range`: 20); 0 turns a heater off.
const LOWEST: f64 = 20.0;
/// Below this the reference asks before the extruder moves
/// (`TEMP_THRESHOLD_ALLOW_E_CTRL`).
const COLD_EXTRUSION: f64 = 170.0;

/// A command read from the page's arguments.
pub fn command_of(args: &Value) -> Result<Command, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or("action was not given")?;
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(format!("{key} was not given"))
    };
    let number = |key: &str| {
        args.get(key)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .ok_or(format!("{key} was not given"))
    };
    Ok(match action {
        "pause" => Command::Pause,
        "resume" => Command::Resume,
        "stop" => Command::Stop,
        "temperature" => Command::Temperature {
            heater: text("heater")?,
            target: number("target")?,
        },
        "fan" => Command::Fan {
            fan: text("fan")?,
            percent: number("percent")?.clamp(0.0, 100.0).round() as u32,
        },
        "speed" => Command::Speed(number("value")?.round() as i64),
        "light" => Command::Light(
            args.get("on")
                .and_then(Value::as_bool)
                .ok_or("on was not given")?,
        ),
        "jog" => {
            let axis = text("axis")?;
            let axis = match axis.as_str() {
                "x" | "y" | "z" => axis.chars().next().unwrap_or('x'),
                other => return Err(format!("{other} is not an axis")),
            };
            let mm = number("mm")?;
            if ![1.0, 10.0].contains(&mm.abs()) {
                return Err("The axes move by 1 or 10 mm.".into());
            }
            Command::Jog { axis, mm }
        }
        "home" => Command::Home,
        "extrude" => {
            let mm = number("mm")?;
            if mm.abs() != 10.0 {
                return Err("The extruder moves by 10 mm.".into());
            }
            Command::Extrude(mm)
        }
        other => return Err(format!("there is no command {other}")),
    })
}

/// Whether the printer is on a job it has not finished.
fn on_job(state: &str) -> bool {
    matches!(state, "printing" | "paused" | "preparing")
}

/// The sentence every command that moves or heats the printer ends with:
/// what to look at on the printer first.
const CHECK: &str = "Before you go on, look at the printer: the bed and the space around the toolhead should be clear, and nobody's hands near it.";

/// Whether `command` may be sent to a printer that can do `caps` and is
/// doing `reading`, and the question to ask first (every command is
/// asked; the reference asks only before "Stop", the owner asks before
/// everything). `printer` is its name, for the question. A target above
/// the heater's limit is brought down to it (the command is changed).
pub fn gate(
    command: &mut Command,
    caps: &Caps,
    reading: &Reading,
    printer: &str,
) -> Result<Confirm, String> {
    let refuse = |why: &str| Err(why.to_owned());
    let state = reading.state.as_str();
    let confirm = |title: String, line: String, check: bool| {
        let mut lines = vec![line];
        if check {
            lines.push(tr(CHECK));
        }
        Ok(Confirm {
            title,
            lines,
            ok: tr("Confirm"),
        })
    };
    let set =
        |what: &str, value: String| fill(&tr("Set %s to %s on %s?"), &[&what, &value, &printer]);
    match command {
        Command::Pause => {
            if !caps.pause {
                return refuse(&tr(NOT_HERE));
            }
            if state != "printing" {
                return refuse(&tr(
                    "The printer is not printing, so there is nothing to pause.",
                ));
            }
            confirm(
                tr("Pause"),
                fill(&tr("Pause the print on %s?"), &[&printer]),
                false,
            )
        }
        Command::Resume => {
            if !caps.resume {
                return refuse(&tr(NOT_HERE));
            }
            if state != "paused" {
                return refuse(&tr("The printer has no paused print to resume."));
            }
            confirm(
                tr("Resume"),
                fill(&tr("Resume the print on %s?"), &[&printer]),
                true,
            )
        }
        Command::Stop => {
            if !caps.stop {
                return refuse(&tr(NOT_HERE));
            }
            if !on_job(state) {
                return refuse(&tr(
                    "The printer is not printing, so there is nothing to stop.",
                ));
            }
            // StatusPanel.cpp l.2051: the reference's own question.
            confirm(
                tr("Cancel print"),
                tr("Are you sure you want to cancel this print?"),
                false,
            )
        }
        Command::Temperature { heater, target } => {
            let Some(cap) = caps.heaters.iter().find(|cap| cap.id == *heater) else {
                return refuse(&tr(NOT_HERE));
            };
            if !cap.settable {
                return refuse(&tr(NOT_HERE));
            }
            if *target < 0.0 {
                return refuse(&tr("A temperature below 0 °C cannot be set."));
            }
            // TempInput: below the lowest is off, above the limit the limit.
            let mut value = target.round();
            if value > 0.0 && value < LOWEST {
                value = LOWEST;
            }
            if value > cap.max {
                value = cap.max;
            }
            *target = value;
            let shown = if value == 0.0 {
                tr("Off")
            } else {
                format!("{value} °C")
            };
            confirm(tr("Confirm"), set(&cap.label, shown), value > 0.0)
        }
        Command::Fan { fan, percent } => {
            let Some(cap) = caps.fans.iter().find(|cap| cap.id == *fan) else {
                return refuse(&tr(NOT_HERE));
            };
            if !cap.settable {
                return refuse(&tr(NOT_HERE));
            }
            confirm(
                tr("Confirm"),
                set(&cap.label, format!("{percent} %")),
                false,
            )
        }
        Command::Speed(value) => {
            let shown = match &caps.speed {
                Some(SpeedCap::Levels(levels)) => match levels.iter().find(|(v, _)| v == value) {
                    Some((_, label)) => label.clone(),
                    None => return refuse(&tr(NOT_HERE)),
                },
                Some(SpeedCap::Percent { min, max }) => {
                    *value = (*value).clamp(*min, *max);
                    format!("{value} %")
                }
                None => return refuse(&tr(NOT_HERE)),
            };
            confirm(tr("Confirm"), set(&tr("Speed"), shown), false)
        }
        Command::Light(on) => {
            if caps.light.is_none() {
                return refuse(&tr(NOT_HERE));
            }
            confirm(
                tr("Confirm"),
                set(&tr("Lamp"), if *on { tr("On") } else { tr("Off") }),
                false,
            )
        }
        Command::Jog { axis, mm } => {
            let Some(jog) = &caps.jog else {
                return refuse(&tr(NOT_HERE));
            };
            if state == "printing" || state == "preparing" {
                return refuse(&tr(MOVING));
            }
            let axis = *axis;
            let name = match axis {
                'z' => jog.z_label.clone(),
                other => other.to_ascii_uppercase().to_string(),
            };
            let mut lines = vec![fill(
                &tr("Move %s by %s mm on %s?"),
                &[&name, &format!("{mm:+}"), &printer],
            )];
            // RecenterDialog: the reference sends the move, then asks for a
            // home when the axis is not homed; here it is said first.
            if reading
                .homed
                .as_ref()
                .is_some_and(|homed| !homed.contains(axis))
            {
                lines.push(format!(
                    "{}{}",
                    tr("Please home all axes (click "),
                    tr(") to locate the toolhead's position. This prevents device moving beyond the printable boundary and causing equipment wear.")
                ));
            }
            lines.push(tr(CHECK));
            Ok(Confirm {
                title: tr("Confirm"),
                lines,
                ok: tr("Confirm"),
            })
        }
        Command::Home => {
            if caps.jog.is_none() {
                return refuse(&tr(NOT_HERE));
            }
            if state == "printing" || state == "preparing" {
                return refuse(&tr(MOVING));
            }
            confirm(
                tr("Go Home"),
                fill(&tr("Home all axes on %s?"), &[&printer]),
                true,
            )
        }
        Command::Extrude(mm) => {
            if !caps.jog.as_ref().is_some_and(|jog| jog.extrude) {
                return refuse(&tr(NOT_HERE));
            }
            if state == "printing" || state == "preparing" {
                return refuse(&tr(MOVING));
            }
            let load = *mm > 0.0;
            let line = if load {
                fill(
                    &tr("Push %s mm of filament through the nozzle on %s?"),
                    &[&mm.abs(), &printer],
                )
            } else {
                fill(
                    &tr("Draw %s mm of filament back from the nozzle on %s?"),
                    &[&mm.abs(), &printer],
                )
            };
            // StatusPanel.cpp l.3626: below 170 °C the reference warns,
            // with "Still load" / "Still unload".
            let cold = reading
                .heaters
                .first()
                .is_some_and(|nozzle| nozzle.now < COLD_EXTRUSION);
            if cold {
                return Ok(Confirm {
                    title: tr("Warning"),
                    lines: vec![
                        tr(
                            "Please heat the nozzle to above 170°C before loading or unloading filament.",
                        ),
                        line,
                    ],
                    ok: if load {
                        tr("Still load")
                    } else {
                        tr("Still unload")
                    },
                });
            }
            confirm(tr("Extruder"), line, false)
        }
    }
}

/// A control the printer has not got.
pub const NOT_HERE: &str = "This printer cannot do that from here.";
/// The axes while a print runs (the reference turns them off then).
pub const MOVING: &str = "The printer is printing: the axes and the extruder cannot be moved now.";

/// `Caps` for the page.
pub fn caps_json(caps: &Caps) -> Value {
    json!({
        "family": caps.family,
        "pause": caps.pause,
        "resume": caps.resume,
        "stop": caps.stop,
        "heaters": caps.heaters.iter().map(|h| json!({ "id": h.id, "label": h.label, "max": h.max, "settable": h.settable })).collect::<Vec<_>>(),
        "fans": caps.fans.iter().map(|f| json!({ "id": f.id, "label": f.label, "settable": f.settable })).collect::<Vec<_>>(),
        "speed": match &caps.speed {
            None => Value::Null,
            Some(SpeedCap::Levels(levels)) => json!({ "kind": "levels", "levels": levels.iter().map(|(v, l)| json!({ "value": v, "label": l })).collect::<Vec<_>>() }),
            Some(SpeedCap::Percent { min, max }) => json!({ "kind": "percent", "min": min, "max": max }),
        },
        "light": caps.light.is_some(),
        "jog": caps.jog.as_ref().map(|jog| json!({ "z_label": jog.z_label, "extrude": jog.extrude })),
        "slots": caps.slots,
        "camera": caps.camera,
        "hms": caps.hms,
        "versions": caps.versions,
    })
}

/// `Reading` for the page.
pub fn reading_json(reading: &Reading) -> Value {
    json!({
        "online": true,
        "state": reading.state,
        "said": reading.said,
        "file": reading.file,
        "percent": reading.percent,
        "left": reading.left,
        "left_text": reading.left.map(|left| crate::ultranet::machine::time_dhm(i64::from(left))),
        "layer": reading.layer,
        "total_layers": reading.total_layers,
        "message": reading.message,
        "heaters": reading.heaters.iter().map(|h| json!({ "id": h.id, "now": h.now, "target": h.target })).collect::<Vec<_>>(),
        "fans": reading.fans.iter().map(|(id, percent)| json!({ "id": id, "percent": percent })).collect::<Vec<_>>(),
        "speed": reading.speed,
        "light": reading.light,
        "homed": reading.homed,
        "slots": reading.slots.iter().map(|(name, slots)| json!({
            "name": name,
            "slots": slots.iter().map(|s| json!({
                "name": s.name, "type": s.kind, "color": s.color, "loaded": s.loaded,
                "active": s.active, "remain": s.remain, "detail": s.detail,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "camera": reading.camera,
        "hms": reading.hms.iter().map(|h| json!({ "code": h.code, "level": h.level, "module": h.module, "text": h.text })).collect::<Vec<_>>(),
        "versions": reading.versions.iter().map(|(name, sw, hw, sn)| json!({ "name": name, "sw": sw, "hw": hw, "sn": sn })).collect::<Vec<_>>(),
    })
}

/// A state of the reference's words (`print_stats.state` and the rest made
/// one of `Reading::state`'s).
pub fn state_said(state: &str) -> String {
    match state {
        "idle" => tr("Idle"),
        "preparing" => tr("Preparing"),
        "printing" => tr("Printing"),
        "paused" => tr("Paused"),
        "finished" => tr("Finished"),
        "cancelled" => tr("Canceled"),
        "error" => tr("Error"),
        "busy" => tr("Busy"),
        _ => tr("Unknown"),
    }
}

/// An address in a printer's answer (a webcam's `/webcam/?action=stream`)
/// made whole against the printer's base address; only http and https.
pub fn absolute(base: &str, path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    if path.starts_with("http://") || path.starts_with("https://") {
        return Some(path.to_owned());
    }
    if path.contains("://") {
        return None;
    }
    let after = base.find("://")? + 3;
    let origin_end = base[after..].find('/').map_or(base.len(), |at| after + at);
    if let Some(rest) = path.strip_prefix('/') {
        Some(format!("{}/{}", &base[..origin_end], rest))
    } else {
        Some(format!("{}{}", base, path))
    }
}
