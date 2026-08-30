use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Where a [`FlagSet`] came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagSource {
    /// Flagsmith answered and its values were merged over the defaults.
    Remote,
    /// Flagsmith is unconfigured, unreachable, or answered unusably.
    Defaults,
}

impl FlagSource {
    /// Lowercase wire form (`"remote"` / `"defaults"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Defaults => "defaults",
        }
    }
}

impl fmt::Display for FlagSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single evaluated flag: its on/off state plus its remote-config payload.
///
/// `value` mirrors Flagsmith's `feature_state_value` verbatim, which may be
/// `null`, a number, a string, or any other JSON value.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedFlag {
    pub enabled: bool,
    pub value: Value,
}

impl ResolvedFlag {
    pub fn new(enabled: bool, value: Value) -> Self {
        Self { enabled, value }
    }

    /// Enabled with no payload.
    pub fn on() -> Self {
        Self::new(true, Value::Null)
    }

    /// Disabled with no payload.
    pub fn off() -> Self {
        Self::new(false, Value::Null)
    }

    /// The payload as an integer.
    ///
    /// Flagsmith remote-config values are frequently typed as strings in the
    /// UI, so a numeric string is accepted as readily as a JSON number.
    pub fn as_int(&self) -> Option<i64> {
        match &self.value {
            Value::Number(number) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|float| float as i64)),
            Value::String(text) => text.trim().parse::<i64>().ok(),
            _ => None,
        }
    }

    /// The payload as a string, when it is a JSON string.
    pub fn as_str(&self) -> Option<&str> {
        self.value.as_str()
    }
}

/// Hardcoded fallbacks, one per known flag.
///
/// These are the values every surface must keep working with when Flagsmith is
/// unavailable, and the floor that remote values are merged over.
#[derive(Clone, Debug, Default)]
pub struct FlagDefaults {
    flags: BTreeMap<String, ResolvedFlag>,
}

impl FlagDefaults {
    pub fn new() -> Self {
        Self::default()
    }

    /// A boolean flag with no remote-config payload.
    pub fn bool(mut self, name: impl Into<String>, enabled: bool) -> Self {
        self.flags
            .insert(name.into(), ResolvedFlag::new(enabled, Value::Null));
        self
    }

    /// An enabled flag carrying an integer remote-config payload.
    pub fn int(mut self, name: impl Into<String>, value: i64) -> Self {
        self.flags
            .insert(name.into(), ResolvedFlag::new(true, Value::from(value)));
        self
    }

    /// An enabled flag carrying a string remote-config payload.
    pub fn string(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.flags.insert(
            name.into(),
            ResolvedFlag::new(true, Value::String(value.into())),
        );
        self
    }

    /// A fully specified default, for payloads that are neither int nor string.
    pub fn flag(mut self, name: impl Into<String>, flag: ResolvedFlag) -> Self {
        self.flags.insert(name.into(), flag);
        self
    }

    pub fn get(&self, name: &str) -> Option<&ResolvedFlag> {
        self.flags.get(name)
    }

    pub fn len(&self) -> usize {
        self.flags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    pub(crate) fn map(&self) -> &BTreeMap<String, ResolvedFlag> {
        &self.flags
    }
}

#[derive(Debug)]
pub(crate) struct FlagData {
    pub(crate) flags: BTreeMap<String, ResolvedFlag>,
    pub(crate) source: FlagSource,
}

/// An immutable, cheaply cloneable set of evaluated flags.
///
/// Every flag of the configured [`FlagDefaults`] is always present, so lookups
/// never need a "was it missing?" branch at the call site.
#[derive(Clone, Debug)]
pub struct FlagSet {
    inner: Arc<FlagData>,
}

impl FlagSet {
    pub(crate) fn new(flags: BTreeMap<String, ResolvedFlag>, source: FlagSource) -> Self {
        Self {
            inner: Arc::new(FlagData { flags, source }),
        }
    }

    pub(crate) fn from_data(inner: Arc<FlagData>) -> Self {
        Self { inner }
    }

    pub(crate) fn data(&self) -> &Arc<FlagData> {
        &self.inner
    }

    /// Whether `name` is on. An unknown flag is off.
    pub fn enabled(&self, name: &str) -> bool {
        self.inner.flags.get(name).is_some_and(|flag| flag.enabled)
    }

    /// The integer payload of `name`, if it has one.
    pub fn int(&self, name: &str) -> Option<i64> {
        self.inner.flags.get(name).and_then(ResolvedFlag::as_int)
    }

    /// The string payload of `name`, if it has one.
    pub fn string(&self, name: &str) -> Option<&str> {
        self.inner.flags.get(name).and_then(ResolvedFlag::as_str)
    }

