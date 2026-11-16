//! Embossed text as a project describes it on its part
//! (`Metadata/model_settings.config`): the reference's
//! `<slic3rpe:shape .../>`, how the shape was raised into a solid
//! (EmbossShape.hpp `EmbossShape`, `EmbossProjection`), and
//! `<slic3rpe:text .../>`, what the text and its style are
//! (TextConfiguration.hpp `TextConfiguration`, `EmbossStyle`, `FontProp`).
//! Read and written as bbs_3mf.cpp `TextConfigurationSerialization` and
//! `to_xml(EmbossShape)` / `read_emboss_shape` do.

use std::fmt::Write as _;

use edge_mesh::Transform;
use quick_xml::{Reader, events::Event};

use crate::{kept::Entry, xml::attributes};

/// Text of a part: what it says and how it was made.
#[derive(Clone, Debug, PartialEq)]
pub struct TextVolume {
    pub text: TextConfiguration,
    pub shape: EmbossShape,
}

/// `FontProp::HorizontalAlign`, written "left", "center", "right".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HorizontalAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// `FontProp::VerticalAlign`, written "top", "middle", "bottom".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VerticalAlign {
    Top,
    #[default]
    Center,
    Bottom,
}

/// How a style's font is found (`EmbossStyle::Type`), as written in
/// `font_descriptor_type`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DescriptorType {
    #[default]
    Undefined,
    /// "file_name": a font file's name (only the name, for privacy).
    FileName,
    /// "wxFontDescriptor_Windows"
    Windows,
    /// "wxFontDescriptor_Linux"
    Linux,
    /// "wxFontDescriptor_MacOsX"
    Mac,
}

impl DescriptorType {
    pub fn name(self) -> &'static str {
        match self {
            DescriptorType::Undefined => "undefined",
            DescriptorType::FileName => "file_name",
            DescriptorType::Windows => "wxFontDescriptor_Windows",
            DescriptorType::Linux => "wxFontDescriptor_Linux",
            DescriptorType::Mac => "wxFontDescriptor_MacOsX",
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "file_name" => DescriptorType::FileName,
            "wxFontDescriptor_Windows" => DescriptorType::Windows,
            "wxFontDescriptor_Linux" => DescriptorType::Linux,
            "wxFontDescriptor_MacOsX" => DescriptorType::Mac,
            _ => DescriptorType::Undefined,
        }
    }
}

/// `TextConfiguration`: the text and its style.
#[derive(Clone, Debug, PartialEq)]
pub struct TextConfiguration {
    pub text: String,
    /// The text's inline shapes, as the JSON the file holds
    /// (`inline_shapes`), kept to be written back; none drawn here.
    pub inline_shapes: Option<String>,
    /// `EmbossStyle::name`.
    pub style_name: String,
    /// `EmbossStyle::path`: how the font is found, which `descriptor_type`
    /// says the meaning of.
    pub font_descriptor: String,
    pub descriptor_type: DescriptorType,
    /// `FontProp`. Each optional one is written only when set, and read as
    /// set only when not zero, as the reference does.
    pub char_gap: Option<i32>,
    pub line_gap: Option<i32>,
    /// `size_in_mm`, written `line_height`.
    pub size_in_mm: f32,
    pub boldness: Option<f32>,
    pub skew: Option<f32>,
    pub per_glyph: bool,
    pub horizontal: HorizontalAlign,
    pub vertical: VerticalAlign,
    /// Which font of a collection; none for the first.
    pub collection: Option<u32>,
    /// What else finds the font where it is not installed under the
    /// descriptor: the wx family ("swiss"), face name ("Helvetica"),
    /// style ("italic") and weight ("bold").
    pub family: Option<String>,
    pub face_name: Option<String>,
    pub style: Option<String>,
    pub weight: Option<String>,
}

impl Default for TextConfiguration {
    fn default() -> Self {
        Self {
            // TextConfiguration.hpp: `text = "None"`.
            text: "None".into(),
            inline_shapes: None,
            style_name: String::new(),
            font_descriptor: String::new(),
            descriptor_type: DescriptorType::Undefined,
            char_gap: None,
            line_gap: None,
            // `FontProp(float line_height = 10.f)`.
            size_in_mm: 10.0,
            boldness: None,
            skew: None,
            per_glyph: false,
            horizontal: HorizontalAlign::Center,
            vertical: VerticalAlign::Center,
            collection: None,
            family: None,
            face_name: None,
            style: None,
            weight: None,
        }
    }
}

