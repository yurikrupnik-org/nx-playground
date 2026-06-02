use crate::{AppInfo, Environment};
use color_eyre::eyre::{self, WrapErr};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_semantic_conventions as semconv;
use std::sync::Mutex;
use tracing::{debug, info};
use tracing_subscriber::{EnvFilter, prelude::*};

/// Held by the caller for the process lifetime; flushes the OTEL pipeline on Drop.
///
/// `init_tracing` always returns a guard, even when OTEL is disabled (Drop is a
/// no-op in that case). Callers should bind it: `let _guard = init_tracing(&env);`.
#[must_use = "binding the guard keeps the OTEL exporter alive until process exit"]
pub struct TracingGuard {
    provider: Option<SdkTracerProvider>,
}

impl Drop for TracingGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            // Flush + shutdown synchronously so queued spans aren't lost when the
            // tokio runtime is dropped at the end of `main`.
            let _ = provider.shutdown();
        }
    }
}

/// Install color-eyre with a project-standard configuration.
pub fn install_color_eyre() {
    let _ = color_eyre::config::HookBuilder::default()
        .display_location_section(true)
        .display_env_section(false)
        .install();
}

/// Initialize tracing: stdout (json or pretty) + ErrorLayer + optional OTLP export.
///
/// OTLP export activates when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. The exporter
/// uses gRPC (default port 4317). `service.name` and `service.version` come from
/// the caller's `AppInfo` (i.e. CARGO_PKG_NAME / CARGO_PKG_VERSION at compile time).
/// `OTEL_SERVICE_NAME` env var still wins if set, per OTEL spec.
///
/// Pass `app_info!()` from the calling crate so the macro resolves to that crate's
/// Cargo.toml metadata, not core_config's.
///
/// Returns a guard that must be held until the process exits. Dropping it flushes
/// any pending spans before the tokio runtime tears down.
pub fn init_tracing(environment: &Environment, app: AppInfo) -> TracingGuard {
    install_color_eyre();

    let is_production = environment.is_production();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if is_production {
            EnvFilter::new("info,tower_http=info,sea_orm=warn")
        } else {
            EnvFilter::new("debug,tower_http=debug,sea_orm=info")
        }
    });

    // Build the OTLP layer if configured. Failures degrade to stdout-only logging
    // rather than crashing the process — observability shouldn't take the app down.
    let (otel_layer, provider) = match build_otel_layer(environment, &app) {
        Ok(Some((layer, provider))) => (Some(layer), Some(provider)),
        Ok(None) => (None, None),
        Err(e) => {
            eprintln!("OTEL initialization failed, continuing without OTLP export: {e:#}");
            (None, None)
        }
    };

    // Layer order matters: OTEL needs LookupSpan from Registry, so it goes
    // first. fmt and ErrorLayer come next; filter is applied last so it
    // governs the whole stack via EnvFilter's global behavior.
    let result = if is_production {
        tracing_subscriber::registry()
            .with(otel_layer)
            .with(tracing_error::ErrorLayer::default())
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_target(false)
                    .flatten_event(true),
            )
            .with(filter)
            .try_init()
    } else {
        tracing_subscriber::registry()
            .with(otel_layer)
            .with(tracing_error::ErrorLayer::default())
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(true)
                    .with_file(true)
                    .with_line_number(true)
                    .pretty(),
            )
            .with(filter)
            .try_init()
    };

    match result {
        Ok(_) => info!(
            otel_enabled = provider.is_some(),
            "Tracing initialized. Environment: {:?}", environment
        ),
        Err(_) => debug!("Tracing already initialized, skipping re-initialization"),
    }

    TracingGuard { provider }
}

type OtelLayer = tracing_opentelemetry::OpenTelemetryLayer<
    tracing_subscriber::Registry,
    opentelemetry_sdk::trace::Tracer,
>;

/// Returns `Ok(None)` when OTEL is not configured (no endpoint env var).
/// Returns `Ok(Some((layer, provider)))` on successful pipeline build.
fn build_otel_layer(
    environment: &Environment,
    app: &AppInfo,
) -> eyre::Result<Option<(OtelLayer, SdkTracerProvider)>> {
    let endpoint = match std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(None),
    };

    // Tests may call init_tracing repeatedly. The OTLP exporter spins up a tonic
    // gRPC client that won't bind cleanly twice; guard with a process-wide flag.
    static INITIALIZED: Mutex<bool> = Mutex::new(false);
    {
        let mut guard = INITIALIZED.lock().expect("OTEL init lock poisoned");
        if *guard {
            return Ok(None);
        }
        *guard = true;
    }

    // OTEL spec: OTEL_SERVICE_NAME env wins over programmatic service.name.
    // Fall back to AppInfo (CARGO_PKG_NAME of the caller crate).
    let service_name = std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| app.name.to_string());

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&endpoint)
        .build()
        .wrap_err_with(|| format!("building OTLP span exporter for {endpoint}"))?;

    let resource = Resource::builder()
        .with_attribute(opentelemetry::KeyValue::new(
            semconv::resource::SERVICE_NAME,
            service_name.clone(),
        ))
        // `deployment.environment.name` is in the experimental semconv set; spell
        // it as a literal so we don't have to opt into the unstable feature flag.
        .with_attribute(opentelemetry::KeyValue::new(
            "deployment.environment.name",
            format!("{environment:?}").to_lowercase(),
        ))
        .build();

    let provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build();

    let tracer = provider.tracer(service_name);
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    opentelemetry::global::set_tracer_provider(provider.clone());

    Ok(Some((layer, provider)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> AppInfo {
        AppInfo {
            name: "test_app",
            version: "0.0.0",
        }
    }

    #[test]
    fn test_init_tracing_development() {
        let _guard = init_tracing(&Environment::Development, test_app());
    }

    #[test]
    fn test_init_tracing_production() {
        let _guard = init_tracing(&Environment::Production, test_app());
    }

    #[test]
    fn test_init_tracing_multiple_calls() {
        let _g1 = init_tracing(&Environment::Development, test_app());
        let _g2 = init_tracing(&Environment::Development, test_app());
    }

    #[test]
    fn test_init_tracing_with_rust_log_env() {
        temp_env::with_var("RUST_LOG", Some("trace"), || {
            let _guard = init_tracing(&Environment::Development, test_app());
        });
    }

    #[test]
    fn test_init_tracing_production_with_custom_log_level() {
        temp_env::with_var("RUST_LOG", Some("warn"), || {
            let _guard = init_tracing(&Environment::Production, test_app());
        });
    }
}
