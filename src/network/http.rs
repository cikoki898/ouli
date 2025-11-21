//! HTTP handler for request/response proxying

use std::sync::Arc;

use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Bytes;
use hyper::{Request, Response, StatusCode};
use tokio::net::TcpStream;
use tracing::debug;

use crate::config::{Config, EndpointConfig};
use crate::{OuliError, Result};

/// HTTP handler for processing connections
pub struct HttpHandler;

impl HttpHandler {
    /// Handle an incoming connection
    ///
    /// # Errors
    ///
    /// Returns error if connection processing fails
    pub fn handle_connection(
        _stream: TcpStream,
        endpoint: &EndpointConfig,
        _config: Arc<Config>,
    ) -> Result<()> {
        debug!(
            "Handling HTTP connection for {}:{}",
            endpoint.target_host, endpoint.target_port
        );

        // TODO: Implement full HTTP/1.1 and HTTP/2 handling
        // This is a stub for Milestone 2
        // Full implementation will come with recording/replay engines

        Ok(())
    }

    /// Create a simple HTTP response
    ///
    /// # Panics
    ///
    /// Panics if response builder fails (should never happen with valid inputs)
    #[must_use]
    pub fn create_response(status: StatusCode, body: &str) -> Response<Full<Bytes>> {
        Response::builder()
            .status(status)
            .body(Full::new(Bytes::from(body.to_string())))
            .expect("Failed to build response")
    }

    /// Create an empty response
    ///
    /// # Panics
    ///
    /// Panics if response builder fails (should never happen)
    #[must_use]
    pub fn empty_response(status: StatusCode) -> Response<Empty<Bytes>> {
        Response::builder()
            .status(status)
            .body(Empty::new())
            .expect("Failed to build response")
    }

    /// Create an error response
    #[must_use]
    pub fn error_response(error: &OuliError) -> Response<Full<Bytes>> {
        let status = match error {
            OuliError::RecordingNotFound(_) | OuliError::FileNotFound(_) => StatusCode::NOT_FOUND,
            OuliError::DataTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        Self::create_response(status, &format!("Error: {error}"))
    }

    /// Parse and validate an incoming request
    ///
    /// # Errors
    ///
    /// Returns error if request is invalid or too large
    pub fn validate_request(
        request: &Request<impl hyper::body::Body>,
        max_size: usize,
    ) -> Result<()> {
        // Check content length
        if let Some(content_length) = request.headers().get(hyper::header::CONTENT_LENGTH) {
            if let Ok(length_str) = content_length.to_str() {
                if let Ok(length) = length_str.parse::<usize>() {
                    if length > max_size {
                        return Err(OuliError::DataTooLarge {
                            size: length,
                            limit: max_size,
                        });
                    }
                }
            }
        }

        // Check header count
        let header_count = request.headers().len();
        if header_count > 128 {
            return Err(OuliError::Other(format!(
                "Too many headers: {header_count}"
            )));
        }

        Ok(())
    }

    /// Read request body with size limit
    ///
    /// # Errors
    ///
    /// Returns error if body is too large or read fails
    pub async fn read_body<B>(body: B, max_size: usize) -> Result<Bytes>
    where
        B: hyper::body::Body,
        B::Error: std::fmt::Display,
    {
        let collected = body
            .collect()
            .await
            .map_err(|e| OuliError::Other(format!("Failed to read body: {e}")))?;

        let bytes = collected.to_bytes();

        if bytes.len() > max_size {
            return Err(OuliError::DataTooLarge {
                size: bytes.len(),
                limit: max_size,
            });
        }

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::Full;

    #[test]
    fn test_create_response() {
        let response = HttpHandler::create_response(StatusCode::OK, "Hello");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn test_empty_response() {
        let response = HttpHandler::empty_response(StatusCode::NO_CONTENT);

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[test]
    fn test_error_response() {
        let error = OuliError::FileNotFound("test.txt".to_string());
        let response = HttpHandler::error_response(&error);

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_validate_request_success() {
        let request = Request::builder()
            .method("GET")
            .uri("/test")
            .body(Empty::<Bytes>::new())
            .unwrap();

        let result = HttpHandler::validate_request(&request, 1024);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_request_too_large() {
        let request = Request::builder()
            .method("POST")
            .uri("/test")
            .header(hyper::header::CONTENT_LENGTH, "10000")
            .body(Empty::<Bytes>::new())
            .unwrap();

        let result = HttpHandler::validate_request(&request, 1024);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_body() {
        let data = Bytes::from("test data");
        let body = Full::new(data.clone());

        let result = HttpHandler::read_body(body, 1024).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), data);
    }

    #[tokio::test]
    async fn test_read_body_too_large() {
        let data = Bytes::from("test data that is too long");
        let body = Full::new(data);

        let result = HttpHandler::read_body(body, 5).await;
        assert!(result.is_err());
    }
}
