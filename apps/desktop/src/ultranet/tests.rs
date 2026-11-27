//! The bridge client against the mock bridge (`mock.rs`), and the
//! reference's rules this module ports: the printer's reports, the AMS
//! mapping, the trays' names, the blacklist, the access code's checks.

use std::time::Duration;

use serde_json::{Value, json};

use super::{
    client::{Client, Launcher, Session, State, Timeouts, is_auth_refusal, scrub},
    machine::{Machine, secured_bit, time_dhm},
    mapping::{self, Filament},
    mock::{Mock, Setup},
    printers,
};

fn quick() -> Timeouts {
    Timeouts {
        hello: Duration::from_secs(5),
        ask: Duration::from_secs(5),
        connect: Duration::from_secs(5),
        started: Duration::from_millis(500),
        restart: Duration::from_millis(0),
        account: Duration::from_secs(5),
    }
}

fn client(mock: &Mock) -> Client {
    let client = Client::new(mock.launcher(), None, quick());
    client.set_codes(vec![mock.setup.lock().unwrap().code.clone()]);
    client
}

fn report(print: Value) -> String {
    let mut print = print;
    print["command"] = json!("push_status");
    json!({ "print": print }).to_string()
}

fn ams_machine(trays: &[(u32, &str, &str)]) -> Machine {
    let mut machine = Machine::default();
    let list: Vec<Value> = trays
        .iter()
        .map(|(slot, kind, colour)| json!({ "id": slot.to_string(), "tray_type": kind, "tray_info_idx": "GFL99", "tray_color": colour }))
        .collect();
    let bits: u32 = trays.iter().map(|(slot, _, _)| 1 << slot).sum();
    machine.read(&report(json!({
        "gcode_state": "IDLE",
        "ams": { "ams_exist_bits": "1", "tray_exist_bits": format!("{bits:x}"), "ams": [{ "id": "0", "info": "1001", "tray": list }] },
    })));
    machine
}

fn filament(id: usize, colour: &str, kind: &str) -> Filament {
    Filament {
        id,
        color: colour.into(),
        kind: kind.into(),
        ..Filament::default()
    }
}

#[test]
fn a_report_is_read_as_the_reference_reads_it() {
    let mut machine = Machine::default();
    assert!(!machine.ready());
    assert!(machine.read(&report(json!({
        "gcode_state": "RUNNING", "mc_percent": 37, "mc_remaining_time": 125, "layer_num": 12, "total_layer_num": 200,
        "subtask_name": "benchy", "job_id": "4711", "nozzle_temper": 219.6, "nozzle_target_temper": 220,
        "bed_temper": 55.1, "bed_target_temper": 55, "chamber_temper": 30, "sdcard": true, "fun": "3EC1CFFF9CFF",
        "fun2": "0x1", "nozzle_diameter": "0.4", "nozzle_type": "hardened_steel",
    }))));
    assert!(machine.ready() && machine.printing() && !machine.paused());
    assert_eq!(
        (
            machine.percent,
            machine.left,
            machine.layer,
            machine.total_layers
        ),
        (Some(37), Some(7500), Some(12), Some(200))
    );
    assert_eq!(
        (machine.subtask.as_str(), machine.job_id.as_str()),
        ("benchy", "4711")
    );
    assert_eq!(machine.nozzle, Some((219.6, 220.0)));
    assert_eq!(machine.bed, Some((55.1, 55.0)));
    assert_eq!(machine.secured, Some(false));
    assert!(machine.emmc);
    assert_eq!(machine.nozzle_type.as_deref(), Some("hardened_steel"));
    // A report of changes only keeps what it does not name.
    machine.read(&report(
        json!({ "mc_percent": 38, "bed_target_temper": 60 }),
    ));
    assert_eq!(machine.percent, Some(38));
    assert_eq!(machine.bed, Some((55.1, 60.0)));
    assert_eq!(machine.print_status, "RUNNING");
    // Developer Mode off: bit 29 of `fun`.
    assert_eq!(secured_bit("20000000"), Some(true));
    assert_eq!(secured_bit("0"), Some(false));
    assert_eq!(secured_bit("zz"), None);
    // The printer refusing an unsigned print.
    machine.read(&json!({ "print": { "command": "project_file", "result": "failed", "reason": "mqtt message verify failed" } }).to_string());
    assert_eq!(machine.refusals, 1);
    assert_eq!(machine.secured, Some(true));
    // `get_bbl_monitor_time_dhm`.
    assert_eq!(time_dhm(7500), "2h5m");
    assert_eq!(time_dhm(59), "0m");
    assert_eq!(time_dhm(90_061), "1d1h1m");
}

