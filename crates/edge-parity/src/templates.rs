//! What the printer's templates and the writer's own commands put in a
//! file, compared line for line, layer by layer: the start and end G-code,
//! the layer change, the timelapse and the tool change. The two slicers'
//! toolpaths differ in their moves, so the moves of features are left out;
//! the moves the templates make (in the start and end G-code, `Custom`, and
//! in a tool change on the prime tower, up to its rows) are kept.
//!
//! Left out on purpose, being different by design or not done here:
//! progress (`M73`, which this engine writes at each layer and the
//! reference wherever its figure changes); the labels of objects for
//! skipping them (`; printing object`, `M624`/`M625`, `EXCLUDE_OBJECT_*`,
//! not done); the line width and height tags; and, inside custom G-code, a
//! feed rate already in force, which the reference's writer takes out and
//! this engine writes as the template has it (both files are read the same
//! way here).

use std::collections::BTreeMap;

/// The lines kept of each layer; the first entry is what comes before the
/// first layer (the start G-code).
pub fn sections(text: &str) -> Vec<Vec<String>> {
    let mut layers: Vec<Vec<String>> = vec![Vec::new()];
    let mut running = false;
    let mut custom = true;
    let mut changing = false;
    let mut feedrate: Option<f64> = None;
    for line in text.lines() {
        let line = line.trim_end();
        if line.contains("EXECUTABLE_BLOCK_START") {
            running = true;
            continue;
        }
        if !running {
            continue;
        }
        if line.contains("EXECUTABLE_BLOCK_END") {
            break;
        }
        if line.trim().is_empty() || line.starts_with("M73 ") {
            continue;
        }
        if matches!(line, "; CHANGE_LAYER" | ";LAYER_CHANGE") {
            layers.push(vec!["(layer change)".to_owned()]);
            custom = false;
            continue;
        }
        if let Some(feature) = line
            .strip_prefix("; FEATURE: ")
            .or_else(|| line.strip_prefix(";TYPE:"))
        {
            custom = feature.trim() == "Custom";
            layers
                .last_mut()
                .unwrap()
                .push(format!("(feature {})", feature.trim()));
            continue;
        }
        if line.starts_with("; CP TOOLCHANGE START") {
            changing = true;
        }
        if line.starts_with("; CP TOOLCHANGE WIPE") || line.starts_with("; CP TOOLCHANGE END") {
            changing = false;
        }
        if left_out(line) {
            continue;
        }
        let code = line.split(';').next().unwrap_or("");
        let words: Vec<&str> = code.split_whitespace().collect();
        let moving = matches!(words.first(), Some(&("G0" | "G1" | "G2" | "G3")));
        let f = words
            .iter()
            .find_map(|word| word.strip_prefix('F')?.parse::<f64>().ok());
        let mut kept = line.to_owned();
        if moving {
            let has = |letter: char| {
                words[1..]
                    .iter()
                    .any(|word| word.starts_with(letter) && word.len() > 1)
            };
            let template = custom || (changing && !((has('X') || has('Y')) && has('E')));
            if let Some(f) = f {
                let in_force = feedrate == Some(f);
                feedrate = Some(f);
                if !template {
                    continue;
                }
                if in_force {
                    kept = without_feedrate(&kept);
                }
            } else if !template {
                continue;
            }
        }
        layers.last_mut().unwrap().push(normal(&kept));
    }
    layers
}

/// Lines that are not compared (see the module's comment).
fn left_out(line: &str) -> bool {
    [
        "; printing object",
        "; stop printing object",
        "; start printing object",
        "EXCLUDE_OBJECT",
        ";WIDTH:",
        ";HEIGHT:",
        ";Z:",
        "; LINE_WIDTH:",
        "; LAYER_HEIGHT:",
        "; Z_HEIGHT:",
    ]
    .iter()
    .any(|start| line.starts_with(start))
        || matches!(line.split_whitespace().next(), Some("M624" | "M625"))
}

fn without_feedrate(line: &str) -> String {
    let (code, comment) = match line.find(';') {
        Some(at) => (&line[..at], &line[at..]),
        None => (line, ""),
    };
    let words: Vec<&str> = code
        .split_whitespace()
        .filter(|word| !(word.starts_with('F') && word[1..].parse::<f64>().is_ok()))
        .collect();
    format!("{} {comment}", words.join(" "))
}

/// Numbers to the thousandth without a leading zero, as the reference
/// writes moves, and runs of spaces as one.
fn normal(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for (index, word) in line.split_whitespace().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(&number(word));
    }
    out
}

fn number(word: &str) -> String {
    let split = word
        .find(|c: char| c.is_ascii_digit() || c == '-' || c == '.')
        .unwrap_or(word.len());
    let (head, tail) = word.split_at(split);
    if !tail.contains('.') {
        return word.to_owned();
    }
    match tail.parse::<f64>() {
        Ok(value) => {
            let text = format!("{value:.3}");
            let text = text.trim_end_matches('0').trim_end_matches('.');
            let text = text.replacen(
                "0.",
                ".",
                usize::from(text.starts_with("0.") || text.starts_with("-0.")),
            );
            format!(
                "{head}{}",
                if text.is_empty() || text == "-" {
                    "0"
                } else {
                    &text
                }
            )
        }
        Err(_) => word.to_owned(),
    }
}

