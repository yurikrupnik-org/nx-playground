//! Output rendering: table, json, yaml.

use std::collections::BTreeSet;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Table,
    Json,
    Yaml,
}

impl Format {
    /// `table` is for humans, `json` for pipes. Defaulting on TTY-ness means
    /// `x get todos | jq` works without a flag, which is the difference
    /// between a CLI that composes and one that needs `-o json` everywhere.
    pub fn resolve(requested: Option<&str>, is_terminal: bool) -> Self {
        match requested {
            Some("json") => Self::Json,
            Some("yaml") => Self::Yaml,
            Some("table") => Self::Table,
            _ if is_terminal => Self::Table,
            _ => Self::Json,
        }
    }
}

/// Column order. `serde_json::Map` is a `BTreeMap` in this build (enabling
/// serde_json's `preserve_order` would change `Map` semantics for every crate
/// in the workspace, including the API responses themselves), so declaration
/// order from the schema is not available. Rather than dump alphabetically —
/// which buries `id` and `title` between `completed` and `created_at` — the
/// identity-ish columns are pulled to the front and the audit timestamps
/// pushed to the back. `--fields` overrides this entirely.
const LEADING: [&str; 5] = ["id", "slug", "name", "title", "email"];
const TRAILING: [&str; 4] = ["created_at", "updated_at", "inserted_at", "deleted_at"];

fn order_columns(mut columns: Vec<String>) -> Vec<String> {
    columns.sort();
    let rank = |c: &String| -> (u8, usize) {
        if let Some(i) = LEADING.iter().position(|l| l == c) {
            return (0, i);
        }
        if let Some(i) = TRAILING.iter().position(|t| t == c) {
            return (2, i);
        }
        (1, 0)
    };
    columns.sort_by_key(|c| (rank(c), c.clone()));
    columns
}

/// Project each object to `fields`, preserving the order the user asked for.
pub fn project(value: Value, fields: &[String]) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| project(item, fields))
                .collect(),
        ),
        Value::Object(object) => {
            let mut out = serde_json::Map::new();
            for field in fields {
                if let Some(v) = object.get(field) {
                    out.insert(field.clone(), v.clone());
                }
            }
            Value::Object(out)
        }
        other => other,
    }
}

pub fn render(value: &Value, format: Format, fields: Option<&[String]>) -> eyre::Result<String> {
    Ok(match format {
        Format::Json => serde_json::to_string_pretty(value)?,
        Format::Yaml => serde_yaml_ng::to_string(value)?,
        Format::Table => table(value, fields),
    })
}

fn table(value: &Value, fields: Option<&[String]>) -> String {
    match value {
        Value::Array(items) if items.iter().all(Value::is_object) => rows(items, fields),
        Value::Object(_) => rows(std::slice::from_ref(value), fields),
        other => scalar(other),
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A cell: scalars render bare, structures render as compact JSON so a row
/// stays one line.
fn cell(value: Option<&Value>) -> String {
    match value {
        None => String::new(),
        Some(Value::Array(a)) if a.is_empty() => String::new(),
        Some(v @ (Value::Array(_) | Value::Object(_))) => v.to_string(),
        Some(other) => scalar(other),
    }
}

fn rows(items: &[Value], fields: Option<&[String]>) -> String {
    if items.is_empty() {
        return "(no rows)".to_owned();
    }

    let columns: Vec<String> = match fields {
        Some(requested) => requested.to_vec(),
        None => {
            let discovered: BTreeSet<String> = items
                .iter()
                .filter_map(Value::as_object)
                .flat_map(|o| o.keys().cloned())
                .collect();
            order_columns(discovered.into_iter().collect())
        }
    };

    if columns.is_empty() {
        return "(no columns)".to_owned();
    }

    let header: Vec<String> = columns.iter().map(|c| c.to_uppercase()).collect();
    let body: Vec<Vec<String>> = items
        .iter()
        .map(|item| {
            columns
                .iter()
                .map(|c| cell(item.as_object().and_then(|o| o.get(c))))
                .collect()
        })
        .collect();

    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(i, _)| {
            body.iter()
                .map(|row| row[i].chars().count())
                .chain(std::iter::once(header[i].chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();

    let mut out = String::new();
    let line = |cells: &[String], widths: &[usize]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if i + 1 == cells.len() {
                    c.clone()
                } else {
                    format!("{:width$}", c, width = widths[i])
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };

    out.push_str(&line(&header, &widths));
    for row in &body {
        out.push('\n');
        out.push_str(&line(row, &widths));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piped_output_defaults_to_json_so_it_composes() {
        assert_eq!(Format::resolve(None, false), Format::Json);
        assert_eq!(Format::resolve(None, true), Format::Table);
        // An explicit flag always wins, terminal or not.
        assert_eq!(Format::resolve(Some("table"), false), Format::Table);
    }

    #[test]
    fn identity_columns_come_first_and_timestamps_last() {
        let ordered = order_columns(vec![
            "created_at".to_owned(),
            "completed".to_owned(),
            "id".to_owned(),
            "title".to_owned(),
        ]);
        assert_eq!(ordered, ["id", "title", "completed", "created_at"]);
    }

    #[test]
    fn fields_projection_keeps_the_requested_order() {
        // Alphabetical ordering would give title,id — the point of --fields is
        // that the user controls both the set and the order.
        let value = serde_json::json!([{"id": "1", "title": "a", "completed": false}]);
        let projected = project(value, &["title".to_owned(), "id".to_owned()]);
        let rendered = render(
            &projected,
            Format::Table,
            Some(&["title".to_owned(), "id".to_owned()]),
        )
        .expect("renders");
        assert_eq!(rendered, "TITLE  ID\na      1");
    }

    #[test]
    fn a_nested_value_stays_on_one_row() {
        let value = serde_json::json!([{"id": "1", "tags": ["x", "y"]}]);
        let rendered = render(&value, Format::Table, None).expect("renders");
        assert_eq!(rendered.lines().count(), 2, "{rendered}");
        assert!(rendered.contains(r#"["x","y"]"#), "{rendered}");
    }

    #[test]
    fn an_empty_collection_says_so_instead_of_printing_nothing() {
        let rendered = render(&serde_json::json!([]), Format::Table, None).expect("renders");
        assert_eq!(rendered, "(no rows)");
    }
}