#[test]
fn the_ams_is_read_with_its_trays() {
    let machine = ams_machine(&[(0, "PLA", "FF0000FF"), (2, "PETG", "0000FFFF")]);
    assert!(machine.has_ams());
    assert_eq!(machine.ams.len(), 1);
    let trays = &machine.ams[0].trays;
    assert_eq!(trays.len(), 2);
    assert!(trays.iter().all(|t| t.exists && t.ready()));
    assert_eq!(machine.tray(2).unwrap().kind, "PETG");
    assert!(machine.tray(1).is_none());
    // An AMS not set up yet (`info` nozzle 0xE) is left out.
    let mut other = Machine::default();
    other.read(&report(
        json!({ "ams": { "ams": [{ "id": "0", "info": "E01", "tray": [] }] } }),
    ));
    assert!(other.ams.is_empty());
}

#[test]
fn trays_are_named_as_the_reference_names_them() {
    assert_eq!(mapping::tray_name(-1), "-");
    assert_eq!(mapping::tray_name(0), "A1");
    assert_eq!(mapping::tray_name(3), "A4");
    assert_eq!(mapping::tray_name(5), "B2");
    assert_eq!(mapping::tray_name(128 * 4), "A");
    assert_eq!(mapping::rgba("#FF000080"), [255, 0, 0, 128]);
    assert_eq!(mapping::rgba("00FF00FF"), [0, 255, 0, 255]);
    assert_eq!(mapping::rgba("junk"), [255; 4]);
    assert!(mapping::color_distance("#FF0000", "FF0000FF") < 1e-9);
    // CIE76 on sRGB in 0..1: pure red to pure green is about 170.
    let red_green = mapping::color_distance("#FF0000", "00FF00FF");
    assert!((red_green - 170.6).abs() < 1.0, "{red_green}");
}

#[test]
fn the_auto_match_picks_the_nearest_colour_of_the_same_type() {
    let machine = ams_machine(&[
        (0, "PLA", "FF0000FF"),
        (1, "PLA", "00FF00FF"),
        (2, "PETG", "0000FFFF"),
    ]);
    // Green PLA to A2, dark red PLA to A1, blue PETG to A3.
    let filaments = [
        filament(0, "#00EE00", "PLA"),
        filament(2, "#AA0000", "PLA"),
        filament(4, "#0000FF", "PETG"),
    ];
    let (code, result) = mapping::auto_map(&machine, &filaments);
    assert_eq!(code, 0);
    let trays: Vec<i64> = result.iter().map(|r| r.tray_id).collect();
    assert_eq!(trays, vec![1, 0, 2]);
    assert_eq!(result[1].ams_id, "0");
    assert_eq!(result[1].slot_id, "0");
    // Two filaments of one colour share a tray.
    let (_, shared) = mapping::auto_map(
        &machine,
        &[filament(0, "#FF0000", "PLA"), filament(1, "#FF0000", "PLA")],
    );
    assert_eq!(
        shared.iter().map(|r| r.tray_id).collect::<Vec<_>>(),
        vec![0, 0]
    );
    // A type no tray holds: none, and the mapping is not valid.
    let (_, none) = mapping::auto_map(&machine, &[filament(0, "#FFFFFF", "TPU")]);
    assert_eq!(none[0].tray_id, -1);
    let mut check = none.clone();
    assert!(!mapping::is_valid(&machine, &mut check, true));
    // The three strings of the job: one entry for each project filament.
    let (v0, v1, info, valid) = mapping::compose(
        &result,
        &filaments,
        5,
        &[
            "GFA00".into(),
            String::new(),
            "GFA01".into(),
            String::new(),
            "GFG00".into(),
        ],
    )
    .unwrap();
    assert!(valid);
    assert_eq!(v0, "[1,-1,0,-1,2]");
    let v1: Value = serde_json::from_str(&v1).unwrap();
    assert_eq!(v1[0], json!({ "ams_id": 0, "slot_id": 1 }));
    assert_eq!(v1[1], json!({ "ams_id": 255, "slot_id": 255 }));
    let info: Value = serde_json::from_str(&info).unwrap();
    assert_eq!(info[0]["filamentId"], "GFA00");
    assert_eq!(info[0]["sourceColor"], "#00EE00");
    assert_eq!(info[0]["targetColor"], "00FF00FF");
    assert_eq!(info[1]["ams"], -1);
    // Nothing mapped: nothing composed.
    assert!(mapping::compose(&none, &[filament(0, "#FFFFFF", "TPU")], 1, &[]).is_none());
}

