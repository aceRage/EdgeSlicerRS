//! A Bambu Lab printer through the UltraNet bridge: the page the template
//! is made from (StatusPanel.cpp), its readings from the printer's MQTT
//! reports (`ultranet::machine`) and its commands as the reference
//! publishes them (DeviceManager.cpp), each through the bridge's
//! `send_message_to_printer`, which the contract already has
//! (ULTRANET_BRIDGE.md section 5). The camera, Storage and a firmware
//! update need more of the bridge, proposed for a later protocol minor
//! (section 15, not in force): built here against that proposal and the
//! mock bridge, each drawn only when the bridge offers its methods, and
//! said to be unavailable, with why, when it answers protocol 1.0.

use crate::ultranet::client::{CAMERA, FILES, UPDATE};

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

use serde_json::{Value, json};

use super::{
    Caps, Command, Device, FanCap, Heater, HeaterCap, Hms, JogCap, Reading, Slot, SpeedCap,
};
use crate::api::i18n::{fill, tr};
use crate::ultranet::{
    client::{Client, Session as Link, sequence},
    machine::Machine,
    mapping,
};

pub struct Bambu {
    pub client: Arc<Client>,
    pub dev_id: String,
    /// The reference's `resources/hms`, for the HMS texts.
    pub hms_dir: Option<PathBuf>,
    /// The catalog's language ("de", "pt-BR"), for the HMS texts.
    pub language: String,
}

/// StatusPanel.cpp's ranges (`nozzle_temp_range`, `bed_temp_range`,
/// `nozzle_chamber_range`), the printer's own limit not being reported here.
const NOZZLE_MAX: f64 = 300.0;
const BED_MAX: f64 = 120.0;
const CHAMBER_MAX: f64 = 60.0;

/// `get_stage_string` (DeviceManager.cpp l.59-137): the stage a print is on.
fn stage(stage: i64) -> Option<String> {
    Some(tr(match stage {
        1 => "Auto bed leveling",
        2 => "Heatbed preheating",
        3 => "Sweeping XY mech mode",
        4 => "Changing filament",
        5 => "M400 pause",
        6 => "Paused due to filament runout",
        7 => "Heating hotend",
        8 => "Calibrating extrusion",
        9 => "Scanning bed surface",
        10 => "Inspecting first layer",
        11 => "Identifying build plate type",
        12 => "Calibrating Micro Lidar",
        13 => "Homing toolhead",
        14 => "Cleaning nozzle tip",
        15 => "Checking extruder temperature",
        16 => "Printing was paused by the user",
        17 => "Pause of front cover falling",
        18 => "Calibrating the micro lidar",
        19 => "Calibrating extrusion flow",
        20 => "Paused due to nozzle temperature malfunction",
        21 => "Paused due to heat bed temperature malfunction",
        22 => "Filament unloading",
        23 => "Skip step pause",
        24 => "Filament loading",
        25 => "Motor noise calibration",
        26 => "Paused due to AMS lost",
        27 => "Paused due to low speed of the heat break fan",
        28 => "Paused due to chamber temperature control error",
        29 => "Cooling chamber",
        30 => "Paused by the G-code inserted by user",
        31 => "Motor noise showoff",
        32 => "Nozzle filament covered detected pause",
        33 => "Cutter error pause",
        34 => "First layer error pause",
        35 => "Nozzle clog pause",
        _ => return None,
    }))
}

/// A fan's 0..255 as the popup's percent in tens (StatusPanel.cpp l.2694:
/// `round(speed / 25.5) * 10`).
fn fan_percent(byte: i64) -> u32 {
    ((byte as f64 / 25.5).round() * 10.0).clamp(0.0, 100.0) as u32
}

/// The HMS item (`HMSItem::parse_hms_info`, `get_long_error_code`).
pub fn hms_of(attr: u64, code: u64) -> (String, i64, String) {
    let module = (attr >> 24) & 0xFF;
    let number = (attr >> 16) & 0xFF;
    let part = (attr >> 8) & 0xFF;
    let level = (code >> 16) as i64;
    let level = if (1..=4).contains(&level) { level } else { 0 };
    let long = format!(
        "{module:02X}{number:02X}{part:02X}00000{level:01X}{:04X}",
        code & 0xFFFF
    );
    let module_name = match module {
        3 => "MC".to_owned(),
        5 => "MainBoard".to_owned(),
        7 => "AMS".to_owned(),
        8 => "TH".to_owned(),
        12 => "XCam".to_owned(),
        other => format!("{}0x{other:x}", tr("Unknown")),
    };
    (long, level, module_name)
}

