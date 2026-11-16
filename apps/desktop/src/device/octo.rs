//! OctoPrint through its own REST API.
//!
//! Beyond the reference: its OctoPrint class asks only `api/version` and
//! sends files (`Utils/OctoPrint.cpp`); its Device tab shows OctoPrint's
//! own page. The calls here are OctoPrint's published API (as of 1.9):
//! `GET api/printer` (`state.flags`, `temperature.tool0`, `bed`,
//! `chamber`; 409 when no printer is connected to OctoPrint), `GET api/job`
//! (`job.file.name`, `progress.completion`, `progress.printTimeLeft`),
//! `POST api/job` (`pause` with `action` `pause` / `resume`, `cancel`),
//! `POST api/printer/tool` (`target`, `extrude`), `api/printer/bed` and
//! `api/printer/chamber` (`target`), `api/printer/printhead` (`jog`,
//! `home`, `feedrate`), `api/printer/command` (`M106`), and
//! `GET api/settings` for `webcam.streamUrl`. Written from that
//! documentation and tested against a mock of it only.

use std::time::Duration;

use serde_json::{Value, json};

use super::{Caps, Command, Device, FanCap, Heater, HeaterCap, JogCap, Reading, SpeedCap};
use crate::api::i18n::tr;
use crate::hosts::{Http, http_error};

pub struct Octo {
    pub http: Http,
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite())
}

impl Octo {
    fn get(&self, path: &str) -> Result<Option<Value>, String> {
        let answer = self.http.get(path)?;
        // 409: OctoPrint answers, but no printer is connected to it.
        if answer.status == 409 {
            return Ok(None);
        }
        if !answer.ok() {
            return Err(http_error(&answer));
        }
        answer
            .json()
            .map(Some)
            .ok_or_else(|| "The printer's answer is not OctoPrint's.".to_owned())
    }

    fn post(&self, path: &str, body: Value) -> Result<(), String> {
        let answer = self.http.post_json(path, &body, Duration::from_secs(30))?;
        if answer.ok() {
            Ok(())
        } else {
            Err(http_error(&answer))
        }
    }

    fn camera(&self) -> Option<String> {
        let settings = self.get("api/settings").ok()??;
        let webcam = &settings["webcam"];
        if webcam["webcamEnabled"].as_bool() == Some(false) {
            return None;
        }
        super::absolute(&self.http.address.base, webcam["streamUrl"].as_str()?)
    }
}