/// How the two files' template lines compare.
#[derive(Debug, Default)]
pub struct Comparison {
    pub layers: (usize, usize),
    /// Lines of either file without their match in the other.
    pub differing: usize,
    /// The first stretches that differ: the layer (0 before the first),
    /// then the reference's lines and this engine's.
    pub hunks: Vec<(usize, Vec<String>, Vec<String>)>,
    /// Every differing line with its numbers taken out, and how often.
    pub kinds: BTreeMap<String, usize>,
}

pub fn compare(reference: &str, ours: &str) -> Comparison {
    let (a, b) = (sections(reference), sections(ours));
    let mut out = Comparison {
        layers: (a.len() - 1, b.len() - 1),
        ..Comparison::default()
    };
    let empty = Vec::new();
    for index in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(index).unwrap_or(&empty),
            b.get(index).unwrap_or(&empty),
        );
        for (only_x, only_y) in differences(x, y) {
            out.differing += only_x.len().max(only_y.len());
            for (side, lines) in [("reference", &only_x), ("here", &only_y)] {
                for line in lines.iter() {
                    let kind: String = line
                        .chars()
                        .map(|c| if c.is_ascii_digit() { '#' } else { c })
                        .collect();
                    *out.kinds.entry(format!("{side}: {kind}")).or_default() += 1;
                }
            }
            if out.hunks.len() < 12 {
                out.hunks.push((index, only_x, only_y));
            }
        }
    }
    out
}

/// The stretches where two runs of lines differ, by their longest common
/// run of lines.
fn differences(x: &[String], y: &[String]) -> Vec<(Vec<String>, Vec<String>)> {
    // Common head and tail first, which is most of it.
    let head = x.iter().zip(y).take_while(|(a, b)| a == b).count();
    let tail = x[head..]
        .iter()
        .rev()
        .zip(y[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (x, y) = (&x[head..x.len() - tail], &y[head..y.len() - tail]);
    if x.is_empty() && y.is_empty() {
        return Vec::new();
    }
    if x.len() * y.len() > 20_000_000 {
        return vec![(x.to_vec(), y.to_vec())];
    }
    let (n, m) = (x.len(), y.len());
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * (m + 1) + j] = if x[i] == y[j] {
                table[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                table[(i + 1) * (m + 1) + j].max(table[i * (m + 1) + j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut only_x, mut only_y) = (Vec::new(), Vec::new());
    while i < n || j < m {
        if i < n && j < m && x[i] == y[j] {
            if !only_x.is_empty() || !only_y.is_empty() {
                out.push((std::mem::take(&mut only_x), std::mem::take(&mut only_y)));
            }
            i += 1;
            j += 1;
        } else if j < m && (i == n || table[i * (m + 1) + j + 1] >= table[(i + 1) * (m + 1) + j]) {
            only_y.push(y[j].clone());
            j += 1;
        } else {
            only_x.push(x[i].clone());
            i += 1;
        }
    }
    if !only_x.is_empty() || !only_y.is_empty() {
        out.push((only_x, only_y));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_templates_lines_are_kept_and_the_toolpaths_moves_are_not() {
        let text = "; EXECUTABLE_BLOCK_START\nM73 P0 R9\n; FEATURE: Custom\nG28\nG1 X10 F600\nG1 Y5 F600\n\
                    ; CHANGE_LAYER\n; Z_HEIGHT: 0.2\nM991 S0 P0\n; FEATURE: Outer wall\nG1 X1 Y1 E0.5 F900\n\
                    M106 S255\n; printing object A\nM624 AQ==\n; EXECUTABLE_BLOCK_END\n";
        let kept = sections(text);
        assert_eq!(kept[0], ["(feature Custom)", "G28", "G1 X10 F600", "G1 Y5"]);
        assert_eq!(
            kept[1],
            [
                "(layer change)",
                "M991 S0 P0",
                "(feature Outer wall)",
                "M106 S255"
            ]
        );
    }

    #[test]
    fn the_two_kinds_of_tags_and_numbers_compare_alike() {
        let bambu = "; EXECUTABLE_BLOCK_START\n; FEATURE: Custom\nG1 Z0.400 F1200\n; CHANGE_LAYER\nM104 S220\n";
        let other =
            "; EXECUTABLE_BLOCK_START\n;TYPE:Custom\nG1 Z.4 F1200\n;LAYER_CHANGE\nM104 S220\n";
        let compared = compare(bambu, other);
        assert_eq!(compared.differing, 0);
        let changed = compare(bambu, &other.replace("S220", "S215"));
        assert_eq!(changed.differing, 1);
        assert_eq!(
            changed.hunks[0],
            (
                1,
                vec!["M104 S220".to_owned()],
                vec!["M104 S215".to_owned()]
            )
        );
    }

    #[test]
    fn a_tower_change_keeps_its_template_moves_up_to_its_rows() {
        let text = "; EXECUTABLE_BLOCK_START\n; CHANGE_LAYER\n; FEATURE: Prime tower\n; CP TOOLCHANGE START\n\
                    G1 X70 F21000\nG1 X15 Y218 E0.5\nT1\n; CP TOOLCHANGE WIPE\nG1 X50 F3000\n";
        assert_eq!(
            sections(text)[1],
            [
                "(layer change)",
                "(feature Prime tower)",
                "; CP TOOLCHANGE START",
                "G1 X70 F21000",
                "T1",
                "; CP TOOLCHANGE WIPE"
            ]
        );
    }
}
