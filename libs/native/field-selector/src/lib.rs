//! Node addon exposing [`field_selector`] semantics with a *runtime* schema.
//!
//! The Rust API ([`field_selector::SelectableFields`]) carries field metadata in
//! `&'static` slices, which is unreachable from JS: callers there only know the
//! schema at runtime. This crate keeps the same rules — validate requested
//! fields, drop restricted ones, drop fields above the caller's role — against a
//! schema built at construction time.
//!
//! Query-string parsing is delegated to [`field_selector::FieldSelector`] so the
//! `fields=a,b,c` grammar (trim, ignore empties) stays single-sourced.

use std::collections::{HashMap, HashSet};

use field_selector::{FieldSelector, UserRole};
use napi_derive::napi;
use serde_json::{Map, Value};

/// Caller privilege, ascending. Mirrors [`field_selector::UserRole`].
#[napi(string_enum = "lowercase")]
pub enum Role {
    Anonymous,
    User,
    Admin,
}

impl From<Role> for UserRole {
    fn from(role: Role) -> Self {
        match role {
            Role::Anonymous => Self::Anonymous,
            Role::User => Self::User,
            Role::Admin => Self::Admin,
        }
    }
}

/// One selectable field of a schema.
#[napi(object)]
pub struct FieldRule {
    /// JSON key this rule governs.
    pub field: String,
    /// Minimum role needed to read the field. Omitted = readable by anyone.
    pub required_role: Option<Role>,
    /// Never emitted, whatever the request or role (blacklist).
    pub restricted: Option<bool>,
}

/// A schema the JS side can reuse across requests.
#[napi]
pub struct FieldSchema {
    /// Declaration order is preserved: it defines the default projection order.
    available: Vec<String>,
    restricted: HashSet<String>,
    /// Only fields above `Anonymous` are stored; a miss means "public".
    required_role: HashMap<String, UserRole>,
}

#[napi]
impl FieldSchema {
    #[napi(constructor)]
    pub fn new(fields: Vec<FieldRule>) -> napi::Result<Self> {
        let mut schema = Self {
            available: Vec::with_capacity(fields.len()),
            restricted: HashSet::new(),
            required_role: HashMap::new(),
        };

        for rule in fields {
            if schema.available.contains(&rule.field) {
                return Err(napi::Error::from_reason(format!(
                    "duplicate field in schema: {}",
                    rule.field
                )));
            }
            if rule.restricted.unwrap_or(false) {
                schema.restricted.insert(rule.field.clone());
            }
            match rule.required_role {
                Some(Role::Anonymous) | None => {}
                Some(role) => {
                    schema.required_role.insert(rule.field.clone(), role.into());
                }
            }
            schema.available.push(rule.field);
        }

        Ok(schema)
    }

    /// Fields the caller may read, in declaration order.
    ///
    /// Throws when `fields` names something absent from the schema; fields the
    /// caller may not read are dropped silently, as in the Rust API.
    #[napi]
    pub fn allowed_fields(
        &self,
        fields: Option<String>,
        role: Option<Role>,
    ) -> napi::Result<Vec<&str>> {
        self.resolve(fields.as_deref(), role)
    }

    /// Project one JSON object down to the allowed fields. Non-objects pass
    /// through. Keys come back alphabetically ordered: `serde_json`'s default
    /// map is a `BTreeMap`, so input order is not observable here.
    #[napi]
    pub fn filter(
        &self,
        value: Value,
        fields: Option<String>,
        role: Option<Role>,
    ) -> napi::Result<Value> {
        let allowed = self.allowed_set(fields.as_deref(), role)?;
        Ok(project(value, &allowed))
    }

    /// Same projection applied to a list; the schema is resolved once.
    #[napi]
    pub fn filter_list(
        &self,
        values: Vec<Value>,
        fields: Option<String>,
        role: Option<Role>,
    ) -> napi::Result<Vec<Value>> {
        let allowed = self.allowed_set(fields.as_deref(), role)?;
        Ok(values
            .into_iter()
            .map(|value| project(value, &allowed))
            .collect())
    }

    fn resolve(&self, fields: Option<&str>, role: Option<Role>) -> napi::Result<Vec<&str>> {
        let role = role.map(UserRole::from).unwrap_or_default();
        let selector = FieldSelector {
            fields: fields.map(str::to_owned),
        };

        let Some(requested) = selector.get_fields() else {
            // No projection requested: everything the caller may read.
            return Ok(self
                .available
                .iter()
                .filter(|field| self.is_visible(field, role))
                .map(String::as_str)
                .collect());
        };

        let mut invalid: Vec<&str> = requested
            .iter()
            .copied()
            .filter(|field| !self.available.iter().any(|known| known == field))
            .collect();
        if !invalid.is_empty() {
            invalid.sort_unstable();
            return Err(napi::Error::from_reason(format!(
                "invalid fields requested: {}",
                invalid.join(", ")
            )));
        }

        Ok(self
            .available
            .iter()
            .filter(|field| requested.contains(field.as_str()) && self.is_visible(field, role))
            .map(String::as_str)
            .collect())
    }

    fn allowed_set(&self, fields: Option<&str>, role: Option<Role>) -> napi::Result<HashSet<&str>> {
        Ok(self.resolve(fields, role)?.into_iter().collect())
    }

    fn is_visible(&self, field: &str, role: UserRole) -> bool {
        !self.restricted.contains(field)
            && self
                .required_role
                .get(field)
                .is_none_or(|required| role.has_permission(required))
    }
}

fn project(value: Value, allowed: &HashSet<&str>) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter(|(key, _)| allowed.contains(key.as_str()))
                .collect::<Map<String, Value>>(),
        ),
        other => other,
    }
}
