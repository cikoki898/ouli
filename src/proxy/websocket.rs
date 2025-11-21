//! WebSocket proxy with recording and replay

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};
use tracing::{debug, info, warn};

use crate::config::{Config, Mode};
use crate::fingerprint::{self, RequestChain};
use crate::network::WebSocketHandler;
use crate::recording::{RecordingEngine, Response as RecordResponse};
use crate::replay::ReplayEngine;
use crate::{OuliError, Result};

/// WebSocket proxy that handles recording and replay
pub struct WebSocketProxy {
    config: Arc<Config>,
    recording_engine: Option<Arc<RecordingEngine>>,
    replay_engine: Option<Arc<ReplayEngine>>,
    request_chain: Arc<RwLock<RequestChain>>,
}

impl WebSocketProxy {
    /// Create a new WebSocket proxy
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        let recording_engine = if config.mode.is_record() {
            Some(Arc::new(RecordingEngine::new(config.recording_dir.clone())))
        } else {
            None
        };

        let replay_engine = if config.mode.is_replay() {
            Some(Arc::new(ReplayEngine::new(
                config.recording_dir.clone(),
                crate::replay::WarmingStrategy::Lazy,
            )))
        } else {
            None
        };

        Self {
            config,
            recording_engine,
            replay_engine,
            request_chain: Arc::new(RwLock::new(RequestChain::new())),
        }
    }

    /// Handle a WebSocket connection
    ///
    /// # Errors
    ///
    /// Returns error if proxying fails
    pub async fn handle_connection(
        &self,
        client_stream: TcpStream,
        target_url: String,
    ) -> Result<()> {
        // Accept client WebSocket connection
        let client_ws = WebSocketHandler::accept_connection(client_stream).await?;

        match self.config.mode {
            Mode::Record => self.handle_record(client_ws, target_url).await,
            Mode::Replay => self.handle_replay(client_ws).await,
        }
    }

    /// Handle WebSocket in record mode
    async fn handle_record(
        &self,
        mut client: WebSocketStream<TcpStream>,
        target_url: String,
    ) -> Result<()> {
        debug!("WebSocket record mode: connecting to {}", target_url);

        // Connect to target WebSocket server
        let mut server = WebSocketHandler::connect_to_endpoint(&target_url).await?;

        // Proxy messages bidirectionally with recording
        loop {
            tokio::select! {
                // Client -> Server
                Some(msg_result) = client.next() => {
                    match msg_result {
                        Ok(msg) => {
                            debug!("Client -> Server: {:?}", msg);

                            // Record if it's a data message
                            if WebSocketHandler::should_record(&msg) {
                                self.record_message("client_to_server", &msg).await?;
                            }

                            // Handle close
                            if msg.is_close() {
                                debug!("Client closed connection");
                                let _ = server.send(msg).await;
                                break;
                            }

                            // Forward to server
                            if let Err(e) = server.send(msg).await {
                                warn!("Failed to forward to server: {e}");
                                break;
                            }
                        }
                        Err(e) => {
                            warn!("Client error: {e}");
                            break;
                        }
                    }
                }
                // Server -> Client
                Some(msg_result) = server.next() => {
                    match msg_result {
                        Ok(msg) => {
                            debug!("Server -> Client: {:?}", msg);

                            // Record if it's a data message
                            if WebSocketHandler::should_record(&msg) {
                                self.record_message("server_to_client", &msg).await?;
                            }

                            // Handle close
                            if msg.is_close() {
                                debug!("Server closed connection");
                                let _ = client.send(msg).await;
                                break;
                            }

                            // Forward to client
                            if let Err(e) = client.send(msg).await {
                                warn!("Failed to forward to client: {e}");
                                break;
                            }
                        }
                        Err(e) => {
                            warn!("Server error: {e}");
                            break;
                        }
                    }
                }
                else => {
                    debug!("Both streams ended");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Handle WebSocket in replay mode
    async fn handle_replay(&self, mut client: WebSocketStream<TcpStream>) -> Result<()> {
        debug!("WebSocket replay mode");

        // In replay mode, serve messages from recording
        loop {
            tokio::select! {
                Some(msg_result) = client.next() => {
                    match msg_result {
                        Ok(msg) => {
                            debug!("Client message: {:?}", msg);

                            // Handle close
                            if msg.is_close() {
                                debug!("Client closed connection");
                                break;
                            }

                            // For recordable messages, try to replay
                            if WebSocketHandler::should_record(&msg) {
                                match self.replay_message(&msg).await {
                                    Ok(response_msg) => {
                                        if let Err(e) = client.send(response_msg).await {
                                            warn!("Failed to send replay response: {e}");
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Replay failed: {e}");
                                        // Send error message
                                        let error_msg = Message::Text(
                                            format!("Replay error: {e}")
                                        );
                                        let _ = client.send(error_msg).await;
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Client error: {e}");
                            break;
                        }
                    }
                }
                else => {
                    debug!("Client stream ended");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Record a WebSocket message
    async fn record_message(&self, direction: &str, msg: &Message) -> Result<()> {
        if let Some(ref engine) = self.recording_engine {
            let data = WebSocketHandler::message_to_bytes(msg);

            // Build request (WebSocket frame as request)
            let request = fingerprint::Request {
                method: "WS".to_string(),
                path: format!("/{direction}"),
                query: vec![],
                headers: vec![],
                body: data.clone(),
            };

            // Build response (echo for now - could be enhanced)
            let response = RecordResponse {
                status: 200,
                headers: vec![],
                body: data,
            };

            engine.record_interaction(None, request, response).await?;
        }

        Ok(())
    }

    /// Replay a WebSocket message
    async fn replay_message(&self, msg: &Message) -> Result<Message> {
        if let Some(ref engine) = self.replay_engine {
            let data = WebSocketHandler::message_to_bytes(msg);

            // Get previous hash
            let prev_hash = {
                let chain = self.request_chain.read().await;
                chain.previous_hash()
            };

            // Build request for fingerprinting
            let request = fingerprint::Request {
                method: "WS".to_string(),
                path: "/client_to_server".to_string(),
                query: vec![],
                headers: vec![],
                body: data.clone(),
            };

            // Update chain
            {
                let mut chain = self.request_chain.write().await;
                chain.process_request(&request);
            }

            // Try to replay
            let cached = engine
                .replay_request(
                    "WS".to_string(),
                    "/client_to_server".to_string(),
                    vec![],
                    vec![],
                    data,
                    prev_hash,
                )
                .map_err(|e| OuliError::Other(format!("WebSocket replay failed: {e}")))?;

            // Convert back to message
            match msg {
                Message::Text(_) => Ok(Message::Text(
                    String::from_utf8_lossy(&cached.body).to_string(),
                )),
                Message::Binary(_) | _ => Ok(Message::Binary(cached.body)),
            }
        } else {
            Err(OuliError::Other(
                "Replay engine not initialized".to_string(),
            ))
        }
    }

    /// Finalize recording (if in record mode)
    ///
    /// # Errors
    ///
    /// Returns error if finalization fails
    pub async fn finalize(&self) -> Result<()> {
        if let Some(ref engine) = self.recording_engine {
            info!("Finalizing WebSocket recording sessions");
            engine.finalize_all().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EndpointConfig, LimitsConfig, RedactionConfig};
    use tempfile::TempDir;

    fn create_test_config(mode: Mode, temp_dir: &TempDir) -> Config {
        Config {
            mode,
            recording_dir: temp_dir.path().to_path_buf(),
            endpoints: vec![EndpointConfig {
                target_host: "example.com".to_string(),
                target_port: 443,
                source_port: 8080,
                target_type: "wss".to_string(),
                source_type: "ws".to_string(),
                redact_request_headers: vec![],
            }],
            redaction: RedactionConfig::default(),
            limits: LimitsConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_websocket_proxy_creation_record_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config = Arc::new(create_test_config(Mode::Record, &temp_dir));
        let proxy = WebSocketProxy::new(config);

        assert!(proxy.recording_engine.is_some());
        assert!(proxy.replay_engine.is_none());
    }

    #[tokio::test]
    async fn test_websocket_proxy_creation_replay_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config = Arc::new(create_test_config(Mode::Replay, &temp_dir));
        let proxy = WebSocketProxy::new(config);

        assert!(proxy.recording_engine.is_none());
        assert!(proxy.replay_engine.is_some());
    }

    #[tokio::test]
    async fn test_finalize_record_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config = Arc::new(create_test_config(Mode::Record, &temp_dir));
        let proxy = WebSocketProxy::new(config);

        let result = proxy.finalize().await;
        assert!(result.is_ok());
    }
}
