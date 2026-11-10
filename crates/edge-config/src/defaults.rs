//! The reference's own defaults, and the text form it writes settings in.
//!
//! `reference_defaults.txt` is the settings block of a G-code file the
//! reference wrote for a project that set nothing (tools/make_defaults.py).
//! A setting a profile leaves out takes its value from there, and a G-code
//! file's settings block lists the settings named there.

use std::sync::OnceLock;

use crate::{Settings, Value};

/// Every setting the reference knows, with its default.
pub fn reference_defaults() -> &'static Settings {
    static DEFAULTS: OnceLock<Settings> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        let mut settings = Settings::default();
        for line in include_str!("reference_defaults.txt").lines() {
            if line.starts_with(['#', '?']) || line.is_empty() {
                continue;
            }
            let (key, text) = line.split_once(" =").unwrap_or((line, ""));
            settings.insert(key, deserialized(text.strip_prefix(' ').unwrap_or(text)));
        }
        settings
    })
}

/// The slicer and version the defaults were taken from, as a G-code
/// file's first line names it, such as `OrcaSlicer 2.4.9.0`.
pub fn reference_producer() -> &'static str {
    include_str!("reference_defaults.txt")
        .lines()
        .find_map(|line| line.strip_prefix("# producer: "))
        .unwrap_or("OrcaSlicer")
}

/// Whether the reference has a setting of this name: one with a default,
/// or one it lists only when a profile gives it. A project carries other
/// things beside settings, a print host's address and keys among them,
/// and those are not for writing into a file sent to a printer.
pub fn reference_knows(key: &str) -> bool {
    reference_defaults().get(key).is_some()
        || include_str!("reference_defaults.txt")
            .lines()
            .any(|line| line.strip_prefix('?') == Some(key))
}

/// Lists of names from a fixed set, which the reference joins by commas
/// as it does numbers. Nothing in their text tells them from a list of
/// free texts, which it joins by semicolons, so they are named here.
const CHOICES: [&str; 6] = [
    "extruder_type",
    "retract_lift_enforce",
    "z_hop_types",
    "nozzle_type",
    "nozzle_volume_type",
    "filament_volume_type",
];

/// Whether an item of a list is written bare and joined by commas.
fn plain(item: &str) -> bool {
    let item = item.trim();
    item == "nil"
        || item.trim_end_matches('%').parse::<f64>().is_ok()
        || item
            .split_once('x')
            .is_some_and(|(x, y)| x.parse::<f64>().is_ok() && y.parse::<f64>().is_ok())
}

