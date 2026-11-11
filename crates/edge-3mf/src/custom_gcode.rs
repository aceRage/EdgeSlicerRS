//! G-code set at a layer of a plate: pauses, custom G-code, the printer's
//! template, filament changes (`libslic3r/CustomGCode.hpp`), as a project
//! keeps them in `Metadata/custom_gcode_per_layer.xml` (bbs_3mf.cpp
//! `_extract_custom_gcode_per_print_z_from_archive` and
//! `_add_custom_gcode_per_print_z_file_to_archive`).

use std::{collections::BTreeMap, fmt::Write as _};

use quick_xml::{Reader, events::Event};

use crate::{
    Error,
    xml::{attributes, malformed, number},
};

/// What is done at a layer (`CustomGCode::Type`), numbered as the file
/// numbers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CustomKind {
    /// A colour change on a single-extruder printer (`M600`). The
    /// reference makes none of these any more and writes none of what
    /// one asks for.
    ColorChange,
    /// "Add Pause": the printer's `machine_pause_gcode`.
    PausePrint,
    /// "Change Filament": the plate prints in another filament from
    /// this layer up.
    ToolChange,
    /// "Add Custom Template": the printer's `template_custom_gcode`.
    Template,
    /// "Add Custom G-code": the text given, as it is.
    Custom,
    /// A number the reference does not know.
    Unknown,
}

impl CustomKind {
    /// The number the file gives it.
    pub fn number(self) -> i32 {
        match self {
            CustomKind::ColorChange => 0,
            CustomKind::PausePrint => 1,
            CustomKind::ToolChange => 2,
            CustomKind::Template => 3,
            CustomKind::Custom => 4,
            CustomKind::Unknown => 5,
        }
    }

    pub fn from_number(number: i32) -> Self {
        match number {
            0 => CustomKind::ColorChange,
            1 => CustomKind::PausePrint,
            2 => CustomKind::ToolChange,
            3 => CustomKind::Template,
            4 => CustomKind::Custom,
            _ => CustomKind::Unknown,
        }
    }
}

/// Which kind of print the items were made for (`CustomGCode::Mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomMode {
    /// One filament in the project.
    SingleExtruder,
    /// Several filaments, the plate printed in one of them: a filament
    /// change switches the whole plate to another.
    MultiAsSingle,
    /// Several filaments printed together.
    MultiExtruder,
}

impl CustomMode {
    /// Its name in the file.
    pub fn name(self) -> &'static str {
        match self {
            CustomMode::SingleExtruder => "SingleExtruder",
            CustomMode::MultiAsSingle => "MultiAsSingle",
            CustomMode::MultiExtruder => "MultiExtruder",
        }
    }

    /// As the reference reads a name: anything but the first two is
    /// `MultiExtruder`.
    pub fn from_name(name: &str) -> Self {
        match name {
            "SingleExtruder" => CustomMode::SingleExtruder,
            "MultiAsSingle" => CustomMode::MultiAsSingle,
            _ => CustomMode::MultiExtruder,
        }
    }
}

/// One thing done at a layer (`CustomGCode::Item`).
#[derive(Clone, Debug, PartialEq)]
pub struct CustomItem {
    /// The top of the layer it is done at, in mm (`top_z`).
    pub print_z: f64,
    pub kind: CustomKind,
    /// The filament, from 1: the one a filament change switches to; for
    /// the rest, the one the plate was printed in when it was made.
    pub extruder: i32,
    /// The colour of a filament change's or a template's filament.
    pub color: String,
    /// The G-code of a custom item; a pause's message, which the
    /// reference no longer sets.
    pub extra: String,
    /// What the file says it writes (`gcode`), which nothing reads back:
    /// the pause or template G-code of the printer it was saved with,
    /// `tool_change`, or the custom text.
    pub gcode: String,
}

/// The items of one plate, in order of height (`CustomGCode::Info`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CustomGcode {
    /// None when the file says nothing; the reference then settles it from
    /// the items (`check_mode_for_custom_gcode_per_print_z`), see
    /// [`CustomGcode::mode`].
    pub mode: Option<CustomMode>,
    pub items: Vec<CustomItem>,
}

impl CustomGcode {
    /// The mode in force: the file's, else as the reference settles it
    /// (CustomGCode.cpp `check_mode_for_custom_gcode_per_print_z`).
    pub fn mode(&self) -> CustomMode {
        if let Some(mode) = self.mode {
            return mode;
        }
        let mut single = true;
        for item in &self.items {
            if item.kind == CustomKind::ToolChange {
                return CustomMode::MultiAsSingle;
            }
            if item.kind == CustomKind::ColorChange && item.extruder > 1 {
                single = false;
            }
        }
        if single {
            CustomMode::SingleExtruder
        } else {
            CustomMode::MultiExtruder
        }
    }

    /// The item at a height, if there is one.
    pub fn at(&self, print_z: f64) -> Option<&CustomItem> {
        self.items.iter().find(|item| item.print_z == print_z)
    }
}

