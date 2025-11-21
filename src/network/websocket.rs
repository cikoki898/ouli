//! WebSocket handler for bidirectional communication

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{accept_async, connect_async, tungstenite::Message, WebSocketStream};
use tracing::{debug, error, warn};

use crate::{OuliError, Result};

/// WebSocket handler
pub struct WebSocketHandler;

impl WebSocketHandler {
    /// Handle a WebSocket upgrade (synchronous entry point)
    ///
    /// # Errors
    ///
    /// Returns error if WebSocket handshake or proxying fails
    ///
    /// # Note
    ///
    /// This is a synchronous wrapper. For full WebSocket proxying with
    /// recording/replay, use `WebSocketProxy` from the proxy module.
    pub fn handle_websocket(_stream: TcpStream, _target_url: String) -> Result<()> {
        // This is a synchronous entry point for compatibility
        // Full WebSocket proxying should use WebSocketProxy::handle_connection
        // which is async and integrates with recording/replay engines
        Ok(())
    }

    /// Accept a WebSocket connection
    ///
    /// # Errors
    ///
    /// Returns error if WebSocket handshake fails
    pub async fn accept_connection(stream: TcpStream) -> Result<WebSocketStream<TcpStream>> {
        accept_async(stream)
            .await
            .map_err(|e| OuliError::Other(format!("WebSocket accept failed: {e}")))
    }

    /// Connect to a WebSocket endpoint
    ///
    /// # Errors
    ///
    /// Returns error if connection fails
    pub async fn connect_to_endpoint(
        url: &str,
    ) -> Result<WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>> {
        connect_async(url)
            .await
            .map(|(ws_stream, _)| ws_stream)
            .map_err(|e| OuliError::Other(format!("WebSocket connect failed: {e}")))
    }

    /// Proxy messages between client and server WebSocket streams
    ///
    /// # Errors
    ///
    /// Returns error if message forwarding fails
    ///
    /// # Note
    ///
    /// This is a stub for Milestone 2. Full implementation will come with
    /// recording/replay engines in Milestones 3-4.
    pub async fn proxy_bidirectional(
        mut client: WebSocketStream<TcpStream>,
        mut server: WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
    ) -> Result<()> {
        loop {
            tokio::select! {
                // Client -> Server
                Some(msg_result) = client.next() => {
                    match msg_result {
                        Ok(msg) => {
                            debug!("Client -> Server: {:?}", msg);
                            if msg.is_close() {
                                debug!("Client closed connection");
                                break;
                            }
                            if let Err(e) = server.send(msg).await {
                                error!("Failed to forward to server: {:?}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Client error: {}", e);
                            break;
                        }
                    }
                }
                // Server -> Client
                Some(msg_result) = server.next() => {
                    match msg_result {
                        Ok(msg) => {
                            debug!("Server -> Client: {:?}", msg);
                            if msg.is_close() {
                                debug!("Server closed connection");
                                break;
                            }
                            if let Err(e) = client.send(msg).await {
                                error!("Failed to forward to client: {:?}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Server error: {}", e);
                            break;
                        }
                    }
                }
                else => {
                    warn!("Both streams ended");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Check if a message should be recorded
    #[must_use]
    pub fn should_record(msg: &Message) -> bool {
        matches!(msg, Message::Text(_) | Message::Binary(_))
    }

    /// Convert message to bytes for storage
    #[must_use]
    pub fn message_to_bytes(msg: &Message) -> Vec<u8> {
        match msg {
            Message::Text(text) => text.as_bytes().to_vec(),
            Message::Binary(data) | Message::Ping(data) | Message::Pong(data) => data.clone(),
            Message::Close(_) | Message::Frame(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_record() {
        assert!(WebSocketHandler::should_record(&Message::Text(
            "test".to_string()
        )));
        assert!(WebSocketHandler::should_record(&Message::Binary(vec![
            1, 2, 3
        ])));
        assert!(!WebSocketHandler::should_record(&Message::Ping(vec![])));
        assert!(!WebSocketHandler::should_record(&Message::Pong(vec![])));
        assert!(!WebSocketHandler::should_record(&Message::Close(None)));
    }

    #[test]
    fn test_message_to_bytes() {
        let text_msg = Message::Text("hello".to_string());
        let bytes = WebSocketHandler::message_to_bytes(&text_msg);
        assert_eq!(bytes, b"hello");

        let binary_msg = Message::Binary(vec![1, 2, 3]);
        let bytes = WebSocketHandler::message_to_bytes(&binary_msg);
        assert_eq!(bytes, vec![1, 2, 3]);
    }
}