fn level_said(level: i64) -> String {
    match level {
        1 => tr("Fatal"),
        2 => tr("Serious"),
        3 => tr("Common"),
        4 => tr("Info"),
        _ => tr("Unknown"),
    }
}

/// The HMS texts of one file, by their long code.
type Texts = Arc<HashMap<String, String>>;

/// The texts of one HMS file (`device_hms` in the language), read once.
fn hms_texts(file: PathBuf, language: &str) -> Texts {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Texts>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(found) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&file) {
        return found.clone();
    }
    let mut texts = HashMap::new();
    if let Some(json) = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
    {
        for item in json["data"]["device_hms"][language]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let (Some(code), Some(intro)) = (item["ecode"].as_str(), item["intro"].as_str()) {
                texts.insert(code.to_uppercase(), intro.to_owned());
            }
        }
    }
    let texts = Arc::new(texts);
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(file, texts.clone());
    texts
}

impl Bambu {
    fn machine(&self) -> Result<Machine, String> {
        let remote = self.client.remote(&self.dev_id);
        if remote.session != Link::Connected || !remote.machine.ready() {
            return Err(tr("Offline"));
        }
        Ok(remote.machine)
    }

    /// An HMS message's text: the printer model's file (the serial's first
    /// three characters), in the language, else English
    /// (HMSQuery::describe_error, without its cloud refresh).
    fn hms_text(&self, long: &str) -> String {
        let Some(dir) = &self.hms_dir else {
            return String::new();
        };
        let kind: String = self.dev_id.chars().take(3).collect();
        for language in [self.language.as_str(), "en"] {
            let file = dir.join(format!("hms_{language}_{kind}.json"));
            if let Some(text) = hms_texts(file, language).get(long) {
                return text.clone();
            }
        }
        fill(
            &tr("Printer error %s - see the printer screen or Bambu's error-code page"),
            &[&long],
        )
    }

    fn publish(&self, message: Value) -> Result<(), String> {
        let ret = self.client.publish(&self.dev_id, &message, 1)?;
        if ret != 0 {
            return Err(format!("The printer was not sent the command ({ret})."));
        }
        Ok(())
    }

    /// Why a part of section 15 cannot be had from the bridge running.
    fn needs(&self, methods: &[&str]) -> String {
        let minor = self.client.hello().map_or(0, |hello| hello.minor);
        format!(
            "This needs an UltraNet bridge of the later protocol minor (ULTRANET_BRIDGE.md section 15, proposed, not yet in force); the bridge running answers protocol 1.{minor} and does not offer {}.",
            methods.join(", ")
        )
    }

    /// A section 15 request, its `ret` checked.
    fn bridge(&self, method: &str, params: Value) -> Result<Value, String> {
        let result = self.client.ask(method, params)?;
        match result["ret"].as_i64() {
            Some(0) => Ok(result),
            ret => Err(match result["refused"].as_str() {
                Some(why) if !why.is_empty() => self.client.scrub(why),
                _ => format!("The printer did not take it ({}).", ret.unwrap_or(-1)),
            }),
        }
    }

    /// `publish_gcode`.
    fn gcode(&self, gcode: &str) -> Result<(), String> {
        self.publish(json!({ "print": { "command": "gcode_line", "param": gcode, "sequence_id": sequence() } }))
    }
}

