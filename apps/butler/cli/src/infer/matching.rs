//! nx's project patterns (`findMatchingProjects`): what `implicitDependencies`
//! and a `targetDefaults` `filter.projects` accept — exact names, name globs,
//! `tag:` and `directory:` patterns, `!` exclusions, and an unlabeled pattern
//! that tries names, then directories.

use eyre::Result;

use super::{glob, is_glob_pattern};

/// The facts a pattern can test.
pub struct Candidate<'a> {
    pub name: &'a str,
    pub root: &'a str,
    pub tags: &'a [String],
}

/// The names `patterns` select among `projects`, in nx's order (insertion
/// order of a JavaScript `Set`: a re-added name moves to the end).
pub fn find_matching_projects(patterns: &[String], projects: &[Candidate]) -> Result<Vec<String>> {
    let mut patterns: Vec<&str> = patterns.iter().map(String::as_str).collect();
    if patterns.iter().all(|p| p.is_empty()) {
        return Ok(Vec::new());
    }
    // A leading exclusion starts from "everything".
    if patterns[0].starts_with('!') {
        patterns.insert(0, "*");
    }
    let mut matched: Vec<String> = Vec::new();
    for raw in patterns {
        if raw.is_empty() || raw.starts_with("nx-cloud:") {
            continue;
        }
        let exclude = raw.starts_with('!');
        let body = raw.strip_prefix('!').unwrap_or(raw);
        let (kind, value) = if projects.iter().any(|p| p.name == body) {
            ("name", body)
        } else {
            match body.split_once(':') {
                Some((k, v)) if matches!(k, "name" | "tag" | "directory" | "unlabeled") => (k, v),
                _ => ("unlabeled", body),
            }
        };
        if value == "*" {
            for p in projects {
                set(&mut matched, p.name, exclude);
            }
            continue;
        }
        let before = matched.clone();
        match kind {
            "tag" => {
                let m = if is_glob_pattern(value) {
                    Some(dot_glob(value)?)
                } else {
                    None
                };
                for p in projects {
                    let hit = p.tags.iter().any(|t| t == value)
                        || m.as_ref()
                            .is_some_and(|m| p.tags.iter().any(|t| m.is_match(t)));
                    if hit {
                        set(&mut matched, p.name, exclude);
                    }
                }
            }
            "directory" => by_directory(value, exclude, projects, &mut matched)?,
            "name" => by_name(value, exclude, projects, &mut matched)?,
            _ => {
                by_name(value, exclude, projects, &mut matched)?;
                if matched == before {
                    by_directory(value, exclude, projects, &mut matched)?;
                }
            }
        }
    }
    Ok(matched)
}

/// Add or remove one name, keeping a JavaScript `Set`'s insertion order.
fn set(matched: &mut Vec<String>, name: &str, exclude: bool) {
    if exclude {
        matched.retain(|n| n != name);
    } else if !matched.iter().any(|n| n == name) {
        matched.push(name.to_string());
    }
}

fn by_name(
    value: &str,
    exclude: bool,
    projects: &[Candidate],
    matched: &mut Vec<String>,
) -> Result<()> {
    if projects.iter().any(|p| p.name == value) {
        set(matched, value, exclude);
        return Ok(());
    }
    if !is_glob_pattern(value) {
        // nx's "word" match: the value delimited by anything but letters,
        // digits, `@` and `-` (so `foo` selects `foo_bar`, not `foo-e2e`),
        // case-insensitively. nx builds a RegExp from the raw value; butler
        // takes it literally.
        for p in projects {
            if word_match(p.name, value) {
                set(matched, p.name, exclude);
            }
        }
        return Ok(());
    }
    let m = dot_glob(value)?;
    for p in projects {
        if m.is_match(p.name) {
            set(matched, p.name, exclude);
        }
    }
    Ok(())
}

