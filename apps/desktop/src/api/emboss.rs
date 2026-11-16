//! The Text tool ("Emboss" [T], Gizmos/GLGizmoEmboss.cpp): the fonts it
//! offers, a text part's description for the page to edit, and the edits
//! that add a text and make it again (`edge_engine::emboss`).
//!
//! Fonts. The reference lists the fonts the system has (wxWidgets'
//! `wxFontEnumerator`, Facenames in GLGizmoEmboss.cpp) and, on Linux and
//! Windows, the fonts it bundles (Label.cpp `initSysFont` registers
//! HarmonyOS Sans SC and NanumGothic as private fonts; not on macOS);
//! a symbol a font lacks comes from its bundled
//! `NotoSansSymbols2-Subset.ttf`. Here:
//! - the system's fonts are read from the platform's standard font
//!   folders only ([`system_dirs`]), never from the network. Each file is
//!   untrusted input: only its table directory and a few small tables are
//!   read to list it (`edge_text::scan_file`), a file over 64 MB is not
//!   read, a font whose tables do not hold together is left out and
//!   counted, never a crash. What `ttf-parser` cannot draw is left out:
//!   CFF2 (variable OpenType) outlines, bitmap and colour fonts. Families
//!   whose name starts with a dot (macOS's hidden system faces) are not
//!   listed, as the system's own list leaves them out;
//! - of the bundled fonts, only those with a licence beside them in the
//!   reference's `resources/fonts` are offered: NanumGothic (`OFL.txt`),
//!   and the symbol fallback (`OFL-NotoSansSymbols2.txt`). HarmonyOS Sans
//!   SC and Noto Sans KR have no licence file there and are not offered;
//! - the page is given names and shapes, never a file's place.
//!
//! The style. The reference's first style is "NORMAL": wx's normal font,
//! Helvetica on macOS where it is installed, at its point size taken as
//! mm (13 on macOS), one mm deep (GLGizmoEmboss.cpp
//! `create_default_styles`, WxFontUtils.cpp `create_emboss_style`). The
//! style list itself (saving, renaming, other styles) is not here yet.
//!
//! How a font is written in a project: on macOS as the reference does,
//! a wx font descriptor (`wxFontDescriptor_MacOsX`, wx's version 2: the
//! font's PostScript name and size as a property list) and the face name,
//! style and weight wx would give it; elsewhere, and for a bundled font,
//! only the file's name (`file_name`). A project's text is matched to a
//! font here by that descriptor's PostScript name, then the face name
//! with its weight and style, then the file's name.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use edge_engine::{
    PartKind, TextConfiguration, Transform,
    emboss::{self, TextFonts},
};
use edge_text::{FaceInfo, Font};
use serde_json::{Value, json};

use super::{File, Session, edit::push_step, i18n::tr, lock, needed, text, whole};

/// The calls answered here.
pub(super) const JSON: [&str; 4] = ["text_fonts", "text_font_preview", "text_part", "text_edit"];

/// The default text (GLGizmoEmboss.cpp `init_create`).
const DEFAULT_TEXT: &str = "Embossed text";

/// The default style's name (`create_default_styles`).
const DEFAULT_STYLE: &str = "NORMAL";

/// The point size of wx's normal font, which the default style takes as
/// its height in mm: the system font's 13 points on macOS. Elsewhere not
/// measured: wx's normal font is 9 points on Windows, 10 on most Linux.
#[cfg(target_os = "macos")]
const DEFAULT_SIZE: f32 = 13.0;
#[cfg(target_os = "windows")]
const DEFAULT_SIZE: f32 = 9.0;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const DEFAULT_SIZE: f32 = 10.0;

/// The bundled fonts offered, each with the licence that must be beside
/// it in the reference's resources.
const BUNDLED: [(&str, &str); 2] = [
    ("NanumGothic-Regular.ttf", "OFL.txt"),
    ("NanumGothic-Bold.ttf", "OFL.txt"),
];

/// The symbol fallback and its licence (FontFallback.cpp
/// `bundled_symbol_font_path`).
const FALLBACK: (&str, &str) = ("NotoSansSymbols2-Subset.ttf", "OFL-NotoSansSymbols2.txt");

/// How deep the font folders are walked.
const MAX_DEPTH: usize = 6;

