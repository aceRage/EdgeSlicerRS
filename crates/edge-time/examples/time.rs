//! `cargo run --release -p edge-time --example time -- file.gcode ...`:
//! each file's time by both planners, beside the time written in it.
use edge_time::{Limits, Model, estimate};

fn clock(seconds: f64) -> String {
    let whole = seconds.round() as u64;
    format!("{}m {:02}s", whole / 60, whole % 60)
}

fn main() {
    for path in std::env::args().skip(1) {
        let text = std::fs::read_to_string(&path).expect("a readable file");
        let Some(limits) = Limits::from_gcode(&text) else {
            println!("{path}: lists no firmware or machine limits");
            continue;
        };
        let written = text
            .lines()
            .find_map(|line| line.strip_prefix("; estimated printing time (normal mode) = "))
            .unwrap_or("none");
        let reference = estimate(&text, &limits, Model::Reference);
        let klipper = estimate(&text, &limits, Model::Klipper);
        println!(
            "{path}\n  written {written}; reference model {} ({:.1} s); Klipper model {} ({:.1} s)",
            clock(reference.seconds),
            reference.seconds,
            clock(klipper.seconds),
            klipper.seconds
        );
    }
}
