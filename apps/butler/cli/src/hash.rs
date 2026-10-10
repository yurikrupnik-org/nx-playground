//! Task hashing, after nx 23's task hasher (`hasher/task-hasher.js` and the
//! native hash planner it drives).
//!
//! A task hash covers what nx's does: the task identity and overrides
//! (`hashCommand`), the project configuration, the target's inputs — file
//! sets split into project files (`{projectRoot}/…`, matched against the files
//! the project owns, nested projects excluded) and workspace files
//! (`{workspaceRoot}/…`), `env` and `runtime` inputs — the npm packages the
//! project uses, and for `^named` inputs the same material of every project
//! dependency, transitively. npm packages hash by the version actually
//! installed (`node_modules/<pkg>/package.json`) and, recursively, by their
//! own installed dependencies: nx hashes lock-file entries, butler hashes
//! what the task will load, which cannot be stale.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, eyre};
use globset::{GlobSet, GlobSetBuilder};
use sha2::{Digest, Sha256};

use crate::config::{Json, JsonMap};
use crate::graph::{Project, ProjectGraph};
use crate::task::Task;

/// One expanded self input.
#[derive(Debug, Clone, PartialEq)]
enum Input {
    FileSet(String),
    Env(String),
    Runtime(String),
    /// Explicit `externalDependencies` (possibly empty).
    External(Vec<String>),
}

/// nx `splitInputsIntoSelfAndDependencies`.
#[derive(Debug, Default)]
struct Split {
    self_inputs: Vec<Input>,
    /// `^named` / `{input, dependencies: true}`.
    deps_inputs: Vec<String>,
    /// `^{projectRoot}/…` / `{fileset, dependencies: true}`.
    deps_filesets: Vec<String>,
    /// `{input, projects}`.
    project_inputs: Vec<(String, Vec<String>)>,
}

pub struct Hasher<'a> {
    root: &'a Path,
    graph: &'a ProjectGraph,
    named_inputs: BTreeMap<String, Vec<Json>>,
    /// Every hashable workspace file (tracked + untracked, not ignored).
    files: Vec<String>,
    /// Project -> the files it owns (longest-root match, like nx's file map).
    owned: HashMap<String, Vec<String>>,
    file_hashes: HashMap<String, String>,
    fileset_memo: HashMap<(String, Vec<String>), String>,
    dep_memo: HashMap<(String, String), String>,
    npm: Npm,
    runtime_memo: HashMap<String, String>,
}

impl<'a> Hasher<'a> {
    pub fn new(
        root: &'a Path,
        graph: &'a ProjectGraph,
        nx_named_inputs: &BTreeMap<String, Vec<Json>>,
        mut files: Vec<String>,
    ) -> Self {
        files.sort();
        files.dedup();
        // nx `getNamedInputs`: `default` is every project file unless nx.json
        // says otherwise.
        let mut named_inputs = BTreeMap::from([(
            "default".to_string(),
            vec![serde_json::json!({"fileset": "{projectRoot}/**/*"})],
        )]);
        named_inputs.extend(nx_named_inputs.clone());
        let mut owned: HashMap<String, Vec<String>> = HashMap::new();
        let roots: HashMap<&str, &str> = graph
            .projects
            .values()
            .map(|p| (p.root.as_str(), p.name.as_str()))
            .collect();
        for f in &files {
            if let Some(owner) = owner_of(f, &roots) {
                owned.entry(owner.to_string()).or_default().push(f.clone());
            }
        }
        Self {
            root,
            graph,
            named_inputs,
            files,
            owned,
            file_hashes: HashMap::new(),
            fileset_memo: HashMap::new(),
            dep_memo: HashMap::new(),
            npm: Npm::new(root),
            runtime_memo: HashMap::new(),
        }
    }

