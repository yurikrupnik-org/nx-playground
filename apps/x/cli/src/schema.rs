//! `x schema`: any JSON Schema or OpenAPI document as a terminal tree.
//!
//! Deliberately separate from [`crate::spec`]. That reader is typed and scoped
//! to the subset this workspace's own documents use, and must stay that way. A
//! viewer has to take whatever a third party publishes — JSON Schema draft-04
//! through 2020-12, Swagger 2.0, OpenAPI 3.0 and 3.1, as JSON or YAML — so it
//! walks a [`Value`] and treats every keyword as optional: an unknown keyword
//! is ignored, never an error.
//!
//! # Bounded output
//!
//! A shared `$ref` is expanded at its first occurrence only; later occurrences
//! print `(see above)`. That keeps the output linear in the document's size — a
//! schema like Taskfile's references `task_call` from many places — and a
//! `$ref` back into its own ancestry prints `(recursive)` instead of looping.
//! `--depth` cuts expansion below a level and marks the cut with `…`.
//!
//! # Order
//!
//! Properties print alphabetically, not in document order: the workspace's
//! `serde_json` is built without `preserve_order`, and enabling that feature
//! would change the key order of every `Value` serialised by any crate built
//! alongside this one.

use std::collections::HashSet;

use serde_json::{Map, Value};

/// Path-item keys that are operations. [`crate::spec::METHODS`] omits `trace`
/// because no CLI verb maps to it; a viewer must still show it.
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

/// Enum values listed before the rest are elided.
const ENUM_LIMIT: usize = 8;

/// Characters of a `default`/`const`/enum value shown before it is elided.
const VALUE_LIMIT: usize = 48;

/// Where to start and how deep to go.
#[derive(Debug, Default, Clone, Copy)]
pub struct View<'a> {
    /// JSON pointer (`/definitions/task`, optionally `#`-prefixed and
    /// percent-encoded as in a `$ref`) of the schema to start at.
    pub at: Option<&'a str>,
    /// Schema levels to expand below each tree root; `None` is unlimited.
    pub depth: Option<usize>,
}

/// Parse a document as JSON, falling back to YAML — OpenAPI documents are
/// published in both. JSON goes first because its errors are the precise ones
/// for the common case.
pub fn parse(source: &str, raw: &str) -> eyre::Result<Value> {
    let value: Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(json) => serde_yaml_ng::from_str(raw)
            .map_err(|yaml| eyre::eyre!("{source}: neither JSON ({json}) nor YAML ({yaml})"))?,
    };
    // Any text is a valid YAML scalar, so "parsed" alone proves nothing.
    if !value.is_object() {
        return Err(eyre::eyre!(
            "{source}: not a JSON Schema or OpenAPI document (top level is not an object)"
        ));
    }
    Ok(value)
}

/// Render `doc` as a tree. `name` labels the root when the document itself is
/// the schema.
pub fn render(name: &str, doc: &Value, view: View<'_>) -> eyre::Result<String> {
    let mut tree = Tree {
        doc,
        id: doc
            .get("$id")
            .or_else(|| doc.get("id"))
            .and_then(Value::as_str)
            .map(|id| id.trim_end_matches('#')),
        depth_limit: view.depth,
        expanded: HashSet::new(),
        ancestry: Vec::new(),
        out: String::new(),
    };

    match view.at {
        Some(at) => {
            let pointer = percent_decode(at.strip_prefix('#').unwrap_or(at));
            let node =
                lookup(doc, &pointer).ok_or_else(|| eyre::eyre!("{name}: nothing at `{at}`"))?;
            let label = pointer_name(&pointer).unwrap_or_else(|| name.to_owned());
            tree.root_schema(&label, pointer, node);
        }
        None if doc.get("openapi").is_some() || doc.get("swagger").is_some() => {
            tree.openapi(name);
            tree.footer();
        }
        None => {
            tree.root_schema(name, String::new(), doc);
            tree.footer();
        }
    }
    Ok(tree.out)
}

#[derive(Debug, Clone, Copy)]
enum Branch {
    Root,
    Mid,
    Last,
}

