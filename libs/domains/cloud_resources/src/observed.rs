//! Cluster-observed cloud inventory (read-only).
//!
//! Reads the `Object` resources produced by the `CloudInventory` composition
//! (`platform/cloud-inventory/`). Those Objects are created with
//! `managementPolicies: ["Observe"]`, so everything here is a mirror of live
//! infrastructure — this module never writes to the cluster and exposes no
//! mutating call.
//!
//! Contract with the composition (labels on each `Object`):
//!
//! | label                                  | meaning                              |
//! |----------------------------------------|--------------------------------------|
//! | `platform.playground.io/inventory`     | claim name — the list selector        |
//! | `platform.playground.io/resource-type` | [`ResourceType`], serde lowercase     |
//!
//! Gated behind the `k8s` feature so postgres-only consumers keep a kube-free
//! dependency tree.

use std::str::FromStr;

use kube::api::{Api, ApiResource, DynamicObject, GroupVersionKind, ListParams};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::models::{ResourceStatus, ResourceType, Tag};

/// Label carrying the owning `CloudInventory` claim name.
pub const INVENTORY_LABEL: &str = "platform.playground.io/inventory";
/// Label carrying the declared [`ResourceType`].
pub const RESOURCE_TYPE_LABEL: &str = "platform.playground.io/resource-type";

const OBJECT_GROUP: &str = "kubernetes.crossplane.io";
const OBJECT_VERSION: &str = "v1alpha2";
const OBJECT_KIND: &str = "Object";

/// Region reported for in-cluster targets — the k8s composition has no
/// geographic region; a future cloud composition fills the real one.
const IN_CLUSTER_REGION: &str = "in-cluster";

/// One observed cloud resource. Field names mirror [`crate::models::CloudResource`]
/// where they overlap, so a UI can render both with the same code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ObservedCloudResource {
    /// `metadata.uid` of the observing `Object` — stable for the lifetime of
    /// the inventory entry.
    pub id: Uuid,
    /// Owning `CloudInventory` claim.
    pub claim: String,
    pub name: String,
    pub api_version: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub resource_type: ResourceType,
    pub status: ResourceStatus,
    pub region: String,
    /// The live manifest (`status.atProvider.manifest`) with `managedFields`
    /// stripped, or `null` before the first successful observation.
    pub configuration: Value,
    /// Labels of the observed resource, sorted by key.
    pub tags: Vec<Tag>,
}

/// Query filter. Everything is optional; unset fields match anything.
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct ObservedFilter {
    pub resource_type: Option<ResourceType>,
    pub status: Option<ResourceStatus>,
    /// Kubernetes kind, case-insensitive (e.g. `Deployment`).
    pub kind: Option<String>,
    pub namespace: Option<String>,
    /// Restrict to a single `CloudInventory` claim (pushed down to a label
    /// selector).
    pub claim: Option<String>,
}

impl ObservedFilter {
    /// Predicates the label selector cannot express.
    fn matches(&self, r: &ObservedCloudResource) -> bool {
        self.resource_type.is_none_or(|t| t == r.resource_type)
            && self.status.is_none_or(|s| s == r.status)
            && self
                .kind
                .as_deref()
                .is_none_or(|k| k.eq_ignore_ascii_case(&r.kind))
            && self
                .namespace
                .as_deref()
                .is_none_or(|ns| Some(ns) == r.namespace.as_deref())
    }
}

/// Declared type wins; otherwise infer from the kubernetes kind.
fn map_resource_type(label: Option<&str>, kind: &str) -> ResourceType {
    if let Some(t) = label.and_then(|l| ResourceType::from_str(l).ok()) {
        return t;
    }
    match kind {
        "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "Pod" | "Job" | "CronJob" => {
            ResourceType::Compute
        }
        "Service" | "Ingress" | "NetworkPolicy" | "Gateway" | "HTTPRoute" => ResourceType::Network,
        // CNPG `postgresql.cnpg.io/v1 Cluster`, and friends.
        "Cluster" | "Database" => ResourceType::Database,
        "PersistentVolume" | "PersistentVolumeClaim" | "StorageClass" | "Bucket" => {
            ResourceType::Storage
        }
        "Function" => ResourceType::Serverless,
        _ => ResourceType::Other,
    }
}