    pub fn task_hash(&mut self, task: &Task) -> Result<String> {
        let ctx = task.id.to_string();
        let project = self.graph.get(&task.id.project)?;
        let default_inputs = vec![
            serde_json::json!({"input": "default"}),
            serde_json::json!({"input": "default", "dependencies": true}),
        ];
        let inputs = task.resolved.inputs.as_ref().unwrap_or(&default_inputs);
        let split = self
            .split(inputs)
            .wrap_err_with(|| format!("{ctx}: inputs"))?;

        let mut h = Sha256::new();
        field(&mut h, "butler-task-v2");
        // nx `hashCommand`.
        field(&mut h, &task.id.project);
        field(&mut h, &task.id.target);
        field(&mut h, task.id.configuration.as_deref().unwrap_or(""));
        field(
            &mut h,
            &canonical(&Json::Object(task.overrides.map.clone())),
        );
        for a in &task.overrides.unparsed {
            field(&mut h, a);
        }
        field(&mut h, &project_config_hash(project)?);
        self.hash_self_inputs(&mut h, project, &split.self_inputs, &task.env)
            .wrap_err_with(|| format!("{ctx}: inputs"))?;

        let mut visited = BTreeSet::new();
        for named in &split.deps_inputs {
            field(&mut h, &format!("^{named}"));
            // The project's own npm packages are dependencies too.
            field(&mut h, &self.npm.project_hash(project)?);
            for dep in &project.deps {
                let d = self
                    .dep_hash(dep, named, &task.env, &mut visited)
                    .wrap_err_with(|| format!("{ctx}: ^{named} via {dep}"))?;
                field(&mut h, &d);
            }
        }
        for fs in &split.deps_filesets {
            field(&mut h, &format!("^fileset {fs}"));
            for dep in self.transitive_deps(project) {
                let p = self.graph.get(&dep)?;
                let fh = self.filesets_hash(p, std::slice::from_ref(fs))?;
                field(&mut h, &fh);
            }
        }
        for (named, patterns) in &split.project_inputs {
            let names = crate::task::find_matching(self.graph, patterns)?;
            for name in names {
                field(&mut h, &format!("{name}:{named}"));
                let p = self.graph.get(&name)?;
                let inputs = self.expand_named(named, &name)?;
                let mut sub = Sha256::new();
                self.hash_self_inputs(&mut sub, p, &inputs, &task.env)?;
                field(&mut h, &hex(&sub.finalize()));
            }
        }
        for tsconfig in ["tsconfig.base.json", "tsconfig.json"] {
            if self.root.join(tsconfig).is_file() {
                field(&mut h, tsconfig);
                field(&mut h, &self.file_hash(tsconfig));
                break;
            }
        }
        Ok(hex(&h.finalize()))
    }

    fn split(&self, inputs: &[Json]) -> Result<Split> {
        let mut out = Split::default();
        let mut self_raw = Vec::new();
        for d in inputs {
            match d {
                Json::String(s) => match s.strip_prefix('^') {
                    Some(rest)
                        if rest.starts_with("{projectRoot}")
                            || rest.starts_with("{workspaceRoot}") =>
                    {
                        out.deps_filesets.push(rest.to_string());
                    }
                    Some(rest) => out.deps_inputs.push(rest.to_string()),
                    None => self_raw.push(d.clone()),
                },
                Json::Object(o) => {
                    let deps = o.get("dependencies").and_then(Json::as_bool) == Some(true);
                    let projects = o.get("projects");
                    match (o.get("fileset"), o.get("input")) {
                        (Some(fs), _) if deps => out.deps_filesets.push(str_of(fs, "fileset")?),
                        (None, Some(input))
                            if deps || projects.and_then(Json::as_str) == Some("dependencies") =>
                        {
                            out.deps_inputs.push(str_of(input, "input")?);
                        }
                        (None, Some(input))
                            if projects
                                .is_some_and(|p| truthy(p) && p.as_str() != Some("self")) =>
                        {
                            let list = match projects {
                                Some(Json::Array(a)) => a
                                    .iter()
                                    .map(|p| str_of(p, "projects"))
                                    .collect::<Result<_>>()?,
                                Some(p) => vec![str_of(p, "projects")?],
                                None => unreachable!("checked above"),
                            };
                            out.project_inputs.push((str_of(input, "input")?, list));
                        }
                        _ => self_raw.push(d.clone()),
                    }
                }
                other => bail!("unsupported input {other}"),
            }
        }
        out.self_inputs = self.expand(&self_raw, &mut Vec::new())?;
        Ok(out)
    }