/// `EmbossBend`: text bent along an arc, kept as read (the bend itself is
/// not made here).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EmbossBend {
    /// The arc's span in degrees (`bend_angle`), or its radius in mm
    /// (`bend_radius`), whichever the file gives; the radius wins.
    pub angle: Option<f32>,
    pub radius: Option<f32>,
    pub inside: bool,
    pub rigid: bool,
}

/// `EmbossShape` with its `EmbossProjection`, as the file holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct EmbossShape {
    /// mm per shape unit (`scale`).
    pub scale: f64,
    /// Whether the shape could not be healed (`unhealed`).
    pub unhealed: bool,
    /// How deep the text is raised, in mm (`depth`).
    pub depth: f64,
    /// Whether the text follows the surface it was cut from
    /// (`use_surface`).
    pub use_surface: bool,
    pub bend: Option<EmbossBend>,
    /// `transform`: what puts the text's own frame back where it was,
    /// from the part's place once the reference has centred its mesh
    /// (EmbossShape.hpp `fix_3mf_tr`).
    pub fix: Option<Transform>,
    /// An SVG's `filepath` and `filepath3mf`, for an embossed picture.
    pub svg_path: Option<String>,
    pub svg_path_in_3mf: Option<String>,
}

impl Default for EmbossShape {
    fn default() -> Self {
        Self {
            scale: 1e-6,
            unhealed: false,
            depth: 1.0,
            use_surface: false,
            bend: None,
            fix: None,
            svg_path: None,
            svg_path_in_3mf: None,
        }
    }
}

/// Whether a part's entry is its shape's or its text's description.
pub(crate) fn is_text_entry(raw: &str) -> bool {
    raw.starts_with("<slic3rpe:text") || raw.starts_with("<slic3rpe:shape")
}

