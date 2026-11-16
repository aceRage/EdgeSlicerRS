//! Glyphs against the reference's own: the pin's stb_truetype run on the
//! pin's font (`data/stbdump.cpp`, its output in `data/stb_glyphs.txt`),
//! every flattened point compared.

use std::path::PathBuf;

use edge_text::{Font, FontProp, HorizontalAlign, flatness, flattened, text_to_shapes};

fn fonts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/EdgeSlicer/resources/fonts")
}

fn bold() -> Font {
    Font::open(&fonts().join("HarmonyOS_Sans_SC_Bold.ttf")).unwrap()
}

struct Dumped {
    ch: char,
    advance: i64,
    contours: Vec<Vec<(i64, i64)>>,
}

fn dumped() -> (f32, Vec<Dumped>) {
    let text = include_str!("data/stb_glyphs.txt");
    let mut flatness = 0.0;
    let mut glyphs: Vec<Dumped> = Vec::new();
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.first().copied() {
            Some("ascent") => flatness = words[7].parse().unwrap(),
            Some("glyph") => glyphs.push(Dumped {
                ch: words[1].chars().next().unwrap(),
                advance: words[5].parse().unwrap(),
                contours: Vec::new(),
            }),
            Some("contour") => glyphs.last_mut().unwrap().contours.push(Vec::new()),
            Some(x) => {
                let y = words[1].parse().unwrap();
                let contour = glyphs.last_mut().unwrap().contours.last_mut().unwrap();
                contour.push((x.parse().unwrap(), y));
            }
            None => {}
        }
    }
    (flatness, glyphs)
}

#[test]
fn glyphs_flatten_point_for_point_as_the_references_stb_truetype() {
    let font = bold();
    let (expected_flatness, glyphs) = dumped();
    assert_eq!((font.ascent, font.descent, font.line_gap), (928, -244, 0));
    assert_eq!(font.units_per_em, 1000);
    let flat = flatness(&font, &FontProp::new(4.0));
    assert_eq!(flat, expected_flatness);
    assert_eq!(glyphs.len(), 18);
    let mut points = 0;
    for glyph in glyphs {
        let (advance, contours) = flattened(&font, glyph.ch, flat).unwrap().unwrap();
        assert_eq!(advance, glyph.advance, "{}", glyph.ch);
        let ours: Vec<Vec<(i64, i64)>> = contours
            .iter()
            .map(|c| c.iter().map(|p| (p.x, p.y)).collect())
            .collect();
        assert_eq!(ours, glyph.contours, "{}", glyph.ch);
        points += ours.iter().map(Vec::len).sum::<usize>();
    }
    assert_eq!(points, 914 - 1 - 18 - glyphs_contours());
}

/// The contour lines of the dump.
fn glyphs_contours() -> usize {
    include_str!("data/stb_glyphs.txt")
        .lines()
        .filter(|line| line.starts_with(" contour"))
        .count()
}

#[test]
fn text_is_laid_by_advances_and_united() {
    let font = bold();
    let mut prop = FontProp::new(4.0);
    prop.horizontal = HorizontalAlign::Left;
    let one = text_to_shapes(&font, None, "0", &prop).unwrap();
    let two = text_to_shapes(&font, None, "00", &prop).unwrap();
    let area = |shapes: &[edge_geom::ExPolygon]| -> f64 {
        shapes.iter().map(edge_geom::ExPolygon::area_mm2).sum()
    };
    // Two zeros apart: twice the area, the second 601 font units on.
    assert_eq!(one.len(), 1);
    assert_eq!(two.len(), 2);
    assert!((area(&two) - 2.0 * area(&one)).abs() < 1e-9 * area(&one).abs().max(1.0));
    let (lo1, _) = edge_text::extents(&one).unwrap();
    let (lo2, _) = edge_text::extents(two.iter().skip(1)).unwrap();
    assert_eq!(lo2.x - lo1.x, 601_000);
    // A character neither font has is dropped and takes no room.
    let dropped = text_to_shapes(&font, None, "0\u{E000}0", &prop).unwrap();
    assert_eq!(dropped, two);
}

#[test]
fn a_symbol_the_font_lacks_comes_from_the_fallback() {
    let font = bold();
    let fallback = Font::open(&fonts().join("NotoSansSymbols2-Subset.ttf")).unwrap();
    let star = '\u{2B50}';
    assert!(!font.covers(star));
    assert!(fallback.covers(star));
    let prop = FontProp::new(4.0);
    assert!(
        text_to_shapes(&font, None, "\u{2B50}", &prop)
            .unwrap()
            .is_empty()
    );
    assert!(
        !text_to_shapes(&font, Some(&fallback), "\u{2B50}", &prop)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_font_says_what_it_is_called() {
    let nanum = Font::open(&fonts().join("NanumGothic-Bold.ttf")).unwrap();
    let info = nanum.info().unwrap();
    assert_eq!(info.family, "NanumGothic");
    assert_eq!(info.postscript, "NanumGothicBold");
    // Its OS/2 weight class is 600, though it is the family's bold.
    assert_eq!((info.style.as_str(), info.weight), ("Bold", 600));
    assert!(!info.italic && !info.cff);
    assert_eq!(nanum.count(), 1);
    assert_eq!(edge_text::fonts_in(b"not a font at all"), 0);
    // What is not a font is refused, not read.
    assert!(Font::from_bytes(b"not a font at all".to_vec(), "x").is_err());
}
