//! Text as flat shapes, read from a TrueType font: what the reference's
//! embosser makes of a string (Emboss.cpp `text2shapes`), for the models
//! the engine cuts text into (the ironing flow calibration's labels, and
//! later a Text tool).
//!
//! A port of the reference's path, step by step:
//! - each glyph's outline is read (here with `ttf-parser`, there with
//!   stb_truetype) and its curves flattened as stb_truetype flattens them
//!   (`stbtt_FlattenCurves`: a curve halved until its middle is within the
//!   flatness of the chord, in single precision), the flatness being the
//!   font's ascent × 0.0125 mm over the text's size ([`flatness`]);
//! - the points are kept in "shape units", a thousandth of a font unit,
//!   rounded, and each glyph's contours are united by the non-zero rule
//!   and cleaned of points closer than 1.415 units ([`heal`]);
//! - the glyphs are laid along a line by their advance widths, with no
//!   kerning, a new line down by the line height, a tab four spaces;
//! - the whole text is united with each glyph grown by one font unit and
//!   shrunk back (`union_with_delta`), which joins letters that touch.
//!
//! A character the font lacks is taken from the fallback font (the
//! reference's bundled `NotoSansSymbols2-Subset.ttf`) when it is a symbol
//! that font may hold (`is_font_fallback_candidate`), sized to the first
//! font's em; one neither font holds is left out without a word and takes
//! no room, as in the reference.
//!
//! Not as the reference, each also said where it is done: the clipping is
//! Clipper2's, where the reference's is Clipper 6 (results agree in shape,
//! not vertex for vertex); the healing of what is left touching itself
//! cuts the reference's tiny squares (three shape units, a few nanometres
//! once scaled) but does not bevel spikes at those points, and a shape that
//! will not heal is kept as it is where the reference puts a frame of its
//! bounding box in its place.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use edge_geom::{Coord, ExPolygon, Point2, Polygon};

mod flatten;
mod heal;

pub use heal::heal;

/// A thousandth of a font unit: what glyph points are kept in (Emboss.cpp
/// `SHAPE_SCALE`).
pub const SHAPE_SCALE: f64 = 0.001;

/// The size of a flattening step, in mm of text (Emboss.cpp
/// `get_glyph`'s `RESOLUTION`).
const RESOLUTION: f32 = 0.0125;

#[derive(Debug, thiserror::Error)]
pub enum TextError {
    #[error("cannot read the font {0}: {1}")]
    Read(String, std::io::Error),
    #[error("{0} is not a font this can read")]
    NotAFont(String),
    #[error("{0} is larger than any font this reads")]
    TooLarge(String),
    #[error(transparent)]
    Clip(#[from] edge_clip::ClipError),
}

/// The largest font file read ([`Font::open`]): a font is untrusted
/// input, and the largest CJK fonts are some 30 MB.
pub const MAX_FONT_BYTES: u64 = 64 << 20;

/// A font file, read whole, and the measures the embosser takes from it
/// (`FontFile::Info`): one font of it, the first of a collection unless
/// another is asked for ([`Font::from_shared`]).
#[derive(Clone)]
pub struct Font {
    data: Arc<Vec<u8>>,
    /// Which font of a collection (`FontProp::collection_number`).
    index: u32,
    name: String,
    /// The `hhea` table's ascent, descent and line gap, as stb_truetype's
    /// `stbtt_GetFontVMetrics` reads them, in font units.
    pub ascent: i32,
    pub descent: i32,
    pub line_gap: i32,
    pub units_per_em: i32,
}

impl Font {
    pub fn open(path: &Path) -> Result<Font, TextError> {
        Self::from_bytes(read_font_file(path)?, &path.display().to_string())
    }

    /// A font from its bytes; `name` is what errors call it.
    pub fn from_bytes(data: Vec<u8>, name: &str) -> Result<Font, TextError> {
        Self::from_shared(Arc::new(data), 0, name)
    }

    /// Font `index` of a file's bytes (a collection holds several, any
    /// other file one), shared with the file's other fonts.
    pub fn from_shared(data: Arc<Vec<u8>>, index: u32, name: &str) -> Result<Font, TextError> {
        let face = ttf_parser::Face::parse(&data, index)
            .map_err(|_| TextError::NotAFont(name.to_owned()))?;
        let hhea = face.tables().hhea;
        let (ascent, descent, line_gap) = (
            i32::from(hhea.ascender),
            i32::from(hhea.descender),
            i32::from(hhea.line_gap),
        );
        let units_per_em = i32::from(face.units_per_em());
        Ok(Font {
            data,
            index,
            name: name.to_owned(),
            ascent,
            descent,
            line_gap,
            units_per_em,
        })
    }