    /// nx `expandSingleProjectInputs`.
    fn expand(&self, inputs: &[Json], stack: &mut Vec<String>) -> Result<Vec<Input>> {
        let mut out = Vec::new();
        for d in inputs {
            match d {
                Json::String(s) => {
                    if s.starts_with('^') {
                        bail!("namedInputs definitions cannot start with ^");
                    }
                    if self.named_inputs.contains_key(s) {
                        out.extend(self.expand_named_in(s, stack)?);
                    } else {
                        out.push(Input::FileSet(s.clone()));
                    }
                }
                Json::Object(o) => {
                    if o.get("projects").is_some_and(truthy)
                        || o.get("dependencies").is_some_and(truthy)
                    {
                        bail!(
                            "namedInputs definitions can only refer to other namedInputs definitions within the same project."
                        );
                    }
                    if let Some(fs) = o.get("fileset").filter(|v| truthy(v)) {
                        out.push(Input::FileSet(str_of(fs, "fileset")?));
                    } else if let Some(e) = o.get("env").filter(|v| truthy(v)) {
                        out.push(Input::Env(str_of(e, "env")?));
                    } else if let Some(r) = o.get("runtime").filter(|v| truthy(v)) {
                        out.push(Input::Runtime(str_of(r, "runtime")?));
                    } else if let Some(x) = o.get("externalDependencies") {
                        let Json::Array(list) = x else {
                            bail!("externalDependencies must be an array");
                        };
                        out.push(Input::External(
                            list.iter()
                                .map(|p| str_of(p, "externalDependencies"))
                                .collect::<Result<_>>()?,
                        ));
                    } else if let Some(kind) =
                        ["dependentTasksOutputFiles", "workingDirectory", "json"]
                            .into_iter()
                            .find(|k| o.get(*k).is_some_and(truthy))
                    {
                        bail!(
                            "`{kind}` inputs are not supported by butler (they hash state other tasks \
                             produce or nx-specific data); run this target through nx"
                        );
                    } else {
                        let input = o
                            .get("input")
                            .ok_or_else(|| eyre!("unsupported input {d}"))?;
                        out.extend(self.expand_named_in(&str_of(input, "input")?, stack)?);
                    }
                }
                other => bail!("unsupported input {other}"),
            }
        }
        Ok(out)
    }

    fn expand_named_in(&self, name: &str, stack: &mut Vec<String>) -> Result<Vec<Input>> {
        let def = self
            .named_inputs
            .get(name)
            .ok_or_else(|| eyre!("Input '{name}' is not defined"))?;
        if stack.iter().any(|s| s == name) {
            bail!("named input `{name}` refers to itself");
        }
        stack.push(name.to_string());
        let r = self.expand(def, stack);
        stack.pop();
        r
    }

    /// nx `expandNamedInput` (project-level `namedInputs` are not part of
    /// butler's graph; nx.json's apply to every project).
    fn expand_named(&self, name: &str, _project: &str) -> Result<Vec<Input>> {
        self.expand_named_in(name, &mut Vec::new())
    }

    fn hash_self_inputs(
        &mut self,
        h: &mut Sha256,
        project: &Project,
        inputs: &[Input],
        env: &BTreeMap<String, String>,
    ) -> Result<()> {
        let filesets: Vec<String> = inputs
            .iter()
            .filter_map(|i| match i {
                Input::FileSet(f) => Some(f.clone()),
                _ => None,
            })
            .collect();
        field(h, &self.filesets_hash(project, &filesets)?);
        let mut explicit_external = false;
        for i in inputs {
            match i {
                Input::FileSet(_) => {}
                Input::Env(name) => {
                    field(h, &format!("env:{name}"));
                    field(h, env.get(name).map_or("", String::as_str));
                }
                Input::Runtime(cmd) => {
                    field(h, &format!("runtime:{cmd}"));
                    let out = self.runtime(cmd, env)?;
                    field(h, &out);
                }
                Input::External(names) => {
                    explicit_external = true;
                    for n in names {
                        field(h, &format!("npm:{n}"));
                        let from = self.root.join(&project.root);
                        let ph = self.npm.package_hash(n, &from)?;
                        field(h, &ph);
                    }
                }
            }
        }
        // nx hashes every npm package the project depends on unless the
        // inputs name them explicitly.
        if !explicit_external {
            field(h, &self.npm.project_hash(project)?);
        }
        Ok(())
    }