fn by_directory(
    value: &str,
    exclude: bool,
    projects: &[Candidate],
    matched: &mut Vec<String>,
) -> Result<()> {
    let m = dot_glob(value)?;
    for p in projects {
        if p.root == value || m.is_match(p.root) {
            set(matched, p.name, exclude);
        }
    }
    Ok(())
}

/// nx matches these patterns with minimatch `dot: true`, which globset does
/// by default.
fn dot_glob(pattern: &str) -> Result<globset::GlobMatcher> {
    glob(pattern)
}

fn word_match(name: &str, value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let boundary =
        |c: Option<char>| !c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '@' || c == '-');
    let hay = name.to_lowercase();
    let needle = value.to_lowercase();
    let mut from = 0;
    while let Some(i) = hay[from..].find(&needle) {
        let start = from + i;
        let end = start + needle.len();
        if boundary(hay[..start].chars().next_back()) && boundary(hay[end..].chars().next()) {
            return true;
        }
        from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// nx's `normalizeImplicitDependencies`: positive patterns expand to project
/// names (never the project itself); each `!pattern` expands to `!name`
/// entries, kept so the edge they name is removed from the graph.
pub fn normalize_implicit_dependencies(
    source: &str,
    declared: &[String],
    projects: &[Candidate],
) -> Result<Vec<String>> {
    if declared.is_empty() {
        return Ok(Vec::new());
    }
    let (negative, positive): (Vec<String>, Vec<String>) =
        declared.iter().cloned().partition(|d| d.starts_with('!'));
    let mut out = if positive.is_empty() {
        Vec::new()
    } else {
        let all: Vec<String> = positive.iter().chain(&negative).cloned().collect();
        find_matching_projects(&all, projects)?
            .into_iter()
            .filter(|n| n != source)
            .collect()
    };
    let stripped: Vec<String> = negative.iter().map(|n| n[1..].to_string()).collect();
    out.extend(
        find_matching_projects(&stripped, projects)?
            .into_iter()
            .map(|n| format!("!{n}")),
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates<'a>(tags: &'a [String]) -> Vec<Candidate<'a>> {
        vec![
            Candidate {
                name: "todo_api",
                root: "apps/todo/api",
                tags,
            },
            Candidate {
                name: "todo-e2e",
                root: "apps/todo/e2e",
                tags: &[],
            },
            Candidate {
                name: "todo-web",
                root: "apps/todo/web",
                tags: &[],
            },
            Candidate {
                name: "zerg_api",
                root: "apps/zerg/api",
                tags: &[],
            },
        ]
    }

    #[test]
    fn unlabeled_word_match_respects_hyphen_boundaries() {
        let tags = vec![];
        let got = find_matching_projects(&["todo".into()], &candidates(&tags)).unwrap();
        // `_` is a boundary, `-` is not.
        assert_eq!(got, vec!["todo_api"]);
    }

    #[test]
    fn exclusions_tags_and_directories() {
        let tags = vec!["scope:todo".to_string()];
        let c = candidates(&tags);
        let got = find_matching_projects(&["!zerg_api".into()], &c).unwrap();
        assert_eq!(got, vec!["todo_api", "todo-e2e", "todo-web"]);
        let got = find_matching_projects(&["tag:scope:*".into()], &c).unwrap();
        assert_eq!(got, vec!["todo_api"]);
        let got = find_matching_projects(&["apps/todo/*".into(), "!todo-web".into()], &c).unwrap();
        assert_eq!(got, vec!["todo_api", "todo-e2e"]);
    }

    #[test]
    fn implicit_dependencies_drop_self_and_keep_exclusions() {
        let tags = vec![];
        let c = candidates(&tags);
        let got = normalize_implicit_dependencies(
            "todo-e2e",
            &["*_api".into(), "todo-e2e".into(), "!zerg_api".into()],
            &c,
        )
        .unwrap();
        assert_eq!(got, vec!["todo_api", "!zerg_api"]);
    }
}
