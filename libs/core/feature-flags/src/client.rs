use crate::config::FlagsmithConfig;
use crate::error::FlagsError;
use crate::flags::{FlagData, FlagDefaults, FlagSet, FlagSource, ResolvedFlag};
use crate::identity::Identity;
use parking_lot::RwLock;
use reqwest::{Client, RequestBuilder};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;

/// Header every Flagsmith SDK endpoint authenticates with.
const ENVIRONMENT_KEY_HEADER: &str = "X-Environment-Key";

/// A Flagsmith feature state as returned by the SDK endpoints.
#[derive(Debug, serde::Deserialize)]
struct WireFlag {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    feature_state_value: Value,
    feature: WireFeature,
}

#[derive(Debug, serde::Deserialize)]
struct WireFeature {
    name: String,
}

/// `GET|POST /identities/` wraps the feature states in an envelope.
#[derive(Debug, serde::Deserialize)]
struct WireIdentity {
    #[serde(default)]
    flags: Vec<WireFlag>,
}

#[derive(Serialize)]
struct TraitPayload<'a> {
    trait_key: &'a str,
    trait_value: &'a Value,
}

#[derive(Serialize)]
struct IdentityRequest<'a> {
    identifier: &'a str,
    traits: Vec<TraitPayload<'a>>,
    /// `false` persists the identity and its traits so segments can target it.
    transient: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CacheKey {
    /// Environment-level flags, no identity.
    Anonymous,
    Identity(String),
}

impl CacheKey {
    fn for_identity(identity: Option<&Identity>) -> Self {
        match identity {
            Some(identity) => Self::Identity(identity.cache_key()),
            None => Self::Anonymous,
        }
    }
}

struct CacheEntry {
    fetched: Instant,
    flags: Arc<FlagData>,
}

struct Inner {
    http: Client,
    config: FlagsmithConfig,
    defaults: BTreeMap<String, ResolvedFlag>,
    /// Pre-built defaults answer, so the degraded path is an `Arc` clone.
    fallback: FlagSet,
    cache: RwLock<HashMap<CacheKey, CacheEntry>>,
}

/// Async Flagsmith client with per-identity evaluation and a TTL cache.
///
/// Cloning is cheap; share one client for the whole process.
#[derive(Clone)]
pub struct FlagClient {
    inner: Arc<Inner>,
}

impl FlagClient {
    /// Build the client. The HTTP client (and its connection pool) is created
    /// once here with the configured timeout.
    ///
    /// This is fallible only because TLS/connector setup can fail; it performs
    /// no network I/O and does not require Flagsmith to be reachable.
    pub fn new(config: FlagsmithConfig, defaults: FlagDefaults) -> Result<Self, FlagsError> {
        let http = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(|source| FlagsError::ClientBuild { source })?;
        let defaults = defaults.map().clone();
        let fallback = FlagSet::new(defaults.clone(), FlagSource::Defaults);

        Ok(Self {
            inner: Arc::new(Inner {
                http,
                config,
                defaults,
                fallback,
                cache: RwLock::new(HashMap::new()),
            }),
        })
    }

    /// Whether a Flagsmith environment key is present. When `false`, [`Self::flags`]
    /// never touches the network.
    pub fn is_configured(&self) -> bool {
        self.inner.config.environment_key.is_some()
    }

    /// Resolve every known flag for `identity` (or the environment defaults when
    /// `identity` is `None`).
    ///
    /// Infallible by design: a flag lookup must never fail a user request. An
    /// unconfigured, unreachable, slow, or malformed Flagsmith yields the
    /// hardcoded defaults with [`FlagSource::Defaults`]. Failures are cached for
    /// the TTL as well, so an outage is not amplified into one upstream request
    /// per inbound request.
    pub async fn flags(&self, identity: Option<&Identity>) -> FlagSet {
        let Some(environment_key) = self.inner.config.environment_key.as_deref() else {
            return self.inner.fallback.clone();
        };

        let key = CacheKey::for_identity(identity);
        if let Some(hit) = self.cached(&key) {
            return hit;
        }

        let resolved = match self.fetch(environment_key, identity).await {
            Ok(remote) => FlagSet::new(self.merge(remote), FlagSource::Remote),
            Err(error) => {
                tracing::warn!(
                    identity = identity.map_or("<anonymous>", Identity::identifier),
                    %error,
                    "Flagsmith lookup failed; serving default feature flags"
                );
                self.inner.fallback.clone()
            }
        };

        self.store(key, &resolved);
        resolved
    }

