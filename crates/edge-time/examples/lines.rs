//! `cargo run --release -p edge-time --example lines -- file.gcode [klipper]`:
//! the seconds each line of a file takes, one figure a line, as the
//! reference's planner times it (or Klipper's), for working out where a
//! file's time goes. The figures add up to the time `time` gives.
use std::io::Write;

use edge_time::{Limits, Model, estimate};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a G-code file");
    let model = match args.next().as_deref() {
        Some("klipper") => Model::Klipper,
        _ => Model::Reference,
    };
    let text = std::fs::read_to_string(&path).expect("a readable file");
    let limits = Limits::from_gcode(&text).expect("a file that lists its firmware");
    let estimate = estimate(&text, &limits, model);
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for seconds in &estimate.by_line {
        writeln!(out, "{seconds:.9}").expect("written");
    }
}
