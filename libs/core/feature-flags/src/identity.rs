use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// The user Flagsmith should evaluate flags for, plus any traits that segments
/// may target.
///
/// Traits are held in a sorted map so the same logical identity always produces
/// the same request body and the same cache key regardless of insertion order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    identifier: String,
    traits: BTreeMap<String, Value>,
}

impl Identity {
    pub fn new(identifier: impl Into<String>) -> Self {
        Self {
            identifier: identifier.into(),
            traits: BTreeMap::new(),
        }
    }

    /// Attach a trait; re-using a key replaces the previous value.
    pub fn with_trait(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.traits.insert(key.into(), value.into());
        self
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub fn traits(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.traits.iter().map(|(key, value)| (key.as_str(), value))
    }

    pub fn has_traits(&self) -> bool {
        !self.traits.is_empty()
    }

    pub(crate) fn trait_map(&self) -> &BTreeMap<String, Value> {
        &self.traits
    }

    /// Cache discriminator.
    ///
    /// Traits are part of the key, not just the identifier: the same user hitting
    /// two different frontends sends a different `app` trait, and those two
    /// requests can legitimately resolve to different flags via segments. Keying
    /// on the identifier alone would serve one app's answer to the other for a
    /// whole TTL.
    pub(crate) fn cache_key(&self) -> String {
        if self.traits.is_empty() {
            return self.identifier.clone();
        }
        let mut key = self.identifier.clone();
        for (name, value) in &self.traits {
            // Unit separator: cannot appear in an identifier or a trait name.
            let _ = write!(key, "\u{1f}{name}={value}");
        }
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn traits_are_sorted_deduplicated_and_optional() {
        let identity = Identity::new("yuri");
        assert_eq!(identity.identifier(), "yuri");
        assert!(!identity.has_traits());
        assert_eq!(identity.cache_key(), "yuri");

        let identity = identity
            .with_trait("app", "web")
            .with_trait("app", "htmx")
            .with_trait("plan", 2);
        let traits: Vec<(&str, &Value)> = identity.traits().collect();
        assert_eq!(traits, vec![("app", &json!("htmx")), ("plan", &json!(2))]);
        assert!(identity.has_traits());
    }

    #[test]
    fn cache_key_separates_the_same_user_on_different_apps() {
        let web = Identity::new("yuri").with_trait("app", "web");
        let htmx = Identity::new("yuri").with_trait("app", "htmx");
        assert_ne!(web.cache_key(), htmx.cache_key());
        assert_eq!(
            htmx.cache_key(),
            Identity::new("yuri").with_trait("app", "htmx").cache_key()
        );
    }
}
