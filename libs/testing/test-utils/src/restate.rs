//! Restate test infrastructure
//!
//! Restate is push-based: the server calls *into* the service endpoint, so a
//! test serves its endpoint on the host and the container reaches it through
//! `host.docker.internal` (mapped to the host gateway, which also works on
//! Linux Docker where that name does not exist by default).

use std::time::{Duration, Instant};

use testcontainers::core::{Host, IntoContainerPort};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// Pinned server version; `restate-sdk` 0.12 needs Restate 1.7 or newer.
const IMAGE: &str = "docker.io/restatedev/restate";
const TAG: &str = "1.7.12";
const INGRESS_PORT: u16 = 8080;
const ADMIN_PORT: u16 = 9070;
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// A Restate server container, stopped and removed on drop.
///
/// # Example
///
/// ```no_run
/// use test_utils::TestRestate;
///
/// # async fn example() {
/// let restate = TestRestate::new().await;
/// // Serve an endpoint on 0.0.0.0:<port> first, then:
/// restate.register_host_endpoint(9080).await;
/// let ingress = restate.ingress_url(); // call handlers here
/// # }
/// ```
pub struct TestRestate {
    #[allow(dead_code)]
    container: ContainerAsync<GenericImage>,
    http: reqwest::Client,
    ingress_url: String,
    admin_url: String,
}

impl TestRestate {
    /// Starts the server and waits until both admin and ingress report healthy.
    pub async fn new() -> Self {
        let container = GenericImage::new(IMAGE, TAG)
            .with_exposed_port(INGRESS_PORT.tcp())
            .with_exposed_port(ADMIN_PORT.tcp())
            .with_host("host.docker.internal", Host::HostGateway)
            .start()
            .await
            .expect("Failed to start Restate container");

        let ingress_port = container
            .get_host_port_ipv4(INGRESS_PORT)
            .await
            .expect("Failed to get Restate ingress port");
        let admin_port = container
            .get_host_port_ipv4(ADMIN_PORT)
            .await
            .expect("Failed to get Restate admin port");

        let this = Self {
            container,
            http: reqwest::Client::new(),
            ingress_url: format!("http://127.0.0.1:{ingress_port}"),
            admin_url: format!("http://127.0.0.1:{admin_port}"),
        };
        this.wait_healthy(&format!("{}/health", this.admin_url))
            .await;
        this.wait_healthy(&format!("{}/restate/health", this.ingress_url))
            .await;
        tracing::info!(ingress_port, admin_port, "Test Restate ready");
        this
    }

    /// Base URL of the ingress, e.g. for `restate_sdk::ingress::ReqwestClient`.
    pub fn ingress_url(&self) -> &str {
        &self.ingress_url
    }

    /// Base URL of the admin API.
    pub fn admin_url(&self) -> &str {
        &self.admin_url
    }

    /// Registers the endpoint the test process serves on `port`. The endpoint
    /// must listen on `0.0.0.0`: on Linux the container reaches the host via
    /// the docker bridge, not loopback.
    pub async fn register_host_endpoint(&self, port: u16) {
        let uri = format!("http://host.docker.internal:{port}");
        let response = self
            .http
            .post(format!("{}/deployments", self.admin_url))
            .json(&serde_json::json!({ "uri": uri, "force": true }))
            .send()
            .await
            .expect("Failed to reach Restate admin API");
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        assert!(
            status.is_success(),
            "registering {uri} failed ({status}): {body}"
        );
    }

    async fn wait_healthy(&self, url: &str) {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            match self.http.get(url).send().await {
                Ok(r) if r.status().is_success() => return,
                _ if Instant::now() >= deadline => {
                    panic!("{url} not healthy within {READY_TIMEOUT:?}")
                }
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }
}
