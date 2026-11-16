//! A Klipper printer through Moonraker: the Snapmaker U1, and every other
//! Klipper printer behind "Octo/Klipper" that answers as a Moonraker.
//!
//! What is drawn comes from the printer's own list of objects
//! (`printer/objects/list`): its extruders, `heater_bed`, each
//! `heater_generic`, `fan` and each `fan_generic`, the first `led`, and the
//! U1's `print_task_config` (what each toolhead holds); the limits from
//! `configfile`. The commands are the ones the fork's own C++ sends a U1
//! (`DeviceControls.hpp`, `RemoteControl.cpp`): `printer/print/pause`,
//! `resume`, `cancel`, and through `printer/gcode/script`
//! `SET_HEATER_TEMPERATURE`, `M106`, `SET_FAN_SPEED`, `M220`, `SET_LED`; the
//! moves as the U1's own page writes them (`G91` / `G1 ... F3600`, Z
//! `F1800`, `G90`), `G28`, and `M83` / `G1 E`.
//!
//! The reference shows a U1 Snapmaker's closed page and any other Klipper
//! printer its own web page: this page is the owner's decision of
//! 2026-11-15, beyond it.

use std::time::Duration;

use serde_json::Value;

use super::{Caps, Command, Device, FanCap, Heater, HeaterCap, JogCap, Reading, Slot, SpeedCap};
use crate::api::i18n::tr;
use crate::hosts::{
    Http, http_error, moonraker,
    snapmaker::{encode, moonraker_status},
};

pub struct Klipper {
    pub http: Http,
    /// A Snapmaker U1: its limits (bed 100 °C, toolheads 300 °C, speed
    /// 50-150 % as its own page has them) and its toolheads.
    pub u1: bool,
}

/// The fork's limits for a U1 (`DeviceControls.hpp`: "U1 bed 100,
/// toolhead 300").
const U1_BED: f64 = 100.0;
const U1_TOOLHEAD: f64 = 300.0;

fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite())
}

/// `extruder`, `extruder1`, ...: its number.
fn extruder_index(name: &str) -> Option<usize> {
    let rest = name.strip_prefix("extruder")?;
    if rest.is_empty() {
        Some(0)
    } else {
        rest.parse().ok()
    }
}

impl Klipper {
    fn result(&self, path: &str) -> Result<Value, String> {
        moonraker::result(&self.http, path)
    }

    fn post(&self, path: &str) -> Result<(), String> {
        let answer = self.http.post_empty(path, Duration::from_secs(30))?;
        if answer.ok() {
            Ok(())
        } else {
            Err(http_error(&answer))
        }
    }

    /// G-code through `printer/gcode/script` (answered once it has run).
    fn script(&self, script: &str) -> Result<(), String> {
        self.post(&format!(
            "printer/gcode/script?script={}",
            encode(script, false)
        ))
    }

