//! "Octo/Klipper": OctoPrint's API, which Moonraker answers too when its
//! OctoPrint compatibility is on. Ported from `Utils/OctoPrint.cpp`
//! (`OctoPrint::test`, `upload_inner_with_host`, `validate_version_text`,
//! `get_test_ok_msg`, `get_test_failed_msg`).

use crate::api::i18n::{fill, tr};

use super::{
    Connection, Failure, Form, Host, Http, Progress, Status, Stored, Timeouts, Upload, http_error,
    snapmaker::SnapmakerLan,
};

pub struct OctoPrint {
    pub http: Http,
    /// The same printer asked as a Moonraker: the reference's Storage tab
    /// and its printer lists speak Moonraker's API to an "Octo/Klipper"
    /// host (PrinterStorage.cpp `target_for_host`, RemoteControl.cpp
    /// `describe_hosts`); an OctoPrint that is not one answers 404.
    moonraker: SnapmakerLan,
}

impl OctoPrint {
    pub fn new(connection: &Connection, timeouts: Timeouts) -> Result<Self, String> {
        Ok(Self {
            http: Http::new(connection, timeouts)?,
            moonraker: SnapmakerLan::new(connection, timeouts)?,
        })
    }
}

/// `GET api/version`: an answer with `api`, and a `text` (when there is
/// one) that starts "OctoPrint". An answer without `api` is said to be
/// another host's. The reference says so too since aceRage/EdgeSlicer#982,
/// in its own words ("Mismatched type of print host: %s"); before it,
/// "Could not connect to OctoPrint: " with nothing after it. Here the
/// words are still this app's.
pub fn version(http: &Http) -> Result<(), String> {
    let answer = http.get("api/version")?;
    if !answer.ok() {
        return Err(http_error(&answer));
    }
    let Some(json) = answer.json() else {
        // The reference does not translate it (OctoPrint.cpp l.277).
        return Err("Could not parse server response".into());
    };
    if json.get("api").is_none() {
        return Err("The printer's answer is not OctoPrint's.".into());
    }
    match json.get("text").and_then(|text| text.as_str()) {
        Some(text) if !text.starts_with("OctoPrint") => {
            Err(fill(&tr("Mismatched type of print host: %s"), &[&text]))
        }
        _ => Ok(()),
    }
}

/// `POST api/files/local`: `print`, `path`, then the file (the order the
/// reference adds them in). Any 2xx is taken as sent.
pub fn upload_file(http: &Http, upload: &Upload, progress: &Progress) -> Result<(), Failure> {
    let (path, file_name) = match upload.name.rsplit_once('/') {
        Some((path, name)) => (path, name),
        None => ("", upload.name.as_str()),
    };
    let total = upload.data.len() as u64;
    progress
        .total
        .store(total, std::sync::atomic::Ordering::Relaxed);
    let form = Form::new()
        .field("print", if upload.start { "true" } else { "false" })
        .field("path", path)
        .file("file", file_name, upload.data.clone(), 0..upload.data.len());
    let answer = http.post_form("api/files/local", &form, progress, (0, total))?;
    if !answer.ok() {
        return Err(Failure::Error(http_error(&answer)));
    }
    Ok(())
}

impl OctoPrint {
    /// An OctoPrint that is not a Moonraker: Moonraker's `server/info`
    /// answers 404.
    pub fn is_octoprint(&self) -> bool {
        matches!(self.moonraker.http.get("server/info"), Ok(answer) if answer.status == 404)
    }
}

impl Host for OctoPrint {
    fn test(&self) -> Result<(), String> {
        version(&self.http)
    }

    fn test_ok(&self) -> String {
        tr("Connection to OctoPrint is working correctly.")
    }

    fn test_failed(&self, reason: &str) -> String {
        format!(
            "{}: {reason}\n\n{}",
            tr("Could not connect to OctoPrint"),
            tr("Note: OctoPrint version 1.1.0 or higher is required.")
        )
    }

    /// As the reference: "Test" first, then the file.
    fn upload(&self, upload: &Upload, progress: &Progress) -> Result<(), Failure> {
        version(&self.http)?;
        upload_file(&self.http, upload, progress)
    }

    /// Moonraker's file API first (the reference's Storage tab speaks only
    /// that, `PrinterStorage.cpp` `target_for_host`); an OctoPrint that is
    /// not a Moonraker answers 404 there, and then OctoPrint's own
    /// `api/files` is used (beyond the reference: the owner's Device pages).
    fn files(&self) -> Option<Result<Vec<Stored>, String>> {
        Some(match self.moonraker.files()? {
            Err(error) if error.starts_with("HTTP 404") => crate::device::octo::files(&self.http),
            other => other,
        })
    }

    /// Moonraker's preview pictures; an OctoPrint's list names none.
    fn thumbnail(&self, path: &str) -> Option<Result<Vec<u8>, String>> {
        self.moonraker.thumbnail(path)
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        match self.moonraker.delete(name) {
            Err(error) if error.starts_with("HTTP 404") && self.is_octoprint() => {
                let answer = self.http.delete(&format!(
                    "api/files/local/{}",
                    super::snapmaker::encode(name, true)
                ))?;
                if answer.ok() {
                    Ok(())
                } else {
                    Err(http_error(&answer))
                }
            }
            other => other,
        }
    }

    fn print(&self, name: &str) -> Result<(), String> {
        if !self.is_octoprint() {
            return self.moonraker.print(name);
        }
        // Refused while it prints (PrinterStorage.cpp l.499), then
        // OctoPrint's `select` with `print`.
        let status = self.status().ok_or("no status")??;
        if matches!(status.state.as_str(), "printing" | "paused") {
            return Err(tr(super::snapmaker::BUSY));
        }
        let answer = self.http.post_json(
            &format!("api/files/local/{}", super::snapmaker::encode(name, true)),
            &serde_json::json!({ "command": "select", "print": true }),
            std::time::Duration::from_secs(30),
        )?;
        if answer.ok() {
            Ok(())
        } else {
            Err(http_error(&answer))
        }
    }

    fn status(&self) -> Option<Result<Status, String>> {
        if !self.is_octoprint() {
            return self.moonraker.status();
        }
        use crate::device::Device;
        let device = crate::device::octo::Octo {
            http: self.http.clone(),
        };
        let caps = match device.caps() {
            Ok(caps) => caps,
            Err(error) => return Some(Err(error)),
        };
        Some(device.read(&caps).map(|reading| {
            Status {
                bed: reading.heater("bed").map(|h| (h.now, h.target)),
                state: match reading.state.as_str() {
                    "idle" => "standby".into(),
                    "finished" => "complete".into(),
                    other => other.to_owned(),
                },
                filename: reading.file,
                message: reading.message,
                percent: reading.percent.unwrap_or(0),
                left: reading.left.unwrap_or(0),
                layer: 0,
                total_layers: 0,
                nozzles: reading
                    .heaters
                    .iter()
                    .filter(|h| h.id.starts_with("tool"))
                    .map(|h| (h.now, h.target))
                    .collect(),
            }
        }))
    }
}
