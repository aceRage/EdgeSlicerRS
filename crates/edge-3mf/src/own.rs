//! Settings set on an object, a part or a height range of an object, as the
//! reference keeps them with the model (`ModelObject::config`,
//! `ModelVolume::config`, `ModelObject::layer_config_ranges`) and writes them
//! (bbs_3mf.cpp `_add_model_config_file_to_archive`: a `<metadata>` for each
//! key of an object's or a part's config; `_add_layer_config_ranges_file_to_archive`:
//! `Metadata/layer_config_ranges.xml`).
//!
//! What a file held is in [`ObjectKept`] and [`PartKept`]; the edits here
//! change it there, so the writer writes what is set as it writes what was
//! read. Nothing here slices by them.

use crate::kept::{Entry, Kept, ObjectKept, PartKept, is_setting};

/// One height range of an object ("Height range Modifier"): a band of the
/// object's height, measured from its bottom, with settings of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct HeightRange {
    /// Its bottom and top, in mm.
    pub min_z: f64,
    pub max_z: f64,
    /// What it sets, key and value as the reference writes them
    /// (`opt_serialize`), in the order written.
    pub settings: Vec<(String, String)>,
}

/// Sets `key` among `entries` in place, or adds it after the last
/// metadata (before what is not metadata, such as `mesh_stat`); none takes
/// it away.
fn set_in(entries: &mut Vec<Entry>, key: &str, value: Option<&str>) {
    let at = entries.iter().position(|entry| entry.key() == Some(key));
    match (at, value) {
        (Some(at), Some(value)) => {
            entries[at] = Entry::Meta {
                key: key.to_owned(),
                value: value.to_owned(),
            };
        }
        (Some(at), None) => {
            entries.remove(at);
        }
        (None, Some(value)) => {
            let at = entries
                .iter()
                .rposition(|entry| matches!(entry, Entry::Meta { .. }))
                .map_or(0, |at| at + 1);
            entries.insert(
                at,
                Entry::Meta {
                    key: key.to_owned(),
                    value: value.to_owned(),
                },
            );
        }
        (None, None) => {}
    }
}

/// The settings among `entries`, in order.
fn settings_in(entries: &[Entry]) -> Vec<(String, String)> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Meta { key, value } if is_setting(key) => Some((key.clone(), value.clone())),
            _ => None,
        })
        .collect()
}

impl ObjectKept {
    /// What an object that no file held keeps, once something is set on
    /// it here.
    pub fn made_here() -> ObjectKept {
        ObjectKept::default()
    }

    /// The object's own settings: every key and value of its metadata but
    /// its name and filament (and what is about where it came from), in
    /// order.
    pub fn own_settings(&self) -> Vec<(String, String)> {
        settings_in(&self.entries)
    }

    /// The object with one of its own settings set (`value`, as the
    /// reference writes it) or taken away (none).
    pub fn with_own_setting(&self, key: &str, value: Option<&str>) -> ObjectKept {
        let mut object = self.clone();
        set_in(&mut object.entries, key, value);
        object
    }

    /// Its height ranges, as `Metadata/layer_config_ranges.xml` has them
    /// (bbs_3mf.cpp `_extract_layer_config_ranges_from_archive`), in the
    /// order written. A range whose heights cannot be read is left out, as
    /// the reference's reader stops at it.
    pub fn height_ranges(&self) -> Vec<HeightRange> {
        self.layer_ranges.as_deref().map(parse).unwrap_or_default()
    }

    /// The object with these height ranges, written as the reference
    /// writes them; none takes them all away.
    pub fn with_height_ranges(&self, ranges: &[HeightRange]) -> ObjectKept {
        let mut object = self.clone();
        object.layer_ranges = (!ranges.is_empty()).then(|| written(ranges));
        object
    }
}

impl PartKept {
    /// What a part that no file held keeps, once something is set on it
    /// here.
    pub fn made_here() -> PartKept {
        PartKept::default()
    }

    /// The part with one of its own settings set (`value`, as the
    /// reference writes it) or taken away (none).
    pub fn with_own_setting(&self, key: &str, value: Option<&str>) -> PartKept {
        let mut part = self.clone();
        set_in(&mut part.entries, key, value);
        part
    }
}

impl Kept {
    /// Whether the file has the height colour patterns of the reference's
    /// "Color Pattern..." (`Metadata/edgeslicer_height_patterns.json`), whose
    /// bands are height ranges of their own that this app does not read.
    pub fn has_height_patterns(&self) -> bool {
        self.files.iter().any(|file| {
            file.name
                .eq_ignore_ascii_case("Metadata/edgeslicer_height_patterns.json")
        })
    }
}