#[test]
fn the_blacklist_is_read_with_the_references_sentences() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/EdgeSlicer/resources/printers/filaments_blacklist.json"),
    )
    .unwrap();
    let list = mapping::read_blacklist(&text);
    assert_eq!(list.len(), 9);
    let tpu = mapping::blacklisted(&list, "Generic", "TPU").unwrap();
    assert_eq!(
        (tpu.action.as_str(), tpu.said),
        ("prohibition", "TPU is not supported by AMS.")
    );
    // Bambu Lab's own PET-CF ("Bambu Lab" is the presets' vendor).
    assert_eq!(
        mapping::blacklisted(&list, "Bambu Lab", "PET-CF")
            .unwrap()
            .action,
        "prohibition"
    );
    // Somebody else's PET-CF is not Bambu's.
    assert!(mapping::blacklisted(&list, "Polymaker", "PET-CF").is_none());
    // "Third Party" entries are for every filament, Bambu Lab's included
    // (aceRage/EdgeSlicer#1039, as Bambu Studio now does).
    assert_eq!(
        mapping::blacklisted(&list, "Polymaker", "PLA-CF")
            .unwrap()
            .action,
        "warning"
    );
    assert_eq!(
        mapping::blacklisted(&list, "Bambu Lab", "PLA-CF")
            .unwrap()
            .action,
        "warning"
    );
    assert_eq!(
        mapping::blacklisted(&list, "Bambu Lab", "PVA")
            .unwrap()
            .action,
        "warning"
    );
    assert_eq!(
        mapping::blacklisted(&list, "Bambu Lab", "TPU")
            .unwrap()
            .action,
        "prohibition"
    );
    assert!(mapping::blacklisted(&list, "Generic", "PLA").is_none());
    assert!(mapping::blacklisted(&list, "", "PLA").is_none());
    assert_eq!(
        mapping::blacklisted(&list, "", "PVA").unwrap().action,
        "warning"
    );
}

#[test]
fn an_address_and_an_access_code_are_checked_as_the_dialog_checks_them() {
    assert!(printers::is_ip("192.168.1.20"));
    assert!(!printers::is_ip("192.168.1"));
    assert!(!printers::is_ip("192.168.1.256"));
    assert!(!printers::is_ip("printer.local"));
    assert!(printers::is_access_code("1a2B3c4D"));
    assert!(!printers::is_access_code("1234567"));
    assert!(!printers::is_access_code("1234-678"));
    assert!(is_auth_refusal("5"));
    assert!(is_auth_refusal("MQTT connect failed, rc=5"));
    assert!(!is_auth_refusal("rc=50"));
    assert!(is_auth_refusal("Not authorized"));
    assert!(!is_auth_refusal("timed out"));
    assert_eq!(
        scrub("code 12345678 refused", &["12345678".into()]),
        "code ******** refused"
    );
}

