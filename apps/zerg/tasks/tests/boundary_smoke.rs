//! Boundary smoke: the service must reject callers it cannot verify.
//!
//! Run against a live server: `cargo test -p zerg_tasks --test boundary_smoke -- --ignored`

use rpc::tasks::v1::ListRequest;
use rpc::tasks::v1::tasks_service_client::TasksServiceClient;

async fn client() -> TasksServiceClient<tonic::transport::Channel> {
    TasksServiceClient::connect("http://[::1]:50051")
        .await
        .expect("tasks service must be running")
}

#[tokio::test]
#[ignore = "requires a running zerg_tasks on :50051"]
async fn direct_call_without_token_is_rejected() {
    let mut c = client().await;
    let err = c
        .list(ListRequest {
            limit: 10,
            ..Default::default()
        })
        .await
        .expect_err("an unauthenticated direct call must fail");
    assert_eq!(err.code(), tonic::Code::Unauthenticated, "{err:?}");
}

#[tokio::test]
#[ignore = "requires a running zerg_tasks on :50051"]
async fn direct_call_with_forged_token_is_rejected() {
    let mut c = client().await;
    let mut req = tonic::Request::new(ListRequest {
        limit: 10,
        ..Default::default()
    });
    // Self-signed HS256 junk: no valid RS256 signature from the IdP's JWKS.
    req.metadata_mut().insert(
        "authorization",
        "Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhdHRhY2tlciIsIm9yZ19pZCI6Im9yZ18wMUZBS0UifQ.sig"
            .parse()
            .unwrap(),
    );
    let err = c.list(req).await.expect_err("a forged token must fail");
    assert_eq!(err.code(), tonic::Code::Unauthenticated, "{err:?}");
}
