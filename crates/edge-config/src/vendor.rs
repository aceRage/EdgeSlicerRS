use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::OnceLock,
};

use crate::{ConfigError, Settings, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A printer model: bed shape assets and the nozzle sizes it is sold with.
    MachineModel,
    /// A printer at one nozzle size.
    Machine,
    Process,
    Filament,
}

impl Kind {
    pub const ALL: [Kind; 4] = [
        Kind::MachineModel,
        Kind::Machine,
        Kind::Process,
        Kind::Filament,
    ];

    /// The list in a vendor's index file that names presets of this kind.
    fn section(self) -> &'static str {
        match self {
            Kind::MachineModel => "machine_model_list",
            Kind::Machine => "machine_list",
            Kind::Process => "process_list",
            Kind::Filament => "filament_list",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::MachineModel => "machine-model",
            Kind::Machine => "machine",
            Kind::Process => "process",
            Kind::Filament => "filament",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl FromStr for Kind {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Kind::ALL
            .into_iter()
            .find(|kind| kind.label() == text)
            .ok_or_else(|| format!("unknown preset kind \"{text}\"; use machine-model, machine, process or filament"))
    }
}

/// Keys that describe one preset file rather than a setting, so a preset
/// never takes them from the presets it inherits.
const IDENTITY_KEYS: [&str; 6] = [
    "name",
    "inherits",
    "instantiation",
    "setting_id",
    "type",
    "from",
];

/// Keys an included template never contributes. Same list as the reference
/// (ConfigBase::parse_json_document), matched case-insensitively as it does.
const INCLUDE_SKIPPED_KEYS: [&str; 12] = [
    "include",
    "name",
    "instantiation",
    "from",
    "inherits",
    "type",
    "setting_id",
    "filament_id",
    "version",
    "url",
    "description",
    "is_custom_defined",
];

/// One vendor's presets: an index of names to files, read lazily on resolve.
#[derive(Debug)]
pub struct Vendor {
    name: String,
    presets: [Section; 4],
    /// The shared filament library other vendors' filaments inherit from.
    library: Option<Box<Vendor>>,
}

#[derive(Debug, Default)]
struct Section {
    /// In index order, which is the order the reference lists them in.
    names: Vec<String>,
    paths: HashMap<String, PathBuf>,
    /// Former preset names to current ones, from each file's `renamed_from`.
    /// Finding them means reading every file in the section, so this is only
    /// built the first time a name is not found directly.
    renames: OnceLock<HashMap<String, String>>,
}

impl Section {
    fn renamed(&self, old: &str) -> Option<&str> {
        let renames = self.renames.get_or_init(|| {
            let mut renames = HashMap::new();
            for name in &self.names {
                // A file that cannot be read reports itself when it is resolved.
                let Ok(preset) = parse(&self.paths[name]) else {
                    continue;
                };
                for old in preset.text("renamed_from").unwrap_or_default().split(';') {
                    let old = old.trim();
                    if !old.is_empty() {
                        renames
                            .entry(old.to_owned())
                            .or_insert_with(|| name.clone());
                    }
                }
            }
            renames
        });
        renames.get(old).map(String::as_str)
    }
}

impl Vendor {
    pub const FILAMENT_LIBRARY: &'static str = "OrcaFilamentLibrary";