/// One font of the list.
#[derive(Clone, Debug)]
pub(super) struct Face {
    pub info: FaceInfo,
    path: PathBuf,
    index: u32,
    bundled: bool,
}

impl Face {
    fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// The fonts found, and what was left out.
#[derive(Debug, Default)]
pub(super) struct FontList {
    pub faces: Vec<Face>,
    /// Why each font left out was, for the trace and the count shown.
    pub skipped: Vec<String>,
}

/// The fonts the session has found and read.
#[derive(Default)]
pub(super) struct Fonts {
    list: Option<Arc<FontList>>,
    /// The folders read in place of the system's (tests).
    dirs: Option<Vec<PathBuf>>,
    /// Fonts read whole, by their place in the list.
    read: HashMap<usize, Arc<Font>>,
    fallback: Option<Option<Arc<Font>>>,
    /// The surface the last text that follows it was cut from, and what it
    /// was made of: made again only when the other parts or the text's
    /// frame change, not on each keystroke.
    surface: Option<(emboss::SurfaceKey, Arc<emboss::Surface>)>,
}

/// The platform's standard font folders. Only macOS's are checked; the
/// Windows and Linux ones are written from the platforms' documentation
/// and are untested.
pub(super) fn system_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = &home {
            dirs.push(home.join("Library/Fonts"));
        }
    } else if cfg!(target_os = "windows") {
        if let Some(windir) = std::env::var_os("WINDIR") {
            dirs.push(PathBuf::from(windir).join("Fonts"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(
                PathBuf::from(local)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
    } else {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(home) = &home {
            dirs.push(home.join(".local/share/fonts"));
            dirs.push(home.join(".fonts"));
        }
    }
    dirs
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

/// Every font file under `dir`, at most [`MAX_DEPTH`] folders down, in a
/// fixed order; links are not followed.
fn font_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<_> = entries.flatten().collect();
    found.sort_by_key(|entry| entry.file_name());
    for entry in found {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() && depth < MAX_DEPTH {
            font_files(&path, depth + 1, out);
        } else if kind.is_file() && is_font_file(&path) {
            out.push(path);
        }
    }
}

/// The fonts in `dirs` (the system's) and the bundled ones in `bundled`
/// (the reference's `resources/fonts`), sorted by family, weight and
/// slant.
pub(super) fn find_fonts(dirs: &[PathBuf], bundled: Option<&Path>) -> FontList {
    let mut list = FontList::default();
    let add = |path: &Path, is_bundled: bool, list: &mut FontList| match edge_text::scan_file(
        path,
        &mut list.skipped,
    ) {
        Ok(faces) => list.faces.extend(faces.into_iter().filter_map(|face| {
            (!face.info.family.starts_with('.')).then(|| Face {
                info: face.info,
                path: path.to_owned(),
                index: face.index,
                bundled: is_bundled,
            })
        })),
        Err(why) => list.skipped.push(why.to_string()),
    };
    if let Some(dir) = bundled {
        for (file, licence) in BUNDLED {
            if dir.join(licence).is_file() {
                add(&dir.join(file), true, &mut list);
            }
        }
    }
    let mut files = Vec::new();
    for dir in dirs {
        font_files(dir, 0, &mut files);
    }
    for file in files {
        add(&file, false, &mut list);
    }
    list.faces.sort_by(|a, b| {
        (
            a.info.family.to_lowercase(),
            a.bundled,
            a.info.italic,
            a.info.weight,
            &a.info.style,
        )
            .cmp(&(
                b.info.family.to_lowercase(),
                b.bundled,
                b.info.italic,
                b.info.weight,
                &b.info.style,
            ))
    });
    list
}

/// wx's name of a weight (WxFontUtils.cpp `type_to_weight`), none for the
/// normal one, which is not written.
fn wx_weight(weight: u16) -> Option<&'static str> {
    Some(match weight {
        0..=149 => "thin",
        150..=249 => "extraLight",
        250..=349 => "light",
        350..=449 => return None,
        450..=549 => "medium",
        550..=649 => "semibold",
        650..=749 => "bold",
        750..=849 => "extraBold",
        850..=949 => "heavy",
        _ => "extraHeavy",
    })
}

/// The weight a wx name stands for.
fn weight_of(name: &str) -> u16 {
    match name {
        "thin" => 100,
        "extraLight" => 200,
        "light" => 300,
        "medium" => 500,
        "semibold" => 600,
        "bold" => 700,
        "extraBold" => 800,
        "heavy" => 900,
        "extraHeavy" => 1000,
        _ => 400,
    }
}

/// wx's version 2 descriptor of a font on macOS (wx `osx/carbon/font.cpp`
/// `wxNativeFontInfo::ToString`): underline, strikethrough, the encoding
/// (`wxFONTENCODING_SYSTEM`, as GLGizmoEmboss.cpp asks for), and the
/// font descriptor's attributes as a property list.
fn mac_descriptor(postscript: &str, size: f32) -> String {
    let escaped = postscript
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "2;0;0;-1;<plist version=\"1.0\">\n<dict>\n<key>NSFontNameAttribute</key>\n<string>{escaped}</string>\n<key>NSFontSizeAttribute</key>\n<real>{}</real>\n</dict>\n</plist>\n",
        edge_engine::g17(f64::from(size))
    )
}

/// The PostScript name a wx descriptor written on macOS names: the
/// property list's `NSFontNameAttribute` (version 2), or the eighth field
/// (versions 0 and 1).
fn descriptor_postscript(descriptor: &str) -> Option<String> {
    if descriptor.starts_with("2;") {
        let at = descriptor.find("<key>NSFontNameAttribute</key>")?;
        let rest = &descriptor[at..];
        let start = rest.find("<string>")? + "<string>".len();
        let end = rest[start..].find("</string>")? + start;
        Some(
            rest[start..end]
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&"),
        )
    } else {
        descriptor
            .split(';')
            .nth(7)
            .map(str::to_owned)
            .filter(|name| !name.is_empty())
    }
}

impl FontList {
    /// The font a project's text was written with, when it is here
    /// (`get_installed_face_name`, `load_wxFont`).
    fn find(&self, config: &TextConfiguration) -> Option<usize> {
        if config.descriptor_type == edge_engine::DescriptorType::Mac
            && let Some(postscript) = descriptor_postscript(&config.font_descriptor)
            && let Some(at) = self
                .faces
                .iter()
                .position(|face| face.info.postscript == postscript)
        {
            return Some(at);
        }
        if let Some(face_name) = &config.face_name {
            let weight = config.weight.as_deref().map_or(400, weight_of);
            let italic = config.style.is_some();
            let best = self
                .faces
                .iter()
                .enumerate()
                .filter(|(_, face)| face.info.family.eq_ignore_ascii_case(face_name))
                .min_by_key(|(_, face)| {
                    (
                        face.info.italic != italic,
                        face.info.weight.abs_diff(weight),
                        face.bundled,
                    )
                });
            if let Some((at, _)) = best {
                return Some(at);
            }
        }
        if config.descriptor_type == edge_engine::DescriptorType::FileName {
            let name = Path::new(&config.font_descriptor)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            return self.faces.iter().position(|face| face.file_name() == name);
        }
        None
    }

    /// The default style's font (`create_default_styles`): Helvetica on
    /// macOS, else the first family; its regular face.
    fn default_face(&self) -> Option<usize> {
        let regular = |family: &str| {
            self.faces
                .iter()
                .enumerate()
                .filter(|(_, face)| face.info.family == family && !face.info.italic)
                .min_by_key(|(_, face)| face.info.weight.abs_diff(400))
                .map(|(at, _)| at)
        };
        cfg!(target_os = "macos")
            .then(|| regular("Helvetica"))
            .flatten()
            .or_else(|| {
                let first = self.faces.iter().find(|face| !face.bundled)?;
                regular(&first.info.family.clone())
            })
            .or_else(|| (!self.faces.is_empty()).then_some(0))
    }
}

/// A style as the page sends it, made into a project's description of
/// the text, keeping what `kept` (the text it edits) had that the page
/// does not set.
fn configuration(
    style: &Value,
    face: &Face,
    kept: Option<&TextConfiguration>,
) -> Result<TextConfiguration, String> {
    let number = |key: &str| style.get(key).and_then(Value::as_f64);
    let size = number("size").ok_or("size was not given")? as f32;
    let optional_int = |key: &str| {
        number(key)
            .map(|value| value.round() as i32)
            .filter(|value| *value != 0)
    };
    let optional_float = |key: &str| {
        number(key)
            .map(|value| value as f32)
            .filter(|value| value.abs() > f32::EPSILON)
    };
    let mut config = kept.cloned().unwrap_or_else(|| TextConfiguration {
        style_name: DEFAULT_STYLE.into(),
        ..TextConfiguration::default()
    });
    config.text = style
        .get("text")
        .and_then(Value::as_str)
        .ok_or("text was not given")?
        .to_owned();
    config.size_in_mm = size;
    // The limits of GLGizmoEmboss.cpp `limits`.
    config.char_gap = optional_int("char_gap").map(|gap| gap.clamp(-20000, 20000));
    config.line_gap = optional_int("line_gap").map(|gap| gap.clamp(-20000, 20000));
    config.boldness = optional_float("boldness").map(|value| value.clamp(-5e5, 5e5));
    config.skew = optional_float("skew").map(|value| value.clamp(-100.0, 100.0));
    config.horizontal = match style.get("horizontal").and_then(Value::as_str) {
        Some("left") => edge_engine::HorizontalAlign::Left,
        Some("right") => edge_engine::HorizontalAlign::Right,
        _ => edge_engine::HorizontalAlign::Center,
    };
    config.vertical = match style.get("vertical").and_then(Value::as_str) {
        Some("top") => edge_engine::VerticalAlign::Top,
        Some("bottom") => edge_engine::VerticalAlign::Bottom,
        _ => edge_engine::VerticalAlign::Center,
    };
    config.per_glyph = style.get("per_glyph").and_then(Value::as_bool) == Some(true);
    config.collection = None;
    config.face_name = Some(face.info.family.clone());
    config.style = face.info.italic.then(|| "italic".to_owned());
    config.weight = wx_weight(face.info.weight).map(str::to_owned);
    config.family = None;
    if cfg!(target_os = "macos") && !face.bundled {
        config.descriptor_type = edge_engine::DescriptorType::Mac;
        config.font_descriptor = mac_descriptor(&face.info.postscript, size);
    } else {
        // Only the file's name, as the reference stores a file's path
        // ("for privacy: only filename is stored into .3mf").
        config.descriptor_type = edge_engine::DescriptorType::FileName;
        config.font_descriptor = face.file_name();
    }
    Ok(config)
}

/// A style of the page made from a description, for the panel.
fn style_of(config: &TextConfiguration, depth: f64, use_surface: bool) -> Value {
    json!({
        "text": config.text,
        "size": config.size_in_mm,
        "depth": depth,
        "char_gap": config.char_gap.unwrap_or(0),
        "line_gap": config.line_gap.unwrap_or(0),
        "boldness": config.boldness.unwrap_or(0.0),
        "skew": config.skew.unwrap_or(0.0),
        "horizontal": match config.horizontal {
            edge_engine::HorizontalAlign::Left => "left",
            edge_engine::HorizontalAlign::Center => "center",
            edge_engine::HorizontalAlign::Right => "right",
        },
        "vertical": match config.vertical {
            edge_engine::VerticalAlign::Top => "top",
            edge_engine::VerticalAlign::Center => "middle",
            edge_engine::VerticalAlign::Bottom => "bottom",
        },
        "per_glyph": config.per_glyph,
        "use_surface": use_surface,
    })
}

fn kind_of(name: Option<&str>) -> Result<PartKind, String> {
    Ok(match name {
        None | Some("part") => PartKind::Normal,
        Some("negative") => PartKind::Negative,
        Some("modifier") => PartKind::Modifier,
        Some(other) => return Err(format!("text cannot be a part of the kind {other}")),
    })
}

fn kind_name(kind: PartKind) -> &'static str {
    match kind {
        PartKind::Negative => "negative",
        PartKind::Modifier => "modifier",
        _ => "part",
    }
}

