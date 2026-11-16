//! What fonts a file holds and what each calls itself, read without
//! reading the whole file: the file's table directory and the few small
//! tables the names and measures are in. A font file is untrusted input:
//! every offset and length is checked against the file, and the tables
//! read are of bounded size; what does not hold together is refused.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::{FaceInfo, MAX_FONT_BYTES, TextError};

/// The most fonts of a collection looked at, and tables of a font.
const MAX_FACES: u32 = 256;
const MAX_TABLES: u16 = 512;
/// The largest of the small tables read (`name` of a large CJK font is
/// some tens of kB).
const MAX_TABLE: u32 = 4 << 20;

/// One font of a file, as the list of fonts shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScannedFace {
    /// Which font of the file it is.
    pub index: u32,
    pub info: FaceInfo,
}

fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

struct Reader {
    file: std::fs::File,
    len: u64,
}

impl Reader {
    fn read(&mut self, offset: u64, len: u64) -> Option<Vec<u8>> {
        if offset.checked_add(len)? > self.len || len > u64::from(MAX_TABLE) {
            return None;
        }
        self.file.seek(SeekFrom::Start(offset)).ok()?;
        let mut out = vec![0; len as usize];
        self.file.read_exact(&mut out).ok()?;
        Some(out)
    }
}

/// The fonts of the file at `path`, each with what it calls itself. A
/// font whose tables do not hold together is left out and said in
/// `skipped` with why; a file larger than [`MAX_FONT_BYTES`] or with no
/// font at all is refused.
pub fn scan_file(path: &Path, skipped: &mut Vec<String>) -> Result<Vec<ScannedFace>, TextError> {
    let named = || path.display().to_string();
    let file = std::fs::File::open(path).map_err(|e| TextError::Read(named(), e))?;
    let len = file
        .metadata()
        .map_err(|e| TextError::Read(named(), e))?
        .len();
    if len > MAX_FONT_BYTES {
        return Err(TextError::TooLarge(named()));
    }
    let mut reader = Reader { file, len };
    let head = reader
        .read(0, 12.min(len))
        .ok_or_else(|| TextError::NotAFont(named()))?;
    let offsets: Vec<u64> = if head.get(..4) == Some(b"ttcf") {
        let count = be_u32(&head, 8).ok_or_else(|| TextError::NotAFont(named()))?;
        let count = count.min(MAX_FACES);
        let table = reader
            .read(12, 4 * u64::from(count))
            .ok_or_else(|| TextError::NotAFont(named()))?;
        (0..count as usize)
            .filter_map(|at| be_u32(&table, 4 * at).map(u64::from))
            .collect()
    } else {
        vec![0]
    };
    let mut faces = Vec::new();
    let mut left_out = Vec::new();
    for (index, offset) in offsets.into_iter().enumerate() {
        match scan_face(&mut reader, offset) {
            Ok(info) => faces.push(ScannedFace {
                index: index as u32,
                info,
            }),
            Err(why) => left_out.push((index, why)),
        }
    }
    if faces.is_empty() {
        // Said once, for the file, with why its first font was left out.
        let why = left_out
            .first()
            .map_or(String::new(), |(_, why)| format!(" ({why})"));
        return Err(TextError::NotAFont(format!("{}{why}", named())));
    }
    skipped.extend(
        left_out
            .into_iter()
            .map(|(index, why)| format!("{} (font {}): {why}", named(), index + 1)),
    );
    Ok(faces)
}

fn scan_face(reader: &mut Reader, offset: u64) -> Result<FaceInfo, &'static str> {
    let header = reader.read(offset, 12).ok_or("no table directory")?;
    let version = be_u32(&header, 0).ok_or("no table directory")?;
    if version != 0x0001_0000 && &header[..4] != b"OTTO" && &header[..4] != b"true" {
        return Err("not a TrueType or OpenType font");
    }
    let count = be_u16(&header, 4).ok_or("no table directory")?;
    if count > MAX_TABLES {
        return Err("too many tables");
    }
    let records = reader
        .read(offset + 12, 16 * u64::from(count))
        .ok_or("its table directory runs past the file")?;
    let mut wanted: [(&[u8; 4], Option<Vec<u8>>); 5] = [
        (b"head", None),
        (b"hhea", None),
        (b"maxp", None),
        (b"name", None),
        (b"OS/2", None),
    ];
    let (mut glyf, mut cff, mut cff2) = (false, false, false);
    for at in 0..count as usize {
        let record = &records[16 * at..16 * at + 16];
        let tag = &record[..4];
        match tag {
            b"glyf" => glyf = true,
            b"CFF " => cff = true,
            b"CFF2" => cff2 = true,
            _ => {}
        }
        if let Some(slot) = wanted.iter_mut().find(|(name, _)| &name[..] == tag) {
            let (start, length) = (
                be_u32(record, 8).ok_or("bad table record")?,
                be_u32(record, 12).ok_or("bad table record")?,
            );
            if length > MAX_TABLE {
                return Err("a table is too large");
            }
            slot.1 = Some(
                reader
                    .read(u64::from(start), u64::from(length))
                    .ok_or("a table runs past the file")?,
            );
        }
    }
    if !glyf && !cff {
        return Err(if cff2 {
            "its outlines are CFF2, which are not read"
        } else {
            "it has no outlines (a bitmap or colour font)"
        });
    }
    let [head, hhea, maxp, name, os2] = wanted.map(|(_, table)| table);
    let tables = ttf_parser::RawFaceTables {
        head: head.as_deref().ok_or("it has no head table")?,
        hhea: hhea.as_deref().ok_or("it has no hhea table")?,
        maxp: maxp.as_deref().ok_or("it has no maxp table")?,
        name: name.as_deref(),
        os2: os2.as_deref(),
        ..Default::default()
    };
    let face = ttf_parser::Face::from_raw_tables(tables).map_err(|_| "its tables do not read")?;
    let mut info = FaceInfo::of(&face);
    info.cff = cff && !glyf;
    if info.family.is_empty() {
        return Err("it has no family name");
    }
    Ok(info)
}