    /// Project file sets against the files the project owns, workspace file
    /// sets against every file.
    fn filesets_hash(&mut self, project: &Project, filesets: &[String]) -> Result<String> {
        let key = (project.name.clone(), filesets.to_vec());
        if let Some(h) = self.fileset_memo.get(&key) {
            return Ok(h.clone());
        }
        let mut project_sets = Vec::new();
        let mut workspace_sets = Vec::new();
        for fs in filesets {
            let (neg, body) = match fs.strip_prefix('!') {
                Some(b) => ("!", b),
                None => ("", fs.as_str()),
            };
            if let Some(rest) = body.strip_prefix("{projectRoot}/") {
                let path = if project.root == "." {
                    rest.to_string()
                } else {
                    format!("{}/{rest}", project.root)
                };
                project_sets.push(format!("{neg}{path}"));
            } else if let Some(rest) = body.strip_prefix("{workspaceRoot}/") {
                workspace_sets.push(format!("{neg}{rest}"));
            } else {
                bail!(
                    "file set `{fs}` must start with {{projectRoot}}/ or {{workspaceRoot}}/ \
                     (nx does not hash bare patterns)"
                );
            }
        }
        let mut h = Sha256::new();
        field(&mut h, "butler-files-v2");
        let project_files = self.owned.get(&project.name).cloned().unwrap_or_default();
        let all_project =
            project_sets.len() == 1 && project_sets[0] == format!("{}/**/*", project.root);
        let matched_project: Vec<String> = if all_project {
            project_files
        } else {
            let m = Matchers::new(&project_sets)?;
            project_files
                .into_iter()
                .filter(|f| m.is_match(f))
                .collect()
        };
        let matched_workspace: Vec<String> = if workspace_sets.is_empty() {
            Vec::new()
        } else {
            let m = Matchers::new(&workspace_sets)?;
            self.files
                .iter()
                .filter(|f| m.is_match(f))
                .cloned()
                .collect()
        };
        for (label, files) in [
            ("project", matched_project),
            ("workspace", matched_workspace),
        ] {
            field(&mut h, label);
            for f in files {
                let fh = self.file_hash(&f);
                field(&mut h, &f);
                field(&mut h, &fh);
            }
        }
        let out = hex(&h.finalize());
        self.fileset_memo.insert(key, out.clone());
        Ok(out)
    }

    /// `^named` material of one dependency: its config, its own named-input
    /// files and npm packages, then its dependencies'.
    fn dep_hash(
        &mut self,
        name: &str,
        named: &str,
        env: &BTreeMap<String, String>,
        visiting: &mut BTreeSet<String>,
    ) -> Result<String> {
        let key = (name.to_string(), named.to_string());
        if let Some(h) = self.dep_memo.get(&key) {
            return Ok(h.clone());
        }
        if !visiting.insert(name.to_string()) {
            return Ok(String::new()); // a dependency cycle contributes once
        }
        let project = self.graph.get(name)?;
        let inputs = self.expand_named(named, name)?;
        let mut h = Sha256::new();
        field(&mut h, "butler-dep-v2");
        field(&mut h, &project_config_hash(project)?);
        self.hash_self_inputs(&mut h, project, &inputs, env)?;
        for dep in &project.deps {
            let d = self.dep_hash(dep, named, env, visiting)?;
            field(&mut h, &d);
        }
        visiting.remove(name);
        let out = hex(&h.finalize());
        self.dep_memo.insert(key, out.clone());
        Ok(out)
    }