/// Every plate's items, by the plate's number, from the file's text. None
/// when the file is not laid out as the reference lays it (its root is not
/// `custom_gcodes_per_layer`), which the reference then does not read.
pub(crate) fn read(text: &str, part: &str) -> Result<Option<BTreeMap<u32, CustomGcode>>, Error> {
    let mut reader = Reader::from_str(text);
    let mut plates: BTreeMap<u32, CustomGcode> = BTreeMap::new();
    // Items straight under the root, for a file without plates: plate 1's.
    let mut loose = CustomGcode::default();
    let mut has_plates = false;
    // The plate being read: its number once known, and its items.
    let mut plate: Option<(Option<u32>, CustomGcode)> = None;
    let mut depth = 0usize;
    let mut rooted = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| malformed(part, e.to_string()))?;
        let (element, empty) = match &event {
            Event::Start(element) => (Some(element.clone()), false),
            Event::Empty(element) => (Some(element.clone()), true),
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 1
                    && let Some((id, info)) = plate.take()
                    && let Some(id) = id
                {
                    plates.entry(id).or_default().merge(info);
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let Some(element) = element else { continue };
        let name = element.local_name();
        let name = name.as_ref();
        match depth {
            0 => {
                if name != "custom_gcodes_per_layer" {
                    return Ok(None);
                }
                rooted = true;
            }
            1 if name == "plate" => {
                has_plates = true;
                if !empty {
                    plate = Some((None, CustomGcode::default()));
                }
            }
            2 if name == "plate_info" => {
                if let Some((own, _)) = &mut plate {
                    attributes(&element, part, |key, value| {
                        if key == "id" {
                            *own = Some(number::<u32>(part, "plate_info id", value)?);
                        }
                        Ok(())
                    })?;
                }
            }
            _ => {
                let into = match (&mut plate, depth) {
                    (Some((_, info)), 2) => Some(info),
                    (None, 1) => Some(&mut loose),
                    _ => None,
                };
                if let Some(info) = into {
                    match name {
                        "mode" => {
                            attributes(&element, part, |key, value| {
                                if key == "value" {
                                    info.mode = Some(CustomMode::from_name(value));
                                }
                                Ok(())
                            })?;
                        }
                        "layer" => info.items.push(item(&element, part)?),
                        _ => {}
                    }
                }
            }
        }
        if !empty {
            depth += 1;
        }
    }
    if !rooted {
        return Ok(None);
    }
    if !has_plates {
        plates.insert(1, loose);
    }
    Ok(Some(plates))
}

impl CustomGcode {
    fn merge(&mut self, other: CustomGcode) {
        if other.mode.is_some() {
            self.mode = other.mode;
        }
        self.items.extend(other.items);
    }
}

/// One `<layer>`, in the layout of today or of PrusaSlicer 2.2 and older,
/// which has no `type` and says what it is by its `gcode`.
fn item(element: &quick_xml::events::BytesStart<'_>, part: &str) -> Result<CustomItem, Error> {
    let mut print_z = None;
    let mut kind = None;
    let mut extruder = None;
    let mut color = None;
    let mut extra = None;
    let mut gcode = None;
    attributes(element, part, |key, value| {
        match key {
            "top_z" => print_z = Some(number::<f64>(part, "top_z", value)?),
            "type" => kind = Some(number::<i32>(part, "type", value)?),
            "extruder" => extruder = Some(number::<i32>(part, "extruder", value)?),
            "color" => color = Some(value.to_owned()),
            "extra" => extra = Some(value.to_owned()),
            "gcode" => gcode = Some(value.to_owned()),
            _ => {}
        }
        Ok(())
    })?;
    let missing = |what: &str| malformed(part, format!("a layer has no {what}"));
    let print_z = print_z.ok_or_else(|| missing("top_z"))?;
    let extruder = extruder.ok_or_else(|| missing("extruder"))?;
    let color = color.ok_or_else(|| missing("color"))?;
    let (kind, extra) = match kind {
        Some(kind) => (
            CustomKind::from_number(kind),
            extra.ok_or_else(|| missing("extra"))?,
        ),
        None => {
            let old = gcode.clone().ok_or_else(|| missing("type or gcode"))?;
            let kind = match old.as_str() {
                "M600" => CustomKind::ColorChange,
                "M601" => CustomKind::PausePrint,
                "tool_change" => CustomKind::ToolChange,
                _ => CustomKind::Custom,
            };
            let extra = match kind {
                CustomKind::PausePrint => color.clone(),
                CustomKind::Custom => old,
                _ => String::new(),
            };
            (kind, extra)
        }
    };
    Ok(CustomItem {
        print_z,
        kind,
        extruder,
        color,
        extra,
        gcode: gcode.unwrap_or_default(),
    })
}

