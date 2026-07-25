/// Re-export tonic's Interceptor trait for convenience
pub use tonic::service::Interceptor;

pub mod auth;
pub mod compose;
pub mod metrics;
pub mod tracing;

pub use auth::AuthInterceptor;
pub use compose::{ComposedInterceptor, compose_interceptors};
pub use metrics::MetricsInterceptor;
pub use tracing::{TracedChannel, TracingInterceptor};