    fn transitive_deps(&self, project: &Project) -> Vec<String> {
        let mut out = BTreeSet::new();
        let mut stack: Vec<String> = project.deps.iter().cloned().collect();
        while let Some(d) = stack.pop() {
            if out.insert(d.clone())
                && let Some(p) = self.graph.projects.get(&d)
            {
                stack.extend(p.deps.iter().cloned());
            }
        }
        out.into_iter().collect()
    }

    fn runtime(&mut self, cmd: &str, env: &BTreeMap<String, String>) -> Result<String> {
        if let Some(h) = self.runtime_memo.get(cmd) {
            return Ok(h.clone());
        }
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(self.root)
            .envs(env)
            .output()
            .wrap_err_with(|| format!("running runtime input `{cmd}`"))?;
        let mut h = Sha256::new();
        h.update(&out.stdout);
        h.update(&out.stderr);
        let v = hex(&h.finalize());
        self.runtime_memo.insert(cmd.to_string(), v.clone());
        Ok(v)
    }

    fn file_hash(&mut self, rel: &str) -> String {
        if let Some(h) = self.file_hashes.get(rel) {
            return h.clone();
        }
        let h = match std::fs::read(self.root.join(rel)) {
            Ok(bytes) => hex(&Sha256::digest(&bytes)),
            // Listed but unreadable (deleted, a dangling link): a fixed marker.
            Err(_) => "missing".into(),
        };
        self.file_hashes.insert(rel.to_string(), h.clone());
        h
    }
}

/// nx `findProjectForPath`: the file itself, then each parent directory, then
/// the workspace root project.
fn owner_of<'r>(file: &str, roots: &HashMap<&str, &'r str>) -> Option<&'r str> {
    let mut cur = file;
    loop {
        if let Some(p) = roots.get(cur) {
            return Some(p);
        }
        match cur.rfind('/') {
            Some(i) => cur = &cur[..i],
            None => return roots.get(".").copied(),
        }
    }
}

/// The project's configuration as nx hashes it: root, tags, implicit
/// dependencies and every target.
fn project_config_hash(project: &Project) -> Result<String> {
    let v = serde_json::json!({
        "root": project.root,
        "tags": project.tags,
        "implicitDependencies": project.implicit_dependencies,
        "targets": serde_json::to_value(&project.targets)?,
    });
    Ok(hex(&Sha256::digest(canonical(&v).as_bytes())))
}

fn str_of(v: &Json, what: &str) -> Result<String> {
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| eyre!("`{what}` must be a string, got {v}"))
}

fn truthy(v: &Json) -> bool {
    match v {
        Json::Null | Json::Bool(false) => false,
        Json::String(s) => !s.is_empty(),
        Json::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        _ => true,
    }
}

/// Length-prefixed so adjacent fields cannot run together.
fn field(h: &mut Sha256, s: &str) {
    h.update((s.len() as u64).to_le_bytes());
    h.update(s.as_bytes());
}