impl Branch {
    fn nth(index: usize, len: usize) -> Self {
        if index + 1 == len {
            Self::Last
        } else {
            Self::Mid
        }
    }

    fn connector(self) -> &'static str {
        match self {
            Self::Root => "",
            Self::Mid => "├── ",
            Self::Last => "└── ",
        }
    }

    fn indent(self, prefix: &str) -> String {
        match self {
            Self::Root => prefix.to_owned(),
            Self::Mid => format!("{prefix}│   "),
            Self::Last => format!("{prefix}    "),
        }
    }
}

/// One sub-schema of a node, labelled by how it is reached.
struct Child<'v> {
    label: String,
    schema: &'v Value,
}

/// One row under an OpenAPI operation.
enum Row<'v> {
    Schema {
        label: String,
        note: Option<&'v str>,
        schema: &'v Value,
    },
    Text(String),
}

struct Tree<'v> {
    doc: &'v Value,
    /// The root `$id`, so an absolute `$ref` into this same document resolves.
    id: Option<&'v str>,
    depth_limit: Option<usize>,
    /// Pointers of every `$ref` target already expanded somewhere.
    expanded: HashSet<String>,
    /// Pointers of the `$ref` targets on the path from the root to here.
    ancestry: Vec<String>,
    out: String,
}

