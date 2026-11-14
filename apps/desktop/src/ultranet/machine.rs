//! What a Bambu Lab printer reports of itself, read from its MQTT reports
//! as the reference reads them (DeviceManager.cpp `MachineObject::parse_json`
//! l.3576-5300): the print (state, file, percent, time left, layers), the
//! temperatures, the storage, the nozzle, the AMS units and their trays,
//! and its answers to commands. Only what this app shows or checks is
//! read; a report holds the fields that changed (the P1 and A1 series send
//! changes only), so a field missing from one leaves what was known.

use serde_json::Value;

/// One AMS tray (`AmsTray`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tray {
    /// Its slot, "0" to "3".
    pub id: String,
    /// The filament type as the reference reads it (`setting_id_to_type`
    /// and the two support types, "PLA-S", "PA-S").
    pub kind: String,
    /// `tray_info_idx`: the filament preset id ("GFA00").
    pub setting_id: String,
    /// "RRGGBBAA".
    pub color: String,
    pub sub_brands: String,
    /// Whether a spool is in it (`tray_exist_bits`).
    pub exists: bool,
    /// 0..100, -1 unknown.
    pub remain: i64,
}

impl Tray {
    /// `AmsTray::is_tray_info_ready`: a colour and a type.
    pub fn ready(&self) -> bool {
        !self.color.is_empty() && !self.kind.is_empty()
    }

    /// `AmsTray::get_filament_type`.
    pub fn filament_type(&self) -> String {
        match self.kind.as_str() {
            "Sup.PLA" | "Support W" => "PLA-S".into(),
            "Sup.PA" | "Support G" => "PA-S".into(),
            "Support" => match self.setting_id.as_str() {
                "GFS01" => "PA-S".into(),
                _ => "PLA-S".into(),
            },
            other => other.to_owned(),
        }
    }

    /// `AmsTray::get_display_filament_type`.
    pub fn display_type(&self) -> String {
        match self.kind.as_str() {
            "PLA-S" => "Sup.PLA".into(),
            "PA-S" => "Sup.PA".into(),
            other => other.to_owned(),
        }
    }
}

/// One AMS unit (`Ams`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ams {
    pub id: String,
    /// 0 dummy, 1 AMS, 2 AMS lite, 3 N3F, 4 N3S (`info` bits 0-3).
    pub kind: i64,
    pub exists: bool,
    pub trays: Vec<Tray>,
}