#[test]
fn hello_is_said_first_and_a_bridge_is_refused_for_its_version() {
    // No bridge: nothing started, said.
    let absent = Client::new(Launcher::Absent, None, quick());
    assert_eq!(absent.state(), State::Absent);
    assert_eq!(absent.start().unwrap_err(), "Network plug-in missing");
    // A good one.
    let mock = Mock::new(Setup::default());
    let good = client(&mock);
    assert_eq!(good.state(), State::Idle);
    good.start().unwrap();
    assert_eq!(good.state(), State::Running);
    assert_eq!(good.hello().unwrap().plugin_version, "02.01.01.52");
    assert_eq!(mock.requests("hello")[0]["protocol"], 1);
    // Too old (protocol 0 answers -101), too new, too few methods, no
    // plug-in, a plug-in it cannot use: each refused and stopped.
    let cases: Vec<(Setup, &str)> = vec![
        (
            Setup {
                protocol: 0,
                ..Setup::default()
            },
            "The UltraNet bridge is too old for this app (protocol 0, this app needs 1).",
        ),
        (
            Setup {
                protocol: 2,
                ..Setup::default()
            },
            "The UltraNet bridge is too new for this app (protocol 2, this app speaks 1).",
        ),
        (
            Setup {
                methods: vec!["hello", "bye", "connect_printer"],
                ..Setup::default()
            },
            "The UltraNet bridge is too old for this app: it does not offer disconnect_printer.",
        ),
        (
            Setup {
                plugin_loaded: false,
                ..Setup::default()
            },
            "Network plug-in missing: libbambu_networking.dylib is not beside the bridge",
        ),
        (
            Setup {
                plugin_compatible: false,
                ..Setup::default()
            },
            "The UltraNet bridge cannot use its network plug-in (plug-in version 01.10.01.01, needed 02.01.01.52).",
        ),
    ];
    for (setup, said) in cases {
        let mock = Mock::new(setup);
        let refused = client(&mock);
        assert_eq!(refused.start().unwrap_err(), said);
        assert_eq!(refused.state(), State::Refused(said.into()));
        // Not started again until "Retry".
        assert_eq!(refused.start().unwrap_err(), said);
        assert_eq!(mock.seen.lock().unwrap().starts, 1);
        // Nothing but hello (and bye) was said to it.
        let methods: Vec<String> = mock
            .seen
            .lock()
            .unwrap()
            .requests
            .iter()
            .map(|(m, _)| m.clone())
            .collect();
        assert!(
            methods.iter().all(|m| m == "hello" || m == "bye"),
            "{methods:?}"
        );
    }
}

#[test]
fn a_printer_is_connected_and_reports() {
    let mock = Mock::new(Setup::default());
    let client = client(&mock);
    let session = client
        .connect("01P09C470100142", "192.168.1.20", "12345678")
        .unwrap();
    assert_eq!(session, Session::Connected);
    let sent = &mock.requests("connect_printer")[0];
    assert_eq!(sent["username"], "bblp");
    assert_eq!(sent["use_ssl"], true);
    // The full report and the versions are asked for (GUI_App.cpp l.2681).
    assert!(
        client.wait_printer("01P09C470100142", Duration::from_secs(5), |r| r
            .machine
            .ready())
    );
    let asked: Vec<String> = mock
        .requests("send_message_to_printer")
        .iter()
        .map(|p| p["json"].as_str().unwrap().to_owned())
        .collect();
    assert!(asked.iter().any(|j| j.contains("\"pushall\"")), "{asked:?}");
    assert!(
        asked.iter().any(|j| j.contains("\"get_version\"")),
        "{asked:?}"
    );
    let machine = client.remote("01P09C470100142").machine;
    assert_eq!(machine.ams[0].trays.len(), 3);
    assert_eq!(machine.print_status, "IDLE");
    // A report pushed later is read.
    mock.report(json!({ "gcode_state": "RUNNING", "mc_percent": 5 }));
    assert!(client.wait_printer(
        "01P09C470100142",
        Duration::from_secs(5),
        |r| r.machine.percent == Some(5)
    ));
}

#[test]
fn a_wrong_access_code_is_refused_and_never_repeated() {
    let mock = Mock::new(Setup {
        echo_code: true,
        ..Setup::default()
    });
    let client = client(&mock);
    client.set_codes(vec!["87654321".into()]);
    let session = client
        .connect("01P09C470100142", "192.168.1.20", "87654321")
        .unwrap();
    let Session::Failed(msg) = session else {
        panic!("{session:?}")
    };
    assert!(client.remote("01P09C470100142").refused);
    // The bridge echoed the code; it was taken out.
    assert!(!msg.contains("87654321"), "{msg}");
    assert!(msg.contains("********"), "{msg}");
}