impl<'v> Tree<'v> {
    fn line(&mut self, prefix: &str, branch: Branch, text: &str) {
        self.out.push_str(prefix);
        self.out.push_str(branch.connector());
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// A tree rooted at the schema at `pointer`, which counts as expanded so a
    /// `$ref` back to it reads `(recursive)`.
    fn root_schema(&mut self, label: &str, pointer: String, node: &'v Value) {
        self.expanded.insert(pointer.clone());
        self.ancestry.push(pointer);
        self.schema("", Branch::Root, label, None, node, 0);
        self.ancestry.pop();
    }

    /// The document pointer a `$ref` names, if it points into this document.
    fn local_pointer(&self, reference: &str) -> Option<String> {
        let fragment = match reference.split_once('#') {
            Some(("", fragment)) => fragment,
            Some((base, fragment)) if Some(base) == self.id => fragment,
            None if Some(reference) == self.id => "",
            _ => return None,
        };
        Some(percent_decode(fragment))
    }

    /// Follow a `$ref` chain to a node that is not itself a bare reference.
    /// Returns the final target's pointer (the identity used for `see above`
    /// and `recursive`) and the node; `None` for anything not resolvable
    /// inside this document (external files, `$anchor` names, dangling).
    fn follow(&self, reference: &str) -> Option<(String, &'v Value)> {
        let mut pointer = self.local_pointer(reference)?;
        let mut node = lookup(self.doc, &pointer)?;
        // A bound, not a cycle check: `a -> b -> a` is a broken document.
        for _ in 0..32 {
            match node.get("$ref").and_then(Value::as_str) {
                Some(next) => {
                    pointer = self.local_pointer(next)?;
                    node = lookup(self.doc, &pointer)?;
                }
                None => return Some((pointer, node)),
            }
        }
        None
    }

    /// `node` itself, or what its `$ref` points to. For OpenAPI parameter,
    /// request-body and response objects, which may be references too.
    fn resolve(&self, node: &'v Value) -> &'v Value {
        node.get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| self.follow(r))
            .map_or(node, |(_, target)| target)
    }

    fn at_limit(&self, depth: usize) -> bool {
        self.depth_limit.is_some_and(|limit| depth >= limit)
    }

    /// One schema node and, unless cut, its children.
    fn schema(
        &mut self,
        prefix: &str,
        branch: Branch,
        label: &str,
        note: Option<&str>,
        node: &'v Value,
        depth: usize,
    ) {
        let Some(own) = node.as_object() else {
            // Boolean schemas (draft-06+): `true` accepts anything.
            let text = match node {
                Value::Bool(true) => "any",
                Value::Bool(false) => "nothing",
                _ => "(not a schema)",
            };
            self.line(prefix, branch, &join(&[label, text]));
            return;
        };

        let (target, key, arrow) = match own.get("$ref").and_then(Value::as_str) {
            None => (own, None, String::new()),
            Some(reference) => match self.follow(reference) {
                Some((key, target)) => {
                    // A chain (`env` → `vars`) names where it lands too, so a
                    // later `(see above)` points at a name that was printed.
                    let written = ref_name(reference);
                    let arrow = match pointer_name(&key) {
                        Some(landed) if landed != written => format!("→ {written} → {landed}"),
                        _ => format!("→ {written}"),
                    };
                    (target.as_object().unwrap_or(own), Some(key), arrow)
                }
                None => {
                    let text = join(&[
                        label,
                        &format!("→ {reference} (unresolved)"),
                        &summary(own, own),
                        &description(own, own, note),
                    ]);
                    self.line(prefix, branch, &text);
                    return;
                }
            },
        };

        let mut children = Vec::new();
        collect_children(target, &mut children);
        // 2019-09+ allows keywords beside `$ref`; they apply on top.
        if !std::ptr::eq(target, own) {
            collect_children(own, &mut children);
        }

        let cut = if children.is_empty() {
            None
        } else if key.as_ref().is_some_and(|k| self.ancestry.contains(k)) {
            Some("(recursive)")
        } else if key.as_ref().is_some_and(|k| self.expanded.contains(k)) {
            Some("(see above)")
        } else if self.at_limit(depth) {
            Some("…")
        } else {
            None
        };

        let text = join(&[
            label,
            &arrow,
            &summary(own, target),
            cut.unwrap_or_default(),
            &description(own, target, note),
        ]);
        self.line(prefix, branch, &text);
        if cut.is_some() || children.is_empty() {
            return;
        }

        if let Some(key) = &key {
            self.expanded.insert(key.clone());
            self.ancestry.push(key.clone());
        }
        let indent = branch.indent(prefix);
        let len = children.len();
        for (index, child) in children.into_iter().enumerate() {
            self.schema(
                &indent,
                Branch::nth(index, len),
                &child.label,
                None,
                child.schema,
                depth + 1,
            );
        }
        if key.is_some() {
            self.ancestry.pop();
        }
    }

    fn openapi(&mut self, name: &str) {
        let doc = self.doc;
        let info = doc.get("info");
        let field = |key: &str| info.and_then(|i| i.get(key)).and_then(Value::as_str);
        let (flavour, dialect) = match doc.get("openapi") {
            Some(v) => ("OpenAPI", v),
            None => ("Swagger", doc.get("swagger").unwrap_or(&Value::Null)),
        };
        let dialect = dialect
            .as_str()
            .map_or_else(|| compact(dialect), str::to_owned);
        let header = join(&[
            field("title").unwrap_or(name),
            field("version").unwrap_or_default(),
            &format!("({flavour} {dialect})"),
            &field("description").map(first_line).unwrap_or_default(),
        ]);
        self.line("", Branch::Root, &header);

        let mut operations = Vec::new();
        for (path, item) in doc
            .get("paths")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let item = self.resolve(item);
            for method in METHODS {
                if let Some(op) = item.get(method) {
                    operations.push((path.as_str(), method, op, item));
                }
            }
        }
        let len = operations.len();
        for (index, (path, method, op, item)) in operations.into_iter().enumerate() {
            self.operation(Branch::nth(index, len), path, method, op, item);
        }
    }

    fn operation(
        &mut self,
        branch: Branch,
        path: &str,
        method: &str,
        op: &'v Value,
        item: &'v Value,
    ) {
        let text_of = |key: &str| op.get(key).and_then(Value::as_str);
        let head = join(&[
            &format!("{} {path}", method.to_uppercase()),
            text_of("operationId").unwrap_or_default(),
            if op.get("deprecated") == Some(&Value::Bool(true)) {
                "deprecated"
            } else {
                ""
            },
            &text_of("summary")
                .or_else(|| text_of("description"))
                .map(|d| format!("— {}", first_line(d)))
                .unwrap_or_default(),
        ]);
        self.line("", branch, &head);

        let mut rows = Vec::new();
        self.parameter_rows(item, op, &mut rows);
        self.body_row(op, &mut rows);
        self.response_rows(op, &mut rows);

        let indent = branch.indent("");
        let len = rows.len();
        for (index, row) in rows.into_iter().enumerate() {
            let branch = Branch::nth(index, len);
            match row {
                Row::Schema {
                    label,
                    note,
                    schema,
                } => self.schema(&indent, branch, &label, note, schema, 0),
                Row::Text(text) => self.line(&indent, branch, &text),
            }
        }
    }