/// The attributes of one element written as text, by name.
fn element_attributes(raw: &str) -> Option<Vec<(String, String)>> {
    let mut reader = Reader::from_str(raw);
    loop {
        match reader.read_event().ok()? {
            Event::Start(element) | Event::Empty(element) => {
                let mut found = Vec::new();
                attributes(&element, "model_settings", |key, value| {
                    found.push((key.to_owned(), value.to_owned()));
                    Ok(())
                })
                .ok()?;
                return Some(found);
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// `xml_escape_double_quotes_attribute_value`: what the reference escapes
/// in these attributes, as it escapes it.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '\r' => out.push_str("&#xD;"),
            '\n' => out.push_str("&#xA;"),
            '\t' => out.push_str("&#x9;"),
            c => out.push(c),
        }
    }
    out
}

/// `bbs_get_attribute_value_int`: `atoi` of the value, 0 when missing.
fn int_of(value: Option<&str>) -> i64 {
    let Some(text) = value else { return 0 };
    let text = text.trim_start();
    let end = text
        .char_indices()
        .find(|&(at, c)| !(c.is_ascii_digit() || (at == 0 && (c == '-' || c == '+'))))
        .map_or(text.len(), |(at, _)| at);
    text[..end].parse().unwrap_or(0)
}

/// `bbs_get_attribute_value_float`: `atof`'s reading, 0 when missing.
fn float_of(value: Option<&str>) -> f64 {
    let Some(text) = value else { return 0.0 };
    let text = text.trim();
    // The longest start of it that reads as a number.
    (1..=text.len())
        .rev()
        .filter(|end| text.is_char_boundary(*end))
        .find_map(|end| text[..end].parse::<f64>().ok())
        .unwrap_or(0.0)
}

impl TextVolume {
    /// The text a part's entries describe, when they hold its
    /// `slic3rpe:text` (with its `slic3rpe:shape`, or a file of before
    /// shapes were written, whose text carries the projection).
    pub fn from_entries(entries: &[Entry]) -> Option<TextVolume> {
        let raw_of = |prefix: &str| {
            entries.iter().find_map(|entry| match entry {
                Entry::Raw(raw) if raw.starts_with(prefix) => Some(raw.as_str()),
                _ => None,
            })
        };
        let text = element_attributes(raw_of("<slic3rpe:text")?)?;
        let shape = raw_of("<slic3rpe:shape").and_then(element_attributes);
        let text_config = TextConfiguration::read(&text);
        let shape = match shape {
            Some(attributes) => EmbossShape::read(&attributes),
            // `read_old`: the projection was on the text.
            None => EmbossShape::read_old(&text),
        };
        Some(TextVolume {
            text: text_config,
            shape,
        })
    }

    /// The part's two entries for it, shape first, as the reference writes
    /// them (bbs_3mf.cpp: the shape after the part's metadata, the text
    /// after it). `mesh_centre` is the middle of the box round the part's
    /// mesh as it is written, which the shape's `transform` puts back.
    pub fn entries(&self, mesh_centre: [f64; 3]) -> [Entry; 2] {
        [
            Entry::Raw(self.shape.written(mesh_centre)),
            Entry::Raw(self.text.written()),
        ]
    }
}

fn get<'a>(attributes: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// An alignment as written: its name, or (PrusaSlicer 2.6.1) its number;
/// none or anything else is the middle.
fn align_index(value: Option<&str>, names: [&str; 3]) -> usize {
    match value {
        Some(text) if text.len() == 1 && text.as_bytes()[0].is_ascii_digit() => {
            (text.as_bytes()[0] - b'0') as usize
        }
        Some(text) => names.iter().position(|name| *name == text).unwrap_or(1),
        None => 1,
    }
}

impl TextConfiguration {
    /// `TextConfigurationSerialization::read`.
    fn read(a: &[(String, String)]) -> TextConfiguration {
        let text_of = |name: &str| get(a, name).filter(|v| !v.is_empty()).map(str::to_owned);
        let char_gap = int_of(get(a, "char_gap"));
        let line_gap = int_of(get(a, "line_gap"));
        let boldness = float_of(get(a, "boldness")) as f32;
        let skew = float_of(get(a, "skew")) as f32;
        let collection = int_of(get(a, "collection"));
        let horizontal = match align_index(get(a, "horizontal"), ["left", "center", "right"]) {
            0 => HorizontalAlign::Left,
            2 => HorizontalAlign::Right,
            _ => HorizontalAlign::Center,
        };
        let vertical = match align_index(get(a, "vertical"), ["top", "middle", "bottom"]) {
            0 => VerticalAlign::Top,
            2 => VerticalAlign::Bottom,
            _ => VerticalAlign::Center,
        };
        TextConfiguration {
            text: get(a, "text").unwrap_or_default().to_owned(),
            inline_shapes: text_of("inline_shapes"),
            style_name: get(a, "style_name").unwrap_or_default().to_owned(),
            font_descriptor: get(a, "font_descriptor").unwrap_or_default().to_owned(),
            descriptor_type: DescriptorType::from_name(
                get(a, "font_descriptor_type").unwrap_or_default(),
            ),
            char_gap: (char_gap != 0).then_some(char_gap as i32),
            line_gap: (line_gap != 0).then_some(line_gap as i32),
            size_in_mm: float_of(get(a, "line_height")) as f32,
            boldness: (boldness.abs() > f32::EPSILON).then_some(boldness),
            skew: (skew.abs() > f32::EPSILON).then_some(skew),
            per_glyph: int_of(get(a, "per_glyph")) == 1,
            horizontal,
            vertical,
            collection: (collection > 0).then_some(collection as u32),
            family: text_of("family"),
            face_name: text_of("face_name"),
            style: text_of("style"),
            weight: text_of("weight"),
        }
    }

    /// `TextConfigurationSerialization::to_xml`, inline shapes written back
    /// as they were read.
    fn written(&self) -> String {
        let mut s = String::from("<slic3rpe:text ");
        let _ = write!(s, "text=\"{}\" ", escape(&self.text));
        if let Some(json) = &self.inline_shapes {
            let _ = write!(s, "inline_shapes=\"{}\" ", escape(json));
        }
        let _ = write!(s, "style_name=\"{}\" ", escape(&self.style_name));
        let _ = write!(s, "font_descriptor=\"{}\" ", escape(&self.font_descriptor));
        let _ = write!(
            s,
            "font_descriptor_type=\"{}\" ",
            self.descriptor_type.name()
        );
        if let Some(gap) = self.char_gap {
            let _ = write!(s, "char_gap=\"{gap}\" ");
        }
        if let Some(gap) = self.line_gap {
            let _ = write!(s, "line_gap=\"{gap}\" ");
        }
        let _ = write!(s, "line_height=\"{}\" ", g17(f64::from(self.size_in_mm)));
        if let Some(boldness) = self.boldness {
            let _ = write!(s, "boldness=\"{}\" ", g17(f64::from(boldness)));
        }
        if let Some(skew) = self.skew {
            let _ = write!(s, "skew=\"{}\" ", g17(f64::from(skew)));
        }
        if self.per_glyph {
            s.push_str("per_glyph=\"1\" ");
        }
        let horizontal = match self.horizontal {
            HorizontalAlign::Left => "left",
            HorizontalAlign::Center => "center",
            HorizontalAlign::Right => "right",
        };
        let vertical = match self.vertical {
            VerticalAlign::Top => "top",
            VerticalAlign::Center => "middle",
            VerticalAlign::Bottom => "bottom",
        };
        let _ = write!(s, "horizontal=\"{horizontal}\" vertical=\"{vertical}\" ");
        if let Some(collection) = self.collection {
            let _ = write!(s, "collection=\"{collection}\" ");
        }
        // Written unescaped by the reference: wx names have no quotes.
        for (name, value) in [
            ("family", &self.family),
            ("face_name", &self.face_name),
            ("style", &self.style),
            ("weight", &self.weight),
        ] {
            if let Some(value) = value {
                let _ = write!(s, "{name}=\"{}\" ", escape(value));
            }
        }
        s.push_str("/>");
        s
    }
}

impl EmbossShape {
    /// `read_emboss_shape`.
    fn read(a: &[(String, String)]) -> EmbossShape {
        let mut depth = float_of(get(a, "depth"));
        if depth.abs() < 1e-9 {
            depth = 10.0;
        }
        let radius = float_of(get(a, "bend_radius")) as f32;
        let angle = float_of(get(a, "bend_angle")) as f32;
        let bend = if radius.is_finite() && radius > 0.0 {
            Some(EmbossBend {
                angle: None,
                radius: Some(radius),
                inside: false,
                rigid: false,
            })
        } else if angle.is_finite() && angle > 0.0 {
            Some(EmbossBend {
                angle: Some(angle.min(359.0)),
                radius: None,
                inside: false,
                rigid: false,
            })
        } else {
            None
        }
        .map(|bend| EmbossBend {
            inside: int_of(get(a, "bend_inside")) == 1,
            rigid: int_of(get(a, "bend_rigid")) == 1,
            ..bend
        });
        let text_of = |name: &str| get(a, name).filter(|v| !v.is_empty()).map(str::to_owned);
        EmbossShape {
            scale: float_of(get(a, "scale")),
            unhealed: int_of(get(a, "unhealed")) == 1,
            depth,
            use_surface: int_of(get(a, "use_surface")) == 1,
            bend,
            fix: get(a, "transform").and_then(fix_of),
            svg_path: text_of("filepath"),
            svg_path_in_3mf: text_of("filepath3mf"),
        }
    }

    /// `TextConfigurationSerialization::read_old`: a text of before shapes
    /// were written carries the projection itself.
    fn read_old(a: &[(String, String)]) -> EmbossShape {
        EmbossShape {
            fix: get(a, "transform").and_then(fix_of),
            use_surface: int_of(get(a, "use_surface")) == 1,
            depth: float_of(get(a, "depth")),
            ..EmbossShape::default()
        }
    }

    /// `to_xml(EmbossShape)`. Its `transform` is the shape's own `fix`
    /// when the file gave one and the mesh is still centred as the file's
    /// was (`create_fix`: the move to the middle of the mesh's box is
    /// nothing then), else the move to the middle of the mesh's box after
    /// it (`prev * fix_trmat`; just the move when there is no earlier one).
    fn written(&self, centre: [f64; 3]) -> String {
        let mut s = String::from("<slic3rpe:shape ");
        if let Some(in_3mf) = &self.svg_path_in_3mf {
            if let Some(path) = &self.svg_path {
                let _ = write!(s, "filepath=\"{}\" ", escape(path));
            }
            let _ = write!(s, "filepath3mf=\"{}\" ", escape(in_3mf));
        }
        let _ = write!(s, "scale=\"{}\" ", g17(self.scale));
        if self.unhealed {
            s.push_str("unhealed=\"1\" ");
        }
        let _ = write!(s, "depth=\"{}\" ", g17(self.depth));
        if self.use_surface {
            s.push_str("use_surface=\"1\" ");
        }
        if let Some(bend) = &self.bend {
            match (bend.radius, bend.angle) {
                (Some(radius), _) => {
                    let _ = write!(s, "bend_radius=\"{}\" ", g17(f64::from(radius)));
                }
                (None, Some(angle)) => {
                    let _ = write!(s, "bend_angle=\"{}\" ", g17(f64::from(angle)));
                }
                (None, None) => {}
            }
            if bend.inside {
                s.push_str("bend_inside=\"1\" ");
            }
            if bend.rigid {
                s.push_str("bend_rigid=\"1\" ");
            }
        }
        let fix = Transform::translation(centre);
        let identity = centre.iter().all(|value| value.abs() <= 1e-5);
        let fix = match self.fix {
            Some(prev) if identity => prev,
            Some(prev) => fix.then(&prev),
            None => fix,
        };
        let numbers: Vec<String> = fix.rows.iter().flatten().map(|value| g17(*value)).collect();
        let _ = write!(s, "transform=\"{}\" ", numbers.join(" "));
        s.push_str("/>");
        s
    }
}

/// `bbs_get_transform_from_3mf_specs_string`: twelve numbers, or none.
fn fix_of(text: &str) -> Option<Transform> {
    let numbers: Vec<f64> = text
        .split(' ')
        .filter(|word| !word.is_empty())
        .map(|word| float_of(Some(word)))
        .collect();
    if numbers.len() != 12 {
        return None;
    }
    let mut rows = [[0.0; 3]; 4];
    for (at, value) in numbers.into_iter().enumerate() {
        rows[at / 3][at % 3] = value;
    }
    Some(Transform { rows })
}

/// A number as the reference writes these (bbs_3mf.cpp sets its stream to
/// `max_digits10` of a double): C's `%.17g`, seventeen significant digits,
/// trailing zeros dropped, in exponent form below 1e-4 and from 1e17. A
/// float is written as the double it widens to.
pub fn g17(value: f64) -> String {
    const PRECISION: i32 = 17;
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.into();
    }
    if !value.is_finite() {
        return if value.is_nan() {
            "nan".into()
        } else if value > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    let exponent = format!("{value:.*e}", (PRECISION - 1) as usize);
    let (mantissa, power) = exponent.split_once('e').unwrap_or((&exponent, "0"));
    let power: i32 = power.parse().unwrap_or(0);
    let trim = |text: String| -> String {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            text
        }
    };
    if !(-4..PRECISION).contains(&power) {
        let mantissa = trim(mantissa.to_owned());
        let sign = if power < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", power.abs())
    } else {
        let decimals = (PRECISION - 1 - power).max(0) as usize;
        trim(format!("{value:.decimals$}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_written_as_the_reference_writes_them() {
        // As the reference wrote them back (`--export-3mf` of a project
        // saved here).
        assert_eq!(g17(10.0), "10");
        assert_eq!(g17(3.4179699923697626e-06), "3.4179699923697626e-06");
        assert_eq!(g17(-0.0518386), "-0.051838599999999999");
        assert_eq!(g17(-0.485), "-0.48499999999999999");
        assert_eq!(g17(0.0001), "0.0001");
        assert_eq!(g17(f64::from(0.1_f32)), "0.10000000149011612");
        assert_eq!(g17(-2.5), "-2.5");
        assert_eq!(g17(1e17), "1e+17");
    }

    #[test]
    fn a_text_and_its_shape_read_back_as_written() {
        let volume = TextVolume {
            text: TextConfiguration {
                text: "Ab \"c\"\nd".into(),
                style_name: "NORMAL".into(),
                font_descriptor: "Helvetica".into(),
                descriptor_type: DescriptorType::Mac,
                char_gap: Some(20),
                boldness: Some(0.5),
                horizontal: HorizontalAlign::Left,
                vertical: VerticalAlign::Bottom,
                face_name: Some("Helvetica".into()),
                weight: Some("bold".into()),
                ..TextConfiguration::default()
            },
            shape: EmbossShape {
                scale: 1e-5,
                depth: 2.5,
                use_surface: true,
                ..EmbossShape::default()
            },
        };
        let entries = volume.entries([1.0, 2.0, 0.5]);
        let back = TextVolume::from_entries(&entries).unwrap();
        assert_eq!(back.text, volume.text);
        assert_eq!(back.shape.depth, 2.5);
        assert!(back.shape.use_surface);
        assert_eq!(
            back.shape.fix,
            Some(Transform::translation([1.0, 2.0, 0.5]))
        );
        // Written again with the mesh still centred, the fix stays.
        let again = back.entries([0.0; 3]);
        assert_eq!(again[0], entries[0]);
    }

    #[test]
    fn old_files_and_missing_values_read_as_the_reference_reads_them() {
        let entries = [Entry::Raw(
            r#"<slic3rpe:text text="x" font_descriptor="a.ttf" font_descriptor_type="file_name" line_height="8" horizontal="0" vertical="2" depth="3" use_surface="1"/>"#.into(),
        )];
        let volume = TextVolume::from_entries(&entries).unwrap();
        assert_eq!(volume.text.horizontal, HorizontalAlign::Left);
        assert_eq!(volume.text.vertical, VerticalAlign::Bottom);
        assert_eq!(volume.text.descriptor_type, DescriptorType::FileName);
        assert_eq!(volume.shape.depth, 3.0);
        assert!(volume.shape.use_surface);
        let shape = [
            Entry::Raw(r#"<slic3rpe:shape scale="1e-06"/>"#.into()),
            entries[0].clone(),
        ];
        // A shape without a depth is ten deep (`read_emboss_shape`).
        assert_eq!(TextVolume::from_entries(&shape).unwrap().shape.depth, 10.0);
    }
}