    /// The objects this printer has.
    fn objects(&self) -> Result<Vec<String>, String> {
        Ok(self.result("printer/objects/list")?["objects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| o.as_str().map(str::to_owned))
            .collect())
    }

    /// The webcam the page may show: the first enabled one Moonraker lists
    /// whose stream is a plain MJPEG stream (an `<img>` can show it; WebRTC
    /// and HLS cannot be shown without the camera's own page).
    fn camera(&self) -> Option<String> {
        let list = self.result("server/webcams/list").ok()?;
        list["webcams"].as_array()?.iter().find_map(|cam| {
            let service = cam["service"].as_str().unwrap_or("mjpegstreamer");
            let plain = service.starts_with("mjpegstreamer");
            let enabled = cam["enabled"].as_bool().unwrap_or(true);
            (plain && enabled)
                .then(|| super::absolute(&self.http.address.base, cam["stream_url"].as_str()?))
                .flatten()
        })
    }
}

/// The name `SET_HEATER_TEMPERATURE` takes for an object.
fn heater_name(object: &str) -> &str {
    object.strip_prefix("heater_generic ").unwrap_or(object)
}

/// The label a heater is shown with.
fn heater_label(object: &str, extruders: usize) -> String {
    if object == "heater_bed" {
        return tr("Bed");
    }
    if let Some(index) = extruder_index(object) {
        return if extruders > 1 {
            format!("T{}", index + 1)
        } else {
            tr("Nozzle")
        };
    }
    let name = heater_name(object);
    if name.contains("chamber") {
        tr("Chamber")
    } else {
        name.to_owned()
    }
}

impl Device for Klipper {
    fn caps(&self) -> Result<Caps, String> {
        let objects = self.objects()?;
        let settings = self
            .result("printer/objects/query?configfile=settings")
            .map(|r| r["status"]["configfile"]["settings"].clone())
            .unwrap_or(Value::Null);
        let extruders = objects
            .iter()
            .filter(|o| extruder_index(o).is_some())
            .count();
        let mut heaters = Vec::new();
        let mut names: Vec<&String> = objects
            .iter()
            .filter(|o| {
                extruder_index(o).is_some()
                    || *o == "heater_bed"
                    || o.starts_with("heater_generic ")
            })
            .collect();
        // Toolheads in order, then the bed, then the rest.
        names.sort_by_key(|o| match extruder_index(o) {
            Some(i) => (0, i, String::new()),
            None if *o == "heater_bed" => (1, 0, String::new()),
            None => (2, 0, o.to_string()),
        });
        for object in names {
            let limit = number(&settings[object.as_str()]["max_temp"]);
            let max = if self.u1 {
                if object == "heater_bed" {
                    U1_BED
                } else {
                    U1_TOOLHEAD
                }
            } else {
                limit.unwrap_or(if object == "heater_bed" { 120.0 } else { 300.0 })
            };
            heaters.push(HeaterCap {
                id: object.clone(),
                label: heater_label(object, extruders),
                max,
                settable: true,
            });
        }
        let mut fans = Vec::new();
        if objects.iter().any(|o| o == "fan") {
            fans.push(FanCap {
                id: "fan".into(),
                label: tr("Part"),
                settable: true,
            });
        }
        for object in objects.iter().filter(|o| o.starts_with("fan_generic ")) {
            let name = &object["fan_generic ".len()..];
            fans.push(FanCap {
                id: object.clone(),
                label: if name == "cavity_fan" {
                    tr("Cham")
                } else {
                    name.to_owned()
                },
                settable: true,
            });
        }
        let light = objects.iter().find(|o| o.starts_with("led ")).cloned();
        Ok(Caps {
            family: if self.u1 { "u1" } else { "klipper" },
            pause: true,
            resume: true,
            stop: true,
            heaters,
            fans,
            speed: Some(if self.u1 {
                SpeedCap::Percent { min: 50, max: 150 }
            } else {
                SpeedCap::Percent { min: 10, max: 300 }
            }),
            light,
            jog: Some(JogCap {
                z_label: "Z".into(),
                extrude: extruders > 0,
            }),
            slots: objects
                .iter()
                .any(|o| o == "print_task_config")
                .then_some("toolheads"),
            camera: self.camera().is_some(),
            hms: false,
            versions: false,
        })
    }