    fn face(&self) -> Result<ttf_parser::Face<'_>, TextError> {
        ttf_parser::Face::parse(&self.data, self.index)
            .map_err(|_| TextError::NotAFont(self.name.clone()))
    }

    /// The bytes of the file the font is of, shared.
    pub fn data(&self) -> &Arc<Vec<u8>> {
        &self.data
    }

    /// Which font of its file it is.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// How many fonts its file holds (Emboss.cpp `create_font_file`'s
    /// `stbtt_GetNumberOfFonts`).
    pub fn count(&self) -> u32 {
        fonts_in(&self.data)
    }

    /// What the font calls itself ([`FaceInfo`]).
    pub fn info(&self) -> Result<FaceInfo, TextError> {
        Ok(FaceInfo::of(&self.face()?))
    }

    /// Whether the font has a glyph for `ch`.
    pub fn covers(&self, ch: char) -> bool {
        self.face()
            .ok()
            .and_then(|face| face.glyph_index(ch))
            .is_some_and(|id| id.0 != 0)
    }

    /// The licence the font's own name table states (name 13), if any.
    pub fn licence(&self) -> Option<String> {
        let face = self.face().ok()?;
        face.names()
            .into_iter()
            .filter(|name| name.name_id == ttf_parser::name_id::LICENSE)
            .find_map(|name| name.to_string())
    }
}

/// How text is laid out (TextConfiguration.hpp `FontProp`, the parts that
/// shape the letters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontProp {
    /// The em of the text, in mm (`size_in_mm`).
    pub size_in_mm: f32,
    pub horizontal: HorizontalAlign,
    pub vertical: VerticalAlign,
    /// Room added after every letter, in font units (`char_gap`).
    pub char_gap: Option<i32>,
    /// Room added between lines, in font units (`line_gap`).
    pub line_gap: Option<i32>,
    /// How far every letter's outline is grown (`boldness`): the
    /// reference grows it by this over the size, in font units, so the
    /// value is not a length of its own (TextConfiguration.hpp calls it
    /// mm, its slider "points"); it is taken as the reference takes it.
    pub boldness: Option<f32>,
    /// How far right each point moves for each unit up (`skew`).
    pub skew: Option<f32>,
}