/// Text with its escapes taken out (Config.cpp `unescape_string_cstyle`).
/// Settings arrive both ways, from vendor presets with their escapes and
/// from projects without, and a text with none is left as it is.
fn unescaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Text with line breaks, quotes and backslashes escaped (`escape_string_cstyle`).
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in unescaped(text).chars() {
        match c {
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// A setting as the reference writes it in a G-code file's settings
/// block (Config.cpp `serialize`): a list of numbers joined by commas, a
/// list of texts by semicolons with any that hold spaces or escapes in
/// quotes (`escape_strings_cstyle`).
pub fn serialized(key: &str, value: &Value) -> String {
    match value {
        Value::One(text) => escaped(text),
        Value::Many(items) => {
            if CHOICES.contains(&key) || items.iter().all(|item| plain(item)) {
                return items.join(",");
            }
            items
                .iter()
                .map(|item| {
                    let item = unescaped(item);
                    let quoted = (items.len() == 1 && item.is_empty())
                        || item
                            .chars()
                            .any(|c| matches!(c, ' ' | '\t' | '\\' | '"' | '\r' | '\n'));
                    if quoted {
                        format!("\"{}\"", escaped(&item))
                    } else {
                        item
                    }
                })
                .collect::<Vec<_>>()
                .join(";")
        }
    }
}

/// The reverse of [`serialized`], as far as the text allows: a value in
/// quotes, or with semicolons and no spaces, is a list of texts; two or
/// more numbers between commas are a list of numbers; anything else is
/// one value.
fn deserialized(text: &str) -> Value {
    let texts = text.starts_with('"') || (text.contains(';') && !text.contains(' '));
    if texts {
        let mut items = Vec::new();
        let mut rest = text;
        loop {
            let item;
            if let Some(quoted) = rest.strip_prefix('"') {
                // Up to the closing quote, stepping over escaped ones.
                let mut end = quoted.len();
                let mut escape = false;
                for (at, c) in quoted.char_indices() {
                    if escape {
                        escape = false;
                    } else if c == '\\' {
                        escape = true;
                    } else if c == '"' {
                        end = at;
                        break;
                    }
                }
                item = quoted[..end].to_owned();
                rest = quoted.get(end + 1..).unwrap_or("");
            } else {
                let end = rest.find(';').unwrap_or(rest.len());
                item = rest[..end].to_owned();
                rest = &rest[end..];
            }
            items.push(item);
            match rest.strip_prefix(';') {
                Some(after) => rest = after,
                None => break,
            }
        }
        return Value::Many(items);
    }
    let pieces: Vec<&str> = text.split(',').collect();
    if pieces.len() >= 2 && pieces.iter().all(|piece| plain(piece)) {
        return Value::Many(pieces.into_iter().map(str::to_owned).collect());
    }
    Value::One(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_is_written_back_as_the_reference_wrote_it() {
        let defaults = reference_defaults();
        let mut count = 0;
        for line in include_str!("reference_defaults.txt").lines() {
            if line.starts_with(['#', '?']) {
                continue;
            }
            let (key, text) = line.split_once(" =").unwrap();
            let text = text.strip_prefix(' ').unwrap_or(text);
            let value = defaults.get(key).unwrap();
            assert_eq!(serialized(key, value), text, "{key}");
            count += 1;
        }
        assert!(count > 700, "{count} defaults");
        assert_eq!(defaults.len(), count);
    }

    #[test]
    fn a_print_hosts_keys_are_not_settings() {
        assert!(reference_knows("filament_type"));
        assert!(reference_knows("printer_variant"), "known, with no default");
        for private in [
            "print_host",
            "printhost_apikey",
            "printhost_password",
            "name",
        ] {
            assert!(!reference_knows(private), "{private}");
        }
    }

    #[test]
    fn defaults_read_as_the_kinds_of_value_they_are() {
        let defaults = reference_defaults();
        assert_eq!(defaults.float("layer_height").unwrap(), Some(0.2));
        assert_eq!(defaults.flag("accel_to_decel_enable").unwrap(), Some(true));
        assert_eq!(defaults.texts("filament_type").unwrap(), ["PLA"]);
        assert_eq!(defaults.texts("printable_area").unwrap().len(), 4);
        assert_eq!(defaults.texts("filament_notes").unwrap(), [""]);
        assert_eq!(defaults.texts("inherits_group").unwrap(), ["", ""]);
        // The start G-code is one text, semicolons and all.
        assert!(
            defaults
                .text("machine_start_gcode")
                .unwrap()
                .starts_with("G28 ; home all axes")
        );
    }

    #[test]
    fn texts_are_written_the_same_whichever_way_they_arrived() {
        let one = |text: &str| serialized("machine_end_gcode", &Value::One(text.into()));
        // From a project, with real line breaks and quotes; from a vendor
        // preset, with both already escaped.
        assert_eq!(one("A \"b\"\nC"), "A \\\"b\\\"\\nC");
        assert_eq!(one("A \\\"b\\\"\\nC"), "A \\\"b\\\"\\nC");
        let many = |items: &[&str]| {
            serialized(
                "filament_settings_id",
                &Value::Many(items.iter().map(|item| (*item).to_owned()).collect()),
            )
        };
        assert_eq!(many(&["Generic PLA", "PETG"]), "\"Generic PLA\";PETG");
        assert_eq!(many(&["", ""]), ";");
        assert_eq!(many(&[""]), "\"\"");
        assert_eq!(
            serialized(
                "z_hop_types",
                &Value::Many(vec!["Auto Lift".into(), "Auto Lift".into()])
            ),
            "Auto Lift,Auto Lift"
        );
    }
}