    /// Names of the vendors under a profiles directory, sorted.
    pub fn list(profiles_dir: &Path) -> Result<Vec<String>, ConfigError> {
        let io = |source| ConfigError::Io {
            path: profiles_dir.to_owned(),
            source,
        };
        let mut names = Vec::new();
        for entry in fs::read_dir(profiles_dir).map_err(io)? {
            let path = entry.map_err(io)?.path();
            let is_index = path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
                && is_index
            {
                names.push(stem.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    /// Reads `<profiles_dir>/<name>.json`, the vendor's index of presets.
    pub fn open(profiles_dir: &Path, name: &str) -> Result<Self, ConfigError> {
        let index_path = profiles_dir.join(format!("{name}.json"));
        let text = read(&index_path)?;
        let json_error = |reason: String| ConfigError::Json {
            path: index_path.clone(),
            reason,
        };
        let index: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| json_error(e.to_string()))?;

        let vendor_dir = profiles_dir.join(name);
        let mut presets: [Section; 4] = Default::default();
        for (kind, section) in Kind::ALL.into_iter().zip(&mut presets) {
            let Some(entries) = index.get(kind.section()) else {
                continue;
            };
            let entries = entries
                .as_array()
                .ok_or_else(|| json_error(format!("{} is not a list", kind.section())))?;
            for entry in entries {
                let field = |key: &str| {
                    entry
                        .get(key)
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| {
                            json_error(format!("an entry of {} has no {key}", kind.section()))
                        })
                };
                let (preset, sub_path) = (field("name")?, field("sub_path")?);
                // The first entry wins when a name is listed twice.
                if !section.paths.contains_key(preset) {
                    section.names.push(preset.to_owned());
                    section
                        .paths
                        .insert(preset.to_owned(), vendor_dir.join(sub_path));
                }
            }
        }

        let library_index = profiles_dir.join(format!("{}.json", Self::FILAMENT_LIBRARY));
        let library = if name != Self::FILAMENT_LIBRARY && library_index.is_file() {
            Some(Box::new(Self::open(profiles_dir, Self::FILAMENT_LIBRARY)?))
        } else {
            None
        };
        Ok(Self {
            name: name.to_owned(),
            presets,
            library,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Preset names of one kind, in the vendor index's order. Includes
    /// abstract bases; check `instantiation` on the resolved preset.
    pub fn names(&self, kind: Kind) -> impl Iterator<Item = &str> {
        self.section(kind).names.iter().map(String::as_str)
    }

    /// Flattens a preset: its own settings, then each ancestor's for whatever
    /// is still unset. Filaments may continue into the shared filament library.
    /// A name a preset used to have (`renamed_from`) finds the current preset,
    /// which is how project files saved by older versions keep resolving.
    pub fn resolve(&self, kind: Kind, name: &str) -> Result<Settings, ConfigError> {
        let mut resolved: Option<Settings> = None;
        let mut chain: Vec<(&str, &str)> = Vec::new();
        let (mut vendor, mut wanted) = (self, name.to_owned());
        loop {
            let Some((owner, current, path)) = vendor.locate(kind, &wanted) else {
                return Err(ConfigError::UnknownPreset {
                    vendor: self.name.clone(),
                    kind,
                    name: wanted,
                });
            };
            // Once a chain enters the library it stays there.
            vendor = owner;
            if chain.contains(&(vendor.name.as_str(), current)) {
                let mut names: Vec<&str> = chain.iter().map(|(_, seen)| *seen).collect();
                names.push(current);
                return Err(ConfigError::InheritanceCycle {
                    name: name.to_owned(),
                    chain: names.join(" -> "),
                });
            }
            chain.push((&vendor.name, current));

            let own = load_preset(path)?;
            let parent = own
                .text("inherits")
                .filter(|parent| !parent.is_empty())
                .map(str::to_owned);
            match &mut resolved {
                None => resolved = Some(own),
                Some(settings) => settings.inherit(&own, |key| IDENTITY_KEYS.contains(&key)),
            }
            match parent {
                Some(parent) => wanted = parent,
                None => return Ok(resolved.expect("the loop stores the leaf before it can end")),
            }
        }
    }

    /// Finds a preset by its current name here or in the library, and only
    /// then by a former name, returning its owner, current name and file.
    fn locate(&self, kind: Kind, name: &str) -> Option<(&Vendor, &str, &Path)> {
        let library = self.library.as_deref().filter(|_| kind == Kind::Filament);
        let owners = [Some(self), library];
        for owner in owners.into_iter().flatten() {
            if let Some((current, path)) = owner.section(kind).paths.get_key_value(name) {
                return Some((owner, current, path));
            }
        }
        for owner in owners.into_iter().flatten() {
            let section = owner.section(kind);
            if let Some((current, path)) = section
                .renamed(name)
                .and_then(|now| section.paths.get_key_value(now))
            {
                return Some((owner, current, path));
            }
        }
        None
    }

    fn section(&self, kind: Kind) -> &Section {
        &self.presets[Kind::ALL
            .iter()
            .position(|k| *k == kind)
            .expect("ALL lists every kind")]
    }
}

/// Reads one preset file and folds in the templates it includes.
///
/// Includes follow the reference: a name resolves against the including
/// file's directory, then against the nearest ancestor named `filament`,
/// `process` or `machine`; the including file's own keys win, then earlier
/// includes over later ones; templates are not themselves expanded. Unlike
/// the reference, which logs a warning and carries on, an include that
/// resolves nowhere is an error here: a preset silently missing its start
/// G-code is worse than one that refuses to load.
fn load_preset(path: &Path) -> Result<Settings, ConfigError> {
    let mut settings = parse(path)?;
    let Some(includes) = settings.remove("include") else {
        return Ok(settings);
    };
    let Value::Many(includes) = includes else {
        return Ok(settings);
    };

    let own_dir = path.parent().unwrap_or(Path::new(""));
    let category_dir = own_dir.ancestors().find(|dir| {
        dir.file_name()
            .and_then(|leaf| leaf.to_str())
            .is_some_and(|leaf| {
                ["filament", "process", "machine"]
                    .iter()
                    .any(|category| leaf.eq_ignore_ascii_case(category))
            })
    });
    for include in includes {
        let file = if include.to_ascii_lowercase().ends_with(".json") {
            include.clone()
        } else {
            format!("{include}.json")
        };
        let template = [Some(own_dir), category_dir]
            .into_iter()
            .flatten()
            .map(|dir| dir.join(&file))
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| ConfigError::MissingInclude {
                path: path.to_owned(),
                include,
            })?;
        settings.inherit(&parse(&template)?, |key| {
            INCLUDE_SKIPPED_KEYS
                .iter()
                .any(|skipped| key.eq_ignore_ascii_case(skipped))
        });
    }
    Ok(settings)
}

fn parse(path: &Path) -> Result<Settings, ConfigError> {
    Settings::from_json(&read(path)?).map_err(|reason| ConfigError::Json {
        path: path.to_owned(),
        reason,
    })
}

fn read(path: &Path) -> Result<String, ConfigError> {
    let bytes = fs::read(path).map_err(|source| ConfigError::Io {
        path: path.to_owned(),
        source,
    })?;
    // Some shipped profiles start with a UTF-8 byte-order mark.
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes));
    Ok(text.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes `files` (path relative to the profiles directory, JSON text).
    fn profiles(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    const INDEX: &str = r#"{
        "name": "Acme",
        "process_list": [
            {"name": "base", "sub_path": "process/base.json"},
            {"name": "0.20 Standard", "sub_path": "process/standard.json"},
            {"name": "0.20 Standard", "sub_path": "process/ignored.json"},
            {"name": "a", "sub_path": "process/a.json"},
            {"name": "b", "sub_path": "process/b.json"},
            {"name": "orphan", "sub_path": "process/orphan.json"}
        ],
        "filament_list": [{"name": "Acme PLA", "sub_path": "filament/pla.json"}],
        "machine_list": [{"name": "Acme One", "sub_path": "machine/sub/one.json"}]
    }"#;

    #[test]
    fn child_overrides_and_identity_stays_with_the_leaf() {
        let dir = profiles(&[
            ("Acme.json", INDEX),
            (
                "Acme/process/base.json",
                r#"{"name": "base", "instantiation": "false", "setting_id": "B", "layer_height": "0.3", "wall_loops": "2"}"#,
            ),
            (
                "Acme/process/standard.json",
                r#"{"name": "0.20 Standard", "inherits": "base", "layer_height": "0.2"}"#,
            ),
        ]);
        let vendor = Vendor::open(dir.path(), "Acme").unwrap();
        assert_eq!(
            vendor.names(Kind::Process).collect::<Vec<_>>(),
            ["base", "0.20 Standard", "a", "b", "orphan"]
        );

        let preset = vendor.resolve(Kind::Process, "0.20 Standard").unwrap();
        assert_eq!(preset.text("layer_height"), Some("0.2"));
        assert_eq!(preset.text("wall_loops"), Some("2"));
        assert_eq!(preset.text("name"), Some("0.20 Standard"));
        assert_eq!(preset.text("inherits"), Some("base"));
        assert_eq!(preset.get("instantiation"), None);
        assert_eq!(preset.get("setting_id"), None);
    }

    #[test]
    fn broken_chains_are_errors() {
        let dir = profiles(&[
            ("Acme.json", INDEX),
            ("Acme/process/a.json", r#"{"name": "a", "inherits": "b"}"#),
            ("Acme/process/b.json", r#"{"name": "b", "inherits": "a"}"#),
            (
                "Acme/process/orphan.json",
                r#"{"name": "orphan", "inherits": "gone"}"#,
            ),
        ]);
        let vendor = Vendor::open(dir.path(), "Acme").unwrap();
        match vendor.resolve(Kind::Process, "a") {
            Err(ConfigError::InheritanceCycle { chain, .. }) => assert_eq!(chain, "a -> b -> a"),
            other => panic!("expected a cycle, got {other:?}"),
        }
        assert!(matches!(
            vendor.resolve(Kind::Process, "orphan"),
            Err(ConfigError::UnknownPreset { name, .. }) if name == "gone"
        ));
        assert!(matches!(
            vendor.resolve(Kind::Machine, "base"),
            Err(ConfigError::UnknownPreset { .. })
        ));
        assert!(matches!(
            vendor.resolve(Kind::Process, "base"),
            Err(ConfigError::Io { .. })
        ));
    }

    #[test]
    fn filaments_continue_into_the_shared_library_only() {
        let dir = profiles(&[
            ("Acme.json", INDEX),
            (
                "Acme/filament/pla.json",
                r#"{"name": "Acme PLA", "inherits": "Library PLA", "filament_cost": ["30"]}"#,
            ),
            (
                "OrcaFilamentLibrary.json",
                r#"{"filament_list": [{"name": "Library PLA", "sub_path": "filament/pla.json"}],
                    "process_list": [{"name": "gone", "sub_path": "process/gone.json"}]}"#,
            ),
            (
                "OrcaFilamentLibrary/filament/pla.json",
                r#"{"name": "Library PLA", "filament_cost": ["20"], "filament_type": ["PLA"]}"#,
            ),
            (
                "OrcaFilamentLibrary/process/gone.json",
                r#"{"name": "gone"}"#,
            ),
            (
                "Acme/process/orphan.json",
                r#"{"name": "orphan", "inherits": "gone"}"#,
            ),
        ]);
        let vendor = Vendor::open(dir.path(), "Acme").unwrap();
        let pla = vendor.resolve(Kind::Filament, "Acme PLA").unwrap();
        assert_eq!(pla.texts("filament_cost").unwrap(), ["30"]);
        assert_eq!(pla.texts("filament_type").unwrap(), ["PLA"]);
        assert!(vendor.resolve(Kind::Process, "orphan").is_err());
        assert_eq!(
            Vendor::list(dir.path()).unwrap(),
            ["Acme", "OrcaFilamentLibrary"]
        );
    }

    #[test]
    fn includes_fill_gaps_from_own_directory_then_category_root() {
        let dir = profiles(&[
            ("Acme.json", INDEX),
            (
                "Acme/machine/sub/one.json",
                r#"{"name": "Acme One", "include": ["near", "far.JSON", "second"], "machine_start_gcode": "own"}"#,
            ),
            (
                "Acme/machine/sub/near.json",
                r#"{"name": "near", "Setting_ID": "X", "machine_start_gcode": "near", "machine_end_gcode": "near"}"#,
            ),
            (
                "Acme/machine/near.json",
                r#"{"machine_end_gcode": "shadowed by the nearer file"}"#,
            ),
            (
                "Acme/machine/far.JSON",
                r#"{"layer_change_gcode": "far", "include": ["never expanded"]}"#,
            ),
            (
                "Acme/machine/second.json",
                r#"{"machine_end_gcode": "second", "layer_change_gcode": "second"}"#,
            ),
        ]);
        let vendor = Vendor::open(dir.path(), "Acme").unwrap();
        let machine = vendor.resolve(Kind::Machine, "Acme One").unwrap();
        assert_eq!(machine.text("machine_start_gcode"), Some("own"));
        assert_eq!(machine.text("machine_end_gcode"), Some("near"));
        assert_eq!(machine.text("layer_change_gcode"), Some("far"));
        assert_eq!(machine.text("name"), Some("Acme One"));
        assert_eq!(machine.get("Setting_ID"), None);
        assert_eq!(machine.get("include"), None);
    }

    #[test]
    fn missing_include_is_an_error() {
        let dir = profiles(&[
            ("Acme.json", INDEX),
            (
                "Acme/machine/sub/one.json",
                r#"{"name": "Acme One", "include": ["absent"]}"#,
            ),
        ]);
        let vendor = Vendor::open(dir.path(), "Acme").unwrap();
        assert!(matches!(
            vendor.resolve(Kind::Machine, "Acme One"),
            Err(ConfigError::MissingInclude { include, .. }) if include == "absent"
        ));
    }

    #[test]
    fn former_names_find_the_current_preset() {
        let dir = profiles(&[
            ("Acme.json", INDEX),
            (
                "Acme/process/base.json",
                r#"{"name": "base", "renamed_from": "old base; older base ;"}"#,
            ),
            (
                "Acme/process/standard.json",
                r#"{"name": "0.20 Standard", "inherits": "older base", "renamed_from": "base"}"#,
            ),
        ]);
        let vendor = Vendor::open(dir.path(), "Acme").unwrap();
        assert_eq!(
            vendor
                .resolve(Kind::Process, "old base")
                .unwrap()
                .text("name"),
            Some("base")
        );
        // A current name is never shadowed by another preset's former name.
        assert_eq!(
            vendor.resolve(Kind::Process, "base").unwrap().text("name"),
            Some("base")
        );
        let standard = vendor.resolve(Kind::Process, "0.20 Standard").unwrap();
        assert_eq!(standard.text("renamed_from"), Some("base"));
        assert!(vendor.resolve(Kind::Process, "never existed").is_err());
    }

    #[test]
    fn kinds_parse_from_their_labels() {
        for kind in Kind::ALL {
            assert_eq!(kind.to_string().parse::<Kind>(), Ok(kind));
        }
        assert!("printer".parse::<Kind>().is_err());
    }
}