    /// Path-item parameters apply to every operation; an operation parameter
    /// with the same `name` and `in` replaces one.
    fn parameter_rows(&self, item: &'v Value, op: &'v Value, rows: &mut Vec<Row<'v>>) {
        let list = |owner: &'v Value| {
            owner
                .get("parameters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|p| self.resolve(p))
        };
        let identity = |p: &'v Value| (p.get("name"), p.get("in"));
        let own: Vec<&Value> = list(op).collect();
        let shared = list(item).filter(|s| !own.iter().any(|o| identity(o) == identity(s)));
        for param in shared.collect::<Vec<_>>().into_iter().chain(own) {
            let name = param.get("name").and_then(Value::as_str).unwrap_or("?");
            let location = param.get("in").and_then(Value::as_str).unwrap_or_default();
            let mut label = match location {
                "path" => format!("{{{name}}}"),
                "query" => format!("?{name}"),
                "body" => "body".to_owned(),
                "formData" => format!("form {name}"),
                other => format!("{other} {name}"),
            };
            if param.get("required") == Some(&Value::Bool(true)) {
                label.push('*');
            }
            // 3.x: `schema` or `content`; Swagger 2.0 non-body parameters carry
            // the schema keywords (`type`, `items`, `enum`) on themselves.
            let schema = param
                .get("schema")
                .or_else(|| media(param).map(|(_, m)| m.get("schema").unwrap_or(m)))
                .unwrap_or(param);
            rows.push(Row::Schema {
                label,
                note: param.get("description").and_then(Value::as_str),
                schema,
            });
        }
    }

    fn body_row(&self, op: &'v Value, rows: &mut Vec<Row<'v>>) {
        let Some(body) = op.get("requestBody").map(|b| self.resolve(b)) else {
            return;
        };
        let mut label = "body".to_owned();
        if body.get("required") == Some(&Value::Bool(true)) {
            label.push('*');
        }
        let note = body.get("description").and_then(Value::as_str);
        match media(body) {
            Some((kind, content)) => {
                if !kind.contains("json") {
                    label = format!("{label} {kind}");
                }
                match content.get("schema") {
                    Some(schema) => rows.push(Row::Schema {
                        label,
                        note,
                        schema,
                    }),
                    None => rows.push(Row::Text(label)),
                }
            }
            None => rows.push(Row::Text(label)),
        }
    }

    fn response_rows(&self, op: &'v Value, rows: &mut Vec<Row<'v>>) {
        let responses = op.get("responses").and_then(Value::as_object);
        for (status, response) in responses.into_iter().flatten() {
            let response = self.resolve(response);
            let note = response.get("description").and_then(Value::as_str);
            // 3.x nests the schema under a media type; 2.0 has it directly.
            let schema = media(response)
                .and_then(|(_, content)| content.get("schema"))
                .or_else(|| response.get("schema"));
            match schema {
                Some(schema) => rows.push(Row::Schema {
                    label: status.clone(),
                    note,
                    schema,
                }),
                None => rows.push(Row::Text(join(&[
                    status,
                    &note
                        .map(|n| format!("— {}", first_line(n)))
                        .unwrap_or_default(),
                ]))),
            }
        }
    }