#[test]
fn a_bridge_that_dies_ends_what_waits_and_is_started_again() {
    let mock = Mock::new(Setup::default());
    let client = client(&mock);
    client
        .connect("01P09C470100142", "192.168.1.20", "12345678")
        .unwrap();
    let job = client
        .start_job(
            "start_local_print",
            json!({ "params": { "password": "12345678" } }),
        )
        .unwrap();
    // The bridge dies.
    mock.die();
    let ended = client.wait_job(job, |_, _, _| {}, || false);
    // The job ended either way: done before it died, or lost with it.
    if let Err(why) = ended {
        assert_eq!(why, "The UltraNet bridge stopped.");
    }
    assert!(
        client.wait_printer("01P09C470100142", Duration::from_secs(5), |r| r.session
            == Session::Lost)
    );
    assert_eq!(
        client.state(),
        State::Failed("The UltraNet bridge stopped.".into())
    );
    // Started again by the next thing that needs it.
    client.start().unwrap();
    assert_eq!(mock.seen.lock().unwrap().starts, 2);
    // Three deaths in a minute: not again until "Retry".
    mock.die();
    std::thread::sleep(Duration::from_millis(100));
    let _ = client.state();
    client.start().unwrap();
    mock.die();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        client.state(),
        State::GaveUp("The UltraNet bridge keeps stopping.".into())
    );
    assert!(client.start().is_err());
    client.retry();
    client.start().unwrap();
    assert_eq!(client.state(), State::Running);
}

/// A machine with the given trays: (tray index, type, colour "RRGGBBAA",
/// preset id), in the AMS units their indexes say.
fn trays_machine(trays: &[(i64, &str, &str, &str)]) -> Machine {
    let mut units: std::collections::BTreeMap<i64, Vec<Value>> = std::collections::BTreeMap::new();
    let mut bits = 0u64;
    for (index, kind, colour, preset) in trays {
        units.entry(index / 4).or_default().push(
            json!({ "id": (index % 4).to_string(), "tray_type": kind, "tray_info_idx": preset, "tray_color": colour }),
        );
        bits |= 1 << index;
    }
    let ams_bits: u64 = units.keys().map(|unit| 1 << unit).sum();
    let ams: Vec<Value> = units
        .into_iter()
        .map(|(unit, trays)| json!({ "id": unit.to_string(), "info": "1001", "tray": trays }))
        .collect();
    let mut machine = Machine::default();
    machine.read(&report(json!({
        "ams": { "ams_exist_bits": format!("{ams_bits:x}"), "tray_exist_bits": format!("{bits:x}"), "ams": ams },
    })));
    machine
}

fn hex(rgb: (u8, u8, u8)) -> String {
    format!("{:02X}{:02X}{:02X}", rgb.0, rgb.1, rgb.2)
}

/// The ten colours of aceRage/EdgeSlicer#1039's table.
const NAMED: [(&str, (u8, u8, u8)); 10] = [
    ("red", (255, 0, 0)),
    ("orange", (255, 128, 0)),
    ("dark red", (128, 0, 0)),
    ("pink", (255, 105, 180)),
    ("black", (0, 0, 0)),
    ("grey", (128, 128, 128)),
    ("white", (255, 255, 255)),
    ("navy", (0, 0, 128)),
    ("blue", (0, 0, 255)),
    ("purple", (128, 0, 128)),
];

fn named(name: &str) -> (u8, u8, u8) {
    NAMED.iter().find(|(n, _)| *n == name).unwrap().1
}

/// One filament, two PLA trays: the tray picked (0 or 1).
fn pick_of(filament_colour: (u8, u8, u8), a: (u8, u8, u8), b: (u8, u8, u8)) -> i64 {
    let (ta, tb) = (format!("{}FF", hex(a)), format!("{}FF", hex(b)));
    let machine = trays_machine(&[(0, "PLA", &ta, "GFL99"), (1, "PLA", &tb, "GFL99")]);
    let (_, result) = mapping::auto_map(
        &machine,
        &[filament(0, &format!("#{}", hex(filament_colour)), "PLA")],
    );
    result[0].tray_id
}