/// Shapes in mm as SVG path data, Y up made Y down, and the box round
/// them: how a font's sample is drawn in the list.
fn path_data(shapes: &[edge_geom::ExPolygon], scale: f64) -> (String, [f64; 4]) {
    use std::fmt::Write as _;
    let mut d = String::new();
    let (mut low, mut high) = ([f64::MAX; 2], [f64::MIN; 2]);
    for region in shapes {
        for ring in region.rings() {
            for (at, p) in ring.points.iter().enumerate() {
                let (x, y) = (p.x as f64 * scale, -(p.y as f64) * scale);
                low = [low[0].min(x), low[1].min(y)];
                high = [high[0].max(x), high[1].max(y)];
                let _ = write!(d, "{}{x:.2} {y:.2}", if at == 0 { "M" } else { "L" });
            }
            d.push('Z');
        }
    }
    if d.is_empty() {
        return (d, [0.0; 4]);
    }
    (d, [low[0], low[1], high[0] - low[0], high[1] - low[1]])
}

impl Session {
    pub(super) fn emboss_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "text_fonts" => self.text_fonts(),
            "text_font_preview" => self.text_font_preview(args),
            "text_part" => self.text_part(args),
            "text_edit" => self.text_edit(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// The reference's `resources/fonts`, beside the profiles.
    fn fonts_dir(&self) -> Option<PathBuf> {
        let state = lock(&self.state);
        state
            .library
            .as_ref()
            .and_then(|library| library.dir().parent().map(|dir| dir.join("fonts")))
    }

    /// The fonts, found once a run (the reference enumerates the system's
    /// when the tool first opens).
    fn font_list(&self) -> Arc<FontList> {
        if let Some(list) = lock(&self.fonts).list.clone() {
            return list;
        }
        let bundled = self.fonts_dir();
        let dirs = lock(&self.fonts).dirs.clone().unwrap_or_else(system_dirs);
        let list = Arc::new(find_fonts(&dirs, bundled.as_deref()));
        if self.traced {
            for why in &list.skipped {
                eprintln!("font left out: {why}");
            }
        }
        lock(&self.fonts).list.get_or_insert(list).clone()
    }

    /// Font `at` of the list, read whole.
    fn font(&self, at: usize) -> Result<Arc<Font>, String> {
        if let Some(font) = lock(&self.fonts).read.get(&at) {
            return Ok(font.clone());
        }
        let list = self.font_list();
        let face = list
            .faces
            .get(at)
            .ok_or_else(|| format!("there is no font {at}"))?;
        let data = edge_text::read_font_file(&face.path).map_err(|e| e.to_string())?;
        let font = Font::from_shared(Arc::new(data), face.index, &face.info.full)
            .map_err(|e| e.to_string())?;
        let font = Arc::new(font);
        lock(&self.fonts).read.insert(at, font.clone());
        Ok(font)
    }

    /// Reads the fonts of `dirs` in place of the system's, from the next
    /// list on.
    #[cfg(test)]
    pub(super) fn use_font_dirs(&self, dirs: Vec<PathBuf>) {
        let mut fonts = lock(&self.fonts);
        *fonts = Fonts::default();
        fonts.dirs = Some(dirs);
    }

    /// The surface a text in `object` (`part` once it is there) standing at
    /// `frame` is cut from: the one kept when it is of the same parts and
    /// frame, else made and kept. None when the object has no other
    /// printed part that bounds a volume.
    fn text_surface(
        &self,
        model: &edge_engine::Model,
        object: u64,
        part: Option<u64>,
        frame: &Transform,
    ) -> Result<Option<Arc<emboss::Surface>>, String> {
        let key = model
            .text_surface_key(object, part, frame)
            .map_err(|e| e.to_string())?;
        if let Some((held, surface)) = &lock(&self.fonts).surface
            && *held == key
        {
            return Ok(Some(surface.clone()));
        }
        let Some(surface) = key.surface().map(Arc::new) else {
            return Ok(None);
        };
        lock(&self.fonts).surface = Some((key, surface.clone()));
        Ok(Some(surface))
    }

    /// The bundled symbol font, when it is there with its licence.
    fn fallback(&self) -> Option<Arc<Font>> {
        if let Some(fallback) = lock(&self.fonts).fallback.clone() {
            return fallback;
        }
        let font = self.fonts_dir().and_then(|dir| {
            dir.join(FALLBACK.1)
                .is_file()
                .then(|| Font::open(&dir.join(FALLBACK.0)).ok().map(Arc::new))
                .flatten()
        });
        lock(&self.fonts).fallback = Some(font.clone());
        font
    }

    /// `text_fonts`: the families, each with its faces (by number, with
    /// their style, weight and slant), which is the default, and how many
    /// fonts were left out.
    fn text_fonts(&self) -> Result<Value, String> {
        let list = self.font_list();
        let mut families: Vec<Value> = Vec::new();
        let mut current: Option<(String, bool)> = None;
        for (at, face) in list.faces.iter().enumerate() {
            let key = (face.info.family.clone(), face.bundled);
            if current.as_ref() != Some(&key) {
                families.push(
                    json!({ "name": face.info.family, "bundled": face.bundled, "faces": [] }),
                );
                current = Some(key);
            }
            if let Some(Value::Array(faces)) = families
                .last_mut()
                .and_then(|family| family.get_mut("faces"))
            {
                faces.push(json!({
                    "id": at,
                    "style": face.info.style,
                    "weight": face.info.weight,
                    "italic": face.info.italic,
                    "ascent": face.info.ascent,
                    "units_per_em": face.info.units_per_em,
                }));
            }
        }
        Ok(json!({
            "families": families,
            "default": list.default_face(),
            "skipped": list.skipped.len(),
            "style": {
                "text": tr(DEFAULT_TEXT),
                "size": DEFAULT_SIZE,
                "depth": 1.0,
                "char_gap": 0, "line_gap": 0, "boldness": 0.0, "skew": 0.0,
                "horizontal": "center", "vertical": "middle",
                "per_glyph": false, "use_surface": false,
            },
        }))
    }

    /// `text_font_preview`: a font's family name (or `text`) drawn in it,
    /// as SVG path data in mm with the box round it.
    fn text_font_preview(&self, args: &Value) -> Result<Value, String> {
        let at = whole(args, "face")? as usize;
        let font = self.font(at)?;
        let list = self.font_list();
        let sample = text(args, "text").unwrap_or(&list.faces[at].info.family);
        // Thirty letters at most, as a sample.
        let sample: String = sample.chars().take(30).collect();
        let mut prop = edge_text::FontProp::new(10.0);
        prop.horizontal = edge_text::HorizontalAlign::Left;
        let shapes =
            edge_text::text_to_shapes(&font, None, &sample, &prop).map_err(|e| e.to_string())?;
        let (d, view) = path_data(&shapes, edge_text::shape_scale(&font, &prop));
        Ok(json!({ "face": at, "d": d, "box": view }))
    }

    /// `text_part`: what a text part is, for the panel: its style, the
    /// font found for it (none when it is not here: the reference's
    /// "unknown font", only the font can be chosen then), its kind, its
    /// turn about its own Z in degrees as the panel shows it (clockwise)
    /// and its distance from the surface, and what of it cannot be made
    /// here yet.
    fn text_part(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        let file = stage.file.as_ref().ok_or("the project has no model")?;
        let (object, part) = (whole(args, "object")?, whole(args, "part")?);
        let model = &file.model;
        let volume = model
            .text_of(object, part)
            .map_err(|e| e.to_string())?
            .ok_or("that part is not text")?;
        let at = model
            .objects()
            .iter()
            .position(|held| held.id() == object)
            .ok_or("there is no such object")?;
        let index = model.objects()[at]
            .shape()
            .part_ids()
            .iter()
            .position(|id| *id == part)
            .ok_or("there is no such part")?;
        let kind = model.objects()[at].shape().parts()[index].kind;
        let frame = model.text_frame(object, part).map_err(|e| e.to_string())?;
        let angle = emboss::calc_up(&frame, emboss::UP_LIMIT).unwrap_or(0.0);
        let copy = model.objects()[at].copies().first().map(|copy| copy.id());
        let distance = copy.and_then(|copy| {
            model
                .text_distance(stage.bed().as_ref(), object, part, copy)
                .ok()
                .flatten()
        });
        let face = self.font_list().find(&volume.text);
        let only_part = model
            .is_only_printed_part(object, part)
            .map_err(|e| e.to_string())?;
        Ok(json!({
            "object": object,
            "part": part,
            "style": style_of(&volume.text, volume.shape.depth, volume.shape.use_surface),
            "face": face,
            "face_name": volume.text.face_name,
            "kind": kind_name(kind),
            "only_part": only_part,
            "angle": -angle.to_degrees(),
            "distance": distance,
            "not_yet": emboss::not_yet(&volume.text, &volume.shape).map(tr),
        }))
    }

    /// `text_edit`: adds a text (`op` "add") or makes one again (`op`
    /// "set"), one step of the history. With `merge`, a change made again
    /// while the tool stays open on the same text is one step with the
    /// change before it, as one editing of the text.
    fn text_edit(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        let bed = stage.bed();
        let op = needed(args, "op")?;
        let style = args.get("style").ok_or("style was not given")?;
        let at = whole(style, "face")? as usize;
        let list = self.font_list();
        let face = list.faces.get(at).ok_or("there is no such font")?.clone();
        let font = self.font(at)?;
        let fallback = self.fallback();
        let fonts = TextFonts {
            font: &font,
            fallback: fallback.as_deref(),
        };
        let depth = style
            .get("depth")
            .and_then(Value::as_f64)
            .ok_or("depth was not given")?
            // `limits.emboss`.
            .clamp(0.01, 1e4);
        let kind = kind_of(text(args, "kind"))?;
        let use_surface = style.get("use_surface").and_then(Value::as_bool) == Some(true);
        let mut model = match &stage.file {
            Some(file) => file.model.clone(),
            None => edge_engine::Model::new(),
        };
        let fail = |error: edge_engine::Failure| tr(&error.to_string());
        let (name, part, made) = match op {
            "add" => {
                let config = configuration(style, &face, None)?;
                let mut text = emboss::emboss(fonts, &config, depth, kind).map_err(fail)?;
                let angle = style
                    .get("angle")
                    .and_then(Value::as_f64)
                    .filter(|angle| *angle != 0.0)
                    .map(|degrees| -degrees.to_radians());
                match args.get("object").and_then(Value::as_u64) {
                    Some(object) => {
                        let copy = whole(args, "copy")?;
                        let hit = match (args.get("origin"), args.get("direction")) {
                            (Some(_), Some(_)) => {
                                let origin = three(args, "origin")?;
                                let direction = three(args, "direction")?;
                                model
                                    .face_hit(bed.as_ref(), copy, origin, direction)
                                    .map_err(|e| e.to_string())?
                                    .map(|hit| (hit.point, hit.normal))
                            }
                            _ => None,
                        };
                        // "Use surface" in the style: cut from the object
                        // where the text lands; where it lands nowhere or
                        // the cut gives nothing, flat
                        // (`CreateVolumesJob::process`).
                        if use_surface && let Some(hit) = hit {
                            let frame = model
                                .text_frame_on(bed.as_ref(), copy, hit, angle, None)
                                .map_err(fail)?;
                            if let Some(surface) =
                                self.text_surface(&model, object, None, &frame)?
                                && let Ok(on) =
                                    emboss::emboss_on_surface(fonts, &config, depth, kind, &surface)
                            {
                                text = on;
                            }
                        }
                        let part = model
                            .add_text(bed.as_ref(), object, copy, hit, angle, None, kind, text)
                            .map_err(fail)?;
                        ("Add Emboss text Volume", Some(part), None)
                    }
                    None => {
                        if model.plates().is_empty() {
                            model.add_plate(bed.as_ref()).map_err(fail)?;
                        }
                        let plate = stage
                            .plate
                            .and_then(|id| model.plates().iter().position(|plate| plate.id() == id))
                            .unwrap_or(0);
                        let near = args
                            .get("at")
                            .and_then(Value::as_array)
                            .and_then(|at| Some((at.first()?.as_f64()?, at.get(1)?.as_f64()?)));
                        let copy = model
                            .add_text_object(bed.as_ref(), plate, near, angle, text)
                            .map_err(fail)?;
                        let part = model
                            .objects()
                            .last()
                            .and_then(|object| object.shape().part_ids().first().copied());
                        ("Add Emboss text object", part, Some(copy))
                    }
                }
            }
            "set" => {
                let (object, part) = (whole(args, "object")?, whole(args, "part")?);
                let was = model
                    .text_of(object, part)
                    .map_err(fail)?
                    .ok_or("that part is not text")?;
                let mut config = configuration(style, &face, Some(&was.text))?;
                // A style name the page did not change stays.
                config.style_name = was.text.style_name.clone();
                if let Some(why) = emboss::not_yet(&config, &was.shape) {
                    return Err(tr(why));
                }
                // "Use surface" is not for the object's only printed part
                // (`start_update_volume`, `is_the_only_one_part`).
                let use_surface =
                    use_surface && !model.is_only_printed_part(object, part).map_err(fail)?;
                let mut frame = model.text_frame(object, part).map_err(fail)?;
                // "Rotation" and "From surface": the part turned about and
                // moved along its own Z (`do_local_z_rotate`,
                // `do_local_z_move`).
                if let Some(degrees) = style.get("angle").and_then(Value::as_f64) {
                    let now = emboss::calc_up(&frame, emboss::UP_LIMIT).unwrap_or(0.0);
                    let wanted = -degrees.to_radians();
                    if (wanted - now).abs() > 1e-9 {
                        frame = emboss::apply_angle_distance(&frame, Some(wanted - now), None);
                    }
                }
                // "From surface" is not used with "Use surface".
                if let Some(wanted) = style.get("distance").and_then(Value::as_f64)
                    && !use_surface
                    && let Some(copy) = args.get("copy").and_then(Value::as_u64)
                {
                    let now = model
                        .text_distance(bed.as_ref(), object, part, copy)
                        .map_err(fail)?
                        .unwrap_or(0.0);
                    if (wanted - now).abs() > 1e-9 {
                        frame = move_along_z(&frame, wanted - now);
                    }
                }
                if let Some(camera) = args.get("face_camera").filter(|v| !v.is_null()) {
                    let copy = whole(args, "copy")?;
                    let eye = camera
                        .get("eye")
                        .map(|_| three(camera, "eye"))
                        .transpose()?;
                    let forward = three(camera, "forward")?;
                    frame = model
                        .text_facing(
                            bed.as_ref(),
                            object,
                            copy,
                            &frame,
                            eye,
                            forward,
                            text_keep_up(args),
                        )
                        .map_err(fail)?;
                }
                let text = if use_surface {
                    // Turned on now: the text's origin moved onto the
                    // surface first (`calc_surface_offset`).
                    if !was.shape.use_surface
                        && let Some(offset) = model
                            .text_surface_offset(object, part, &frame)
                            .map_err(fail)?
                    {
                        frame = emboss::moved_in_frame(&frame, offset);
                    }
                    let surface = self
                        .text_surface(&model, object, Some(part), &frame)?
                        .ok_or_else(|| tr(emboss::NO_SURFACE))?;
                    emboss::emboss_on_surface(fonts, &config, depth, kind, &surface)
                        .map_err(fail)?
                } else {
                    let mut text = emboss::emboss(fonts, &config, depth, kind).map_err(fail)?;
                    text.volume.shape.bend = None;
                    text
                };
                let kind = args.get("kind").map(|_| kind);
                model
                    .set_text(bed.as_ref(), object, part, text, Some(frame), kind)
                    .map_err(fail)?;
                ("Emboss attribute change", Some(part), None)
            }
            other => return Err(format!("there is no text edit {other}")),
        };
        let next = Arc::new(File::of(model));
        {
            let mut state = lock(&self.state);
            let at = state
                .index_of(stage.id)
                .ok_or_else(|| format!("there is no project {}", stage.id))?;
            let project = &mut state.projects[at];
            let still = match (&project.file, &stage.file) {
                (Some(now), Some(then)) => Arc::ptr_eq(now, then),
                (None, None) => true,
                _ => false,
            };
            if !still {
                return Err("the project changed meanwhile; nothing was done".into());
            }
            let mut before = project.file.replace(next);
            if args.get("merge").and_then(Value::as_bool) == Some(true) {
                project.history.amend(name, &mut before);
            }
            push_step(project, name, before, None);
        }
        self.edited(
            stage.id,
            json!({ "done": name, "part": part, "made": made.map(|copy| vec![copy]).unwrap_or_default(), "note": null }),
        )
    }
}

/// Whether "Keep up" is on for "Set text to face camera".
fn text_keep_up(args: &Value) -> bool {
    args.get("keep_up").and_then(Value::as_bool) != Some(false)
}

/// The frame moved `by` mm along its own Z, as the panel's "From surface"
/// moves it.
fn move_along_z(frame: &Transform, by: f64) -> Transform {
    let z = frame.rows[2];
    let length = (z[0] * z[0] + z[1] * z[1] + z[2] * z[2]).sqrt();
    let mut out = *frame;
    if length > 0.0 {
        for (offset, along) in out.rows[3].iter_mut().zip(z) {
            *offset += by * along / length;
        }
    }
    out
}

fn three(args: &Value, key: &str) -> Result<[f64; 3], String> {
    let list: Vec<f64> = args
        .get(key)
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    <[f64; 3]>::try_from(list).map_err(|_| format!("{key} is not three numbers"))
}

#[cfg(test)]
mod tests;
