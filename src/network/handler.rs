//! Main network handler

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::JoinSet;
use tracing::{error, info, warn};

use crate::config::{Config, EndpointConfig};
use crate::{OuliError, Result};

use super::connection_pool::ConnectionPool;
use super::{HttpHandler, SHUTDOWN_TIMEOUT_MS};

/// Main network handler that manages all endpoints
pub struct NetworkHandler {
    config: Arc<Config>,
    connection_pool: ConnectionPool,
    shutdown_tx: broadcast::Sender<()>,
}

impl NetworkHandler {
    /// Create a new network handler
    #[must_use]
    pub fn new(config: Config) -> Self {
        let max_connections = config.limits.max_connections;
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            config: Arc::new(config),
            connection_pool: ConnectionPool::new(max_connections),
            shutdown_tx,
        }
    }

    /// Run the network handler
    ///
    /// # Errors
    ///
    /// Returns error if any endpoint fails to start
    pub async fn run(self) -> Result<()> {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let mut tasks = JoinSet::new();

        // Start all endpoint listeners
        for endpoint in &self.config.endpoints {
            let handler = Self {
                config: Arc::clone(&self.config),
                connection_pool: self.connection_pool.clone(),
                shutdown_tx: self.shutdown_tx.clone(),
            };

            let endpoint = endpoint.clone();

            tasks.spawn(async move { handler.run_endpoint(endpoint).await });
        }

        // Set up signal handlers
        let shutdown_signal = async {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("Received SIGINT, shutting down");
                }
                _ = shutdown_rx.recv() => {
                    info!("Received shutdown signal");
                }
            }
        };

        tokio::select! {
            () = shutdown_signal => {
                info!("Initiating graceful shutdown");
            }
            Some(result) = tasks.join_next() => {
                if let Err(e) = result {
                    error!("Endpoint task failed: {}", e);
                    return Err(OuliError::Other(format!("Endpoint task failed: {e}")));
                }
            }
        }

        // Graceful shutdown
        self.shutdown_tx.send(()).ok();

        // Wait for tasks with timeout
        let shutdown_timeout = Duration::from_millis(SHUTDOWN_TIMEOUT_MS);
        tokio::time::timeout(shutdown_timeout, async {
            while let Some(result) = tasks.join_next().await {
                if let Err(e) = result {
                    warn!("Task cleanup error: {}", e);
                }
            }
        })
        .await
        .ok();

        info!("Shutdown complete");
        Ok(())
    }

    /// Run a single endpoint
    async fn run_endpoint(&self, endpoint: EndpointConfig) -> Result<()> {
        let addr = SocketAddr::from(([0, 0, 0, 0], endpoint.source_port));
        let listener = TcpListener::bind(addr).await?;

        info!(
            "Listening on {} (proxy to {}:{})",
            addr, endpoint.target_host, endpoint.target_port
        );

        let mut shutdown_rx = self.shutdown_tx.subscribe();

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            if !self.connection_pool.can_accept() {
                                warn!("Connection limit reached, rejecting {}", peer_addr);
                                drop(stream);
                                continue;
                            }

                            let config = Arc::clone(&self.config);
                            let pool = self.connection_pool.clone();
                            let endpoint = endpoint.clone();

                            tokio::spawn(async move {
                                let _guard = pool.acquire().await;

                                if let Err(e) = HttpHandler::handle_connection(stream, &endpoint, config) {
                                    error!("Connection error: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Endpoint {} shutting down", addr);
                    break;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LimitsConfig, Mode};
    use std::path::PathBuf;

    fn test_config() -> Config {
        Config {
            mode: Mode::Record,
            recording_dir: PathBuf::from("/tmp"),
            endpoints: vec![EndpointConfig {
                target_host: "example.com".to_string(),
                target_port: 443,
                source_port: 8080,
                target_type: "https".to_string(),
                source_type: "http".to_string(),
                redact_request_headers: vec![],
            }],
            redaction: crate::config::RedactionConfig::default(),
            limits: LimitsConfig {
                max_connections: 10,
                ..Default::default()
            },
        }
    }

    #[test]
    fn test_network_handler_creation() {
        let config = test_config();
        let handler = NetworkHandler::new(config);

        assert_eq!(handler.connection_pool.max_connections(), 10);
    }

    #[tokio::test]
    async fn test_shutdown_signal() {
        let config = test_config();
        let handler = NetworkHandler::new(config);

        let shutdown = handler.shutdown_tx.clone();

        // Spawn handler in background
        let handle = tokio::spawn(async move { handler.run().await });

        // Wait a bit then trigger shutdown
        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown.send(()).ok();

        // Handler should complete
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok());
    }
}
