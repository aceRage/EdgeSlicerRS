//! The Elegoo Centauri Carbon (the first; SDCP over its websocket), as the
//! fork's own C++ reads and drives it (`Utils/ElegooSdcp.cpp`,
//! `ElegooSdcpMonitor.cpp`): command 0's status (`CurrentStatus`,
//! `PrintInfo`, the temperatures, `CurrentFanSpeed`, `LightStatus`), pause
//! 129, stop 130, resume 131, fans and light through 403
//! (`TargetFanSpeed`, `LightStatus`). Temperatures and speed are readings
//! only, as there (`ElegooSdcp.cpp` `caps`: "its own command table leaves
//! that commented out for the CC1"). The reference's Device tab shows the
//! printer's own page and the CANVAS card: this page is beyond it (the
//! owner's decision of 2026-11-15); the readings and commands are the ones
//! its Printers tab and phone hub use.

use serde_json::{Value, json};

use super::{Caps, Command, Device, FanCap, Heater, HeaterCap, Reading};
use crate::api::i18n::tr;
use crate::hosts::elegoo::ElegooLink;

pub struct Sdcp {
    pub link: ElegooLink,
}

/// `ElegooSdcp.hpp`: `CurrentStatus` 1 is printing; `PrintInfo.Status`.
const MACHINE_PRINTING: i64 = 1;
const PRINT_PAUSING: i64 = 5;
const PRINT_PAUSED: i64 = 6;
const PRINT_STOPPING: i64 = 7;
const PRINT_STOPPED: i64 = 8;
const PRINT_COMPLETE: i64 = 9;
const PRINT_ERROR: i64 = 14;
const PRINT_FEEDING_OUT_ABNORMAL: i64 = 25;
const PRINT_FEEDING_OUT_PAUSED: i64 = 26;

fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite())
}

/// `CurrentStatus`: a list, or one number on older firmware.
pub fn machine(status: &Value) -> Vec<i64> {
    match &status["CurrentStatus"] {
        Value::Array(list) => list.iter().filter_map(Value::as_i64).collect(),
        Value::Number(n) => n.as_i64().into_iter().collect(),
        _ => Vec::new(),
    }
}

/// `hub_state`, in the page's states.
pub fn state_of(status: &Value) -> &'static str {
    let print = status["PrintInfo"]["Status"].as_i64().unwrap_or(0);
    if machine(status).contains(&MACHINE_PRINTING) {
        return match print {
            PRINT_PAUSED | PRINT_FEEDING_OUT_PAUSED => "paused",
            PRINT_STOPPED => "cancelled",
            PRINT_COMPLETE => "finished",
            PRINT_ERROR | PRINT_FEEDING_OUT_ABNORMAL => "error",
            PRINT_PAUSING | PRINT_STOPPING => "busy",
            _ => "printing",
        };
    }
    match print {
        PRINT_COMPLETE => "finished",
        PRINT_STOPPED => "cancelled",
        PRINT_ERROR => "error",
        _ => "idle",
    }
}

/// `CurrentFanSpeed`: "ModeFan" is the same fan under a misspelt name some
/// firmware uses.
fn fan(status: &Value, id: &str) -> Option<i64> {
    let fans = &status["CurrentFanSpeed"];
    match id {
        "part" => fans["ModelFan"]
            .as_i64()
            .or_else(|| fans["ModeFan"].as_i64()),
        "aux" => fans["AuxiliaryFan"].as_i64(),
        _ => fans["BoxFan"].as_i64(),
    }
}

impl Sdcp {
    fn status(&self) -> Result<Value, String> {
        self.link.ask(0, json!({}))
    }

    /// A command and its `Ack` (0 done; `ElegooSdcp.cpp` `ack_text`).
    fn send(&self, cmd: u32, data: Value) -> Result<(), String> {
        let answer = self.link.ask(cmd, data)?;
        match answer["Ack"].as_i64().unwrap_or(0) {
            0 => Ok(()),
            1 if (129..=131).contains(&cmd) => {
                Err("The printer is busy and did not take the command.".into())
            }
            2 if (129..=131).contains(&cmd) => {
                Err("The printer could not find the print job.".into())
            }
            ack => Err(format!("The printer refused the command (Ack {ack}).")),
        }
    }
}

