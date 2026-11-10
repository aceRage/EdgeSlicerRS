use std::io::{Read, Seek};

use zip::ZipArchive;

use crate::Error;

/// Largest part accepted once decompressed. 3MF files are opened from
/// anywhere, and a few kilobytes of zip can otherwise expand without bound.
const MAX_PART_BYTES: u64 = 1 << 30;

/// The zip container of a 3MF file.
pub(crate) struct Package<R> {
    archive: ZipArchive<R>,
}

impl<R: Read + Seek> Package<R> {
    pub fn open(reader: R) -> Result<Self, Error> {
        ZipArchive::new(reader)
            .map(|archive| Self { archive })
            .map_err(|e| Error::Archive(e.to_string()))
    }

    /// The text of a part, or `None` when the package has no such part.
    /// Part names are compared without regard to case, as the packaging
    /// convention requires, and a leading slash is optional.
    pub fn read(&mut self, part: &str) -> Result<Option<String>, Error> {
        let wanted = part.trim_start_matches('/');
        let index = self.archive.index_for_name(wanted).or_else(|| {
            (0..self.archive.len()).find(|&index| {
                self.archive
                    .name_for_index(index)
                    .and_then(Result::ok)
                    .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
            })
        });
        let Some(index) = index else { return Ok(None) };

        let malformed = |reason: String| Error::Malformed {
            part: wanted.to_owned(),
            reason,
        };
        let file = self
            .archive
            .by_index(index)
            .map_err(|e| malformed(e.to_string()))?;
        let mut bytes = Vec::with_capacity(file.size().min(MAX_PART_BYTES) as usize);
        file.take(MAX_PART_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| malformed(e.to_string()))?;
        if bytes.len() as u64 > MAX_PART_BYTES {
            return Err(malformed(format!(
                "larger than {} MiB once decompressed",
                MAX_PART_BYTES >> 20
            )));
        }
        if bytes.starts_with(b"\xEF\xBB\xBF") {
            bytes.drain(..3);
        }
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| malformed("not UTF-8 text".into()))
    }
}
