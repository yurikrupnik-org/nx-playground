use eyre::Result;
use futures::TryStreamExt;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{
    Api, Client,
    api::{ApiResource, DynamicObject, GroupVersionKind, ListParams, Patch, PatchParams},
    runtime::watcher::{self, Event as WatcherEvent},
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::{info, warn};

// ─── Types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Namespaced,
    Cluster,
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scope::Namespaced => write!(f, "Namespaced"),
            Scope::Cluster => write!(f, "Cluster"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CrdInstance {
    pub name: String,
    pub namespace: Option<String>,
    pub api_version: String,
    pub kind: String,
    pub group: String,
    pub version: String,
    pub scope: Scope,
    pub labels: BTreeMap<String, String>,
    pub creation_timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrdSummary {
    pub name: String,
    pub group: String,
    pub kind: String,
    pub version: String,
    pub scope: Scope,
    pub instance_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardData {
    pub by_group: BTreeMap<String, Vec<CrdInstance>>,
    pub by_kind: BTreeMap<String, Vec<CrdInstance>>,
    pub by_namespace: BTreeMap<String, Vec<CrdInstance>>,
    pub by_version: BTreeMap<String, Vec<CrdInstance>>,
    pub by_scope: BTreeMap<String, Vec<CrdInstance>>,
    pub crd_summaries: Vec<CrdSummary>,
    pub total_crds: usize,
    pub total_instances: usize,
    pub total_namespaced: usize,
    pub total_cluster_scoped: usize,
}

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum GroupBy {
    Group,
    Kind,
    Namespace,
    Version,
    Scope,
}

impl std::fmt::Display for GroupBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GroupBy::Group => write!(f, "group"),
            GroupBy::Kind => write!(f, "kind"),
            GroupBy::Namespace => write!(f, "namespace"),
            GroupBy::Version => write!(f, "version"),
            GroupBy::Scope => write!(f, "scope"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupedCrds {
    pub group_by: GroupBy,
    pub groups: BTreeMap<String, Vec<CrdInstance>>,
    pub total_crds: usize,
    pub total_instances: usize,
}

// ─── Live state ───────────────────────────────────────────────────────

/// Shared live state that watchers update and the API reads.
pub struct LiveState {
    data: RwLock<DashboardData>,
    tx: broadcast::Sender<()>,
}

impl LiveState {
    pub fn new() -> (Arc<Self>, broadcast::Receiver<()>) {
        let (tx, rx) = broadcast::channel(64);
        let state = Arc::new(Self {
            data: RwLock::new(DashboardData {
                by_group: BTreeMap::new(),
                by_kind: BTreeMap::new(),
                by_namespace: BTreeMap::new(),
                by_version: BTreeMap::new(),
                by_scope: BTreeMap::new(),
                crd_summaries: Vec::new(),
                total_crds: 0,
                total_instances: 0,
                total_namespaced: 0,
                total_cluster_scoped: 0,
            }),
            tx,
        });
        (state, rx)
    }

    pub async fn get_dashboard(&self) -> DashboardData {
        self.data.read().await.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }

    async fn update(&self, data: DashboardData) {
        let mut w = self.data.write().await;
        *w = data;
        let _ = self.tx.send(()); // notify SSE subscribers
    }
}

// ─── Building dashboard from instances ────────────────────────────────

fn group_key(key: GroupBy, inst: &CrdInstance) -> String {
    match key {
        GroupBy::Group => inst.group.clone(),
        GroupBy::Kind => inst.kind.clone(),
        GroupBy::Namespace => inst
            .namespace
            .clone()
            .unwrap_or_else(|| "<cluster-scoped>".to_string()),
        GroupBy::Version => inst.api_version.clone(),
        GroupBy::Scope => inst.scope.to_string(),
    }
}

fn build_groups(instances: &[CrdInstance], key: GroupBy) -> BTreeMap<String, Vec<CrdInstance>> {
    let mut groups: BTreeMap<String, Vec<CrdInstance>> = BTreeMap::new();
    for inst in instances {
        groups
            .entry(group_key(key, inst))
            .or_default()
            .push(inst.clone());
    }
    groups
}

fn build_dashboard(
    all_instances: &[CrdInstance],
    summaries: &[CrdSummary],
    total_crds: usize,
) -> DashboardData {
    let total_instances = all_instances.len();
    let total_namespaced = all_instances
        .iter()
        .filter(|i| i.scope == Scope::Namespaced)
        .count();

    DashboardData {
        by_group: build_groups(all_instances, GroupBy::Group),
        by_kind: build_groups(all_instances, GroupBy::Kind),
        by_namespace: build_groups(all_instances, GroupBy::Namespace),
        by_version: build_groups(all_instances, GroupBy::Version),
        by_scope: build_groups(all_instances, GroupBy::Scope),
        crd_summaries: summaries.to_vec(),
        total_crds,
        total_instances,
        total_namespaced,
        total_cluster_scoped: total_instances - total_namespaced,
    }
}

// ─── Initial full scan ────────────────────────────────────────────────

pub async fn collect_all_instances(
    client: &Client,
) -> Result<(Vec<CrdInstance>, Vec<CrdSummary>, usize)> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&ListParams::default()).await?;
    let total_crds = crds.items.len();

    info!("Found {} CRDs in cluster", total_crds);

    let mut all_instances: Vec<CrdInstance> = Vec::new();
    let mut summaries: Vec<CrdSummary> = Vec::new();

    for crd in &crds.items {
        let crd_name = crd.metadata.name.as_deref().unwrap_or("unknown");
        let spec = &crd.spec;
        let group = &spec.group;
        let kind = &spec.names.kind;
        let scope = if spec.scope == "Namespaced" {
            Scope::Namespaced
        } else {
            Scope::Cluster
        };

        let version = spec
            .versions
            .iter()
            .find(|v| v.storage)
            .or(spec.versions.first())
            .map(|v| v.name.as_str())
            .unwrap_or("v1");

        let gvk = GroupVersionKind::gvk(group, version, kind);
        let ar = ApiResource::from_gvk(&gvk);

        let result: kube::Result<kube::api::ObjectList<DynamicObject>> =
            Api::<DynamicObject>::all_with(client.clone(), &ar)
                .list(&ListParams::default())
                .await;

        match result {
            Ok(list) => {
                let count = list.items.len();
                if count > 0 {
                    info!("  {}: {} instances", crd_name, count);
                }
                summaries.push(CrdSummary {
                    name: crd_name.to_string(),
                    group: group.clone(),
                    kind: kind.clone(),
                    version: version.to_string(),
                    scope,
                    instance_count: count,
                });
                for obj in list.items {
                    let meta = &obj.metadata;
                    all_instances.push(CrdInstance {
                        name: meta.name.clone().unwrap_or_default(),
                        namespace: meta.namespace.clone(),
                        api_version: format!("{}/{}", group, version),
                        kind: kind.clone(),
                        group: group.clone(),
                        version: version.to_string(),
                        scope,
                        labels: meta.labels.clone().unwrap_or_default(),
                        creation_timestamp: meta
                            .creation_timestamp
                            .as_ref()
                            .map(|t| t.0.to_string()),
                    });
                }
            }
            Err(e) => {
                summaries.push(CrdSummary {
                    name: crd_name.to_string(),
                    group: group.clone(),
                    kind: kind.clone(),
                    version: version.to_string(),
                    scope,
                    instance_count: 0,
                });
                // 404s are expected for GKE-managed CRDs whose API isn't served
                tracing::debug!("  {}: failed to list: {}", crd_name, e);
            }
        }
    }

    Ok((all_instances, summaries, total_crds))
}

// ─── Background watcher ───────────────────────────────────────────────

/// Spawn background watchers that keep `LiveState` up-to-date.
///
/// Strategy:
/// 1. Do an initial full scan to populate state
/// 2. Watch the CRD list for new/removed types
/// 3. For CRD types that have instances, spawn individual watchers
/// 4. On any change, rebuild the affected parts of the dashboard
///
/// For simplicity and reliability we use a hybrid approach:
/// - Watch CRDs for structural changes (new CRD added/removed)
/// - Periodically re-scan instance counts for active CRDs (every 10s)
/// - Individual watchers for CRDs that had >0 instances (real-time)
pub fn spawn_watchers(client: Client, state: Arc<LiveState>) {
    let ready = Arc::new(tokio::sync::Notify::new());

    // Initial full scan
    let client_scan = client.clone();
    let state_scan = state.clone();
    let ready_signal = ready.clone();
    tokio::spawn(async move {
        match collect_all_instances(&client_scan).await {
            Ok((instances, summaries, total_crds)) => {
                let data = build_dashboard(&instances, &summaries, total_crds);
                info!(
                    "Initial scan complete: {} CRDs, {} instances",
                    total_crds,
                    instances.len()
                );
                state_scan.update(data).await;
            }
            Err(e) => warn!("Initial scan failed: {}", e),
        }
        ready_signal.notify_waiters();
    });

    // Watch CRD list — waits for initial scan, then watches for new/removed types
    let client_crd = client.clone();
    let state_crd = state.clone();
    let ready_crd = ready.clone();
    tokio::spawn(async move {
        ready_crd.notified().await;
        watch_crd_list(client_crd, state_crd).await;
    });

    // Watch individual active resource types — waits for initial scan
    let client_res = client.clone();
    let state_res = state.clone();
    let ready_res = ready.clone();
    tokio::spawn(async move {
        ready_res.notified().await;
        watch_active_resources(client_res, state_res).await;
    });
}

/// Watch the CRD list itself. When CRDs are added or removed after init, do a full rescan.
async fn watch_crd_list(client: Client, state: Arc<LiveState>) {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let wc = watcher::Config::default();

    loop {
        let stream = watcher::watcher(crd_api.clone(), wc.clone());
        futures::pin_mut!(stream);
        let mut initialized = false;

        while let Ok(Some(event)) = stream.try_next().await {
            match event {
                WatcherEvent::InitDone => {
                    initialized = true;
                    info!("CRD watcher initialized");
                }
                WatcherEvent::Apply(_) | WatcherEvent::Delete(_) if initialized => {
                    info!("CRD list changed, triggering full rescan...");
                    match collect_all_instances(&client).await {
                        Ok((instances, summaries, total_crds)) => {
                            state
                                .update(build_dashboard(&instances, &summaries, total_crds))
                                .await;
                        }
                        Err(e) => warn!("Rescan after CRD change failed: {}", e),
                    }
                }
                _ => {}
            }
        }

        warn!("CRD watcher stream ended, restarting in 5s...");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

/// For each CRD type that has instances, watch for changes.
/// When any instance changes, rebuild dashboard state.
async fn watch_active_resources(client: Client, state: Arc<LiveState>) {
    // Discover which CRD types have instances
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = match crd_api.list(&ListParams::default()).await {
        Ok(list) => list.items,
        Err(e) => {
            warn!("Failed to list CRDs for watchers: {}", e);
            return;
        }
    };

    let mut active_types: Vec<(ApiResource, String, String, String, Scope)> = Vec::new();

    for crd in &crds {
        let spec = &crd.spec;
        let group = &spec.group;
        let kind = &spec.names.kind;
        let scope = if spec.scope == "Namespaced" {
            Scope::Namespaced
        } else {
            Scope::Cluster
        };
        let version = spec
            .versions
            .iter()
            .find(|v| v.storage)
            .or(spec.versions.first())
            .map(|v| v.name.as_str())
            .unwrap_or("v1");

        let gvk = GroupVersionKind::gvk(group, version, kind);
        let ar = ApiResource::from_gvk(&gvk);

        // Quick check if this type has any instances
        let api: Api<DynamicObject> = Api::all_with(client.clone(), &ar);
        if let Ok(list) = api.list(&ListParams::default().limit(1)).await {
            if !list.items.is_empty() {
                active_types.push((
                    ar,
                    group.clone(),
                    version.to_string(),
                    kind.clone(),
                    scope,
                ));
            }
        }
    }

    info!(
        "Starting watchers for {} active CRD types",
        active_types.len()
    );

    // Shared change counter — when any watcher detects a change, trigger rebuild
    let change_notify = Arc::new(tokio::sync::Notify::new());

    // Spawn a watcher for each active type
    for (ar, _group, _version, kind, _scope) in &active_types {
        let client = client.clone();
        let ar = ar.clone();
        let kind = kind.clone();
        let notify = change_notify.clone();

        tokio::spawn(async move {
            let api: Api<DynamicObject> = Api::all_with(client, &ar);
            let wc = watcher::Config::default();

            loop {
                let stream = watcher::watcher(api.clone(), wc.clone());
                futures::pin_mut!(stream);
                let mut initialized = false;

                while let Ok(Some(event)) = stream.try_next().await {
                    match event {
                        WatcherEvent::InitDone => {
                            initialized = true;
                        }
                        WatcherEvent::Apply(_) | WatcherEvent::Delete(_) if initialized => {
                            // Only notify after initial list is done
                            notify.notify_one();
                        }
                        _ => {}
                    }
                }

                warn!("Watcher for {} ended, restarting in 5s...", kind);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }

    // Rebuilder: coalesces rapid changes and does a full rebuild
    loop {
        change_notify.notified().await;

        // Coalesce: wait a short time for more changes before rebuilding
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Drain any additional notifications
        loop {
            match tokio::time::timeout(
                std::time::Duration::from_millis(100),
                change_notify.notified(),
            )
            .await
            {
                Ok(()) => continue,
                Err(_) => break,
            }
        }

        info!("Resource change detected, rebuilding dashboard...");
        match collect_all_instances(&client).await {
            Ok((instances, summaries, total_crds)) => {
                state
                    .update(build_dashboard(&instances, &summaries, total_crds))
                    .await;
                info!("Dashboard rebuilt: {} instances", instances.len());
            }
            Err(e) => warn!("Rebuild failed: {}", e),
        }
    }
}

// ─── Single group query (for CLI) ─────────────────────────────────────

pub async fn discover_and_group(client: Client, group_by: GroupBy) -> Result<GroupedCrds> {
    let (all_instances, _, total_crds) = collect_all_instances(&client).await?;
    let total_instances = all_instances.len();
    let groups = build_groups(&all_instances, group_by);

    Ok(GroupedCrds {
        group_by,
        groups,
        total_crds,
        total_instances,
    })
}

pub async fn discover_dashboard(client: &Client) -> Result<DashboardData> {
    let (all_instances, summaries, total_crds) = collect_all_instances(client).await?;
    Ok(build_dashboard(&all_instances, &summaries, total_crds))
}

// ─── Clean up server-managed fields ───────────────────────────────────

/// Strip Kubernetes-generated fields that clutter the YAML view.
fn clean_resource_yaml(val: &mut serde_json::Value) {
    if let Some(obj) = val.as_object_mut() {
        // Remove top-level status (server-generated)
        obj.remove("status");

        if let Some(meta) = obj.get_mut("metadata").and_then(|m| m.as_object_mut()) {
            // Remove server-managed metadata fields
            meta.remove("managedFields");
            meta.remove("resourceVersion");
            meta.remove("uid");
            meta.remove("generation");
            meta.remove("creationTimestamp");
            meta.remove("selfLink");

            // Clean annotations
            if let Some(annotations) = meta.get_mut("annotations").and_then(|a| a.as_object_mut())
            {
                let generated_prefixes = [
                    "kubectl.kubernetes.io/last-applied-configuration",
                    "crossplane.io/composition-resource-name",
                ];
                let to_remove: Vec<String> = annotations
                    .keys()
                    .filter(|k| generated_prefixes.iter().any(|p| k.starts_with(p)))
                    .cloned()
                    .collect();
                for k in to_remove {
                    annotations.remove(&k);
                }
                // Remove annotations entirely if empty
                if annotations.is_empty() {
                    meta.remove("annotations");
                }
            }

            // Remove finalizers if empty
            if let Some(finalizers) = meta.get("finalizers") {
                if finalizers.as_array().is_some_and(|a| a.is_empty()) {
                    meta.remove("finalizers");
                }
            }
        }
    }
}

// ─── Single resource YAML get/apply ───────────────────────────────────

pub async fn get_resource_yaml(
    client: &Client,
    group: &str,
    version: &str,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
) -> Result<String> {
    let gvk = GroupVersionKind::gvk(group, version, kind);
    let ar = ApiResource::from_gvk(&gvk);

    let api: Api<DynamicObject> = match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
        None => Api::all_with(client.clone(), &ar),
    };

    let obj = api.get(name).await?;
    let mut json_val = serde_json::to_value(&obj)?;
    clean_resource_yaml(&mut json_val);
    let yaml = serde_yaml::to_string(&json_val)?;
    Ok(yaml)
}

pub async fn apply_resource_yaml(
    client: &Client,
    group: &str,
    version: &str,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
    yaml_str: &str,
) -> Result<String> {
    let gvk = GroupVersionKind::gvk(group, version, kind);
    let ar = ApiResource::from_gvk(&gvk);

    let api: Api<DynamicObject> = match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
        None => Api::all_with(client.clone(), &ar),
    };

    let data: serde_json::Value = serde_yaml::from_str(yaml_str)?;
    let params = PatchParams::apply("crd-grouper");
    let obj = api.patch(name, &params, &Patch::Apply(data)).await?;

    let yaml = serde_yaml::to_string(&obj)?;
    Ok(yaml)
}

pub async fn create_client() -> Result<Client> {
    Ok(Client::try_default().await?)
}