/// JSON with object keys sorted at every level.
pub fn canonical(v: &Json) -> String {
    match v {
        Json::Object(m) => {
            let sorted: BTreeMap<&String, &Json> = m.iter().collect();
            let body: Vec<String> = sorted
                .into_iter()
                .map(|(k, v)| format!("{}:{}", Json::String(k.clone()), canonical(v)))
                .collect();
            format!("{{{}}}", body.join(","))
        }
        Json::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

/// Include/exclude glob sets (`!` excludes); a file must match an include.
struct Matchers {
    include: GlobSet,
    exclude: GlobSet,
}

impl Matchers {
    fn new(patterns: &[String]) -> Result<Self> {
        let mut include = GlobSetBuilder::new();
        let mut exclude = GlobSetBuilder::new();
        for p in patterns {
            let (set, body) = match p.strip_prefix('!') {
                Some(b) => (&mut exclude, b),
                None => (&mut include, p.as_str()),
            };
            let g = globset::GlobBuilder::new(body)
                .literal_separator(true)
                .build()
                .wrap_err_with(|| format!("input pattern `{p}`"))?;
            set.add(g);
        }
        Ok(Self {
            include: include.build()?,
            exclude: exclude.build()?,
        })
    }

    fn is_match(&self, path: &str) -> bool {
        self.include.is_match(path) && !self.exclude.is_match(path)
    }
}

/// Installed npm packages, hashed by version and (recursively) by their
/// installed dependencies, resolved like Node does.
struct Npm {
    root: PathBuf,
    memo: HashMap<PathBuf, String>,
    project_memo: HashMap<String, String>,
}

const DEP_FIELDS: [&str; 3] = ["dependencies", "optionalDependencies", "peerDependencies"];

impl Npm {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            memo: HashMap::new(),
            project_memo: HashMap::new(),
        }
    }

    /// Every package the project's `package.json` declares (nx's npm edges
    /// for a package.json project).
    fn project_hash(&mut self, project: &Project) -> Result<String> {
        if let Some(h) = self.project_memo.get(&project.name) {
            return Ok(h.clone());
        }
        let dir = self.root.join(&project.root);
        let mut h = Sha256::new();
        field(&mut h, "butler-npm-v1");
        if let Some(manifest) = read_manifest(&dir.join("package.json"))? {
            let fields = [
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ];
            for (name, spec) in declared(&manifest, &fields) {
                if spec.starts_with("workspace:") {
                    continue; // a project edge, not an npm package
                }
                field(&mut h, &name);
                let ph = self.package_hash(&name, &dir)?;
                field(&mut h, &ph);
            }
        }
        let out = hex(&h.finalize());
        self.project_memo.insert(project.name.clone(), out.clone());
        Ok(out)
    }

    fn package_hash(&mut self, name: &str, from: &Path) -> Result<String> {
        self.package_hash_inner(name, from, &mut BTreeSet::new())
    }

    fn package_hash_inner(
        &mut self,
        name: &str,
        from: &Path,
        visiting: &mut BTreeSet<PathBuf>,
    ) -> Result<String> {
        let Some(dir) = resolve_package(name, from, &self.root) else {
            return Ok(format!("not-installed:{name}"));
        };
        if let Some(h) = self.memo.get(&dir) {
            return Ok(h.clone());
        }
        let Some(manifest) = read_manifest(&dir.join("package.json"))? else {
            return Ok(format!("not-installed:{name}"));
        };
        let version = manifest.get("version").and_then(Json::as_str).unwrap_or("");
        if !visiting.insert(dir.clone()) {
            return Ok(format!("{name}@{version}"));
        }
        let mut h = Sha256::new();
        field(&mut h, &format!("{name}@{version}"));
        for (dep, _) in declared(&manifest, &DEP_FIELDS) {
            let dh = self.package_hash_inner(&dep, &dir, visiting)?;
            field(&mut h, &dep);
            field(&mut h, &dh);
        }
        visiting.remove(&dir);
        let out = hex(&h.finalize());
        self.memo.insert(dir, out.clone());
        Ok(out)
    }
}

fn read_manifest(path: &Path) -> Result<Option<JsonMap>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).wrap_err_with(|| format!("reading {}", path.display())),
    };
    let v: Json =
        serde_json::from_str(&raw).wrap_err_with(|| format!("parsing {}", path.display()))?;
    Ok(v.as_object().cloned())
}

/// `(name, spec)` over the given dependency fields, sorted and unique.
fn declared(manifest: &JsonMap, fields: &[&str]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for f in fields {
        for (k, v) in manifest
            .get(*f)
            .and_then(Json::as_object)
            .into_iter()
            .flatten()
        {
            out.entry(k.clone())
                .or_insert_with(|| v.as_str().unwrap_or_default().to_string());
        }
    }
    out
}

