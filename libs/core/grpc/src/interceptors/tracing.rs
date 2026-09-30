use opentelemetry::global;
use opentelemetry::propagation::Injector;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tonic::{Request, Status};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// A [`Channel`] wrapped with the [`TracingInterceptor`].
///
/// Use this as the transport type parameter for generated tonic clients whose
/// outbound calls should carry distributed-tracing context, e.g.
/// `TasksServiceClient<TracedChannel>`.
pub type TracedChannel = InterceptedService<Channel, TracingInterceptor>;

/// Interceptor for distributed tracing.
///
/// On every outbound RPC it:
/// - injects the current span's **W3C Trace Context** (`traceparent` / `tracestate`)
///   so the receiving service continues the same trace, and
/// - stamps a unique `x-request-id` for log correlation.
///
/// The trace context is read from the current `tracing` span via
/// [`OpenTelemetrySpanExt::context`] and serialized with the globally-installed
/// text-map propagator (set by `core_config::tracing::init_tracing`). When no OTEL
/// layer/propagator is active the injected context is empty, so this degrades to
/// injecting only the request id.
///
/// # Example
/// ```ignore
/// use grpc_client::interceptors::TracingInterceptor;
/// use rpc::tasks::v1::tasks_service_client::TasksServiceClient;
///
/// let channel = create_channel("http://[::1]:50051").await?;
/// let client = TasksServiceClient::with_interceptor(channel, TracingInterceptor::new());
/// ```
#[derive(Clone, Debug, Default)]
pub struct TracingInterceptor;

impl TracingInterceptor {
    /// Create a new tracing interceptor.
    pub fn new() -> Self {
        Self
    }
}

impl tonic::service::Interceptor for TracingInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        // Propagate the active trace into the request metadata (W3C traceparent).
        inject_trace_context(&Span::current().context(), request.metadata_mut());

        // Stamp a per-call request id for log correlation across services.
        let request_id = uuid::Uuid::new_v4().to_string();
        request.metadata_mut().insert(
            "x-request-id",
            request_id
                .parse()
                .map_err(|_| Status::internal("Failed to create request ID"))?,
        );

        tracing::debug!(
            target: "grpc_client",
            request_id = %request_id,
            "Outgoing gRPC request"
        );

        Ok(request)
    }
}

/// Serialize `cx` into `metadata` using the globally-installed text-map propagator.
fn inject_trace_context(cx: &opentelemetry::Context, metadata: &mut tonic::metadata::MetadataMap) {
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(cx, &mut MetadataInjector(metadata));
    });
}

/// Adapts a tonic [`MetadataMap`](tonic::metadata::MetadataMap) to the OpenTelemetry
/// [`Injector`] interface so a propagator can write `traceparent`/`tracestate` headers.
struct MetadataInjector<'a>(&'a mut tonic::metadata::MetadataMap);

impl Injector for MetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let Ok(key) = tonic::metadata::MetadataKey::from_bytes(key.as_bytes())
            && let Ok(value) = tonic::metadata::AsciiMetadataValue::try_from(value.as_str())
        {
            self.0.insert(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{
        SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
    };
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use tonic::service::Interceptor;

    #[test]
    fn injects_request_id() {
        let mut tracing = TracingInterceptor::new();
        let request = Request::new(());
        let req = tracing.call(request).expect("interceptor call");
        let request_id = req
            .metadata()
            .get("x-request-id")
            .expect("x-request-id present");
        let id_str = request_id.to_str().unwrap();
        assert!(uuid::Uuid::parse_str(id_str).is_ok(), "valid uuid");
    }

    #[test]
    fn injects_w3c_traceparent_from_context() {
        // The interceptor relies on a globally-installed propagator.
        global::set_text_map_propagator(TraceContextPropagator::new());

        // A known, sampled remote span context to propagate.
        let trace_id = TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap();
        let span_id = SpanId::from_hex("b7ad6b7169203331").unwrap();
        let span_context = SpanContext::new(
            trace_id,
            span_id,
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        );
        let cx = opentelemetry::Context::new().with_remote_span_context(span_context);

        let mut metadata = tonic::metadata::MetadataMap::new();
        inject_trace_context(&cx, &mut metadata);

        let traceparent = metadata
            .get("traceparent")
            .expect("traceparent injected")
            .to_str()
            .unwrap();
        // Format: version-traceid-spanid-flags; must carry our trace id and sampled flag.
        assert!(
            traceparent.contains("0af7651916cd43dd8448eb211c80319c"),
            "traceparent missing trace id: {traceparent}"
        );
        assert!(
            traceparent.starts_with("00-"),
            "unexpected version: {traceparent}"
        );
        assert!(traceparent.ends_with("-01"), "not sampled: {traceparent}");
    }
}