    pub fn get(&self, name: &str) -> Option<&ResolvedFlag> {
        self.inner.flags.get(name)
    }

    pub fn source(&self) -> FlagSource {
        self.inner.source
    }

    pub fn len(&self) -> usize {
        self.inner.flags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.flags.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ResolvedFlag)> {
        self.inner
            .flags
            .iter()
            .map(|(name, flag)| (name.as_str(), flag))
    }

    /// The `flags` object of the `GET /api/flags` wire contract:
    /// `{ "<flag>": { "enabled": bool, "value": <json> } }`.
    pub fn to_json(&self) -> Value {
        let mut object = serde_json::Map::with_capacity(self.inner.flags.len());
        for (name, flag) in &self.inner.flags {
            let mut entry = serde_json::Map::with_capacity(2);
            entry.insert("enabled".to_string(), Value::Bool(flag.enabled));
            entry.insert("value".to_string(), flag.value.clone());
            object.insert(name.clone(), Value::Object(entry));
        }
        Value::Object(object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalogue() -> FlagDefaults {
        FlagDefaults::new()
            .bool("todo_write", true)
            .int("todo_max_items", -1)
    }

    #[test]
    fn resolved_flag_reads_ints_from_numbers_and_numeric_strings() {
        assert_eq!(ResolvedFlag::new(true, json!(7)).as_int(), Some(7));
        assert_eq!(ResolvedFlag::new(true, json!("7")).as_int(), Some(7));
        assert_eq!(ResolvedFlag::new(true, json!(" -3 ")).as_int(), Some(-3));
        assert_eq!(ResolvedFlag::new(true, json!(2.9)).as_int(), Some(2));
        assert_eq!(ResolvedFlag::new(true, json!("many")).as_int(), None);
        assert_eq!(ResolvedFlag::new(true, Value::Null).as_int(), None);
    }

    #[test]
    fn resolved_flag_reads_strings() {
        assert_eq!(
            ResolvedFlag::new(true, json!("blue")).as_str(),
            Some("blue")
        );
        assert_eq!(ResolvedFlag::new(true, json!(1)).as_str(), None);
    }

    #[test]
    fn flag_set_lookups_treat_unknown_flags_as_off() {
        let set = FlagSet::new(catalogue().map().clone(), FlagSource::Defaults);
        assert!(set.enabled("todo_write"));
        assert!(!set.enabled("todo_nope"));
        assert_eq!(set.int("todo_max_items"), Some(-1));
        assert_eq!(set.int("todo_nope"), None);
        assert_eq!(set.string("todo_max_items"), None);
        assert_eq!(set.source(), FlagSource::Defaults);
        assert_eq!(set.len(), 2);
        assert!(!set.is_empty());
    }

    #[test]
    fn flag_set_iterates_in_name_order() {
        let set = FlagSet::new(catalogue().map().clone(), FlagSource::Defaults);
        let names: Vec<&str> = set.iter().map(|(name, _)| name).collect();
        assert_eq!(names, vec!["todo_max_items", "todo_write"]);
    }

    #[test]
    fn to_json_matches_the_wire_contract() {
        let set = FlagSet::new(
            FlagDefaults::new()
                .bool("todo_app_web", true)
                .bool("todo_write", false)
                .int("todo_max_items", -1)
                .string("todo_theme", "dark")
                .map()
                .clone(),
            FlagSource::Remote,
        );
        assert_eq!(
            set.to_json(),
            json!({
                "todo_app_web": { "enabled": true, "value": null },
                "todo_max_items": { "enabled": true, "value": -1 },
                "todo_theme": { "enabled": true, "value": "dark" },
                "todo_write": { "enabled": false, "value": null },
            })
        );
    }

    #[test]
    fn flag_source_renders_lowercase() {
        assert_eq!(FlagSource::Remote.to_string(), "remote");
        assert_eq!(FlagSource::Defaults.as_str(), "defaults");
        assert_eq!(
            serde_json::to_value(FlagSource::Remote).ok(),
            Some(json!("remote"))
        );
    }

    #[test]
    fn defaults_builder_overwrites_by_name() {
        let defaults = FlagDefaults::new()
            .bool("todo_write", true)
            .bool("todo_write", false);
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults.get("todo_write"), Some(&ResolvedFlag::off()));
        assert!(FlagDefaults::new().is_empty());
        assert_eq!(
            FlagDefaults::new()
                .flag("todo_shape", ResolvedFlag::new(true, json!({ "a": 1 })))
                .get("todo_shape")
                .map(|flag| flag.value.clone()),
            Some(json!({ "a": 1 }))
        );
    }
}