#[test]
fn the_auto_match_picks_as_the_reference_does_since_1039() {
    // aceRage/EdgeSlicer#1039's table: the 13 of 360 choices that changed
    // there (filament, tray A, tray B, the pick now).
    let changed = [
        ("orange", "pink", "grey", "grey"),
        ("orange", "black", "purple", "black"),
        ("dark red", "pink", "grey", "pink"),
        ("dark red", "black", "grey", "black"),
        ("pink", "black", "blue", "black"),
        ("pink", "white", "navy", "white"),
        ("grey", "orange", "navy", "orange"),
        ("grey", "dark red", "pink", "pink"),
        ("grey", "dark red", "black", "black"),
        ("white", "red", "black", "black"),
        ("blue", "dark red", "white", "dark red"),
        ("purple", "dark red", "black", "black"),
        ("purple", "dark red", "blue", "blue"),
    ];
    for (f, a, b, now) in changed {
        assert_eq!(
            pick_of(named(f), named(a), named(b)),
            if now == a { 0 } else { 1 },
            "{f} with {a} and {b}"
        );
    }
    // The whole table: every one of the 360 picks is the nearer tray in
    // CIE76 on 0..1, as AmsMatch.cpp computes it.
    let colour = |c: (u8, u8, u8)| format!("#{}", hex(c));
    let mut cases = 0;
    for (f, fc) in NAMED {
        for (i, (a, ac)) in NAMED.iter().enumerate() {
            for (b, bc) in NAMED.iter().skip(i + 1) {
                if f == *a || f == *b {
                    continue;
                }
                cases += 1;
                let nearer_a = mapping::color_distance(&colour(fc), &colour(*ac))
                    <= mapping::color_distance(&colour(fc), &colour(*bc));
                assert_eq!(
                    pick_of(fc, *ac, *bc),
                    if nearer_a { 0 } else { 1 },
                    "{f} with {a} and {b}"
                );
            }
        }
    }
    assert_eq!(cases, 360);
    // The distance itself: black to white 100, black to red 117.34.
    assert!((mapping::color_distance("#000000", "#FFFFFF") - 100.0).abs() < 0.05);
    assert!((mapping::color_distance("#000000", "#FF0000") - 117.34).abs() < 0.1);
    // Picks with an obvious answer: the same colour, a near shade, the
    // only tray of the type, a near white.
    let machine = trays_machine(&[
        (0, "PLA", "FF0000FF", "GFL99"),
        (1, "PLA", "0000FFFF", "GFL99"),
        (2, "PETG", "FFFFFFFF", "GFL99"),
        (3, "PLA", "FFFFFFFF", "GFL99"),
    ]);
    let filaments = [
        filament(0, "#FF0000", "PLA"),
        filament(1, "#1414C8", "PLA"),
        filament(2, "#00FF00", "PETG"),
        filament(3, "#FAFAFA", "PLA"),
    ];
    let (_, result) = mapping::auto_map(&machine, &filaments);
    assert_eq!(
        result.iter().map(|r| r.tray_id).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    // A tie goes to the tray holding the filament's own preset, whatever
    // the tray numbers: the second AMS only, trays 4 and 7 both black.
    let machine = trays_machine(&[
        (4, "PLA", "000000FF", "GFL99"),
        (7, "PLA", "000000FF", "GFA01"),
    ]);
    let mut black = filament(0, "#000000", "PLA");
    black.filament_id = "GFA01".into();
    assert_eq!(
        mapping::auto_map(&machine, std::slice::from_ref(&black)).1[0].tray_id,
        7
    );
    // On the first AMS with its four trays, as before.
    let machine = trays_machine(&[
        (0, "PLA", "000000FF", "GFL99"),
        (1, "PLA", "000000FF", "GFA01"),
        (2, "PLA", "090909FF", "GFL99"),
        (3, "PLA", "000000FF", "GFL99"),
    ]);
    assert_eq!(mapping::auto_map(&machine, &[black]).1[0].tray_id, 1);
    // A pick keeps its own distance; a see-through filament on an opaque
    // tray is far (999999), which only the last fallback drops.
    let machine = trays_machine(&[
        (0, "PLA", "C80000FF", "GFL99"),
        (1, "PLA", "0000FFFF", "GFL99"),
    ]);
    let (_, r) = mapping::auto_map(
        &machine,
        &[
            filament(0, "#FF0000", "PLA"),
            filament(1, "#0000FF80", "PLA"),
        ],
    );
    assert_eq!(r[0].tray_id, 0);
    assert!((r[0].distance - mapping::color_distance("#FF0000", "C80000FF")).abs() < 1e-9);
    assert_eq!(r[1].tray_id, 1);
    assert_eq!(r[1].distance, 999_999.0);
}
