//! Rust SDK for the `DevEnvironment` platform API (Crossplane claim).
//!
//! Same surface as the python/node SDKs (platform/sdk/*): create / get /
//! connection / delete against `platform.playground.io/v1alpha1 DevEnvironment`.

use k8s_openapi::api::core::v1::Secret;
use kube::Client;
use kube::api::{Api, ApiResource, DeleteParams, DynamicObject, Patch, PatchParams};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const GROUP: &str = "platform.playground.io";
const VERSION: &str = "v1alpha1";
const KIND: &str = "DevEnvironment";
const PLURAL: &str = "devenvironments";
const FIELD_MANAGER: &str = "devenv-sdk";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("kubernetes api error: {0}")]
    Kube(#[from] kube::Error),
    #[error("environment '{0}' is not provisioned yet (no status.environment)")]
    NotReady(String),
    #[error("secret '{secret}' in namespace '{namespace}' is missing key '{key}'")]
    MissingSecretKey {
        secret: String,
        namespace: String,
        key: String,
    },
}

/// Claim status as reported by the composition.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Environment {
    pub namespace: String,
    #[serde(default)]
    pub postgres_secret: Option<String>,
    #[serde(default)]
    pub redis_host: Option<String>,
    #[serde(default)]
    pub nats_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub ready: bool,
    pub environment: Option<Environment>,
    pub conditions: Vec<Value>,
}

#[derive(Debug, Clone)]
pub struct PostgresConnection {
    pub uri: String,
    pub username: String,
    pub password: String,
    pub host: String,
    pub port: String,
    pub dbname: String,
}

#[derive(Debug, Clone)]
pub struct Connection {
    pub postgres: Option<PostgresConnection>,
    pub redis_host: Option<String>,
    pub nats_url: Option<String>,
}

pub struct DevEnvClient {
    client: Client,
}

impl DevEnvClient {
    /// Connect using default kubeconfig loading rules (KUBECONFIG / ~/.kube/config).
    pub async fn new() -> Result<Self, Error> {
        Ok(Self {
            client: Client::try_default().await?,
        })
    }

    pub fn from_client(client: Client) -> Self {
        Self { client }
    }

    fn api(&self, namespace: &str) -> Api<DynamicObject> {
        let ar = ApiResource {
            group: GROUP.into(),
            version: VERSION.into(),
            api_version: format!("{GROUP}/{VERSION}"),
            kind: KIND.into(),
            plural: PLURAL.into(),
        };
        Api::namespaced_with(self.client.clone(), namespace, &ar)
    }

    /// Idempotently create/update a claim (server-side apply).
    pub async fn create(
        &self,
        name: &str,
        namespace: &str,
        postgres: bool,
        redis: bool,
        nats: bool,
    ) -> Result<(), Error> {
        let claim = json!({
            "apiVersion": format!("{GROUP}/{VERSION}"),
            "kind": KIND,
            "metadata": { "name": name, "namespace": namespace },
            "spec": { "parameters": {
                "postgres": { "enabled": postgres },
                "redis": { "enabled": redis },
                "nats": { "enabled": nats },
            }}
        });
        self.api(namespace)
            .patch(
                name,
                &PatchParams::apply(FIELD_MANAGER).force(),
                &Patch::Apply(&claim),
            )
            .await?;
        Ok(())
    }

    /// Claim status; tolerates a claim that has no status yet.
    pub async fn get(&self, name: &str, namespace: &str) -> Result<Status, Error> {
        let obj = self.api(namespace).get(name).await?;
        let status = obj.data.get("status").cloned().unwrap_or(Value::Null);
        let conditions = status
            .get("conditions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let ready = conditions.iter().any(|c| {
            c.get("type").and_then(Value::as_str) == Some("Ready")
                && c.get("status").and_then(Value::as_str) == Some("True")
        });
        let environment = status
            .get("environment")
            .cloned()
            .and_then(|v| serde_json::from_value(rename_keys(v)).ok());
        Ok(Status {
            ready,
            environment,
            conditions,
        })
    }

    /// Resolve endpoints + postgres credentials (reads the CNPG secret).
    pub async fn connection(&self, name: &str, namespace: &str) -> Result<Connection, Error> {
        let status = self.get(name, namespace).await?;
        let env = status
            .environment
            .ok_or_else(|| Error::NotReady(name.to_string()))?;

        let postgres = match &env.postgres_secret {
            None => None,
            Some(secret_name) => {
                let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &env.namespace);
                let secret = secrets.get(secret_name).await?;
                let data = secret.data.unwrap_or_default();
                let field = |key: &str| -> Result<String, Error> {
                    data.get(key)
                        .map(|v| String::from_utf8_lossy(&v.0).into_owned())
                        .ok_or_else(|| Error::MissingSecretKey {
                            secret: secret_name.clone(),
                            namespace: env.namespace.clone(),
                            key: key.to_string(),
                        })
                };
                Some(PostgresConnection {
                    uri: field("uri")?,
                    username: field("username")?,
                    password: field("password")?,
                    host: field("host")?,
                    port: field("port")?,
                    dbname: field("dbname")?,
                })
            }
        };

        Ok(Connection {
            postgres,
            redis_host: env.redis_host,
            nats_url: env.nats_url,
        })
    }

    pub async fn delete(&self, name: &str, namespace: &str) -> Result<(), Error> {
        self.api(namespace)
            .delete(name, &DeleteParams::default())
            .await?;
        Ok(())
    }
}

/// status.environment uses camelCase on the wire; map to the struct's fields.
fn rename_keys(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| {
                    let k = match k.as_str() {
                        "postgresSecret" => "postgres_secret".to_string(),
                        "redisHost" => "redis_host".to_string(),
                        "natsUrl" => "nats_url".to_string(),
                        _ => k,
                    };
                    (k, v)
                })
                .collect(),
        ),
        other => other,
    }
}