/// What is known of a printer (`MachineObject`).
#[derive(Clone, Debug, Default)]
pub struct Machine {
    /// `gcode_state`: "IDLE", "PREPARE", "RUNNING", "PAUSE", "FINISH",
    /// "FAILED", "SLICING"; "" before the first report.
    pub print_status: String,
    pub percent: Option<i64>,
    /// Seconds (`mc_remaining_time` is in minutes).
    pub left: Option<i64>,
    pub layer: Option<i64>,
    pub total_layers: Option<i64>,
    pub subtask: String,
    pub gcode_file: String,
    pub job_id: String,
    pub print_error: i64,
    /// `stg_cur`: the stage of a print (`get_stage_string`).
    pub stage: Option<i64>,
    /// (now, target), °C.
    pub nozzle: Option<(f64, f64)>,
    pub bed: Option<(f64, f64)>,
    pub chamber: Option<f64>,
    /// `sdcard`: whether a card is in.
    pub sdcard: Option<bool>,
    /// From `fun` bit 29: Developer Mode off (DeviceManager.cpp
    /// `update_secured_mode_from_fun`).
    pub secured: Option<bool>,
    pub nozzle_diameter: Option<f64>,
    /// "hardened_steel", "stainless_steel", "brass".
    pub nozzle_type: Option<String>,
    /// `fun2` bit 0: prints from its internal storage without a card
    /// (`is_support_print_with_emmc`, DeviceManager.cpp l.6431).
    pub emmc: bool,
    /// `support_print_without_sd`.
    pub without_sd: bool,
    pub ams: Vec<Ams>,
    /// `tray_now`.
    pub tray_now: String,
    /// Push reports read (`m_push_count`).
    pub reports: u64,
    /// The printer answered `get_version` (`module_vers`).
    pub versions: bool,
    /// The printer refused a `project_file` it was sent, unsigned ("mqtt
    /// message verify failed": `project_file_refusals`).
    pub refusals: u64,
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn integer(value: &Value) -> Option<i64> {
    number(value).map(|n| n as i64)
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

/// `bambu_fun_secured_bit` (BambuSendDiagnosis.cpp l.101).
pub fn secured_bit(fun: &str) -> Option<bool> {
    if !fun.chars().next()?.is_ascii_hexdigit() {
        return None;
    }
    let digits: String = fun.chars().take_while(char::is_ascii_hexdigit).collect();
    u64::from_str_radix(&digits, 16)
        .ok()
        .map(|value| (value >> 29) & 1 == 1)
}

/// `bambu_reply_is_unsigned_refusal` (BambuSendDiagnosis.cpp l.116): a
/// failed reply whose error code or reason says the command was not
/// signed.
fn unsigned_refusal(reply: &Value) -> bool {
    let result = reply["result"].as_str().unwrap_or_default().to_lowercase();
    if !result.starts_with("fail") {
        return false;
    }
    let code = integer(&reply["err_code"]).unwrap_or(0);
    let reason = reply["reason"].as_str().unwrap_or_default().to_lowercase();
    (84_033_543..=84_033_548).contains(&code)
        || ["verify fail", "verification fail", "verify_fail"]
            .iter()
            .any(|needle| reason.contains(needle))
}

impl Machine {
    /// Whether the first report was read (`is_info_ready`, which wants the
    /// versions too; a printer that does not answer `get_version` would
    /// never be ready there, so a push report is enough here).
    pub fn ready(&self) -> bool {
        self.reports > 0
    }

    /// `is_in_printing`.
    pub fn printing(&self) -> bool {
        matches!(
            self.print_status.as_str(),
            "PAUSE" | "RUNNING" | "SLICING" | "PREPARE"
        )
    }

    /// `can_resume`.
    pub fn paused(&self) -> bool {
        self.print_status == "PAUSE"
    }

    pub fn has_ams(&self) -> bool {
        self.ams
            .iter()
            .any(|ams| ams.exists || !ams.trays.is_empty())
    }

    /// The tray `ams_id * 4 + slot` of an AMS that is there.
    pub fn tray(&self, tray_id: i64) -> Option<&Tray> {
        let ams = self
            .ams
            .iter()
            .find(|ams| ams.id == (tray_id / 4).to_string())?;
        ams.trays
            .iter()
            .find(|tray| tray.id == (tray_id % 4).to_string())
    }

    /// Reads one report (`parse_json`). Answers whether it was a push
    /// report.
    pub fn read(&mut self, message: &str) -> bool {
        let Ok(root) = serde_json::from_str::<Value>(message) else {
            return false;
        };
        if let Some(info) = root.get("info")
            && info["command"] == "get_version"
            && info.get("module").is_some()
        {
            self.versions = true;
        }
        let Some(print) = root.get("print") else {
            return false;
        };
        match print["command"].as_str() {
            Some("push_status") => {
                self.read_status(print);
                self.reports += 1;
                true
            }
            Some("project_file") => {
                if unsigned_refusal(print) {
                    self.refusals += 1;
                    self.secured = Some(true);
                }
                false
            }
            _ => false,
        }
    }

    fn read_status(&mut self, p: &Value) {
        if let Some(v) = integer(&p["mc_percent"]) {
            self.percent = Some(v);
        }
        if let Some(v) = integer(&p["mc_remaining_time"]) {
            self.left = Some(v * 60);
        }
        if let Some(v) = integer(&p["print_error"]) {
            self.print_error = v;
        }
        if let Some(v) = p["sdcard"].as_bool() {
            self.sdcard = Some(v);
        }
        if let Some(v) = text(&p["gcode_state"]) {
            self.print_status = v;
        }
        if let Some(v) = text(&p["job_id"]) {
            self.job_id = v;
        } else if let Some(v) = integer(&p["job_id"]) {
            self.job_id = v.to_string();
        }
        if let Some(v) = text(&p["subtask_name"]) {
            self.subtask = v;
        }
        if let Some(v) = integer(&p["layer_num"]) {
            self.layer = Some(v);
        }
        if let Some(v) = integer(&p["total_layer_num"]) {
            self.total_layers = Some(v);
        }
        if let Some(v) = text(&p["gcode_file"]) {
            self.gcode_file = v;
        }
        if let Some(v) = integer(&p["stg_cur"]) {
            self.stage = Some(v);
        }
        let pair = |now: &Value, target: &Value, was: Option<(f64, f64)>| match (
            number(now),
            number(target),
        ) {
            (None, None) => was,
            (now, target) => {
                let (old_now, old_target) = was.unwrap_or((0.0, 0.0));
                Some((now.unwrap_or(old_now), target.unwrap_or(old_target)))
            }
        };
        self.nozzle = pair(&p["nozzle_temper"], &p["nozzle_target_temper"], self.nozzle);
        self.bed = pair(&p["bed_temper"], &p["bed_target_temper"], self.bed);
        if let Some(v) = number(&p["chamber_temper"]) {
            self.chamber = Some(v);
        }
        if let Some(fun) = p["fun"].as_str()
            && let Some(secured) = secured_bit(fun)
        {
            self.secured = Some(secured);
        }
        if let Some(fun2) = p["fun2"].as_str() {
            let digits = fun2.trim().trim_start_matches("0x");
            self.emmc = digits
                .chars()
                .last()
                .and_then(|c| c.to_digit(16))
                .is_some_and(|d| d & 1 == 1);
        }
        if let Some(v) = p["support_print_without_sd"].as_bool() {
            self.without_sd = v;
        }
        if let Some(v) = number(&p["nozzle_diameter"]) {
            self.nozzle_diameter = Some(v);
        }
        if let Some(v) = text(&p["nozzle_type"]) {
            self.nozzle_type = Some(v);
        }
        if let Some(ams) = p.get("ams") {
            self.read_ams(ams);
        }
    }

    /// The AMS block (DeviceManager.cpp l.4849-5200).
    fn read_ams(&mut self, block: &Value) {
        if let Some(now) = text(&block["tray_now"]) {
            self.tray_now = now;
        }
        let Some(units) = block["ams"].as_array() else {
            return;
        };
        let bits = |key: &str| {
            block[key]
                .as_str()
                .and_then(|text| i64::from_str_radix(text, 16).ok())
        };
        let ams_bits = bits("ams_exist_bits");
        let tray_bits = bits("tray_exist_bits");
        let mut seen = Vec::new();
        for unit in units {
            let Some(id) = text(&unit["id"]) else {
                continue;
            };
            let (kind, nozzle) = match unit["info"]
                .as_str()
                .and_then(|info| i64::from_str_radix(info, 16).ok())
            {
                Some(info) => (info & 0xF, (info >> 8) & 0xF),
                None => (1, 0),
            };
            // An AMS not set up yet.
            if nozzle == 0xE {
                continue;
            }
            seen.push(id.clone());
            let index = match self.ams.iter().position(|ams| ams.id == id) {
                Some(index) => index,
                None => {
                    self.ams.push(Ams {
                        id: id.clone(),
                        ..Ams::default()
                    });
                    self.ams.len() - 1
                }
            };
            let id_number: i64 = id.parse().unwrap_or(0);
            let ams = &mut self.ams[index];
            ams.kind = kind;
            if let Some(bits) = ams_bits
                && kind < 4
            {
                ams.exists = bits & (1 << id_number) != 0;
            }
            let Some(trays) = unit["tray"].as_array() else {
                continue;
            };
            let mut tray_seen = Vec::new();
            for slot in trays {
                let Some(tray_id) = text(&slot["id"]) else {
                    continue;
                };
                tray_seen.push(tray_id.clone());
                let at = match ams.trays.iter().position(|tray| tray.id == tray_id) {
                    Some(at) => at,
                    None => {
                        ams.trays.push(Tray {
                            id: tray_id.clone(),
                            remain: -1,
                            ..Tray::default()
                        });
                        ams.trays.len() - 1
                    }
                };
                let tray = &mut ams.trays[at];
                match (text(&slot["tray_info_idx"]), text(&slot["tray_type"])) {
                    (Some(setting), Some(kind)) => {
                        tray.kind = match setting.as_str() {
                            "GFS00" => "PLA-S".into(),
                            "GFS01" => "PA-S".into(),
                            _ => kind,
                        };
                        tray.setting_id = setting;
                    }
                    _ => {
                        tray.setting_id.clear();
                        tray.kind.clear();
                    }
                }
                tray.sub_brands = text(&slot["tray_sub_brands"]).unwrap_or_default();
                tray.color = text(&slot["tray_color"]).unwrap_or_default();
                tray.remain = integer(&slot["remain"]).unwrap_or(-1);
                let slot_number: i64 = tray_id.parse().unwrap_or(0);
                tray.exists = match tray_bits {
                    Some(bits) if kind < 4 => bits & (1 << (id_number * 4 + slot_number)) != 0,
                    // A unit that says nothing: a tray it describes is there.
                    _ => tray.ready(),
                };
            }
            ams.trays.retain(|tray| tray_seen.contains(&tray.id));
            ams.trays.sort_by(|a, b| a.id.cmp(&b.id));
        }
        self.ams.retain(|ams| seen.contains(&ams.id));
        self.ams
            .sort_by_key(|ams| ams.id.parse::<i64>().unwrap_or(i64::MAX));
    }
}

/// `get_bbl_monitor_time_dhm` (libslic3r/Utils.hpp l.725): "1d2h3m",
/// "2h3m", "3m".
pub fn time_dhm(seconds: i64) -> String {
    if seconds < 0 {
        return String::new();
    }
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    if days > 0 {
        format!("{days}d{hours}h{minutes}m")
    } else if hours > 0 {
        format!("{hours}h{minutes}m")
    } else {
        format!("{minutes}m")
    }
}