/// Crossplane `Ready` condition of the observing `Object` → domain status.
/// Absent condition means the provider has not reconciled it yet.
fn map_status(conditions: Option<&Value>) -> ResourceStatus {
    let ready = conditions
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|c| c["type"].as_str() == Some("Ready"))
        .and_then(|c| c["status"].as_str());
    match ready {
        Some("True") => ResourceStatus::Active,
        Some(_) => ResourceStatus::Failed,
        None => ResourceStatus::Creating,
    }
}

fn strip_managed_fields(manifest: &Value) -> Value {
    let mut manifest = manifest.clone();
    if let Some(meta) = manifest.get_mut("metadata").and_then(Value::as_object_mut) {
        meta.remove("managedFields");
    }
    manifest
}

fn tags_from(manifest: &Value) -> Vec<Tag> {
    let Some(labels) = manifest["metadata"]["labels"].as_object() else {
        return Vec::new();
    };
    // serde_json maps are BTreeMap-backed only with `preserve_order` off; sort
    // explicitly so the API response order is stable either way.
    let mut tags: Vec<Tag> = labels
        .iter()
        .filter_map(|(key, value)| {
            Some(Tag {
                key: key.clone(),
                value: value.as_str()?.to_owned(),
            })
        })
        .collect();
    tags.sort_by(|a, b| a.key.cmp(&b.key));
    tags
}

/// Map one observing `Object` to an inventory entry. `None` (with a warning)
/// for Objects that do not carry the composition's contract — a stray
/// hand-written `Object` must not break the whole listing.
pub fn map_object(obj: &DynamicObject) -> Option<ObservedCloudResource> {
    let object_name = obj.metadata.name.as_deref().unwrap_or("<unnamed>");

    let Some(claim) = obj
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get(INVENTORY_LABEL))
        .cloned()
    else {
        tracing::warn!(
            object = object_name,
            "skipping Object without {INVENTORY_LABEL} label"
        );
        return None;
    };

    let Some(Ok(id)) = obj.metadata.uid.as_deref().map(Uuid::from_str) else {
        tracing::warn!(
            object = object_name,
            "skipping Object with missing/invalid uid"
        );
        return None;
    };

    // Prefer the live manifest; fall back to the desired one so a
    // not-yet-observed target is still listed (status: creating).
    let observed = obj.data.pointer("/status/atProvider/manifest");
    let Some(manifest) = observed.or_else(|| obj.data.pointer("/spec/forProvider/manifest")) else {
        tracing::warn!(object = object_name, "skipping Object without a manifest");
        return None;
    };

    let (Some(api_version), Some(kind), Some(name)) = (
        manifest["apiVersion"].as_str(),
        manifest["kind"].as_str(),
        manifest["metadata"]["name"].as_str(),
    ) else {
        tracing::warn!(
            object = object_name,
            "skipping Object with an incomplete manifest"
        );
        return None;
    };

    let resource_type = map_resource_type(
        obj.metadata
            .labels
            .as_ref()
            .and_then(|l| l.get(RESOURCE_TYPE_LABEL))
            .map(String::as_str),
        kind,
    );

    Some(ObservedCloudResource {
        id,
        claim,
        name: name.to_owned(),
        api_version: api_version.to_owned(),
        kind: kind.to_owned(),
        namespace: manifest["metadata"]["namespace"]
            .as_str()
            .map(str::to_owned),
        resource_type,
        status: map_status(obj.data.pointer("/status/conditions")),
        region: IN_CLUSTER_REGION.to_owned(),
        configuration: observed.map_or(Value::Null, strip_managed_fields),
        tags: tags_from(manifest),
    })
}

