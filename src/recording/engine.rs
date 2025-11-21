//! Recording engine for capturing traffic

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{debug, info};

use crate::fingerprint::{fingerprint_request, Request};
use crate::{OuliError, Result};

use super::session::SessionManager;
use super::DEFAULT_SESSION;

/// HTTP response for recording
pub struct Response {
    /// HTTP status code
    pub status: u16,
    /// Response headers
    pub headers: Vec<(String, String)>,
    /// Response body
    pub body: Vec<u8>,
}

/// Recording engine for capturing HTTP/WebSocket traffic
pub struct RecordingEngine {
    session_manager: Arc<SessionManager>,
}

impl RecordingEngine {
    /// Create a new recording engine
    #[must_use]
    pub fn new(recording_dir: PathBuf) -> Self {
        Self {
            session_manager: Arc::new(SessionManager::new(recording_dir)),
        }
    }

    /// Record a request/response interaction
    ///
    /// # Errors
    ///
    /// Returns error if recording fails
    pub async fn record_interaction(
        &self,
        test_name: Option<&str>,
        request: Request,
        response: Response,
    ) -> Result<()> {
        let test_name = test_name.unwrap_or(DEFAULT_SESSION);

        // Get or create session
        let session = self.session_manager.get_or_create_session(test_name)?;

        // Get chain and compute fingerprint
        let mut chain = session.chain().await;
        let prev_hash = chain.previous_hash();
        let request_hash = fingerprint_request(&request, prev_hash);

        // Update chain
        chain.process_request(&request);
        drop(chain);

        // Serialize request and response
        let request_data = serialize_request(&request);
        let response_data = serialize_response(&response);

        // Write to storage
        let mut writer_guard = session.writer().await;
        if let Some(writer) = writer_guard.as_mut() {
            writer.append_interaction(request_hash, prev_hash, &request_data, &response_data)?;
        } else {
            return Err(OuliError::Other("Session already finalized".to_string()));
        }
        drop(writer_guard);

        // Update metrics
        session.increment_interactions();

        debug!(
            "Recorded interaction: {} (session: {}, count: {})",
            hex::encode(&request_hash[..8]),
            test_name,
            session.interaction_count()
        );

        Ok(())
    }

    /// Get the number of active sessions
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.session_manager.session_count()
    }

    /// Finalize all sessions
    ///
    /// # Errors
    ///
    /// Returns error if finalization fails
    pub async fn finalize_all(&self) -> Result<()> {
        info!("Finalizing all recording sessions");
        self.session_manager.finalize_all().await?;
        info!("All sessions finalized");
        Ok(())
    }
}

/// Serialize a request for storage
fn serialize_request(request: &Request) -> Vec<u8> {
    // Simple serialization for Milestone 3
    // TODO: Use proper binary format in future milestones
    let mut data = Vec::new();

    // Method
    data.extend_from_slice((request.method.len() as u16).to_le_bytes().as_ref());
    data.extend_from_slice(request.method.as_bytes());

    // Path
    data.extend_from_slice((request.path.len() as u16).to_le_bytes().as_ref());
    data.extend_from_slice(request.path.as_bytes());

    // Query count
    data.extend_from_slice((request.query.len() as u16).to_le_bytes().as_ref());
    for (key, value) in &request.query {
        data.extend_from_slice((key.len() as u16).to_le_bytes().as_ref());
        data.extend_from_slice(key.as_bytes());
        data.extend_from_slice((value.len() as u16).to_le_bytes().as_ref());
        data.extend_from_slice(value.as_bytes());
    }

    // Headers count
    data.extend_from_slice((request.headers.len() as u16).to_le_bytes().as_ref());
    for (name, value) in &request.headers {
        data.extend_from_slice((name.len() as u16).to_le_bytes().as_ref());
        data.extend_from_slice(name.as_bytes());
        data.extend_from_slice((value.len() as u16).to_le_bytes().as_ref());
        data.extend_from_slice(value.as_bytes());
    }

    // Body
    data.extend_from_slice((request.body.len() as u32).to_le_bytes().as_ref());
    data.extend_from_slice(&request.body);

    data
}

/// Serialize a response for storage
fn serialize_response(response: &Response) -> Vec<u8> {
    // Simple serialization for Milestone 3
    // TODO: Use proper binary format in future milestones
    let mut data = Vec::new();

    // Status
    data.extend_from_slice(response.status.to_le_bytes().as_ref());

    // Headers count
    data.extend_from_slice((response.headers.len() as u16).to_le_bytes().as_ref());
    for (name, value) in &response.headers {
        data.extend_from_slice((name.len() as u16).to_le_bytes().as_ref());
        data.extend_from_slice(name.as_bytes());
        data.extend_from_slice((value.len() as u16).to_le_bytes().as_ref());
        data.extend_from_slice(value.as_bytes());
    }

    // Body
    data.extend_from_slice((response.body.len() as u32).to_le_bytes().as_ref());
    data.extend_from_slice(&response.body);

    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_recording_engine_creation() {
        let temp_dir = TempDir::new().unwrap();
        let engine = RecordingEngine::new(temp_dir.path().to_path_buf());

        assert_eq!(engine.session_count(), 0);
    }

    #[tokio::test]
    async fn test_record_single_interaction() {
        let temp_dir = TempDir::new().unwrap();
        let engine = RecordingEngine::new(temp_dir.path().to_path_buf());

        let request = Request {
            method: "GET".to_string(),
            path: "/api/test".to_string(),
            query: vec![],
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: vec![],
        };
        let response = Response {
            status: 200,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: b"{\"result\":\"ok\"}".to_vec(),
        };
        let result = engine
            .record_interaction(Some("test1"), request, response)
            .await;

        assert!(result.is_ok());
        assert_eq!(engine.session_count(), 1);
    }

    #[tokio::test]
    async fn test_record_multiple_interactions() {
        let temp_dir = TempDir::new().unwrap();
        let engine = RecordingEngine::new(temp_dir.path().to_path_buf());

        for i in 0..5 {
            let request = Request {
                method: "GET".to_string(),
                path: format!("/api/test/{i}"),
                query: vec![],
                headers: vec![],
                body: vec![],
            };
            let response = Response {
                status: 200,
                headers: vec![],
                body: vec![],
            };
            engine
                .record_interaction(Some("test1"), request, response)
                .await
                .unwrap();
        }

        assert_eq!(engine.session_count(), 1);
    }

    #[tokio::test]
    async fn test_finalize_sessions() {
        let temp_dir = TempDir::new().unwrap();
        let engine = RecordingEngine::new(temp_dir.path().to_path_buf());

        let request = Request {
            method: "GET".to_string(),
            path: "/api/test".to_string(),
            query: vec![],
            headers: vec![],
            body: vec![],
        };
        let response = Response {
            status: 200,
            headers: vec![],
            body: vec![],
        };
        engine
            .record_interaction(Some("test1"), request, response)
            .await
            .unwrap();

        engine.finalize_all().await.unwrap();

        assert_eq!(engine.session_count(), 0);
    }

    #[test]
    fn test_serialize_request() {
        let request = Request {
            method: "POST".to_string(),
            path: "/api/test".to_string(),
            query: vec![("key".to_string(), "value".to_string())],
            headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
            body: b"test body".to_vec(),
        };

        let data = serialize_request(&request);
        assert!(!data.is_empty());
    }

    #[test]
    fn test_serialize_response() {
        let response = Response {
            status: 200,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: b"{\"status\":\"ok\"}".to_vec(),
        };

        let data = serialize_response(&response);
        assert!(!data.is_empty());
    }
}
