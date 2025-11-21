//! Recording session management

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use tokio::sync::Mutex;

use crate::fingerprint::RequestChain;
use crate::storage::RecordingWriter;
use crate::{OuliError, Result};

use super::MAX_SESSIONS;

/// Recording session manager
pub struct SessionManager {
    sessions: DashMap<String, Arc<RecordingSession>>,
    recording_dir: PathBuf,
    session_count: AtomicUsize,
}

impl SessionManager {
    /// Create a new session manager
    #[must_use]
    pub fn new(recording_dir: PathBuf) -> Self {
        Self {
            sessions: DashMap::new(),
            recording_dir,
            session_count: AtomicUsize::new(0),
        }
    }

    /// Get or create a recording session
    ///
    /// # Errors
    ///
    /// Returns error if session limit reached or session creation fails
    pub fn get_or_create_session(&self, test_name: &str) -> Result<Arc<RecordingSession>> {
        // Check if session exists
        if let Some(session) = self.sessions.get(test_name) {
            return Ok(Arc::clone(&session));
        }

        // Check session limit
        let current_count = self.session_count.load(Ordering::Relaxed);
        if current_count >= MAX_SESSIONS {
            return Err(OuliError::Other(format!(
                "Session limit reached: {MAX_SESSIONS}"
            )));
        }

        // Validate test name
        validate_test_name(test_name)?;

        // Create new session
        let session = Arc::new(RecordingSession::new(test_name, &self.recording_dir)?);

        // Insert and increment count
        self.sessions
            .insert(test_name.to_string(), Arc::clone(&session));
        self.session_count.fetch_add(1, Ordering::Relaxed);

        Ok(session)
    }

    /// Get the number of active sessions
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.session_count.load(Ordering::Relaxed)
    }

    /// Finalize all sessions
    ///
    /// # Errors
    ///
    /// Returns error if any session fails to finalize
    pub async fn finalize_all(&self) -> Result<()> {
        let sessions: Vec<_> = self
            .sessions
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect();

        for session in sessions {
            session.finalize().await?;
        }

        self.sessions.clear();
        self.session_count.store(0, Ordering::Relaxed);

        Ok(())
    }
}

/// A single recording session
pub struct RecordingSession {
    test_name: String,
    writer: Mutex<Option<RecordingWriter>>,
    chain: Mutex<RequestChain>,
    created_at: SystemTime,
    interaction_count: AtomicUsize,
}

impl RecordingSession {
    /// Create a new recording session
    ///
    /// # Errors
    ///
    /// Returns error if writer cannot be created
    fn new(test_name: &str, recording_dir: &Path) -> Result<Self> {
        let file_path = recording_dir.join(format!("{test_name}.ouli"));

        // Generate recording ID from test name
        let recording_id = generate_recording_id(test_name);

        let writer = RecordingWriter::create(&file_path, recording_id)?;

        Ok(Self {
            test_name: test_name.to_string(),
            writer: Mutex::new(Some(writer)),
            chain: Mutex::new(RequestChain::new()),
            created_at: SystemTime::now(),
            interaction_count: AtomicUsize::new(0),
        })
    }

    /// Get the test name
    #[must_use]
    pub fn test_name(&self) -> &str {
        &self.test_name
    }

    /// Get the request chain
    pub async fn chain(&self) -> tokio::sync::MutexGuard<'_, RequestChain> {
        self.chain.lock().await
    }

    /// Get the writer
    pub async fn writer(&self) -> tokio::sync::MutexGuard<'_, Option<RecordingWriter>> {
        self.writer.lock().await
    }

    /// Get interaction count
    #[must_use]
    pub fn interaction_count(&self) -> usize {
        self.interaction_count.load(Ordering::Relaxed)
    }

    /// Increment interaction count
    pub fn increment_interactions(&self) {
        self.interaction_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Get session age
    #[must_use]
    pub fn age(&self) -> std::time::Duration {
        SystemTime::now()
            .duration_since(self.created_at)
            .unwrap_or_default()
    }

    /// Finalize the session
    ///
    /// # Errors
    ///
    /// Returns error if writer fails to finalize
    pub async fn finalize(&self) -> Result<()> {
        let mut writer_guard = self.writer.lock().await;

        // Get final chain state
        let final_chain_state = {
            let chain = self.chain.lock().await;
            chain.current_hash()
        };

        if let Some(writer) = writer_guard.take() {
            writer.finalize(final_chain_state)?;
        }

        Ok(())
    }
}

