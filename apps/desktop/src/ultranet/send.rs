//! The send itself, as `PrintJob::process` does it for a printer in LAN
//! mode (Jobs/PrintJob.cpp l.140-770): the access-code check, the upload
//! and start (`start_local_print`), the progress as the reference counts
//! it, the wait for the printer to take the job, and each failure in the
//! reference's words (BambuSendDiagnosis.cpp `bambu_send_failure_text`),
//! the cause decided by what the bridge proves (`job_done`'s `failure`,
//! ULTRANET_BRIDGE.md section 17) where it says.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde_json::{Value, json};

use super::client::{Client, Ended, Failure};
use crate::api::i18n::tr;

/// What the page is told of a send.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// 0..100; -1 once it failed.
    pub percent: i64,
    pub message: String,
    pub done: bool,
    pub ok: bool,
    /// The error's headline (shown on the status line), and what to do.
    pub error: Option<String>,
    pub detail: Option<String>,
    pub code: i64,
    /// The printer refused the access code: the page offers to re-enter it.
    pub refused: bool,
    /// The access-code check failed for a reason the bridge did not prove:
    /// the page offers to enter the code again, without saying it was refused.
    pub ask_code: bool,
}

/// What is sent: `PrintParams` but the password, which is added here.
pub struct Plan {
    pub dev_id: String,
    pub ip: String,
    pub code: String,
    /// `PrintParams` as JSON, every field but `password`.
    pub params: Value,
    /// The small file of the access-code check; none when the printer
    /// has no card and prints from its internal storage (no check).
    pub verify: Option<PathBuf>,
    /// The sliced file, removed when the send ends.
    pub file: PathBuf,
}

/// `BambuLanFailure` (BambuSendDiagnosis.cpp l.135).
fn classify(code: i64, detail: &str, refused: bool) -> &'static str {
    if refused {
        return "refused";
    }
    if [-4010, -2040, -15].contains(&code) {
        return "too_large";
    }
    if [-2100, -14].contains(&code) {
        return "missing";
    }
    let d = detail.to_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|n| d.contains(n));
    if any(&[
        "530",
        "login incorrect",
        "access code",
        "password",
        "not authori",
        "authenticat",
    ]) {
        return "access_code";
    }
    if any(&["verify fail", "verification fail", "verify_fail"]) {
        return "refused";
    }
    if [-2, -6010].contains(&code)
        || any(&[
            "connect",
            "timed out",
            "timeout",
            "unreachable",
            "no route",
            "refused",
        ])
    {
        return "connect";
    }
    if [-20, -2130, -4020, -5010].contains(&code) {
        return "upload";
    }
    if [-4, -4030].contains(&code) {
        return "publish";
    }
    "other"
}

const DEV_MODE_FIX: &str = "Newer Bambu firmware only accepts commands from software other than Bambu's own when both LAN Only Mode and Developer Mode are turned on in the printer's network settings. Turn both on, re-enter the access code the printer then shows on the Device tab, and send again.";

/// The class of a failure the bridge proved (`job_done`'s `failure`,
/// ULTRANET_BRIDGE.md section 17), which wins over reading the code and
/// the plug-in's text.
fn proven(failure: Failure) -> &'static str {
    match failure {
        Failure::AccessCode => "access_code",
        Failure::Unreachable => "connect",
        Failure::Upload => "transfer",
    }
}

