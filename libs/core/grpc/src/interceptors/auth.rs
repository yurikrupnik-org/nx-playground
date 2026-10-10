use tonic::{Request, Status};

/// Interceptor for injecting authentication headers
///
/// Supports multiple authentication schemes including Bearer tokens,
/// API keys, and custom authorization headers.
///
/// # Example
/// ```ignore
/// use grpc_client::interceptors::AuthInterceptor;
/// use rpc::tasks::v1::tasks_service_client::TasksServiceClient;
///
/// let auth = AuthInterceptor::bearer("my-jwt-token");
/// let channel = create_channel("http://[::1]:50051").await?;
/// let client = TasksServiceClient::with_interceptor(channel, auth);
/// ```
#[derive(Clone, Debug)]
pub struct AuthInterceptor {
    header_value: String,
}

impl AuthInterceptor {
    /// Create an interceptor with a Bearer token (OAuth 2.0 / JWT)
    ///
    /// # Example
    /// ```ignore
    /// let auth = AuthInterceptor::bearer("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...");
    /// ```
    pub fn bearer(token: impl Into<String>) -> Self {
        Self {
            header_value: format!("Bearer {}", token.into()),
        }
    }

    /// Create an interceptor with a custom authorization header value
    ///
    /// # Example
    /// ```ignore
    /// let auth = AuthInterceptor::custom("Basic dXNlcjpwYXNz");
    /// ```
    pub fn custom(value: impl Into<String>) -> Self {
        Self {
            header_value: value.into(),
        }
    }

    /// Create an interceptor with an API key
    ///
    /// # Example
    /// ```ignore
    /// let auth = AuthInterceptor::api_key("my-api-key-12345");
    /// ```
    pub fn api_key(key: impl Into<String>) -> Self {
        Self {
            header_value: key.into(),
        }
    }
}

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        request.metadata_mut().insert(
            "authorization",
            self.header_value
                .parse()
                .map_err(|_| Status::internal("Invalid auth header"))?,
        );
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::service::Interceptor;

    #[test]
    fn test_bearer_token() {
        let mut auth = AuthInterceptor::bearer("test-token");
        let request = Request::new(());
        let result = auth.call(request);
        assert!(result.is_ok());
        let req = result.unwrap();
        let auth_header = req.metadata().get("authorization").unwrap();
        assert_eq!(auth_header, "Bearer test-token");
    }

    #[test]
    fn test_api_key() {
        let mut auth = AuthInterceptor::api_key("my-key");
        let request = Request::new(());
        let result = auth.call(request);
        assert!(result.is_ok());
        let req = result.unwrap();
        let auth_header = req.metadata().get("authorization").unwrap();
        assert_eq!(auth_header, "my-key");
    }

    #[test]
    fn test_custom() {
        let mut auth = AuthInterceptor::custom("Basic xyz123");
        let request = Request::new(());
        let result = auth.call(request);
        assert!(result.is_ok());
        let req = result.unwrap();
        let auth_header = req.metadata().get("authorization").unwrap();
        assert_eq!(auth_header, "Basic xyz123");
    }
}