    /// Name the reusable schemas the tree does not start from, with the
    /// `--at` pointer that views each.
    fn footer(&mut self) {
        for pointer in ["/definitions", "/$defs", "/components/schemas"] {
            let Some(map) = self.doc.pointer(pointer).and_then(Value::as_object) else {
                continue;
            };
            if map.is_empty() {
                continue;
            }
            let names: Vec<&str> = map.keys().map(String::as_str).collect();
            self.out.push_str(&format!(
                "\n{} under {pointer} (view one: --at {pointer}/<name>):\n  {}\n",
                names.len(),
                names.join(", ")
            ));
        }
    }
}

/// Sub-schemas in display order: fields, then element types, then
/// combinators and conditionals.
fn collect_children<'v>(schema: &'v Map<String, Value>, out: &mut Vec<Child<'v>>) {
    let required = required_names(schema);
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            let star = if required.contains(&name.as_str()) {
                "*"
            } else {
                ""
            };
            out.push(Child {
                label: format!("{name}{star}"),
                schema: property,
            });
        }
    }
    if let Some(patterns) = schema.get("patternProperties").and_then(Value::as_object) {
        for (pattern, property) in patterns {
            out.push(Child {
                label: format!("/{pattern}/"),
                schema: property,
            });
        }
    }
    if let Some(extra @ Value::Object(_)) = schema.get("additionalProperties") {
        out.push(Child {
            label: "<key>".to_owned(),
            schema: extra,
        });
    }
    match schema.get("items") {
        Some(items @ Value::Object(_)) => out.push(Child {
            label: "[]".to_owned(),
            schema: items,
        }),
        // Draft-04..2019-09 tuple form.
        Some(Value::Array(tuple)) => indexed(tuple, "", out),
        _ => {}
    }
    if let Some(Value::Array(tuple)) = schema.get("prefixItems") {
        indexed(tuple, "", out);
    }
    for keyword in ["allOf", "anyOf", "oneOf"] {
        if let Some(Value::Array(alternatives)) = schema.get(keyword) {
            indexed(alternatives, keyword, out);
        }
    }
    for keyword in ["not", "if", "then", "else", "contains"] {
        if let Some(sub @ Value::Object(_)) = schema.get(keyword) {
            out.push(Child {
                label: keyword.to_owned(),
                schema: sub,
            });
        }
    }
}

fn indexed<'v>(list: &'v [Value], keyword: &str, out: &mut Vec<Child<'v>>) {
    for (index, schema) in list.iter().enumerate() {
        out.push(Child {
            label: format!("{keyword}[{index}]"),
            schema,
        });
    }
}

fn required_names(schema: &Map<String, Value>) -> Vec<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Type and constraints on one line. `own` wins over `target`, so keywords
/// beside a `$ref` override what it points to.
fn summary(own: &Map<String, Value>, target: &Map<String, Value>) -> String {
    let get = |key: &str| own.get(key).or_else(|| target.get(key));
    let has = |key: &str| get(key).is_some();

    let mut ty = match get("type") {
        Some(Value::String(one)) => one.clone(),
        Some(Value::Array(many)) => many
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" | "),
        _ if has("properties") || has("patternProperties") || has("additionalProperties") => {
            "object".to_owned()
        }
        _ if has("items") || has("prefixItems") => "array".to_owned(),
        _ => String::new(),
    };
    // OpenAPI 3.0 spelling of an optional value.
    if get("nullable") == Some(&Value::Bool(true)) && !ty.contains("null") {
        ty.push_str(" | null");
    }
    if let Some(format) = get("format").and_then(Value::as_str) {
        ty = if ty.is_empty() {
            format!("({format})")
        } else {
            format!("{ty} ({format})")
        };
    }

    let mut parts = vec![ty];
    for keyword in ["oneOf", "anyOf", "allOf"] {
        if has(keyword) {
            parts.push(keyword.to_owned());
        }
    }
    if let Some(Value::Array(values)) = get("enum") {
        let mut shown: Vec<String> = values.iter().take(ENUM_LIMIT).map(compact).collect();
        if values.len() > ENUM_LIMIT {
            shown.push(format!("… {} more", values.len() - ENUM_LIMIT));
        }
        parts.push(format!("enum: {}", shown.join(" | ")));
    }
    if let Some(value) = get("const") {
        parts.push(format!("const: {}", compact(value)));
    }
    if let Some(value) = get("default") {
        parts.push(format!("default: {}", compact(value)));
    }
    // A schema that only constrains which keys exist, as in
    // `anyOf: [{required: [a]}, {required: [b]}]`.
    let required = required_names(target);
    if !required.is_empty() && !has("properties") {
        parts.push(format!("requires: {}", required.join(", ")));
    }
    for (keyword, word) in [
        ("deprecated", "deprecated"),
        ("readOnly", "read-only"),
        ("writeOnly", "write-only"),
    ] {
        if get(keyword) == Some(&Value::Bool(true)) {
            parts.push(word.to_owned());
        }
    }
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    join(&parts)
}