/// `bambu_send_failure_text` for a LAN send that ran: (headline, detail).
/// `failure`, when the bridge proved one, decides the class.
pub fn failure_text(
    code: i64,
    detail: &str,
    refused: bool,
    failure: Option<Failure>,
) -> (String, String) {
    let with = |text: &str| tr(text).replace("%1%", &code.to_string());
    let class = match failure {
        Some(failure) if !refused => proven(failure),
        _ => classify(code, detail, refused),
    };
    let (headline, more) = match class {
        "connect" => (
            with("LAN send failed: could not connect to the printer (error %1%)."),
            tr(
                "Check that the printer is on and that this computer is on the same network (not a guest Wi-Fi, another VLAN or a VPN), and that no firewall blocks the printer's ports: MQTT 8883 and FTPS 990.",
            ),
        ),
        "access_code" => (
            with("LAN send failed: the printer rejected the access code (error %1%)."),
            tr(
                "The access code changes when LAN Only Mode is switched or the printer is reset. Read the current one from the printer's screen, re-enter it on the Device tab, then send again.",
            ),
        ),
        "upload" => (
            with("LAN send failed: uploading the file to the printer failed (error %1%)."),
            tr(
                "Check that no firewall blocks FTPS (port 990) to the printer, that the access code is current, and that the microSD card has free space and is not damaged or write-protected.",
            ),
        ),
        // Logged in, so the code is not why: the reference's "upload"
        // detail asks to check it all the same.
        "transfer" => (
            with("LAN send failed: uploading the file to the printer failed (error %1%)."),
            tr(
                "The printer accepted the access code, but the file did not reach it whole. Check that the microSD card has free space and is not damaged or write-protected, then send again.",
            ),
        ),
        "publish" => (
            with(
                "LAN send failed: the file was uploaded, but the printer did not take the print command (error %1%).",
            ),
            format!(
                "{} {}",
                tr(
                    "The command goes over MQTT (port 8883). If the printer is connected on the Device tab, the most likely cause is the printer refusing it."
                ),
                tr(DEV_MODE_FIX)
            ),
        ),
        "refused" => (
            with("The printer refused the print command from EdgeSlicer (error %1%)."),
            tr(DEV_MODE_FIX),
        ),
        "too_large" => (
            with("The print file exceeds the maximum allowable size (1GB) (error %1%)."),
            tr("Simplify the model and slice again."),
        ),
        "missing" => (
            with("Print file not found (error %1%)."),
            tr("Slice again, then send."),
        ),
        _ => (
            with("LAN send failed (error %1%)."),
            if detail.is_empty() {
                String::new()
            } else {
                tr("The network plug-in reported: %1%").replace("%1%", detail)
            },
        ),
    };
    let more = format!(
        "{more} {}",
        tr(
            "(There is no Bambu cloud fallback: EdgeSlicer's network plug-in prints over the local network only.)"
        )
    );
    (headline, more.trim().to_owned())
}

/// The progress a stage stands for (`StagePercentPoint`).
const STAGE_PERCENT: [i64; 7] = [20, 30, 70, 75, 97, 100, 100];

