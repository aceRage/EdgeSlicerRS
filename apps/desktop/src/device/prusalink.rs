//! PrusaLink (and Prusa Connect's local PrusaLink), read as the fork's own
//! C++ reads it for its Printers tab (`Utils/PrusaLinkStatus.cpp`):
//! `GET api/v1/status` (`printer.state`, `temp_bed` / `target_bed`,
//! `temp_nozzle` / `target_nozzle`, the job's `progress`, `time_remaining`)
//! and, only while a job is loaded, `GET api/v1/job` for the file
//! (`file.display_name`, else `name`). The key goes in `X-Api-Key`, as
//! `PrusaLink::set_auth` sends it; the digest login ("Authorization Type"
//! user) is not built. The reference's code has no command for PrusaLink,
//! so none is drawn: the page shows its readings only. Beyond the
//! reference, whose Device tab shows the printer's own page.

use serde_json::Value;

use super::{Caps, Command, Device, Heater, HeaterCap, Reading};
use crate::api::i18n::tr;
use crate::hosts::{Http, http_error};

pub struct PrusaLink {
    pub http: Http,
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite())
}

/// `to_klipper_state`, in the page's states.
fn state_of(raw: &str) -> &'static str {
    match raw {
        "PRINTING" => "printing",
        "PAUSED" => "paused",
        "FINISHED" => "finished",
        "STOPPED" => "cancelled",
        "IDLE" | "READY" => "idle",
        "ERROR" | "ATTENTION" => "error",
        "BUSY" => "busy",
        _ => "idle",
    }
}

impl PrusaLink {
    fn get(&self, path: &str) -> Result<Value, String> {
        let answer = self.http.get(path)?;
        if !answer.ok() {
            return Err(http_error(&answer));
        }
        answer
            .json()
            .filter(|json| json["printer"].is_object() || path.ends_with("job"))
            .ok_or_else(|| "This printer did not answer as a PrusaLink printer.".to_owned())
    }
}

impl Device for PrusaLink {
    fn caps(&self) -> Result<Caps, String> {
        let status = self.get("api/v1/status")?;
        let printer = &status["printer"];
        let mut heaters = Vec::new();
        if number(&printer["temp_nozzle"]).is_some() {
            heaters.push(HeaterCap {
                id: "nozzle".into(),
                label: tr("Nozzle"),
                max: 300.0,
                settable: false,
            });
        }
        if number(&printer["temp_bed"]).is_some() {
            heaters.push(HeaterCap {
                id: "bed".into(),
                label: tr("Bed"),
                max: 120.0,
                settable: false,
            });
        }
        Ok(Caps {
            family: "prusalink",
            heaters,
            ..Caps::default()
        })
    }

    fn read(&self, caps: &Caps) -> Result<Reading, String> {
        let status = self.get("api/v1/status")?;
        let printer = &status["printer"];
        let mut state = state_of(printer["state"].as_str().unwrap_or_default());
        let mut job = status["job"].clone();
        let on_job = matches!(state, "printing" | "paused");
        // The file name lives only in `api/v1/job`, asked only while a job
        // is loaded (one request for an idle printer).
        let mut file = String::new();
        if on_job && let Ok(full) = self.get("api/v1/job") {
            let named = &full["file"];
            file = named["display_name"]
                .as_str()
                .filter(|n| !n.is_empty())
                .or_else(|| named["name"].as_str())
                .unwrap_or_default()
                .to_owned();
            for key in ["progress", "time_remaining"] {
                if full.get(key).is_some() {
                    job[key] = full[key].clone();
                }
            }
            if printer["state"].as_str().unwrap_or_default().is_empty() {
                state = state_of(full["state"].as_str().unwrap_or_default());
            }
        }
        let progress = number(&job["progress"]).or_else(|| number(&printer["progress"]));
        let heaters = caps
            .heaters
            .iter()
            .filter_map(|cap| {
                let (now, target) = match cap.id.as_str() {
                    "nozzle" => ("temp_nozzle", "target_nozzle"),
                    _ => ("temp_bed", "target_bed"),
                };
                Some(Heater {
                    id: cap.id.clone(),
                    now: number(&printer[now])?,
                    target: number(&printer[target]).unwrap_or(0.0),
                })
            })
            .collect();
        Ok(Reading {
            state: state.into(),
            said: super::state_said(state),
            file,
            percent: progress
                .filter(|_| on_job || state == "finished")
                .map(|p| p.clamp(0.0, 100.0).round() as u32),
            left: number(&job["time_remaining"])
                .filter(|t| *t >= 0.0 && on_job)
                .map(|t| t as u32),
            heaters,
            ..Reading::default()
        })
    }

    fn command(&self, _caps: &Caps, _command: &Command) -> Result<(), String> {
        Err(tr(super::NOT_HERE))
    }
}