    fn read(&self, caps: &Caps) -> Result<Reading, String> {
        let status = moonraker_status(&self.http)?;
        let mut objects: Vec<String> = caps.heaters.iter().map(|h| h.id.clone()).collect();
        objects.extend(caps.fans.iter().map(|f| f.id.clone()));
        objects.extend(caps.light.iter().cloned());
        objects.push("gcode_move".into());
        objects.push("toolhead".into());
        if caps.slots.is_some() {
            objects.push("print_task_config".into());
        }
        let query: Vec<String> = objects.iter().map(|o| encode(o, false)).collect();
        let result = self.result(&format!("printer/objects/query?{}", query.join("&")))?;
        let s = &result["status"];
        let heaters = caps
            .heaters
            .iter()
            .filter_map(|cap| {
                Some(Heater {
                    id: cap.id.clone(),
                    now: number(&s[cap.id.as_str()]["temperature"])?,
                    target: number(&s[cap.id.as_str()]["target"]).unwrap_or(0.0),
                })
            })
            .collect();
        let fans = caps
            .fans
            .iter()
            .filter_map(|cap| {
                let speed = number(&s[cap.id.as_str()]["speed"])?;
                Some((
                    cap.id.clone(),
                    (speed * 100.0).round().clamp(0.0, 100.0) as u32,
                ))
            })
            .collect();
        let light = caps.light.as_ref().and_then(|led| {
            let data = s[led.as_str()]["color_data"].as_array()?;
            Some(data.iter().any(|led| {
                led.as_array()
                    .is_some_and(|c| c.iter().any(|v| v.as_f64().unwrap_or(0.0) > 0.0))
            }))
        });
        let state = match status.state.as_str() {
            "standby" => "idle",
            "printing" => "printing",
            "paused" => "paused",
            "complete" => "finished",
            "cancelled" => "cancelled",
            "error" => "error",
            _ => "idle",
        };
        let on_job = matches!(state, "printing" | "paused");
        let mut slots = Vec::new();
        if caps.slots.is_some() {
            let heads: Vec<Slot> = moonraker::toolheads_of(s)
                .iter()
                .map(|head| Slot {
                    name: format!("T{}", head.index + 1),
                    kind: head.filament_type.clone(),
                    color: head.color.clone(),
                    loaded: head.loaded,
                    active: false,
                    remain: -1,
                    detail: [head.vendor.as_str(), head.sub_type.as_str()]
                        .iter()
                        .filter(|t| !t.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join(" "),
                })
                .collect();
            slots.push((tr("Toolheads"), heads));
        }
        Ok(Reading {
            state: state.into(),
            said: super::state_said(state),
            file: status.filename,
            percent: (on_job || state == "finished").then_some(status.percent),
            left: on_job.then_some(status.left).filter(|l| *l > 0),
            layer: (status.total_layers > 0 && (on_job || state == "finished"))
                .then_some(status.layer),
            total_layers: (status.total_layers > 0 && (on_job || state == "finished"))
                .then_some(status.total_layers),
            message: status.message,
            heaters,
            fans,
            speed: number(&s["gcode_move"]["speed_factor"]).map(|f| (f * 100.0).round() as i64),
            light,
            homed: s["toolhead"]["homed_axes"].as_str().map(str::to_owned),
            slots,
            camera: if caps.camera { self.camera() } else { None },
            hms: Vec::new(),
            versions: Vec::new(),
        })
    }

    fn command(&self, caps: &Caps, command: &Command) -> Result<(), String> {
        match command {
            Command::Pause => self.post("printer/print/pause"),
            Command::Resume => self.post("printer/print/resume"),
            Command::Stop => self.post("printer/print/cancel"),
            Command::Temperature { heater, target } => {
                if !caps.heaters.iter().any(|h| h.id == *heater) {
                    return Err(tr(super::NOT_HERE));
                }
                self.script(&format!(
                    "SET_HEATER_TEMPERATURE HEATER={} TARGET={}",
                    heater_name(heater),
                    target.round()
                ))
            }
            Command::Fan { fan, percent } => {
                if fan == "fan" {
                    self.script(&format!(
                        "M106 S{}",
                        (f64::from(*percent) * 255.0 / 100.0).round()
                    ))
                } else {
                    let name = fan
                        .strip_prefix("fan_generic ")
                        .ok_or_else(|| tr(super::NOT_HERE))?;
                    self.script(&format!(
                        "SET_FAN_SPEED FAN={name} SPEED={:.2}",
                        f64::from(*percent) / 100.0
                    ))
                }
            }
            Command::Speed(percent) => self.script(&format!("M220 S{percent}")),
            Command::Light(on) => {
                let led = caps.light.as_ref().ok_or_else(|| tr(super::NOT_HERE))?;
                let name = led.strip_prefix("led ").unwrap_or(led);
                self.script(&format!(
                    "SET_LED LED={name} WHITE={}",
                    if *on { 1 } else { 0 }
                ))
            }
            Command::Jog { axis, mm } => {
                let feed = if *axis == 'z' { 1800 } else { 3600 };
                self.script(&format!(
                    "G91\nG1 {}{} F{feed}\nG90",
                    axis.to_ascii_uppercase(),
                    mm
                ))
            }
            Command::Home => self.script("G28"),
            // The speed is this app's: the U1's page's is not readable
            // from its bundle (5 mm/s, slow enough for any hot end).
            Command::Extrude(mm) => self.script(&format!("M83\nG1 E{mm} F300")),
        }
    }
}