/// Sends `plan`, telling `progress` as it goes, until it ends.
pub fn run(client: &Client, plan: Plan, progress: &Arc<Mutex<Progress>>, cancel: &Arc<AtomicBool>) {
    let set = |update: &dyn Fn(&mut Progress)| {
        let mut p = progress.lock().unwrap_or_else(|poison| poison.into_inner());
        update(&mut p);
    };
    let over_lan = tr("Sending print job over LAN");
    set(&|p| p.message = over_lan.clone());
    let fail = |headline: String, detail: String, code: i64, refused: bool| {
        set(&|p| {
            p.percent = -1;
            p.done = true;
            p.ok = false;
            p.message = headline.clone();
            p.error = Some(headline.clone());
            p.detail = (!detail.is_empty()).then(|| detail.clone());
            p.code = code;
            p.refused = refused;
        });
    };
    let finish = |plan: &Plan| {
        let _ = std::fs::remove_file(&plan.file);
    };
    let remote = client.remote(&plan.dev_id);
    // A secured printer rejects the unsigned project_file: said now
    // (PrintJob.cpp l.482).
    if remote.machine.secured == Some(true) {
        fail(
            tr(
                "This printer is not in Developer Mode, so it rejects commands from EdgeSlicer. Turn on LAN Only + Developer Mode on the printer.",
            ),
            String::new(),
            0,
            false,
        );
        finish(&plan);
        return;
    }
    let mut params = plan.params.clone();
    params["password"] = json!(plan.code);
    params["dev_ip"] = json!(plan.ip);
    params["username"] = json!("bblp");
    // The access-code check: a small upload to the printer's card.
    if let Some(verify) = &plan.verify {
        let mut check = params.clone();
        check["project_name"] = json!("verify_job");
        check["filename"] = json!(verify.to_string_lossy());
        // What the plug-in said of a failure, for a cause it does not prove.
        let mut check_detail = String::new();
        let result = client
            .start_job(
                "start_send_gcode_to_sdcard",
                json!({ "params": check, "wait": false }),
            )
            .and_then(|job| {
                client.wait_job(
                    job,
                    |_, code, info| {
                        if code < 0 && !info.is_empty() {
                            check_detail = info.to_owned();
                        }
                    },
                    || cancel.load(Ordering::SeqCst),
                )
            });
        match result {
            Ok(Ended { ret: 0, .. }) => {}
            Ok(Ended { ret: -18, .. }) => {
                fail(tr("Task canceled."), String::new(), -18, false);
                finish(&plan);
                return;
            }
            // **Differs**: the reference asks for the code again after any
            // failed check (`m_enter_ip_address_fun_fail`, PrintJob.cpp
            // l.228-233), and UltraNet answers -5010 for every failed
            // upload, so a printer it could not reach or a full card was
            // said to have refused the code. Here the code is said to be
            // refused only when the bridge proves it (an FTP 530); a
            // printer not reached and a failed transfer are said as such;
            // and a failure the bridge does not explain is said without a
            // cause, the code offered to be entered again all the same.
            Ok(Ended { ret, failure }) => {
                let detail = client.scrub(&check_detail);
                match failure {
                    Some(failure) => {
                        let refused = failure == Failure::AccessCode;
                        let (headline, more) = failure_text(ret, &detail, false, Some(failure));
                        fail(headline, more, ret, refused);
                    }
                    None => {
                        let headline = tr(
                            "The access-code check failed: the printer did not take the small test file (error %1%).",
                        )
                        .replace("%1%", &ret.to_string());
                        let mut more = tr(
                            "The printer may have refused the access code, it may not be reachable over FTPS (port 990), or its card may be full or write-protected.",
                        );
                        if !detail.is_empty() {
                            more = format!(
                                "{more} {}",
                                tr("The network plug-in reported: %1%").replace("%1%", &detail)
                            );
                        }
                        fail(headline, more, ret, false);
                        set(&|p| p.ask_code = true);
                    }
                }
                finish(&plan);
                return;
            }
            Err(why) => {
                fail(why, String::new(), -1, false);
                finish(&plan);
                return;
            }
        }
    }
    if cancel.load(Ordering::SeqCst) {
        fail(tr("Task canceled."), String::new(), -18, false);
        finish(&plan);
        return;
    }
    let before = client.remote(&plan.dev_id);
    let (job_before, printing_before, reporting) = (
        before.machine.job_id.clone(),
        before.machine.printing(),
        before.machine.ready(),
    );
    let refusals_before = before.machine.refusals;
    let mut last_detail = String::new();
    let result = client
        .start_job("start_local_print", json!({ "params": params }))
        .and_then(|job| {
            client.wait_job(
                job,
                |stage, code, info| {
                    if !(0..=100).contains(&code) || stage == 7 {
                        if !info.is_empty() {
                            last_detail = info.to_owned();
                        }
                        return;
                    }
                    let mut percent = STAGE_PERCENT.get(stage as usize).copied().unwrap_or(0);
                    if (stage == 1 || stage == 3)
                        && code > 0
                        && (stage as usize + 1) < STAGE_PERCENT.len()
                    {
                        let next = STAGE_PERCENT[stage as usize + 1];
                        percent += (next - percent) * code / 100;
                    }
                    let message = if stage == 1 && !info.is_empty() {
                        format!("{over_lan}({info})")
                    } else {
                        over_lan.clone()
                    };
                    set(&|p| {
                        p.percent = percent;
                        p.message = message.clone();
                    });
                },
                || cancel.load(Ordering::SeqCst),
            )
        });
    let (mut code, failure) = match result {
        Ok(Ended { ret, failure }) => (ret, failure),
        Err(why) => {
            fail(why, String::new(), -1, false);
            finish(&plan);
            return;
        }
    };
    // `lan_started_fn`: the printer's own report says it took the job.
    if code == 0 && !cancel.load(Ordering::SeqCst) && reporting {
        let taken = client.wait_printer(&plan.dev_id, client.timeouts.started, |remote| {
            remote.machine.refusals != refusals_before
                || (!remote.machine.job_id.is_empty() && remote.machine.job_id != job_before)
                || (!printing_before && remote.machine.printing())
        });
        let refused = client.remote(&plan.dev_id).machine.refusals != refusals_before;
        if !taken || refused {
            code = -4030;
        }
    }
    let refused = client.remote(&plan.dev_id).machine.refusals != refusals_before;
    finish(&plan);
    match code {
        0 => set(&|p| {
            p.percent = 100;
            p.done = true;
            p.ok = true;
            p.message = tr("Successfully sent. Will automatically jump to the device page in %ss")
                .replace("%s", "3");
        }),
        -18 => fail(tr("Task canceled."), String::new(), -18, false),
        // Every other failure of the LAN route is told by the diagnosis
        // (`use_diag`, PrintJob.cpp l.716), the printer not taking the
        // job included (-4030).
        _ => {
            let (headline, detail) =
                failure_text(code, &client.scrub(&last_detail), refused, failure);
            fail(headline, detail, code, false);
        }
    }
}
