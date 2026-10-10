use feature_flags::{FlagClient, FlagDefaults, FlagSource, FlagsmithConfig, Identity};
use std::time::Duration;

#[tokio::test]
async fn live_smoke() {
    let client = FlagClient::new(
        FlagsmithConfig {
            api_url: "http://localhost:8000/api/v1".to_string(),
            environment_key: Some("ser.bogus-not-provisioned".to_string()),
            cache_ttl: Duration::from_millis(1),
            request_timeout: Duration::from_secs(3),
        },
        FlagDefaults::new()
            .bool("todo_write", true)
            .int("todo_max_items", -1),
    )
    .expect("client");

    let anon = client.flags(None).await;
    let ident = client
        .flags(Some(&Identity::new("yuri").with_trait("app", "htmx")))
        .await;
    println!("anon   source={} json={}", anon.source(), anon.to_json());
    println!("ident  source={} json={}", ident.source(), ident.to_json());
    assert_eq!(anon.source(), FlagSource::Defaults);
    assert_eq!(ident.source(), FlagSource::Defaults);
    assert!(anon.enabled("todo_write"));
    assert_eq!(anon.int("todo_max_items"), Some(-1));
}