/// Read-only client over the composition's observing `Object`s.
#[derive(Clone)]
pub struct K8sInventory {
    api: Api<DynamicObject>,
}

impl std::fmt::Debug for K8sInventory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("K8sInventory")
    }
}

impl K8sInventory {
    /// Build from the ambient kubeconfig / in-cluster service account.
    pub async fn try_default() -> Result<Self, kube::Error> {
        let client = kube::Client::try_default().await?;
        let gvk = GroupVersionKind::gvk(OBJECT_GROUP, OBJECT_VERSION, OBJECT_KIND);
        let api_resource = ApiResource::from_gvk(&gvk);
        Ok(Self {
            api: Api::all_with(client, &api_resource),
        })
    }

    /// All inventory entries matching `filter`, sorted by claim then name.
    pub async fn list(
        &self,
        filter: &ObservedFilter,
    ) -> Result<Vec<ObservedCloudResource>, kube::Error> {
        let selector = match filter.claim.as_deref() {
            Some(claim) => format!("{INVENTORY_LABEL}={claim}"),
            None => INVENTORY_LABEL.to_owned(),
        };
        let objects = self
            .api
            .list(&ListParams::default().labels(&selector))
            .await?;

        let mut resources: Vec<ObservedCloudResource> = objects
            .items
            .iter()
            .filter_map(map_object)
            .filter(|r| filter.matches(r))
            .collect();
        resources.sort_by(|a, b| (&a.claim, &a.name).cmp(&(&b.claim, &b.name)));
        Ok(resources)
    }

