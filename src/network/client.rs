//! HTTP client for forwarding requests to target endpoints

use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tracing::{debug, warn};

use crate::{OuliError, Result};

/// HTTP client for forwarding requests
pub struct HttpClient {
    client: Client<HttpConnector, Full<Bytes>>,
}

impl HttpClient {
    /// Create a new HTTP client
    #[must_use]
    pub fn new() -> Self {
        let client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(10)
            .build_http();

        Self { client }
    }

    /// Forward a request to the target endpoint
    ///
    /// # Errors
    ///
    /// Returns error if the request fails
    pub async fn forward_request(&self, request: &ForwardRequest<'_>) -> Result<ForwardedResponse> {
        // Build URI
        let uri = build_uri(
            "http",
            request.target_host,
            request.target_port,
            request.path,
            request.query,
        )?;

        debug!("Forwarding {} to {}", request.method, uri);

        // Parse method
        let method = request.method.parse::<Method>().map_err(|e| {
            OuliError::Other(format!("Invalid HTTP method '{}': {e}", request.method))
        })?;

        // Build request
        let mut request_builder = Request::builder().method(method).uri(uri);

        // Add headers
        for (name, value) in request.headers {
            request_builder = request_builder.header(name, value);
        }

        // Add body
        let http_request = request_builder
            .body(Full::new(Bytes::copy_from_slice(request.body)))
            .map_err(|e| OuliError::Other(format!("Failed to build request: {e}")))?;

        // Send request
        let response = self.client.request(http_request).await.map_err(|e| {
            warn!("Request failed: {e}");
            OuliError::Other(format!("Request failed: {e}"))
        })?;

        // Extract response details
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or("<invalid>").to_string(),
                )
            })
            .collect();

        // Read body
        let body_bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|e| OuliError::Other(format!("Failed to read response body: {e}")))?
            .to_bytes();

        Ok(ForwardedResponse {
            status,
            headers,
            body: body_bytes.to_vec(),
        })
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Request to be forwarded
#[derive(Debug)]
pub struct ForwardRequest<'a> {
    /// HTTP method
    pub method: &'a str,
    /// Target host
    pub target_host: &'a str,
    /// Target port
    pub target_port: u16,
    /// Request path
    pub path: &'a str,
    /// Query parameters
    pub query: &'a [(String, String)],
    /// Request headers
    pub headers: &'a [(String, String)],
    /// Request body
    pub body: &'a [u8],
}

/// Response from forwarded request
#[derive(Debug, Clone)]
pub struct ForwardedResponse {
    /// HTTP status code
    pub status: u16,
    /// Response headers
    pub headers: Vec<(String, String)>,
    /// Response body
    pub body: Vec<u8>,
}

/// Build a URI from components
fn build_uri(
    scheme: &str,
    host: &str,
    port: u16,
    path: &str,
    query: &[(String, String)],
) -> Result<Uri> {
    let mut uri = format!("{scheme}://{host}:{port}{path}");

    if !query.is_empty() {
        uri.push('?');
        for (i, (key, value)) in query.iter().enumerate() {
            if i > 0 {
                uri.push('&');
            }
            uri.push_str(&urlencoding::encode(key));
            uri.push('=');
            uri.push_str(&urlencoding::encode(value));
        }
    }

    uri.parse::<Uri>()
        .map_err(|e| OuliError::Other(format!("Invalid URI '{uri}': {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_uri_simple() {
        let uri = build_uri("http", "example.com", 80, "/api/test", &[]).unwrap();
        assert_eq!(uri.to_string(), "http://example.com:80/api/test");
    }

    #[test]
    fn test_build_uri_with_query() {
        let query = vec![
            ("key1".to_string(), "value1".to_string()),
            ("key2".to_string(), "value2".to_string()),
        ];
        let uri = build_uri("http", "example.com", 80, "/api/test", &query).unwrap();
        assert_eq!(
            uri.to_string(),
            "http://example.com:80/api/test?key1=value1&key2=value2"
        );
    }

    #[test]
    fn test_build_uri_with_encoding() {
        let query = vec![("key".to_string(), "value with spaces".to_string())];
        let uri = build_uri("http", "example.com", 80, "/api/test", &query).unwrap();
        assert_eq!(
            uri.to_string(),
            "http://example.com:80/api/test?key=value%20with%20spaces"
        );
    }

    #[test]
    fn test_http_client_creation() {
        let client = HttpClient::new();
        assert!(std::mem::size_of_val(&client) > 0);
    }
}