/// The file's text, as the reference writes it: Boost's XML writer with a
/// new line put between every two tags, numbers to 17 digits, values
/// escaped as Boost escapes them (new lines kept). `plates` is each plate's
/// number and items, in order.
pub(crate) fn text<'a>(plates: impl IntoIterator<Item = (u32, &'a CustomGcode)>) -> String {
    let mut out =
        String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<custom_gcodes_per_layer>\n");
    for (id, info) in plates {
        let _ = write!(out, "<plate>\n<plate_info id=\"{id}\"/>\n");
        for item in &info.items {
            let _ = writeln!(
                out,
                "<layer top_z=\"{}\" type=\"{}\" extruder=\"{}\" color=\"{}\" extra=\"{}\" gcode=\"{}\"/>",
                g17(item.print_z),
                item.kind.number(),
                item.extruder,
                boost_escape(&item.color),
                boost_escape(&item.extra),
                boost_escape(&item.gcode),
            );
        }
        // The reference writes an undefined mode as `MultiExtruder`.
        let mode = info.mode.unwrap_or(CustomMode::MultiExtruder);
        let _ = write!(out, "<mode value=\"{}\"/>\n</plate>\n", mode.name());
    }
    out.push_str("</custom_gcodes_per_layer>");
    out
}

/// A value as Boost's XML writer gives it (`encode_char_entities`): the
/// five entities, and a value of spaces only with its first written
/// `&#32;`. A new line is written as it is.
fn boost_escape(value: &str) -> String {
    if !value.is_empty() && value.bytes().all(|byte| byte == b' ') {
        return format!("&#32;{}", &value[1..]);
    }
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// A number as C++'s stream writes it at 17 significant digits, the
/// precision Boost gives a property tree's `double` (`%.17g`).
pub(crate) fn g17(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_nan() {
            "nan".into()
        } else if value > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.into();
    }
    // 17 significant digits: one before the point, sixteen after.
    let scientific = format!("{value:.16e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let (sign, digits) = match mantissa.strip_prefix('-') {
        Some(rest) => ("-", rest.replace('.', "")),
        None => ("", mantissa.replace('.', "")),
    };
    if !(-4..17).contains(&exponent) {
        // Exponential, trailing zeros of the mantissa dropped.
        let (first, rest) = digits.split_at(1);
        let rest = rest.trim_end_matches('0');
        let point = if rest.is_empty() { "" } else { "." };
        let (esign, e) = if exponent < 0 {
            ("-", -exponent)
        } else {
            ("+", exponent)
        };
        return format!("{sign}{first}{point}{rest}e{esign}{e:02}");
    }
    let text = if exponent >= 0 {
        let whole = (exponent + 1) as usize;
        let (int, frac) = digits.split_at(whole.min(digits.len()));
        format!("{int}.{frac}")
    } else {
        format!("0.{}{}", "0".repeat((-exponent - 1) as usize), digits)
    };
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    format!("{sign}{text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_written_as_cpp_writes_them_at_17_digits() {
        assert_eq!(g17(0.25), "0.25");
        assert_eq!(g17(0.45), "0.45000000000000001");
        assert_eq!(g17(0.65), "0.65000000000000002");
        assert_eq!(g17(0.65 + 0.2), "0.85000000000000009");
        assert_eq!(g17(1.0), "1");
        assert_eq!(g17(12.0), "12");
        assert_eq!(g17(-2.5), "-2.5");
        assert_eq!(g17(1e-5), "1.0000000000000001e-05");
        assert_eq!(g17(0.0001), "0.0001");
        assert_eq!(g17(1e17), "1e+17");
    }

    #[test]
    fn plates_and_both_layouts_are_read() {
        let text = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<custom_gcodes_per_layer>\n<plate>\n\
                    <plate_info id=\"2\"/>\n<layer top_z=\"1.05\" type=\"1\" extruder=\"1\" color=\"\" extra=\"\" gcode=\"M600\"/>\n\
                    <layer top_z=\"2\" type=\"4\" extruder=\"1\" color=\"\" extra=\"M117 a&amp;b\nG4 P1\" gcode=\"x\"/>\n\
                    <mode value=\"MultiAsSingle\"/>\n</plate>\n</custom_gcodes_per_layer>";
        let plates = read(text, "x").unwrap().unwrap();
        let two = &plates[&2];
        assert_eq!(two.mode, Some(CustomMode::MultiAsSingle));
        assert_eq!(two.items.len(), 2);
        assert_eq!(two.items[0].kind, CustomKind::PausePrint);
        assert_eq!(two.items[1].extra, "M117 a&b\nG4 P1");
        // Written again, read again: the same.
        let again = text_of(&plates);
        assert_eq!(read(&again, "x").unwrap().unwrap(), plates);
        assert!(again.contains("extra=\"M117 a&amp;b\nG4 P1\""));

        // No plates: plate 1's. The old layout says what it is by its G-code.
        let old = "<custom_gcodes_per_layer><layer top_z=\"0.6\" extruder=\"1\" color=\"Stop\" gcode=\"M601\"/></custom_gcodes_per_layer>";
        let plates = read(old, "x").unwrap().unwrap();
        assert_eq!(plates[&1].items[0].kind, CustomKind::PausePrint);
        assert_eq!(plates[&1].items[0].extra, "Stop");
        assert_eq!(plates[&1].mode(), CustomMode::SingleExtruder);

        // Another layout is not read.
        assert!(read("<custom_gcodes_per_print_z/>", "x").unwrap().is_none());
    }

    fn text_of(plates: &BTreeMap<u32, CustomGcode>) -> String {
        text(plates.iter().map(|(id, info)| (*id, info)))
    }
}