    /// One entry by [`ObservedCloudResource::id`]. The kubernetes API has no
    /// lookup-by-uid, so this filters a listing.
    pub async fn get(&self, id: Uuid) -> Result<Option<ObservedCloudResource>, kube::Error> {
        Ok(self
            .list(&ObservedFilter::default())
            .await?
            .into_iter()
            .find(|r| r.id == id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: serde_json::Value) -> DynamicObject {
        serde_json::from_value(value).expect("fixture parses as DynamicObject")
    }

    fn coredns_object() -> DynamicObject {
        object(json!({
            "apiVersion": "kubernetes.crossplane.io/v1alpha2",
            "kind": "Object",
            "metadata": {
                "name": "demo-coredns",
                "uid": "2f1c4b6e-0000-4000-8000-000000000001",
                "labels": {
                    INVENTORY_LABEL: "demo",
                    RESOURCE_TYPE_LABEL: "compute",
                },
            },
            "spec": {
                "managementPolicies": ["Observe"],
                "forProvider": { "manifest": {
                    "apiVersion": "apps/v1",
                    "kind": "Deployment",
                    "metadata": { "name": "coredns", "namespace": "kube-system" },
                }},
            },
            "status": {
                "conditions": [
                    { "type": "Synced", "status": "True" },
                    { "type": "Ready", "status": "True" },
                ],
                "atProvider": { "manifest": {
                    "apiVersion": "apps/v1",
                    "kind": "Deployment",
                    "metadata": {
                        "name": "coredns",
                        "namespace": "kube-system",
                        "labels": { "k8s-app": "kube-dns", "tier": "control-plane" },
                        "managedFields": [{ "manager": "kubectl" }],
                    },
                    "spec": { "replicas": 2 },
                }},
            },
        }))
    }

    #[test]
    fn declared_resource_type_wins_over_kind_inference() {
        // `analytics` is not inferable from `Deployment` — the label decides.
        assert_eq!(
            map_resource_type(Some("analytics"), "Deployment"),
            ResourceType::Analytics
        );
    }

    #[test]
    fn resource_type_falls_back_to_kind() {
        assert_eq!(
            map_resource_type(None, "StatefulSet"),
            ResourceType::Compute
        );
        assert_eq!(map_resource_type(None, "Service"), ResourceType::Network);
        assert_eq!(map_resource_type(None, "Cluster"), ResourceType::Database);
        assert_eq!(
            map_resource_type(None, "PersistentVolumeClaim"),
            ResourceType::Storage
        );
        assert_eq!(map_resource_type(None, "ConfigMap"), ResourceType::Other);
        // Unparseable label behaves like no label.
        assert_eq!(
            map_resource_type(Some("Compute"), "Service"),
            ResourceType::Network
        );
    }

    #[test]
    fn ready_condition_maps_to_status() {
        let ready = json!([{ "type": "Ready", "status": "True" }]);
        let not_ready = json!([{ "type": "Ready", "status": "False" }]);
        let other = json!([{ "type": "Synced", "status": "True" }]);

        assert_eq!(map_status(Some(&ready)), ResourceStatus::Active);
        assert_eq!(map_status(Some(&not_ready)), ResourceStatus::Failed);
        assert_eq!(map_status(Some(&other)), ResourceStatus::Creating);
        assert_eq!(map_status(None), ResourceStatus::Creating);
    }

    #[test]
    fn maps_observed_object_to_inventory_entry() {
        let r = map_object(&coredns_object()).expect("maps");

        assert_eq!(r.claim, "demo");
        assert_eq!(r.name, "coredns");
        assert_eq!(r.api_version, "apps/v1");
        assert_eq!(r.kind, "Deployment");
        assert_eq!(r.namespace.as_deref(), Some("kube-system"));
        assert_eq!(r.resource_type, ResourceType::Compute);
        assert_eq!(r.status, ResourceStatus::Active);
        assert_eq!(r.region, "in-cluster");
        assert_eq!(
            r.tags,
            vec![
                Tag {
                    key: "k8s-app".into(),
                    value: "kube-dns".into()
                },
                Tag {
                    key: "tier".into(),
                    value: "control-plane".into()
                },
            ]
        );
        // Live manifest, minus server bookkeeping.
        assert_eq!(r.configuration["spec"]["replicas"], json!(2));
        assert!(
            r.configuration["metadata"].get("managedFields").is_none(),
            "managedFields must be stripped from configuration"
        );
    }

    #[test]
    fn falls_back_to_desired_manifest_before_first_observation() {
        let mut obj = coredns_object();
        obj.data["status"]
            .as_object_mut()
            .unwrap()
            .remove("atProvider");
        obj.data["status"] = json!({});

        let r = map_object(&obj).expect("maps from spec.forProvider");
        assert_eq!(r.name, "coredns");
        assert_eq!(r.status, ResourceStatus::Creating);
        assert_eq!(r.configuration, Value::Null);
        assert!(r.tags.is_empty());
    }

    #[test]
    fn skips_objects_outside_the_inventory_contract() {
        let mut unlabelled = coredns_object();
        unlabelled.metadata.labels = None;
        assert!(map_object(&unlabelled).is_none());

        let mut no_uid = coredns_object();
        no_uid.metadata.uid = None;
        assert!(map_object(&no_uid).is_none());

        let mut no_manifest = coredns_object();
        no_manifest.data = json!({});
        assert!(map_object(&no_manifest).is_none());
    }

    #[test]
    fn filter_matches_on_non_selector_fields() {
        let r = map_object(&coredns_object()).expect("maps");

        assert!(ObservedFilter::default().matches(&r));
        assert!(
            ObservedFilter {
                kind: Some("deployment".into()),
                namespace: Some("kube-system".into()),
                resource_type: Some(ResourceType::Compute),
                status: Some(ResourceStatus::Active),
                claim: None,
            }
            .matches(&r)
        );
        assert!(
            !ObservedFilter {
                namespace: Some("default".into()),
                ..Default::default()
            }
            .matches(&r)
        );
        assert!(
            !ObservedFilter {
                status: Some(ResourceStatus::Failed),
                ..Default::default()
            }
            .matches(&r)
        );
    }
}