impl Device for Bambu {
    fn caps(&self) -> Result<Caps, String> {
        let m = self.machine()?;
        let mut heaters = vec![
            HeaterCap {
                id: "nozzle".into(),
                label: tr("Nozzle"),
                max: NOZZLE_MAX,
                settable: true,
            },
            HeaterCap {
                id: "bed".into(),
                label: tr("Bed"),
                max: BED_MAX,
                settable: true,
            },
        ];
        if m.chamber.is_some() {
            heaters.push(HeaterCap {
                id: "chamber".into(),
                label: tr("Chamber"),
                max: CHAMBER_MAX,
                // Read only unless the printer says it can be set
                // (`is_support_chamber_edit`).
                settable: m.chamber_edit,
            });
        }
        let mut fans = vec![FanCap {
            id: "part".into(),
            label: tr("Part"),
            settable: true,
        }];
        if m.has_aux_fan {
            fans.push(FanCap {
                id: "aux".into(),
                label: tr("Aux"),
                settable: true,
            });
        }
        if m.has_chamber_fan {
            fans.push(FanCap {
                id: "chamber".into(),
                label: tr("Cham"),
                settable: true,
            });
        }
        Ok(Caps {
            family: "bambu",
            pause: true,
            resume: true,
            stop: true,
            heaters,
            fans,
            speed: Some(SpeedCap::Levels(vec![
                (1, tr("Silent")),
                (2, tr("Standard")),
                (3, tr("Sport")),
                (4, tr("Ludicrous")),
            ])),
            light: Some("chamber_light".into()),
            jog: Some(JogCap {
                // `is_core_xy`: every printer the bridge reaches but the A1
                // series (whose serials start "039" and "030").
                z_label: if self.dev_id.starts_with("039") || self.dev_id.starts_with("030") {
                    "Z".into()
                } else {
                    tr("Bed")
                },
                extrude: true,
            }),
            slots: Some("ams"),
            camera: false,
            hms: true,
            versions: true,
            camera_frames: self.client.offers(&CAMERA),
            storage: self.client.offers(&FILES),
            update: self.client.offers(&UPDATE),
            unavailable: [
                ("camera", &CAMERA[..]),
                ("storage", &FILES[..]),
                ("update", &UPDATE[..]),
            ]
            .into_iter()
            .filter(|(_, methods)| !self.client.offers(methods))
            .map(|(part, methods)| (part, self.needs(methods)))
            .collect(),
        })
    }

    fn read(&self, caps: &Caps) -> Result<Reading, String> {
        let m = self.machine()?;
        let state = match m.print_status.as_str() {
            "IDLE" => "idle",
            "PREPARE" | "SLICING" => "preparing",
            "RUNNING" => "printing",
            "PAUSE" => "paused",
            "FINISH" => "finished",
            "FAILED" => "cancelled",
            _ => "idle",
        };
        let on_job = m.printing() || m.print_status == "FINISH";
        let said = match (state, m.stage.and_then(stage)) {
            ("printing" | "paused", Some(stage)) => stage,
            _ => super::state_said(state),
        };
        let mut heaters = Vec::new();
        if let Some((now, target)) = m.nozzle {
            heaters.push(Heater {
                id: "nozzle".into(),
                now,
                target,
            });
        }
        if let Some((now, target)) = m.bed {
            heaters.push(Heater {
                id: "bed".into(),
                now,
                target,
            });
        }
        if let Some(now) = m.chamber {
            heaters.push(Heater {
                id: "chamber".into(),
                now,
                target: 0.0,
            });
        }
        let mut fans = Vec::new();
        for (id, value) in [
            ("part", m.fan_part),
            ("aux", m.fan_aux),
            ("chamber", m.fan_chamber),
        ] {
            if let Some(value) = value
                && caps.fans.iter().any(|f| f.id == id)
            {
                fans.push((id.to_owned(), fan_percent(value)));
            }
        }
        let slots = m
            .ams
            .iter()
            .map(|ams| {
                let unit = ams.id.parse::<i64>().unwrap_or(0);
                let trays = ams
                    .trays
                    .iter()
                    .map(|tray| {
                        let id = unit * 4 + tray.id.parse::<i64>().unwrap_or(0);
                        Slot {
                            name: mapping::tray_name(id),
                            kind: tray.display_type(),
                            color: if tray.color.len() >= 6 {
                                format!("#{}", &tray.color[..6])
                            } else {
                                String::new()
                            },
                            loaded: tray.exists,
                            active: m.tray_now == id.to_string(),
                            remain: tray.remain,
                            detail: tray.sub_brands.clone(),
                        }
                    })
                    .collect();
                (format!("{} {}", tr("AMS"), unit + 1), trays)
            })
            .collect();
        let hms = m
            .hms
            .iter()
            .map(|(attr, code)| {
                let (long, level, module) = hms_of(*attr, *code);
                Hms {
                    text: self.client.scrub(&self.hms_text(&long)),
                    code: long,
                    level: level_said(level),
                    module,
                }
            })
            .collect();
        let homed: String = ['x', 'y', 'z'].iter().filter(|a| m.homed(**a)).collect();
        Ok(Reading {
            state: state.into(),
            said,
            file: self.client.scrub(if m.subtask.is_empty() {
                &m.gcode_file
            } else {
                &m.subtask
            }),
            percent: m.percent.filter(|_| on_job).map(|p| p.clamp(0, 100) as u32),
            left: m.left.filter(|_| on_job).map(|l| l.max(0) as u32),
            layer: m.layer.filter(|_| on_job).map(|l| l.max(0) as u32),
            total_layers: m.total_layers.filter(|_| on_job).map(|l| l.max(0) as u32),
            message: if m.print_error != 0 {
                self.hms_text(&format!("{:08X}", m.print_error))
            } else {
                String::new()
            },
            heaters,
            fans,
            speed: m.speed_level,
            light: m.light,
            homed: m.home_flag.map(|_| homed),
            slots,
            camera: None,
            hms,
            versions: m
                .modules
                .iter()
                .map(|(name, sw, hw, sn)| {
                    (name.clone(), sw.clone(), hw.clone(), self.client.scrub(sn))
                })
                .collect(),
            update: m.upgrade.display.map(|_| m.upgrade.clone()),
        })
    }

