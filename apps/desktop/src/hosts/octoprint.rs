//! "Octo/Klipper": OctoPrint's API, which Moonraker answers too when its
//! OctoPrint compatibility is on. Ported from `Utils/OctoPrint.cpp`
//! (`OctoPrint::test`, `upload_inner_with_host`, `validate_version_text`,
//! `get_test_ok_msg`, `get_test_failed_msg`).

use crate::api::i18n::{fill, tr};

use super::{Connection, Failure, Form, Host, Http, Progress, Timeouts, Upload, http_error};

pub struct OctoPrint {
    pub http: Http,
}

impl OctoPrint {
    pub fn new(connection: &Connection, timeouts: Timeouts) -> Result<Self, String> {
        Ok(Self {
            http: Http::new(connection, timeouts)?,
        })
    }
}

/// `GET api/version`: an answer with `api`, and a `text` (when there is
/// one) that starts "OctoPrint". The reference fails an answer without
/// `api` and says nothing about why (OctoPrint.cpp l.264: an empty
/// message, so "Could not connect to OctoPrint: "); here it is said.
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
}
