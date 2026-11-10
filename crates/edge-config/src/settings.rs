use std::collections::BTreeMap;

use crate::ConfigError;

/// A setting exactly as the reference slicer serialises it: every value is
/// text, either alone or as a list with one entry per extruder or filament.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    One(String),
    Many(Vec<String>),
}

/// A length given outright in mm or as a percentage of some other length,
/// which the setting's meaning decides (a line width is a percentage of the
/// nozzle diameter, for instance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FloatOrPercent {
    Value(f64),
    Percent(f64),
}

impl FloatOrPercent {
    pub fn resolve(self, whole: f64) -> f64 {
        match self {
            Self::Value(value) => value,
            Self::Percent(percent) => whole * percent / 100.0,
        }
    }
}

/// Flat key to value settings, ordered by key so output is deterministic.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings(BTreeMap<String, Value>);

impl Settings {
    /// Parses a JSON object of strings and string lists. Numbers and booleans
    /// are accepted and kept as their text; nested structures are rejected.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let json: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let serde_json::Value::Object(object) = json else {
            return Err("top level is not an object".into());
        };
        let mut settings = BTreeMap::new();
        for (key, value) in object {
            let value = match value {
                serde_json::Value::Array(items) => Value::Many(
                    items
                        .into_iter()
                        .map(|item| {
                            scalar(item).ok_or_else(|| format!("{key} holds a nested value"))
                        })
                        .collect::<Result<_, _>>()?,
                ),
                other => {
                    Value::One(scalar(other).ok_or_else(|| format!("{key} is not text or a list"))?)
                }
            };
            settings.insert(key, value);
        }
        Ok(Self(settings))
    }

    pub fn to_json(&self) -> String {
        let object: serde_json::Map<String, serde_json::Value> = self
            .0
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    Value::One(text) => serde_json::Value::from(text.as_str()),
                    Value::Many(items) => serde_json::Value::from(items.as_slice()),
                };
                (key.clone(), value)
            })
            .collect();
        serde_json::to_string_pretty(&object).expect("strings always serialise")
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn insert(&mut self, key: impl Into<String>, value: Value) -> Option<Value> {
        self.0.insert(key.into(), value)
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.0.remove(key)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.iter().map(|(key, value)| (key.as_str(), value))
    }

    /// Copies every setting this one lacks from `parent`, except `skip` keys.
    pub fn inherit(&mut self, parent: &Settings, skip: impl Fn(&str) -> bool) {
        for (key, value) in &parent.0 {
            if !skip(key) && !self.0.contains_key(key) {
                self.0.insert(key.clone(), value.clone());
            }
        }
    }

    /// Takes every setting from `other`, replacing those already present.
    pub fn overlay(&mut self, other: &Settings) {
        for (key, value) in &other.0 {
            self.0.insert(key.clone(), value.clone());
        }
    }

    /// A single-valued setting; `None` when absent or stored as a list.
    pub fn text(&self, key: &str) -> Option<&str> {
        match self.0.get(key)? {
            Value::One(text) => Some(text),
            Value::Many(_) => None,
        }
    }

    /// A setting's entries; a single value reads as a one-entry list.
    pub fn texts(&self, key: &str) -> Option<&[String]> {
        Some(match self.0.get(key)? {
            Value::One(text) => std::slice::from_ref(text),
            Value::Many(items) => items,
        })
    }

    pub fn float(&self, key: &str) -> Result<Option<f64>, ConfigError> {
        let Some(value) = self.0.get(key) else {
            return Ok(None);
        };
        let Value::One(text) = value else {
            return Err(bad(key, "a list", "a single number"));
        };
        parse_float(key, text).map(Some)
    }

    /// A whole number, such as a count of walls.
    pub fn integer(&self, key: &str) -> Result<Option<i64>, ConfigError> {
        let Some(text) = self.first(key) else {
            return Ok(None);
        };
        text.trim()
            .parse()
            .map(Some)
            .map_err(|_| bad(key, text, "a whole number"))
    }

    /// An on/off setting, stored as `1` or `0`.
    pub fn flag(&self, key: &str) -> Result<Option<bool>, ConfigError> {
        match self.first(key).map(str::trim) {
            None => Ok(None),
            Some("1" | "true") => Ok(Some(true)),
            Some("0" | "false") => Ok(Some(false)),
            Some(other) => Err(bad(key, other, "1 or 0")),
        }
    }

    /// A number with an optional trailing `%`.
    pub fn float_or_percent(&self, key: &str) -> Result<Option<FloatOrPercent>, ConfigError> {
        let Some(text) = self.first(key) else {
            return Ok(None);
        };
        let text = text.trim();
        Ok(Some(match text.strip_suffix('%') {
            Some(percent) => FloatOrPercent::Percent(parse_float(key, percent)?),
            None => FloatOrPercent::Value(parse_float(key, text)?),
        }))
    }

    /// The value itself, or the first entry of a per-extruder list.
    fn first(&self, key: &str) -> Option<&str> {
        self.texts(key)?.first().map(String::as_str)
    }

    /// Per-extruder or per-filament numbers. An entry of `nil` means "not
    /// overridden here" and reads as `None`.
    pub fn floats(&self, key: &str) -> Result<Option<Vec<Option<f64>>>, ConfigError> {
        let Some(items) = self.texts(key) else {
            return Ok(None);
        };
        items
            .iter()
            .map(|item| {
                if item == "nil" {
                    Ok(None)
                } else {
                    parse_float(key, item).map(Some)
                }
            })
            .collect::<Result<_, _>>()
            .map(Some)
    }
}