/// `— first line of the description`, preferring the caller's note (an
/// OpenAPI parameter's or response's own description), then the schema's
/// description, then its title.
fn description(
    own: &Map<String, Value>,
    target: &Map<String, Value>,
    note: Option<&str>,
) -> String {
    let text = |key: &str| {
        own.get(key)
            .or_else(|| target.get(key))
            .and_then(Value::as_str)
    };
    note.or_else(|| text("description"))
        .or_else(|| text("title"))
        .map(first_line)
        .filter(|line| !line.is_empty())
        .map(|line| format!("— {line}"))
        .unwrap_or_default()
}

/// First non-blank line, with `…` when more text follows.
fn first_line(text: &str) -> String {
    let text = text.trim();
    match text.split_once('\n') {
        Some((first, _)) => format!("{} …", first.trim_end()),
        None => text.to_owned(),
    }
}

/// A JSON value on one line, elided past [`VALUE_LIMIT`] characters.
fn compact(value: &Value) -> String {
    let text = value.to_string();
    match text.char_indices().nth(VALUE_LIMIT) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

/// Non-empty parts separated by two spaces.
fn join(parts: &[&str]) -> String {
    let mut out = String::new();
    for part in parts.iter().filter(|p| !p.is_empty()) {
        if !out.is_empty() {
            out.push_str("  ");
        }
        out.push_str(part);
    }
    out
}

/// The media type an OpenAPI 3.x body/response/parameter would be read as:
/// a JSON one when offered, else the first.
fn media(owner: &Value) -> Option<(&str, &Value)> {
    let content = owner.get("content")?.as_object()?;
    content
        .iter()
        .find(|(kind, _)| kind.contains("json"))
        .or_else(|| content.iter().next())
        .map(|(kind, value)| (kind.as_str(), value))
}

/// `#/definitions/task` → `task`; anything else is shown as written.
fn ref_name(reference: &str) -> String {
    reference
        .split_once('#')
        .and_then(|(_, fragment)| pointer_name(&percent_decode(fragment)))
        .unwrap_or_else(|| reference.to_owned())
}

/// Last token of a JSON pointer, unescaped; `None` for the root.
fn pointer_name(pointer: &str) -> Option<String> {
    let last = pointer.rsplit('/').next().filter(|s| !s.is_empty())?;
    Some(last.replace("~1", "/").replace("~0", "~"))
}

/// A pointer into `doc`; the empty pointer is the document itself. A fragment
/// that is not a pointer (an `$anchor` name) resolves to nothing.
fn lookup<'v>(doc: &'v Value, pointer: &str) -> Option<&'v Value> {
    if pointer.is_empty() {
        Some(doc)
    } else {
        doc.pointer(pointer)
    }
}