/// The ranges inside one `<object>` of `layer_config_ranges.xml`. The
/// reference writes the file with Boost's XML and every tag on a line of
/// its own; it is read here by its tags.
fn parse(text: &str) -> Vec<HeightRange> {
    let unescaped = |value: &str| -> String {
        quick_xml::escape::unescape(value)
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| value.to_owned())
    };
    let attribute = |tag: &str, name: &str| -> Option<String> {
        let pattern = format!(" {name}=\"");
        let at = tag.find(&pattern)? + pattern.len();
        let rest = &tag[at..];
        Some(unescaped(&rest[..rest.find('"')?]))
    };
    let mut ranges = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<range") {
        let tail = &rest[start..];
        let Some(head_end) = tail.find('>') else {
            break;
        };
        let head = &tail[..head_end];
        let (body, next) = if head.ends_with('/') {
            ("", &tail[head_end + 1..])
        } else {
            match tail.find("</range>") {
                Some(close) => (
                    &tail[head_end + 1..close],
                    &tail[close + "</range>".len()..],
                ),
                None => (&tail[head_end + 1..], ""),
            }
        };
        rest = next;
        let height =
            |name| attribute(head, name).and_then(|value| value.trim().parse::<f64>().ok());
        let (Some(min_z), Some(max_z)) = (height("min_z"), height("max_z")) else {
            continue;
        };
        let mut settings = Vec::new();
        let mut options = body;
        while let Some(at) = options.find("<option") {
            let element = &options[at..];
            let Some(open_end) = element.find('>') else {
                break;
            };
            let key = attribute(&element[..open_end], "opt_key").unwrap_or_default();
            let after = &element[open_end + 1..];
            let close = after.find("</option>").unwrap_or(after.len());
            if !key.is_empty() {
                settings.push((key, unescaped(after[..close].trim())));
            }
            options = &after[close..];
        }
        ranges.push(HeightRange {
            min_z,
            max_z,
            settings,
        });
    }
    ranges
}

/// The inside of an `<object>` of `layer_config_ranges.xml`, as the
/// reference lays it out (Boost's XML, then every tag on a line of its own,
/// a range indented two, an option three), the writer putting the
/// `<object>` round it.
fn written(ranges: &[HeightRange]) -> String {
    let mut out = String::new();
    for range in ranges {
        out.push_str(&format!(
            "\n  <range min_z=\"{}\" max_z=\"{}\">",
            range.min_z, range.max_z
        ));
        for (key, value) in &range.settings {
            out.push_str(&format!(
                "\n   <option opt_key=\"{}\">{}</option>",
                crate::write::escape(key),
                crate::write::escape(value)
            ));
        }
        out.push_str("\n  </range>");
    }
    out.push_str("\n ");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_written_as_the_reference_lays_them_out_and_read_back() {
        let ranges = vec![
            HeightRange {
                min_z: 0.0,
                max_z: 2.0,
                settings: vec![
                    ("extruder".into(), "0".into()),
                    ("layer_height".into(), "0.1".into()),
                ],
            },
            HeightRange {
                min_z: 2.0,
                max_z: 4.5,
                settings: vec![("sparse_infill_pattern".into(), "<gyroid & co>".into())],
            },
        ];
        let object = ObjectKept::made_here().with_height_ranges(&ranges);
        assert_eq!(
            object.layer_ranges.as_deref(),
            Some(
                "\n  <range min_z=\"0\" max_z=\"2\">\n   <option opt_key=\"extruder\">0</option>\n   <option opt_key=\"layer_height\">0.1</option>\n  </range>\n  <range min_z=\"2\" max_z=\"4.5\">\n   <option opt_key=\"sparse_infill_pattern\">&lt;gyroid &amp; co&gt;</option>\n  </range>\n "
            )
        );
        assert_eq!(object.height_ranges(), ranges);
        assert_eq!(object.with_height_ranges(&[]).layer_ranges, None);
    }

    #[test]
    fn a_setting_is_set_in_place_added_after_the_metadata_and_taken_away() {
        let part = PartKept {
            entries: vec![
                Entry::Meta {
                    key: "name".into(),
                    value: "a".into(),
                },
                Entry::Meta {
                    key: "wall_loops".into(),
                    value: "2".into(),
                },
                Entry::Raw("<mesh_stat/>".into()),
            ],
            ..PartKept::default()
        };
        let set = part
            .with_own_setting("wall_loops", Some("5"))
            .with_own_setting("sparse_infill_density", Some("60%"));
        assert_eq!(
            set.settings(),
            vec![
                ("wall_loops".to_owned(), "5".to_owned()),
                ("sparse_infill_density".to_owned(), "60%".to_owned())
            ]
        );
        assert!(matches!(set.entries.last(), Some(Entry::Raw(_))));
        let back = set
            .with_own_setting("sparse_infill_density", None)
            .with_own_setting("wall_loops", Some("2"));
        assert_eq!(back, part);
        let object = ObjectKept::made_here().with_own_setting("wall_loops", Some("5"));
        assert_eq!(
            object.own_settings(),
            vec![("wall_loops".to_owned(), "5".to_owned())]
        );
    }
}