    fn command(&self, _caps: &Caps, command: &Command) -> Result<(), String> {
        let m = self.machine()?;
        // Every `print` and `system` command of a printer not in Developer
        // Mode is refused unsigned (BambuSendDiagnosis.cpp l.131).
        if m.secured == Some(true) {
            return Err(tr(
                "This printer is not in Developer Mode, so it rejects commands from EdgeSlicer. Turn on LAN Only + Developer Mode on the printer.",
            ));
        }
        let print = |command: &str| {
            self.publish(
                json!({ "print": { "command": command, "param": "", "sequence_id": sequence() } }),
            )
        };
        match command {
            // `command_task_pause`, `_resume`, `_abort`.
            Command::Pause => print("pause"),
            Command::Resume => print("resume"),
            Command::Stop => print("stop"),
            // `command_set_nozzle`, `command_set_bed`, `command_set_chamber`.
            Command::Temperature { heater, target } => match heater.as_str() {
                "nozzle" => self.gcode(&format!("M104 S{}\n", target.round())),
                "bed" => self.gcode(&format!("M140 S{}\n", target.round())),
                "chamber" => self.publish(json!({ "print": { "sequence_id": sequence(), "command": "set_ctt", "ctt_val": target.round() as i64 } })),
                _ => Err(tr(super::NOT_HERE)),
            },
            // `command_control_fan_val`: `M106 P<1|2|3> S<0..255>`, the
            // popup's tens as `floor(step * 25.5)`.
            Command::Fan { fan, percent } => {
                let index = match fan.as_str() {
                    "part" => 1,
                    "aux" => 2,
                    "chamber" => 3,
                    _ => return Err(tr(super::NOT_HERE)),
                };
                let byte = (f64::from(*percent / 10) * 25.5).floor() as i64;
                self.gcode(&format!("M106 P{index} S{byte} \n"))
            }
            // `command_set_printing_speed`.
            Command::Speed(level) => self.publish(json!({ "print": { "command": "print_speed", "sequence_id": sequence(), "param": level.to_string() } })),
            // `command_set_chamber_light` (ChamberLights.hpp l.92).
            Command::Light(on) => self.publish(json!({ "system": {
                "command": "ledctrl", "led_node": "chamber_light", "sequence_id": sequence(),
                "led_mode": if *on { "on" } else { "off" },
                "led_on_time": 500, "led_off_time": 500, "loop_times": 1, "interval_time": 1000,
            } })),
            // `command_axis_control` (DeviceManager.cpp l.2604): Y and Z
            // the other way round on a printer that is not CoreXY.
            Command::Jog { axis, mm } => {
                let core_xy = !(self.dev_id.starts_with("039") || self.dev_id.starts_with("030"));
                let value = if !core_xy && *axis != 'x' { -mm } else { *mm };
                let speed = if *axis == 'z' { 900 } else { 3000 };
                self.gcode(&format!(
                    "M211 S \nM211 X1 Y1 Z1\nM1002 push_ref_mode\nG91 \nG1 {}{value:.1} F{speed}\nM1002 pop_ref_mode\nM211 R\n",
                    axis.to_ascii_uppercase()
                ))
            }
            // RecenterDialog's "Go Home" (`command_go_home`).
            Command::Home => self.gcode("G28 \n"),
            // `command_axis_control("E", 1.0, ±10, 900)`: StatusPanel.cpp
            // l.3636-3662, the reference's own extruder buttons.
            Command::Extrude(mm) => self.gcode(&format!("M83 \nG0 E{mm:.1} F900\n")),
            // Section 15's `firmware_upgrade` (proposed): the bridge sends
            // the reference's `command_upgrade_confirm` (DeviceManager.cpp
            // l.2095) only to a printer that offers that version and is
            // not printing. Never through `send_message_to_printer`.
            Command::Update { version } => {
                if !self.client.offers(&UPDATE) {
                    return Err(self.needs(&UPDATE));
                }
                self.bridge(
                    "firmware_upgrade",
                    json!({ "dev_id": self.dev_id, "module": "ota", "version": version }),
                )
                .map(|_| ())
            }
        }
    }