    /// Remote values win over defaults per flag; flags Flagsmith did not return
    /// keep their default, and flags only Flagsmith knows about are passed through.
    fn merge(&self, remote: Vec<WireFlag>) -> BTreeMap<String, ResolvedFlag> {
        let mut merged = self.inner.defaults.clone();
        for flag in remote {
            merged.insert(
                flag.feature.name,
                ResolvedFlag::new(flag.enabled, flag.feature_state_value),
            );
        }
        merged
    }

    async fn fetch(
        &self,
        environment_key: &str,
        identity: Option<&Identity>,
    ) -> Result<Vec<WireFlag>, FlagsError> {
        let api_url = &self.inner.config.api_url;
        match identity {
            None => {
                let url = format!("{api_url}/flags/");
                let request = self.inner.http.get(&url);
                self.send(request, environment_key, &url).await
            }
            // Traits present: upsert them so Flagsmith segments can target
            // "user X on app Y". The GET form is read-only and would drop them.
            Some(identity) if identity.has_traits() => {
                let url = format!("{api_url}/identities/");
                let body = IdentityRequest {
                    identifier: identity.identifier(),
                    traits: identity
                        .trait_map()
                        .iter()
                        .map(|(trait_key, trait_value)| TraitPayload {
                            trait_key,
                            trait_value,
                        })
                        .collect(),
                    transient: false,
                };
                let request = self.inner.http.post(&url).json(&body);
                let envelope: WireIdentity = self.send(request, environment_key, &url).await?;
                Ok(envelope.flags)
            }
            Some(identity) => {
                let url = format!("{api_url}/identities/");
                let request = self
                    .inner
                    .http
                    .get(&url)
                    .query(&[("identifier", identity.identifier())]);
                let envelope: WireIdentity = self.send(request, environment_key, &url).await?;
                Ok(envelope.flags)
            }
        }
    }