impl Device for Octo {
    fn caps(&self) -> Result<Caps, String> {
        let printer = self.get("api/printer?exclude=sd")?;
        let temps = printer
            .as_ref()
            .map(|p| p["temperature"].clone())
            .unwrap_or(Value::Null);
        let mut heaters = Vec::new();
        let mut tools: Vec<String> = temps
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, value)| key.starts_with("tool") && value.is_object())
            .map(|(key, _)| key.clone())
            .collect();
        if tools.is_empty() {
            tools.push("tool0".into());
        }
        tools.sort();
        let several = tools.len() > 1;
        for tool in &tools {
            let index: usize = tool[4..].parse().unwrap_or(0);
            heaters.push(HeaterCap {
                id: tool.clone(),
                label: if several {
                    format!("T{}", index + 1)
                } else {
                    tr("Nozzle")
                },
                max: 300.0,
                settable: true,
            });
        }
        if printer.is_none() || temps["bed"].is_object() {
            heaters.push(HeaterCap {
                id: "bed".into(),
                label: tr("Bed"),
                max: 120.0,
                settable: true,
            });
        }
        if temps["chamber"].is_object() {
            heaters.push(HeaterCap {
                id: "chamber".into(),
                label: tr("Chamber"),
                max: 60.0,
                settable: true,
            });
        }
        Ok(Caps {
            family: "octoprint",
            pause: true,
            resume: true,
            stop: true,
            heaters,
            // OctoPrint sends `M106` but reports no fan speed: the fan is
            // set, its speed shown as not known.
            fans: vec![FanCap {
                id: "fan".into(),
                label: tr("Part"),
                settable: true,
            }],
            speed: Some(SpeedCap::Percent { min: 50, max: 200 }),
            light: None,
            jog: Some(JogCap {
                z_label: "Z".into(),
                extrude: true,
            }),
            slots: None,
            camera: self.camera().is_some(),
            hms: false,
            versions: false,
        })
    }

    fn read(&self, caps: &Caps) -> Result<Reading, String> {
        let Some(printer) = self.get("api/printer?exclude=sd")? else {
            // OctoPrint is there, the printer is not connected to it.
            return Ok(Reading {
                state: "error".into(),
                said: tr("Offline"),
                message: "OctoPrint answers, but no printer is connected to it.".into(),
                ..Reading::default()
            });
        };
        let job = self.get("api/job")?.unwrap_or(Value::Null);
        let flags = &printer["state"]["flags"];
        let flag = |key: &str| flags[key].as_bool().unwrap_or(false);
        let state = if flag("error") || flag("closedOrError") {
            "error"
        } else if flag("cancelling") {
            "busy"
        } else if flag("paused") || flag("pausing") {
            "paused"
        } else if flag("printing") {
            "printing"
        } else {
            "idle"
        };
        let on_job = matches!(state, "printing" | "paused");
        let temps = &printer["temperature"];
        let heaters = caps
            .heaters
            .iter()
            .filter_map(|cap| {
                Some(Heater {
                    id: cap.id.clone(),
                    now: number(&temps[cap.id.as_str()]["actual"])?,
                    target: number(&temps[cap.id.as_str()]["target"]).unwrap_or(0.0),
                })
            })
            .collect();
        let completion = number(&job["progress"]["completion"]);
        Ok(Reading {
            state: state.into(),
            said: super::state_said(state),
            file: job["job"]["file"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            percent: completion
                .filter(|_| on_job)
                .map(|c| c.round().clamp(0.0, 100.0) as u32),
            left: number(&job["progress"]["printTimeLeft"])
                .filter(|_| on_job)
                .map(|l| l.max(0.0).round() as u32),
            layer: None,
            total_layers: None,
            message: printer["state"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            heaters,
            fans: Vec::new(),
            speed: None,
            light: None,
            homed: None,
            slots: Vec::new(),
            camera: if caps.camera { self.camera() } else { None },
            hms: Vec::new(),
            versions: Vec::new(),
        })
    }

    fn command(&self, caps: &Caps, command: &Command) -> Result<(), String> {
        match command {
            Command::Pause => self.post("api/job", json!({ "command": "pause", "action": "pause" })),
            Command::Resume => {
                self.post("api/job", json!({ "command": "pause", "action": "resume" }))
            }
            Command::Stop => self.post("api/job", json!({ "command": "cancel" })),
            Command::Temperature { heater, target } => {
                if !caps.heaters.iter().any(|h| h.id == *heater) {
                    return Err(tr(super::NOT_HERE));
                }
                let target = target.round();
                match heater.as_str() {
                    "bed" => self.post("api/printer/bed", json!({ "command": "target", "target": target })),
                    "chamber" => self.post(
                        "api/printer/chamber",
                        json!({ "command": "target", "target": target }),
                    ),
                    tool => self.post(
                        "api/printer/tool",
                        json!({ "command": "target", "targets": { tool: target } }),
                    ),
                }
            }
            Command::Fan { percent, .. } => self.post(
                "api/printer/command",
                json!({ "commands": [format!("M106 S{}", (f64::from(*percent) * 255.0 / 100.0).round())] }),
            ),
            Command::Speed(factor) => self.post(
                "api/printer/printhead",
                json!({ "command": "feedrate", "factor": factor }),
            ),
            Command::Light(_) => Err(tr(super::NOT_HERE)),
            Command::Jog { axis, mm } => {
                let mut body = json!({ "command": "jog" });
                body[axis.to_string()] = json!(mm);
                self.post("api/printer/printhead", body)
            }
            Command::Home => self.post(
                "api/printer/printhead",
                json!({ "command": "home", "axes": ["x", "y", "z"] }),
            ),
            Command::Extrude(mm) => self.post(
                "api/printer/tool",
                json!({ "command": "extrude", "amount": mm }),
            ),
        }
    }
}

/// OctoPrint's own file list (`GET api/files/local?recursive=true`), for
/// Storage when the printer is not a Moonraker.
pub fn files(http: &Http) -> Result<Vec<crate::hosts::Stored>, String> {
    let answer = http.get("api/files/local?recursive=true")?;
    if !answer.ok() {
        return Err(http_error(&answer));
    }
    let json = answer
        .json()
        .ok_or("The printer's answer is not OctoPrint's.")?;
    let mut out = Vec::new();
    fn walk(list: &Value, out: &mut Vec<crate::hosts::Stored>) {
        for item in list.as_array().into_iter().flatten() {
            if item["type"] == "folder" {
                walk(&item["children"], out);
                continue;
            }
            let Some(path) = item["path"].as_str() else {
                continue;
            };
            let analysis = &item["gcodeAnalysis"];
            let mm: f64 = analysis["filament"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(_, tool)| tool["length"].as_f64())
                .sum();
            out.push(crate::hosts::Stored {
                name: path.to_owned(),
                size: item["size"].as_u64(),
                modified: item["date"].as_f64(),
                printable: crate::hosts::moonraker::printable(path),
                print_time: analysis["estimatedPrintTime"]
                    .as_f64()
                    .map(|t| t.round() as u32),
                filament_mm: (mm > 0.0).then_some(mm),
                ..crate::hosts::Stored::default()
            });
        }
    }
    walk(&json["files"], &mut out);
    out.sort_by(|a, b| {
        b.modified
            .partial_cmp(&a.modified)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}