impl FontProp {
    /// `FontProp(size)`: centred, as the reference's default alignment.
    pub fn new(size_in_mm: f32) -> Self {
        Self {
            size_in_mm,
            horizontal: HorizontalAlign::Center,
            vertical: VerticalAlign::Center,
            char_gap: None,
            line_gap: None,
            boldness: None,
            skew: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HorizontalAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalAlign {
    Top,
    Center,
    Bottom,
}

/// mm per shape unit of text laid out with `prop` in `font`
/// (`get_text_shape_scale`).
pub fn shape_scale(font: &Font, prop: &FontProp) -> f64 {
    f64::from(prop.size_in_mm) / f64::from(font.units_per_em) * SHAPE_SCALE
}

/// The flatness curves are flattened to, in font units (Emboss.cpp
/// `get_glyph`): the ascent × 0.0125 mm over the size, never under 0.0125.
pub fn flatness(font: &Font, prop: &FontProp) -> f32 {
    let flatness = font.ascent as f32 * RESOLUTION / prop.size_in_mm;
    if flatness < RESOLUTION {
        RESOLUTION
    } else {
        flatness
    }
}

/// One glyph as the embosser keeps it: its shape in shape units, its
/// origin on the baseline, and how far it moves the next one on.
#[derive(Clone, Debug, Default)]
pub struct Glyph {
    pub shape: Vec<ExPolygon>,
    pub advance: Coord,
}

/// `C++ static_cast<int>(value / SHAPE_SCALE)`: truncated, as the
/// reference turns font units into shape units.
fn to_shape_units(value: f64) -> Coord {
    (value / SHAPE_SCALE) as Coord
}

/// The glyph of `ch` in `font`, flattened at `flatness` (in font units)
/// and healed, before any style is applied (Emboss.cpp `get_glyph`);
/// `None` when the font has no glyph for it. Its advance is in font units.
pub fn raw_glyph(font: &Font, ch: char, flatness: f32) -> Result<Option<Glyph>, TextError> {
    let Some((advance, contours)) = flattened(font, ch, flatness)? else {
        return Ok(None);
    };
    let mut polygons = Vec::new();
    for contour in contours {
        // "check minimal length for triangle"; the last point is the
        // first again.
        if contour.len() < 4 {
            continue;
        }
        let mut points = contour[..contour.len() - 1].to_vec();
        // "change outer cw to ccw and inner ccw to cw order"
        points.reverse();
        polygons.push(Polygon { points });
    }
    let shape = if polygons.is_empty() {
        Vec::new()
    } else {
        heal::heal_polygons(&polygons, 10)?
    };
    Ok(Some(Glyph { shape, advance }))
}

/// Contours of points, each a closed ring given with its first point again
/// at its end.
pub type Contours = Vec<Vec<Point2>>;

/// The advance (font units) of the glyph of `ch` and its contours as
/// stb_truetype flattens them, each point in shape units and each contour
/// ending where it began, in the font's own order; `None` when the font
/// has no glyph for it.
pub fn flattened(
    font: &Font,
    ch: char,
    flatness: f32,
) -> Result<Option<(Coord, Contours)>, TextError> {
    let face = font.face()?;
    let Some(id) = face.glyph_index(ch).filter(|id| id.0 != 0) else {
        return Ok(None);
    };
    let advance = face.glyph_hor_advance(id).map_or(0, Coord::from);
    let mut outline = flatten::Outline::default();
    if face.outline_glyph(id, &mut outline).is_none() {
        return Ok(Some((advance, Vec::new())));
    }
    let contours = flatten::flatten(&outline.vertices, flatness)
        .into_iter()
        .map(|contour| {
            contour
                .iter()
                .map(|p| Point2 {
                    x: (f64::from(p[0]) / SHAPE_SCALE).round() as Coord,
                    y: (f64::from(p[1]) / SHAPE_SCALE).round() as Coord,
                })
                .collect()
        })
        .collect();
    Ok(Some((advance, contours)))
}

/// Whether a character missing from a font is looked for in the fallback
/// font (FontFallback.cpp `is_font_fallback_candidate`): not controls,
/// spaces, marks, private use or non-characters.
pub fn is_fallback_candidate(cp: u32) -> bool {
    !(cp <= 0x20
        || (0x7F..=0xA0).contains(&cp)
        || cp == 0x00AD
        || cp == 0x061C
        || cp == 0x180E
        || (0x2000..=0x200F).contains(&cp)
        || (0x2028..=0x202F).contains(&cp)
        || (0x205F..=0x206F).contains(&cp)
        || cp == 0x3000
        || cp == 0xFEFF
        || (0xD800..=0xDFFF).contains(&cp)
        || (0xE000..=0xF8FF).contains(&cp)
        || (0xFDD0..=0xFDEF).contains(&cp)
        || (cp & 0xFFFE) == 0xFFFE
        || cp > 0x10FFFF)
}

/// Fonts and the glyphs already made of them, for one size of text.
pub struct Typesetter<'a> {
    font: &'a Font,
    fallback: Option<&'a Font>,
    prop: FontProp,
    glyphs: HashMap<char, Option<Glyph>>,
}

impl<'a> Typesetter<'a> {
    pub fn new(font: &'a Font, fallback: Option<&'a Font>, prop: FontProp) -> Self {
        Self {
            font,
            fallback,
            prop,
            glyphs: HashMap::new(),
        }
    }

    /// The glyph of `ch` with the style applied (advance in shape units),
    /// from the font, else the fallback font; `None` when neither has it.
    fn glyph(&mut self, ch: char) -> Result<Option<&Glyph>, TextError> {
        if !self.glyphs.contains_key(&ch) {
            let made = self.make(ch)?;
            self.glyphs.insert(ch, made);
        }
        Ok(self.glyphs.get(&ch).and_then(Option::as_ref))
    }

    fn make(&self, ch: char) -> Result<Option<Glyph>, TextError> {
        let flatness = flatness(self.font, &self.prop);
        if let Some(mut glyph) = raw_glyph(self.font, ch, flatness)? {
            self.apply_font_prop(&mut glyph)?;
            return Ok(Some(glyph));
        }
        // Emboss.cpp `get_fallback_glyph`.
        let Some(fallback) = self.fallback else {
            return Ok(None);
        };
        if !is_fallback_candidate(u32::from(ch)) {
            return Ok(None);
        }
        let em_scale = if self.font.units_per_em <= 0 || fallback.units_per_em <= 0 {
            1.0
        } else {
            f64::from(self.font.units_per_em) / f64::from(fallback.units_per_em)
        };
        let own = (f64::from(flatness) / em_scale) as f32;
        let Some(mut glyph) = raw_glyph(fallback, ch, own)? else {
            return Ok(None);
        };
        if em_scale != 1.0 {
            let scale = |p: &mut Point2| {
                p.x = (p.x as f64 * em_scale).round() as Coord;
                p.y = (p.y as f64 * em_scale).round() as Coord;
            };
            for region in &mut glyph.shape {
                region.contour.points.iter_mut().for_each(scale);
                for hole in &mut region.holes {
                    hole.points.iter_mut().for_each(scale);
                }
            }
            glyph.advance = (glyph.advance as f64 * em_scale).round() as Coord;
        }
        self.apply_font_prop(&mut glyph)?;
        Ok(Some(glyph))
    }

    /// What the style does to every glyph (Emboss.cpp `apply_font_prop`):
    /// the gap added to its advance (in font units), the advance put in
    /// shape units, the outline grown by the boldness and skewed.
    fn apply_font_prop(&self, glyph: &mut Glyph) -> Result<(), TextError> {
        let prop = &self.prop;
        if let Some(gap) = prop.char_gap {
            glyph.advance += Coord::from(gap);
        }
        glyph.advance = to_shape_units(glyph.advance as f64);
        if glyph.shape.is_empty() {
            return Ok(());
        }
        if let Some(boldness) = prop.boldness {
            // A float in the reference, handed to Clipper as a double;
            // here a whole shape unit (a millionth of the em).
            let delta = (f64::from(boldness) / SHAPE_SCALE / f64::from(prop.size_in_mm)) as f32;
            let grown =
                edge_clip::offset(&glyph.shape, delta.round() as Coord, edge_clip::Join::MITER)?;
            glyph.shape = edge_clip::union_ex(&grown)?;
        }
        if let Some(skew) = prop.skew {
            let ratio = f64::from(skew);
            for region in &mut glyph.shape {
                for ring in std::iter::once(&mut region.contour).chain(&mut region.holes) {
                    for p in &mut ring.points {
                        p.x += (p.y as f64 * ratio).round() as Coord;
                    }
                }
            }
        }
        Ok(())
    }

    /// The line height in shape units (`get_line_height`).
    fn line_height(&self) -> Coord {
        let f = self.font;
        let gap = self.prop.line_gap.unwrap_or(0);
        to_shape_units(f64::from(f.ascent - f.descent + f.line_gap + gap))
    }

    /// `text` as shapes in shape units (Emboss.cpp `text2shapes`): its
    /// glyphs laid out and aligned, then united, each grown by a font unit
    /// and shrunk back, and healed.
    pub fn shapes(&mut self, text: &str) -> Result<Vec<ExPolygon>, TextError> {
        let letters = self.letters(text)?;
        Ok(union_with_delta(&letters, to_shape_units(1.0), 10)?.0)
    }

    /// Each character's shapes, laid out and aligned (`text2vshapes`): one
    /// entry for every character, a new line or a character no font has
    /// an empty one.
    pub fn letters(&mut self, text: &str) -> Result<Vec<Vec<ExPolygon>>, TextError> {
        let chars: Vec<char> = text.chars().collect();
        let mut cursor = Point2::default();
        let mut result = Vec::with_capacity(chars.len());
        for &ch in &chars {
            match ch {
                '\n' => {
                    cursor.x = 0;
                    cursor.y -= self.line_height();
                    result.push(Vec::new());
                }
                '\t' => {
                    // "'\t' = 4*space => same as imgui"
                    if let Some(space) = self.glyph(' ')? {
                        cursor.x += 4 * space.advance;
                    }
                    result.push(Vec::new());
                }
                '\r' => result.push(Vec::new()),
                _ => match self.glyph(ch)? {
                    Some(glyph) => {
                        let shift = cursor;
                        let mut shape = glyph.shape.clone();
                        cursor.x += glyph.advance;
                        translate(&mut shape, shift);
                        result.push(shape);
                    }
                    None => result.push(Vec::new()),
                },
            }
        }
        self.align(&mut result, &chars);
        Ok(result)
    }

    /// Emboss.cpp `align_shape`.
    fn align(&self, shapes: &mut [Vec<ExPolygon>], chars: &[char]) {
        let lines = if chars.is_empty() {
            0
        } else {
            1 + chars.iter().filter(|ch| **ch == '\n').count()
        };
        if lines == 0 {
            return;
        }
        // `get_align_y_offset`: into a float, then an int.
        let line_height = self.line_height();
        let ascent = to_shape_units(f64::from(self.font.ascent));
        // `ASCENT_CENTER`, a third of the ascent.
        let line_center = (ascent as f64 * (1.0 / 3.0)).round() as f32;
        let y = match self.prop.vertical {
            VerticalAlign::Bottom => (line_height * (lines as Coord - 1)) as f32,
            VerticalAlign::Top => -ascent as f32,
            VerticalAlign::Center => {
                (f64::from(-line_center) + (line_height * (lines as Coord - 1)) as f64 / 2.0) as f32
            }
        } as Coord;
        if self.prop.horizontal == HorizontalAlign::Left {
            for shape in shapes.iter_mut() {
                translate(shape, Point2 { x: 0, y });
            }
            return;
        }
        // An empty box is the reference's default one, at the origin.
        let empty = (Point2::default(), Point2::default());
        let (lo, hi) = extents(shapes.iter().flatten()).unwrap_or(empty);
        let line_box = |from: usize| {
            extents(
                (from..chars.len())
                    .take_while(|at| chars[*at] != '\n')
                    .flat_map(|at| &shapes[at]),
            )
            .unwrap_or(empty)
        };
        let x_offset = |(llo, lhi): (Point2, Point2)| -> Coord {
            let (size, line_size) = (hi.x - lo.x, lhi.x - llo.x);
            match self.prop.horizontal {
                HorizontalAlign::Right => -hi.x + (size - line_size),
                HorizontalAlign::Center => -((lo.x + hi.x) / 2) + (size - line_size) / 2,
                HorizontalAlign::Left => 0,
            }
        };
        // Each character's offset, worked out before anything moves.
        let mut offset = x_offset(line_box(0));
        let offsets: Vec<Option<Coord>> = (0..chars.len())
            .map(|at| {
                if chars[at] == '\n' {
                    offset = x_offset(line_box(at + 1));
                    None
                } else {
                    Some(offset)
                }
            })
            .collect();
        for (shape, x) in shapes.iter_mut().zip(offsets) {
            if let Some(x) = x {
                translate(shape, Point2 { x, y });
            }
        }
    }
}

/// Letters united into one shape (Emboss.cpp `union_with_delta`): each
/// grown by `delta` (shape units), united, shrunk back and healed with at
/// most `max_heal` rounds. Says too whether the healing came out clean
/// (`HealedExPolygons::is_healed`).
pub fn union_with_delta(
    letters: &[Vec<ExPolygon>],
    delta: Coord,
    max_heal: u32,
) -> Result<(Vec<ExPolygon>, bool), TextError> {
    let mut grown = Vec::new();
    for shape in letters.iter().filter(|shape| !shape.is_empty()) {
        grown.extend(edge_clip::offset(shape, delta, edge_clip::Join::MITER)?);
    }
    let united = edge_clip::union_ex(&grown)?;
    let mut shapes = edge_clip::offset(&united, -delta, edge_clip::Join::MITER)?;
    let healed = heal(&mut shapes, max_heal)?;
    Ok((shapes, healed))
}

/// How many fonts a file's bytes hold: a collection's count, one for any
/// other font, none for what is not a font.
pub fn fonts_in(data: &[u8]) -> u32 {
    match ttf_parser::fonts_in_collection(data) {
        Some(count) => count,
        None => u32::from(ttf_parser::Face::parse(data, 0).is_ok()),
    }
}

/// Reads a font file whole, refusing one larger than [`MAX_FONT_BYTES`].
pub fn read_font_file(path: &Path) -> Result<Vec<u8>, TextError> {
    use std::io::Read;
    let named = || path.display().to_string();
    let file = std::fs::File::open(path).map_err(|e| TextError::Read(named(), e))?;
    let mut data = Vec::new();
    file.take(MAX_FONT_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| TextError::Read(named(), e))?;
    if data.len() as u64 > MAX_FONT_BYTES {
        return Err(TextError::TooLarge(named()));
    }
    Ok(data)
}

/// What a font calls itself, from its `name` and `OS/2` tables: what the
/// list of fonts shows and what a project names the font by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceInfo {
    /// The family ("Helvetica"): the typographic family (name 16) when
    /// the font has one, else name 1. What wx calls the face name.
    pub family: String,
    /// The style within the family ("Bold Oblique"): name 17, else 2.
    pub style: String,
    /// The full name ("Helvetica Bold Oblique", name 4).
    pub full: String,
    /// The PostScript name ("Helvetica-BoldOblique", name 6): what macOS
    /// finds a font by.
    pub postscript: String,
    /// `OS/2` usWeightClass, 400 regular, 700 bold.
    pub weight: u16,
    pub italic: bool,
    /// Whether its outlines are cubic (CFF), not TrueType's quadratic.
    pub cff: bool,
}

impl FaceInfo {
    fn of(face: &ttf_parser::Face<'_>) -> FaceInfo {
        let name = |ids: &[u16]| -> String {
            ids.iter()
                .find_map(|id| english_name(face, *id))
                .unwrap_or_default()
        };
        let family = name(&[
            ttf_parser::name_id::TYPOGRAPHIC_FAMILY,
            ttf_parser::name_id::FAMILY,
        ]);
        let style = name(&[
            ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY,
            ttf_parser::name_id::SUBFAMILY,
        ]);
        FaceInfo {
            full: name(&[ttf_parser::name_id::FULL_NAME]),
            postscript: name(&[ttf_parser::name_id::POST_SCRIPT_NAME]),
            family,
            style,
            weight: face.weight().to_number(),
            italic: face.is_italic() || face.is_oblique(),
            cff: face.tables().cff.is_some(),
        }
    }
}

/// A name of the font in English, from a Unicode record (Windows'
/// US English first) or a Macintosh Roman one read as ASCII.
fn english_name(face: &ttf_parser::Face<'_>, id: u16) -> Option<String> {
    let names = face.names();
    let records: Vec<_> = names
        .into_iter()
        .filter(|name| name.name_id == id)
        .collect();
    let unicode = |english: bool| {
        records.iter().find_map(|name| {
            let wanted = !english
                || name.language_id == 0x0409
                || name.platform_id == ttf_parser::PlatformId::Unicode;
            if wanted { name.to_string() } else { None }
        })
    };
    let found = unicode(true).or_else(|| unicode(false)).or_else(|| {
        records.iter().find_map(|name| {
            (name.platform_id == ttf_parser::PlatformId::Macintosh
                && name.encoding_id == 0
                && name.name.is_ascii())
            .then(|| String::from_utf8_lossy(name.name).into_owned())
        })
    })?;
    let trimmed = found.trim_matches(char::from(0)).trim().to_owned();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// The box round the outlines of `shapes`: least and greatest corners.
pub fn extents<'s>(shapes: impl IntoIterator<Item = &'s ExPolygon>) -> Option<(Point2, Point2)> {
    let mut found: Option<(Point2, Point2)> = None;
    for p in shapes.into_iter().flat_map(|s| &s.contour.points) {
        let (lo, hi) = found.get_or_insert((*p, *p));
        lo.x = lo.x.min(p.x);
        lo.y = lo.y.min(p.y);
        hi.x = hi.x.max(p.x);
        hi.y = hi.y.max(p.y);
    }
    found
}

/// Moves every point of `shapes` by `by`.
pub fn translate(shapes: &mut [ExPolygon], by: Point2) {
    for shape in shapes {
        for ring in std::iter::once(&mut shape.contour).chain(&mut shape.holes) {
            for p in &mut ring.points {
                p.x += by.x;
                p.y += by.y;
            }
        }
    }
}

/// `text` in `font` (with `fallback` for the symbols it lacks), laid out
/// with `prop`, as shapes in shape units ([`shape_scale`] gives their mm).
pub fn text_to_shapes(
    font: &Font,
    fallback: Option<&Font>,
    text: &str,
    prop: &FontProp,
) -> Result<Vec<ExPolygon>, TextError> {
    Typesetter::new(font, fallback, *prop).shapes(text)
}
