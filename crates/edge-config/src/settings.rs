use std::collections::BTreeMap;

use crate::ConfigError;

/// A setting exactly as the reference slicer serialises it: every value is
/// text, either alone or as a list with one entry per extruder or filament.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    One(String),
    Many(Vec<String>),
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