fn scalar(value: serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn parse_float(key: &str, text: &str) -> Result<f64, ConfigError> {
    match text.trim().parse::<f64>() {
        Ok(number) if number.is_finite() => Ok(number),
        _ => Err(bad(key, text, "a number")),
    }
}

fn bad(key: &str, found: &str, expected: &'static str) -> ConfigError {
    ConfigError::BadValue {
        key: key.into(),
        found: found.into(),
        expected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trips_and_keeps_text_exact() {
        let source = r#"{"layer_height": "0.20", "nozzle_diameter": ["0.4", "0.6"], "count": 3, "on": true}"#;
        let settings = Settings::from_json(source).unwrap();
        assert_eq!(settings.text("layer_height"), Some("0.20"));
        assert_eq!(settings.text("count"), Some("3"));
        assert_eq!(settings.text("on"), Some("true"));
        assert_eq!(settings.texts("nozzle_diameter").unwrap(), ["0.4", "0.6"]);
        assert_eq!(Settings::from_json(&settings.to_json()).unwrap(), settings);
    }

    #[test]
    fn rejects_shapes_presets_never_use() {
        assert!(Settings::from_json("[]").is_err());
        assert!(Settings::from_json(r#"{"a": {"b": "c"}}"#).is_err());
        assert!(Settings::from_json(r#"{"a": [["b"]]}"#).is_err());
        assert!(Settings::from_json(r#"{"a": null}"#).is_err());
    }

    #[test]
    fn numbers_parse_strictly() {
        let settings = Settings::from_json(
            r#"{"h": "0.2", "pct": "50%", "temps": ["210", "nil"], "list": ["1", "2"], "one": "7"}"#,
        )
        .unwrap();
        assert_eq!(settings.float("h").unwrap(), Some(0.2));
        assert_eq!(settings.float("missing").unwrap(), None);
        assert!(settings.float("pct").is_err());
        assert!(settings.float("list").is_err());
        assert_eq!(
            settings.floats("temps").unwrap(),
            Some(vec![Some(210.0), None])
        );
        assert_eq!(settings.floats("one").unwrap(), Some(vec![Some(7.0)]));
    }

    #[test]
    fn typed_values_read_scalars_and_first_list_entries() {
        let settings = Settings::from_json(
            r#"{"wall_loops": "3", "overlap": "15%", "width": ["0.42", "0.6"], "arc": "1", "bad": "x"}"#,
        )
        .unwrap();
        assert_eq!(settings.integer("wall_loops").unwrap(), Some(3));
        assert_eq!(
            settings.float_or_percent("overlap").unwrap(),
            Some(FloatOrPercent::Percent(15.0))
        );
        assert_eq!(
            settings.float_or_percent("width").unwrap(),
            Some(FloatOrPercent::Value(0.42))
        );
        assert_eq!(settings.flag("arc").unwrap(), Some(true));
        assert_eq!(settings.flag("missing").unwrap(), None);
        assert!(settings.integer("overlap").is_err() && settings.flag("bad").is_err());
        assert_eq!(FloatOrPercent::Percent(150.0).resolve(0.4), 0.6);
        assert_eq!(FloatOrPercent::Value(0.5).resolve(0.4), 0.5);
    }

    #[test]
    fn overlay_replaces_and_adds() {
        let mut base = Settings::from_json(r#"{"a": "1", "b": "1"}"#).unwrap();
        base.overlay(&Settings::from_json(r#"{"b": "2", "c": "2"}"#).unwrap());
        assert_eq!(
            base.to_json(),
            Settings::from_json(r#"{"a": "1", "b": "2", "c": "2"}"#)
                .unwrap()
                .to_json()
        );
    }

    #[test]
    fn inherit_fills_gaps_only() {
        let mut child = Settings::from_json(r#"{"a": "child", "name": "c"}"#).unwrap();
        let parent =
            Settings::from_json(r#"{"a": "parent", "b": "parent", "name": "p", "x": "p"}"#)
                .unwrap();
        child.inherit(&parent, |key| key == "x");
        assert_eq!(child.text("a"), Some("child"));
        assert_eq!(child.text("b"), Some("parent"));
        assert_eq!(child.text("name"), Some("c"));
        assert_eq!(child.get("x"), None);
    }
}