/// Validate a test name
///
/// # Errors
///
/// Returns error if test name is invalid
fn validate_test_name(name: &str) -> Result<()> {
    // Check length
    if name.is_empty() {
        return Err(OuliError::InvalidTestName(
            "Test name cannot be empty".to_string(),
        ));
    }

    if name.len() > 255 {
        return Err(OuliError::InvalidTestName(format!(
            "Test name too long: {} > 255",
            name.len()
        )));
    }

    // Check for path separators
    if name.contains('/') || name.contains('\\') {
        return Err(OuliError::InvalidTestName(
            "Test name cannot contain path separators".to_string(),
        ));
    }

    // Check for hidden files
    if name.starts_with('.') {
        return Err(OuliError::InvalidTestName(
            "Test name cannot start with dot".to_string(),
        ));
    }

    // Check for null bytes
    if name.contains('\0') {
        return Err(OuliError::InvalidTestName(
            "Test name cannot contain null bytes".to_string(),
        ));
    }

    // Check for path traversal
    if name.contains("..") {
        return Err(OuliError::InvalidTestName(
            "Test name cannot contain '..'".to_string(),
        ));
    }

    Ok(())
}

/// Generate a recording ID from test name
fn generate_recording_id(test_name: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(test_name.as_bytes());

    // Add timestamp for uniqueness
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_nanos();
    hasher.update(timestamp.to_le_bytes());

    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_session_manager_create() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        assert_eq!(manager.session_count(), 0);

        let session = manager.get_or_create_session("test1").unwrap();
        assert_eq!(session.test_name(), "test1");
        assert_eq!(manager.session_count(), 1);
    }

    #[tokio::test]
    async fn test_session_manager_reuse() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        let session1 = manager.get_or_create_session("test1").unwrap();
        let session2 = manager.get_or_create_session("test1").unwrap();

        assert_eq!(manager.session_count(), 1);
        assert_eq!(session1.test_name(), session2.test_name());
    }

    #[tokio::test]
    async fn test_session_manager_multiple() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        manager.get_or_create_session("test1").unwrap();
        manager.get_or_create_session("test2").unwrap();
        manager.get_or_create_session("test3").unwrap();

        assert_eq!(manager.session_count(), 3);
    }

    #[tokio::test]
    async fn test_session_finalize() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SessionManager::new(temp_dir.path().to_path_buf());

        manager.get_or_create_session("test1").unwrap();
        manager.get_or_create_session("test2").unwrap();

        manager.finalize_all().await.unwrap();

        assert_eq!(manager.session_count(), 0);
    }

    #[test]
    fn test_validate_test_name() {
        assert!(validate_test_name("valid_test").is_ok());
        assert!(validate_test_name("test-123").is_ok());
        assert!(validate_test_name("Test_Name_123").is_ok());

        assert!(validate_test_name("").is_err());
        assert!(validate_test_name(".hidden").is_err());
        assert!(validate_test_name("test/path").is_err());
        assert!(validate_test_name("test\\path").is_err());
        assert!(validate_test_name("test..name").is_err());
        assert!(validate_test_name("test\0name").is_err());
    }

    #[test]
    fn test_generate_recording_id() {
        let id1 = generate_recording_id("test1");
        let id2 = generate_recording_id("test1");

        // IDs should be different due to timestamp
        assert_ne!(id1, id2);
    }
}