    async fn send<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        environment_key: &str,
        url: &str,
    ) -> Result<T, FlagsError> {
        let response = request
            .header(ENVIRONMENT_KEY_HEADER, environment_key)
            .send()
            .await
            .map_err(|source| FlagsError::Request {
                url: url.to_string(),
                source,
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(FlagsError::Status {
                status: status.as_u16(),
                url: url.to_string(),
            });
        }

        response
            .json::<T>()
            .await
            .map_err(|source| FlagsError::Decode {
                url: url.to_string(),
                source,
            })
    }

    fn cached(&self, key: &CacheKey) -> Option<FlagSet> {
        let ttl = self.inner.config.cache_ttl;
        let cache = self.inner.cache.read();
        cache
            .get(key)
            .filter(|entry| entry.fetched.elapsed() < ttl)
            .map(|entry| FlagSet::from_data(Arc::clone(&entry.flags)))
    }

    fn store(&self, key: CacheKey, flags: &FlagSet) {
        self.inner.cache.write().insert(
            key,
            CacheEntry {
                fetched: Instant::now(),
                flags: Arc::clone(flags.data()),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use axum::extract::RawQuery;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;
    use axum::{Json, Router};
    use parking_lot::Mutex;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn catalogue() -> FlagDefaults {
        FlagDefaults::new()
            .bool("todo_app_web", true)
            .bool("todo_app_htmx", true)
            .bool("todo_app_astro", true)
            .bool("todo_realtime", true)
            .bool("todo_write", true)
            .int("todo_max_items", -1)
    }

    fn client(config: FlagsmithConfig) -> FlagClient {
        FlagClient::new(config, catalogue()).unwrap()
    }

    // ---------------------------------------------------------------- mapping

    #[test]
    fn remote_flags_override_defaults_and_gaps_keep_theirs() {
        let remote: Vec<WireFlag> = serde_json::from_value(json!([
            { "enabled": false, "feature_state_value": null,
              "feature": { "id": 1, "name": "todo_write" } },
            { "enabled": true, "feature_state_value": 25,
              "feature": { "id": 2, "name": "todo_max_items" } },
            { "enabled": true, "feature_state_value": "7",
              "feature": { "id": 3, "name": "todo_app_htmx" } },
            { "enabled": true, "feature_state_value": "dark",
              "feature": { "id": 4, "name": "todo_theme" } }
        ]))
        .unwrap();

        let merged = FlagSet::new(
            client(FlagsmithConfig::default()).merge(remote),
            FlagSource::Remote,
        );

        // overridden
        assert!(!merged.enabled("todo_write"));
        assert_eq!(merged.int("todo_max_items"), Some(25));
        // numeric string parses as an int, and the raw value is preserved
        assert_eq!(merged.int("todo_app_htmx"), Some(7));
        assert_eq!(
            merged.get("todo_app_htmx").map(|f| f.value.clone()),
            Some(json!("7"))
        );
        // string payload
        assert_eq!(merged.string("todo_theme"), Some("dark"));
        // gap keeps its default
        assert!(merged.enabled("todo_realtime"));
        assert_eq!(
            merged.get("todo_realtime").map(|f| f.value.clone()),
            Some(Value::Null)
        );
        // unknown remote flag is passed through, defaults are still all present
        assert_eq!(merged.len(), 7);
        assert_eq!(merged.source(), FlagSource::Remote);
    }

    #[test]
    fn a_missing_feature_state_value_decodes_as_null() {
        let remote: Vec<WireFlag> = serde_json::from_value(json!([
            { "enabled": true, "feature": { "id": 9, "name": "todo_write" } }
        ]))
        .unwrap();
        let merged = FlagSet::new(
            client(FlagsmithConfig::default()).merge(remote),
            FlagSource::Remote,
        );
        assert_eq!(
            merged.get("todo_write").map(|f| f.value.clone()),
            Some(Value::Null)
        );
    }

    #[test]
    fn to_json_carries_every_catalogue_flag() {
        let set = FlagSet::new(catalogue().map().clone(), FlagSource::Defaults);
        assert_eq!(
            set.to_json(),
            json!({
                "todo_app_astro": { "enabled": true, "value": null },
                "todo_app_htmx": { "enabled": true, "value": null },
                "todo_app_web": { "enabled": true, "value": null },
                "todo_max_items": { "enabled": true, "value": -1 },
                "todo_realtime": { "enabled": true, "value": null },
                "todo_write": { "enabled": true, "value": null },
            })
        );
    }

    // ----------------------------------------------------------- degradation

    #[tokio::test]
    async fn an_unconfigured_client_serves_defaults_without_network_io() {
        let client = client(FlagsmithConfig {
            // A closed port: reaching for it at all would surface as a hang/refusal.
            api_url: "http://127.0.0.1:1/api/v1".to_string(),
            environment_key: None,
            ..FlagsmithConfig::default()
        });
        assert!(!client.is_configured());

        for identity in [None, Some(&Identity::new("yuri").with_trait("app", "web"))] {
            let flags = client.flags(identity).await;
            assert_eq!(flags.source(), FlagSource::Defaults);
            assert_eq!(flags.len(), 6);
            assert!(flags.enabled("todo_write"));
            assert!(flags.enabled("todo_realtime"));
            assert!(flags.enabled("todo_app_astro"));
            assert_eq!(flags.int("todo_max_items"), Some(-1));
        }
    }

    #[tokio::test]
    async fn an_unreachable_flagsmith_serves_defaults() {
        let client = client(FlagsmithConfig {
            api_url: "http://127.0.0.1:1/api/v1".to_string(),
            environment_key: Some("ser.test".to_string()),
            cache_ttl: Duration::from_millis(50),
            request_timeout: Duration::from_millis(300),
        });
        assert!(client.is_configured());

        let anonymous = client.flags(None).await;
        assert_eq!(anonymous.source(), FlagSource::Defaults);
        assert_eq!(anonymous.len(), 6);

        let identified = client
            .flags(Some(&Identity::new("yuri").with_trait("app", "htmx")))
            .await;
        assert_eq!(identified.source(), FlagSource::Defaults);
        assert!(identified.enabled("todo_write"));
    }

    #[tokio::test]
    async fn an_http_error_serves_defaults() {
        let server = TestServer::spawn_failing().await;
        let client = client(FlagsmithConfig {
            api_url: server.api_url.clone(),
            environment_key: Some("ser.test".to_string()),
            cache_ttl: Duration::from_millis(50),
            request_timeout: Duration::from_millis(500),
        });

        let flags = client.flags(None).await;
        assert_eq!(flags.source(), FlagSource::Defaults);
        assert_eq!(server.hits(), 1);
    }

    // ------------------------------------------------------------ remote path

    #[tokio::test]
    async fn an_anonymous_lookup_uses_get_flags() {
        let server = TestServer::spawn().await;
        let client = client(FlagsmithConfig {
            api_url: server.api_url.clone(),
            environment_key: Some("ser.test".to_string()),
            cache_ttl: Duration::from_millis(50),
            request_timeout: Duration::from_secs(5),
        });

        let flags = client.flags(None).await;
        assert_eq!(flags.source(), FlagSource::Remote);
        assert!(!flags.enabled("todo_write"));
        assert_eq!(flags.int("todo_max_items"), Some(3));
        assert_eq!(server.calls(), vec!["GET /flags/".to_string()]);
        assert_eq!(server.keys(), vec!["ser.test".to_string()]);
    }

    #[tokio::test]
    async fn an_identity_without_traits_uses_get_identities() {
        let server = TestServer::spawn().await;
        let client = client(FlagsmithConfig {
            api_url: server.api_url.clone(),
            environment_key: Some("ser.test".to_string()),
            cache_ttl: Duration::from_millis(50),
            request_timeout: Duration::from_secs(5),
        });

        let flags = client.flags(Some(&Identity::new("yuri kr@x.io"))).await;
        assert_eq!(flags.source(), FlagSource::Remote);
        assert_eq!(flags.int("todo_max_items"), Some(3));
        assert_eq!(
            server.calls(),
            vec!["GET /identities/?identifier=yuri+kr%40x.io".to_string()]
        );
    }

    #[tokio::test]
    async fn an_identity_with_traits_posts_them() {
        let server = TestServer::spawn().await;
        let client = client(FlagsmithConfig {
            api_url: server.api_url.clone(),
            environment_key: Some("ser.test".to_string()),
            cache_ttl: Duration::from_millis(50),
            request_timeout: Duration::from_secs(5),
        });

        let flags = client
            .flags(Some(&Identity::new("yuri").with_trait("app", "htmx")))
            .await;
        assert_eq!(flags.source(), FlagSource::Remote);
        assert_eq!(
            server.calls(),
            vec![
                json!({
                    "identifier": "yuri",
                    "traits": [{ "trait_key": "app", "trait_value": "htmx" }],
                    "transient": false
                })
                .to_string()
            ]
        );
    }

    // ------------------------------------------------------------- ttl cache

    #[tokio::test]
    async fn repeated_lookups_hit_the_cache_until_the_ttl_expires() {
        let server = TestServer::spawn().await;
        let client = client(FlagsmithConfig {
            api_url: server.api_url.clone(),
            environment_key: Some("ser.test".to_string()),
            cache_ttl: Duration::from_millis(50),
            request_timeout: Duration::from_secs(5),
        });
        let identity = Identity::new("yuri").with_trait("app", "web");

        assert_eq!(
            client.flags(Some(&identity)).await.source(),
            FlagSource::Remote
        );
        assert_eq!(
            client.flags(Some(&identity)).await.source(),
            FlagSource::Remote
        );
        assert_eq!(server.hits(), 1, "second lookup must be served from cache");

        // Different identity => different cache slot.
        client.flags(Some(&Identity::new("other"))).await;
        assert_eq!(server.hits(), 2);
        // Same user, different app trait => different cache slot.
        client
            .flags(Some(&Identity::new("yuri").with_trait("app", "htmx")))
            .await;
        assert_eq!(server.hits(), 3);

        tokio::time::sleep(Duration::from_millis(80)).await;
        client.flags(Some(&identity)).await;
        assert_eq!(server.hits(), 4, "an expired entry must be refetched");
    }

    #[tokio::test]
    async fn a_failed_lookup_is_cached_for_the_ttl() {
        let server = TestServer::spawn_failing().await;
        let client = client(FlagsmithConfig {
            api_url: server.api_url.clone(),
            environment_key: Some("ser.test".to_string()),
            cache_ttl: Duration::from_millis(200),
            request_timeout: Duration::from_secs(5),
        });

        for _ in 0..3 {
            assert_eq!(client.flags(None).await.source(), FlagSource::Defaults);
        }
        assert_eq!(server.hits(), 1, "a down Flagsmith must not be hammered");
    }

    // ----------------------------------------------------------- test server

    #[derive(Default)]
    struct Recorder {
        hits: AtomicUsize,
        calls: Mutex<Vec<String>>,
        keys: Mutex<Vec<String>>,
    }

    impl Recorder {
        fn record(&self, call: String, key: Option<String>) {
            self.hits.fetch_add(1, Ordering::SeqCst);
            self.calls.lock().push(call);
            if let Some(key) = key {
                self.keys.lock().push(key);
            }
        }
    }

    struct TestServer {
        api_url: String,
        recorder: Arc<Recorder>,
    }

    impl TestServer {
        fn hits(&self) -> usize {
            self.recorder.hits.load(Ordering::SeqCst)
        }

        fn calls(&self) -> Vec<String> {
            self.recorder.calls.lock().clone()
        }

        fn keys(&self) -> Vec<String> {
            self.recorder.keys.lock().clone()
        }

        /// Serves a Flagsmith-shaped response on all three SDK endpoints while
        /// recording every request.
        async fn spawn() -> Self {
            let recorder = Arc::new(Recorder::default());
            let states = json!([
                { "enabled": false, "feature_state_value": null,
                  "feature": { "id": 1, "name": "todo_write" } },
                { "enabled": true, "feature_state_value": "3",
                  "feature": { "id": 2, "name": "todo_max_items" } }
            ]);
            let flags_body = Arc::new(states.clone());
            let identity_body = Arc::new(json!({ "flags": states, "traits": [] }));

            let flags = {
                let recorder = Arc::clone(&recorder);
                let body = Arc::clone(&flags_body);
                get(move |headers: HeaderMap| {
                    let recorder = Arc::clone(&recorder);
                    let body = Arc::clone(&body);
                    async move {
                        recorder.record("GET /flags/".to_string(), environment_key(&headers));
                        Json((*body).clone())
                    }
                })
            };

            let identities_get = {
                let recorder = Arc::clone(&recorder);
                let body = Arc::clone(&identity_body);
                get(move |RawQuery(query): RawQuery, headers: HeaderMap| {
                    let recorder = Arc::clone(&recorder);
                    let body = Arc::clone(&body);
                    async move {
                        let query = query.unwrap_or_default();
                        recorder.record(
                            format!("GET /identities/?{query}"),
                            environment_key(&headers),
                        );
                        Json((*body).clone())
                    }
                })
            };

            let identities = {
                let recorder = Arc::clone(&recorder);
                let body = Arc::clone(&identity_body);
                identities_get.post(move |headers: HeaderMap, Json(payload): Json<Value>| {
                    let recorder = Arc::clone(&recorder);
                    let body = Arc::clone(&body);
                    async move {
                        recorder.record(payload.to_string(), environment_key(&headers));
                        Json((*body).clone())
                    }
                })
            };

            let router = Router::new()
                .route("/flags/", flags)
                .route("/identities/", identities);
            Self::serve(router, recorder).await
        }

        /// Answers every request with `500`.
        async fn spawn_failing() -> Self {
            let recorder = Arc::new(Recorder::default());
            let router = Router::new().fallback({
                let recorder = Arc::clone(&recorder);
                move |headers: HeaderMap| {
                    let recorder = Arc::clone(&recorder);
                    async move {
                        recorder.record("ERROR".to_string(), environment_key(&headers));
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                }
            });
            Self::serve(router, recorder).await
        }

        async fn serve(router: Router, recorder: Arc<Recorder>) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let _ = axum::serve(listener, router).await;
            });
            Self {
                api_url: format!("http://{addr}"),
                recorder,
            }
        }
    }

    fn environment_key(headers: &HeaderMap) -> Option<String> {
        headers
            .get(ENVIRONMENT_KEY_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    }
}