/// `$ref` fragments are URI fragments, so `{` in a path key arrives as `%7B`.
fn percent_decode(raw: &str) -> String {
    if !raw.contains('%') {
        return raw.to_owned();
    }
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| raw.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tree(doc: &Value, view: View<'_>) -> String {
        render("doc.json", doc, view).expect("renders")
    }

    #[test]
    fn a_self_referencing_schema_terminates_and_says_so() {
        // A linked list: without ancestry tracking this recurses forever.
        let doc = json!({
            "$ref": "#/definitions/node",
            "definitions": {"node": {
                "type": "object",
                "required": ["value"],
                "properties": {
                    "value": {"type": "integer"},
                    "next": {"$ref": "#/definitions/node"}
                }
            }}
        });
        let out = tree(&doc, View::default());
        assert!(
            out.contains("├── next  → node  object  (recursive)"),
            "{out}"
        );
        assert!(out.contains("└── value*  integer"), "{out}");
    }

    #[test]
    fn a_shared_reference_expands_once() {
        let doc = json!({
            "type": "object",
            "properties": {
                "a": {"$ref": "#/$defs/point"},
                "b": {"$ref": "#/$defs/point"}
            },
            "$defs": {"point": {"type": "object", "properties": {"x": {"type": "number"}}}}
        });
        let out = tree(&doc, View::default());
        assert_eq!(out.matches("x  number").count(), 1, "{out}");
        assert!(out.contains("└── b  → point  object  (see above)"), "{out}");
    }

    #[test]
    fn a_cut_by_depth_does_not_count_as_seen() {
        // `a` is cut at the depth limit; `b` reaches the same target at a
        // shallower level and must expand it rather than point "above" at
        // something that was never printed.
        let doc = json!({
            "type": "object",
            "properties": {
                "a": {"type": "object", "properties": {"deep": {"$ref": "#/$defs/leafy"}}},
                "b": {"$ref": "#/$defs/leafy"}
            },
            "$defs": {"leafy": {"type": "object", "properties": {"leaf": {"type": "string"}}}}
        });
        let out = tree(
            &doc,
            View {
                at: None,
                depth: Some(2),
            },
        );
        assert!(out.contains("deep  → leafy  object  …"), "{out}");
        assert!(out.contains("leaf  string"), "{out}");
        assert!(!out.contains("see above"), "{out}");
    }

    #[test]
    fn at_accepts_a_percent_encoded_ref_fragment() {
        let doc = json!({"$defs": {"a/b": {"type": "string", "description": "slash"}}});
        let out = tree(
            &doc,
            View {
                at: Some("#/%24defs/a~1b"),
                depth: None,
            },
        );
        assert_eq!(out, "a/b  string  — slash\n");
        assert!(
            render(
                "doc.json",
                &doc,
                View {
                    at: Some("/nope"),
                    depth: None
                }
            )
            .is_err()
        );
    }

    #[test]
    fn openapi_operations_show_parameters_body_and_responses() {
        let doc = json!({
            "openapi": "3.1.0",
            "info": {"title": "Todos", "version": "1"},
            "paths": {"/todos/{id}": {
                "parameters": [{"name": "id", "in": "path", "required": true,
                                "schema": {"type": "string", "format": "uuid"}}],
                "put": {
                    "operationId": "update_todo",
                    "requestBody": {"$ref": "#/components/requestBodies/Update"},
                    "responses": {
                        "200": {"description": "Updated",
                                "content": {"application/json": {
                                    "schema": {"$ref": "#/components/schemas/Todo"}}}},
                        "404": {"description": "Missing"}
                    }
                }
            }},
            "components": {
                "requestBodies": {"Update": {"required": true, "content": {
                    "application/json": {"schema": {"$ref": "#/components/schemas/Todo"}}}}},
                "schemas": {"Todo": {"type": "object", "required": ["title"],
                                     "properties": {"title": {"type": "string"}}}}
            }
        });
        let out = tree(&doc, View::default());
        let expected = "\
Todos  1  (OpenAPI 3.1.0)
└── PUT /todos/{id}  update_todo
    ├── {id}*  string (uuid)
    ├── body*  → Todo  object
    │   └── title*  string
    ├── 200  → Todo  object  (see above)  — Updated
    └── 404  — Missing

1 under /components/schemas (view one: --at /components/schemas/<name>):
  Todo
";
        assert_eq!(out, expected);
    }

    #[test]
    fn yaml_documents_parse_and_scalars_are_rejected() {
        let doc = parse("x.yaml", "openapi: 3.0.3\npaths: {}\n").expect("yaml parses");
        assert_eq!(doc["openapi"], "3.0.3");
        assert!(parse("x.txt", "just words").is_err());
    }
}