/// Node resolution: `<dir>/node_modules/<name>` from `from` upwards, stopping
/// at the filesystem root; the result is canonicalized so symlinked stores
/// (bun, pnpm) key by the real package directory.
fn resolve_package(name: &str, from: &Path, _root: &Path) -> Option<PathBuf> {
    let mut dir = Some(from);
    while let Some(d) = dir {
        let candidate = d.join("node_modules").join(name);
        if candidate.join("package.json").is_file() {
            return candidate.canonicalize().ok();
        }
        dir = d.parent();
    }
    None
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(name: &str, root: &str) -> Project {
        Project {
            name: name.into(),
            root: root.into(),
            project_type: None,
            tags: vec![],
            implicit_dependencies: vec![],
            targets: BTreeMap::new(),
            deps: BTreeSet::new(),
            build_deps: BTreeSet::new(),
        }
    }

    fn hasher<'a>(
        graph: &'a ProjectGraph,
        named: &BTreeMap<String, Vec<Json>>,
        files: &[&str],
    ) -> Hasher<'a> {
        Hasher::new(
            Path::new("/nonexistent-butler-ws"),
            graph,
            named,
            files.iter().map(|f| (*f).to_string()).collect(),
        )
    }

    #[test]
    fn nested_projects_own_their_files() {
        let g = ProjectGraph {
            projects: [
                ("outer".to_string(), project("outer", "libs/outer")),
                ("inner".to_string(), project("inner", "libs/outer/inner")),
            ]
            .into(),
            ..Default::default()
        };
        let h = hasher(
            &g,
            &BTreeMap::new(),
            &["libs/outer/a.rs", "libs/outer/inner/b.rs", "x.txt"],
        );
        assert_eq!(h.owned["outer"], vec!["libs/outer/a.rs"]);
        assert_eq!(h.owned["inner"], vec!["libs/outer/inner/b.rs"]);
    }

    #[test]
    fn split_follows_nx() {
        let g = ProjectGraph::default();
        let named = BTreeMap::from([
            (
                "production".to_string(),
                vec![json!("default"), json!("!{projectRoot}/**/*.spec.ts")],
            ),
            (
                "sharedGlobals".to_string(),
                vec![json!("{workspaceRoot}/ci.yml")],
            ),
        ]);
        let h = hasher(&g, &named, &[]);
        let s = h
            .split(&[
                json!("production"),
                json!("^production"),
                json!("^{projectRoot}/x"),
                json!({"externalDependencies": ["@biomejs/biome"]}),
                json!({"env": "CI"}),
                json!({"input": "production", "projects": ["a"]}),
            ])
            .unwrap();
        assert_eq!(
            s.self_inputs,
            vec![
                Input::FileSet("{projectRoot}/**/*".into()),
                Input::FileSet("!{projectRoot}/**/*.spec.ts".into()),
                Input::External(vec!["@biomejs/biome".into()]),
                Input::Env("CI".into()),
            ]
        );
        assert_eq!(s.deps_inputs, vec!["production"]);
        assert_eq!(s.deps_filesets, vec!["{projectRoot}/x"]);
        assert_eq!(
            s.project_inputs,
            vec![("production".to_string(), vec!["a".to_string()])]
        );
        assert!(h.split(&[json!("nope"), json!({"input": "nope"})]).is_err());
        assert!(
            h.split(&[json!({"dependentTasksOutputFiles": "**/*.d.ts"})])
                .is_err()
        );
    }

    #[test]
    fn workspace_and_project_filesets_hash_different_universes() {
        let g = ProjectGraph {
            projects: [("a".to_string(), project("a", "libs/a"))].into(),
            ..Default::default()
        };
        let mut h = hasher(
            &g,
            &BTreeMap::new(),
            &["libs/a/src/x.rs", "libs/b/y.rs", "Cargo.lock"],
        );
        let a = g.projects["a"].clone();
        let base = h.filesets_hash(&a, &["{projectRoot}/**/*".into()]).unwrap();
        let ws = h
            .filesets_hash(&a, &["{workspaceRoot}/Cargo.lock".into()])
            .unwrap();
        assert_ne!(base, ws);
        assert!(h.filesets_hash(&a, &["src/**/*".into()]).is_err());
    }

    #[test]
    fn canonical_sorts_keys() {
        assert_eq!(
            canonical(&json!({"b": 1, "a": {"d": [1, {"z": 0, "y": 1}], "c": null}})),
            r#"{"a":{"c":null,"d":[1,{"y":1,"z":0}]},"b":1}"#
        );
    }
}
