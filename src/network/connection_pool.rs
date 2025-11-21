//! Connection pool with bounded concurrency

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Semaphore;

use super::MAX_CONNECTIONS;

/// Connection pool that enforces a maximum number of concurrent connections
#[derive(Clone)]
pub struct ConnectionPool {
    semaphore: Arc<Semaphore>,
    active_count: Arc<AtomicUsize>,
    max_connections: usize,
}

impl ConnectionPool {
    /// Create a new connection pool
    ///
    /// # Panics
    ///
    /// Panics if `max_connections` is 0
    #[must_use]
    pub fn new(max_connections: usize) -> Self {
        assert!(max_connections > 0, "max_connections must be > 0");

        Self {
            semaphore: Arc::new(Semaphore::new(max_connections)),
            active_count: Arc::new(AtomicUsize::new(0)),
            max_connections,
        }
    }

    /// Check if a new connection can be accepted
    #[must_use]
    pub fn can_accept(&self) -> bool {
        self.active_count.load(Ordering::Relaxed) < self.max_connections
    }

    /// Try to acquire a connection permit
    ///
    /// Returns `None` if no permits are available
    pub fn try_acquire(&self) -> Option<ConnectionGuard> {
        match Arc::clone(&self.semaphore).try_acquire_owned() {
            Ok(permit) => {
                self.active_count.fetch_add(1, Ordering::Relaxed);
                Some(ConnectionGuard {
                    _permit: permit,
                    active_count: Arc::clone(&self.active_count),
                })
            }
            Err(_) => None,
        }
    }

    /// Acquire a connection permit (waits if necessary)
    ///
    /// # Panics
    ///
    /// Panics if semaphore is closed (should never happen)
    pub async fn acquire(&self) -> ConnectionGuard {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .expect("Semaphore should never close");

        self.active_count.fetch_add(1, Ordering::Relaxed);

        ConnectionGuard {
            _permit: permit,
            active_count: Arc::clone(&self.active_count),
        }
    }

    /// Get the current number of active connections
    #[must_use]
    pub fn active_connections(&self) -> usize {
        self.active_count.load(Ordering::Relaxed)
    }

    /// Get the maximum number of connections
    #[must_use]
    pub fn max_connections(&self) -> usize {
        self.max_connections
    }
}

impl Default for ConnectionPool {
    fn default() -> Self {
        Self::new(MAX_CONNECTIONS)
    }
}

/// Guard that releases a connection permit when dropped
pub struct ConnectionGuard {
    _permit: tokio::sync::OwnedSemaphorePermit,
    active_count: Arc<AtomicUsize>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.active_count.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_pool_basic() {
        let pool = ConnectionPool::new(2);

        assert_eq!(pool.active_connections(), 0);
        assert_eq!(pool.max_connections(), 2);
        assert!(pool.can_accept());
    }

    #[tokio::test]
    async fn test_connection_pool_acquire() {
        let pool = ConnectionPool::new(2);

        let _guard1 = pool.acquire().await;
        assert_eq!(pool.active_connections(), 1);

        let _guard2 = pool.acquire().await;
        assert_eq!(pool.active_connections(), 2);

        // Pool is full
        assert!(!pool.can_accept());
    }

    #[tokio::test]
    async fn test_connection_pool_release() {
        let pool = ConnectionPool::new(2);

        {
            let _guard1 = pool.acquire().await;
            let _guard2 = pool.acquire().await;
            assert_eq!(pool.active_connections(), 2);
        } // guards dropped here

        // Wait a moment for cleanup
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        assert_eq!(pool.active_connections(), 0);
        assert!(pool.can_accept());
    }

    #[tokio::test]
    async fn test_connection_pool_try_acquire() {
        let pool = ConnectionPool::new(1);

        let _guard = pool.acquire().await;
        assert_eq!(pool.active_connections(), 1);

        // Should fail since pool is full
        let guard2 = pool.try_acquire();
        assert!(guard2.is_none());
    }

    #[test]
    #[should_panic(expected = "max_connections must be > 0")]
    fn test_connection_pool_zero_panic() {
        let _ = ConnectionPool::new(0);
    }
}