impl Device for Sdcp {
    fn caps(&self) -> Result<Caps, String> {
        let s = self.status()?;
        let mut heaters = Vec::new();
        for (id, key, label, max) in [
            ("nozzle", "TempOfNozzle", tr("Nozzle"), 300.0),
            ("bed", "TempOfHotbed", tr("Bed"), 110.0),
            ("chamber", "TempOfBox", tr("Chamber"), 60.0),
        ] {
            if number(&s[key]).is_some() {
                heaters.push(HeaterCap {
                    id: id.into(),
                    label,
                    max,
                    settable: false,
                });
            }
        }
        let mut fans = Vec::new();
        for (id, label) in [
            ("part", tr("Part")),
            ("aux", tr("Aux")),
            ("chamber", tr("Cham")),
        ] {
            if fan(&s, id).is_some() {
                fans.push(FanCap {
                    id: id.into(),
                    label,
                    settable: true,
                });
            }
        }
        Ok(Caps {
            family: "elegoo",
            pause: true,
            resume: true,
            stop: true,
            heaters,
            fans,
            speed: None,
            light: s["LightStatus"]["SecondLight"]
                .as_i64()
                .map(|_| "SecondLight".to_owned()),
            jog: None,
            slots: None,
            camera: false,
            hms: false,
            versions: false,
            ..Caps::default()
        })
    }

    fn read(&self, caps: &Caps) -> Result<Reading, String> {
        let s = self.status()?;
        let state = state_of(&s);
        let info = &s["PrintInfo"];
        let on_job = matches!(state, "printing" | "paused" | "busy");
        let target = |key: &str| number(&s[key]).unwrap_or(0.0);
        let heaters = caps
            .heaters
            .iter()
            .filter_map(|cap| {
                let (now, aim) = match cap.id.as_str() {
                    "nozzle" => ("TempOfNozzle", "TempTargetNozzle"),
                    "bed" => ("TempOfHotbed", "TempTargetHotbed"),
                    _ => ("TempOfBox", "TempTargetBox"),
                };
                Some(Heater {
                    id: cap.id.clone(),
                    now: number(&s[now])?,
                    target: target(aim),
                })
            })
            .collect();
        let fans = caps
            .fans
            .iter()
            .filter_map(|cap| Some((cap.id.clone(), fan(&s, &cap.id)?.clamp(0, 100) as u32)))
            .collect();
        let left = match (number(&info["TotalTicks"]), number(&info["CurrentTicks"])) {
            (Some(total), Some(now)) => Some((total - now).max(0.0).round() as u32),
            _ => None,
        };
        let total_layers = info["TotalLayer"].as_u64().filter(|n| *n > 0);
        Ok(Reading {
            state: state.into(),
            said: super::state_said(state),
            file: info["Filename"].as_str().unwrap_or_default().to_owned(),
            percent: number(&info["Progress"])
                .filter(|_| on_job || state == "finished")
                .map(|p| p.round().clamp(0.0, 100.0) as u32),
            left: left.filter(|_| on_job),
            layer: total_layers
                .and(info["CurrentLayer"].as_u64())
                .map(|n| n as u32),
            total_layers: total_layers.map(|n| n as u32),
            message: String::new(),
            heaters,
            fans,
            speed: info["PrintSpeedPct"]
                .as_i64()
                .or_else(|| info["PrintSpeed"].as_i64()),
            light: s["LightStatus"]["SecondLight"].as_i64().map(|on| on != 0),
            homed: None,
            slots: Vec::new(),
            camera: None,
            hms: Vec::new(),
            versions: Vec::new(),
            ..Reading::default()
        })
    }

    fn command(&self, _caps: &Caps, command: &Command) -> Result<(), String> {
        match command {
            Command::Pause => self.send(129, json!({})),
            Command::Stop => self.send(130, json!({})),
            Command::Resume => self.send(131, json!({})),
            // `fan_data`: {"TargetFanSpeed":{"<key>":percent}}.
            Command::Fan { fan, percent } => {
                let key = match fan.as_str() {
                    "part" => "ModelFan",
                    "aux" => "AuxiliaryFan",
                    "chamber" => "BoxFan",
                    _ => return Err(tr(super::NOT_HERE)),
                };
                self.send(403, json!({ "TargetFanSpeed": { key: percent } }))
            }
            // `light_data`.
            Command::Light(on) => self.send(
                403,
                json!({ "LightStatus": { "SecondLight": u8::from(*on), "RgbLight": if *on { [255, 255, 255] } else { [0, 0, 0] } } }),
            ),
            _ => Err(tr(super::NOT_HERE)),
        }
    }
}