    /// Section 15's camera: started by the bridge with the code it holds;
    /// its frames handed to the page as data, never the printer's address.
    fn camera(&self, action: &str) -> Result<Value, String> {
        if !self.client.offers(&CAMERA) {
            return Err(self.needs(&CAMERA));
        }
        match action {
            "start" => {
                self.machine()?;
                let result = self.bridge("camera_start", json!({ "dev_id": self.dev_id }))?;
                Ok(json!({ "kind": result["kind"] }))
            }
            "frame" => Ok(match self.client.frame(&self.dev_id) {
                Some((seq, jpeg)) => {
                    json!({ "seq": seq, "picture": format!("data:image/jpeg;base64,{jpeg}") })
                }
                None => json!({ "seq": null, "picture": null }),
            }),
            "stop" => {
                self.client.forget_frame(&self.dev_id);
                self.bridge("camera_stop", json!({ "dev_id": self.dev_id }))?;
                Ok(json!({}))
            }
            other => Err(format!("there is no camera action {other}")),
        }
    }

    /// Section 15's `files_list` (`PrinterFileSystem` `LIST_INFO`): the
    /// card's timelapses, videos or models.
    fn files(&self, kind: &str) -> Result<Vec<super::DeviceFile>, String> {
        if !self.client.offers(&FILES) {
            return Err(self.needs(&FILES));
        }
        if !matches!(kind, "timelapse" | "video" | "model") {
            return Err(format!("there are no files of the kind {kind}"));
        }
        self.machine()?;
        let result = self.bridge(
            "files_list",
            json!({ "dev_id": self.dev_id, "type": kind, "storage": "external" }),
        )?;
        Ok(result["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|file| {
                let path = file["path"].as_str().filter(|p| !p.is_empty())?;
                Some(super::DeviceFile {
                    name: self.client.scrub(file["name"].as_str().unwrap_or(path)),
                    path: path.to_owned(),
                    time: file["time"].as_f64().filter(|t| *t > 0.0),
                    size: file["size"].as_u64(),
                })
            })
            .collect())
    }

    /// Section 15's `files_delete` (`FILE_DEL`), once confirmed; never the
    /// file a print is on.
    fn delete_file(&self, path: &str) -> Result<(), String> {
        if !self.client.offers(&FILES) {
            return Err(self.needs(&FILES));
        }
        let m = self.machine()?;
        let name = path.rsplit('/').next().unwrap_or(path);
        let on = |file: &str| !file.is_empty() && file.rsplit('/').next() == Some(name);
        if m.printing() && (on(&m.gcode_file) || on(&m.subtask)) {
            return Err(
                "The printer is printing this file. Finish or cancel the print first.".into(),
            );
        }
        self.bridge(
            "files_delete",
            json!({ "dev_id": self.dev_id, "paths": [path] }),
        )
        .map(|_| ())
    }
}
