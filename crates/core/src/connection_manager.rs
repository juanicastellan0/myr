use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use thiserror::Error;

use crate::profiles::ConnectionProfile;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BackendError {
    message: String,
}

impl BackendError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait ConnectionBackend {
    type Connection: Send;

    async fn connect(&self, profile: &ConnectionProfile) -> Result<Self::Connection, BackendError>;
    async fn ping(&self, connection: &mut Self::Connection) -> Result<(), BackendError>;
    async fn disconnect(&self, connection: Self::Connection) -> Result<(), BackendError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionStatus {
    pub profile_name: Option<String>,
    pub is_connected: bool,
    pub last_latency: Option<Duration>,
    pub last_health_check_at: Option<SystemTime>,
}

impl ConnectionStatus {
    #[must_use]
    pub fn disconnected() -> Self {
        Self {
            profile_name: None,
            is_connected: false,
            last_latency: None,
            last_health_check_at: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum ConnectionManagerError {
    #[error("active connection already exists for profile `{active_profile}`")]
    AlreadyConnected { active_profile: String },
    #[error("connection manager is not connected")]
    NotConnected,
    #[error("connection backend failed: {0}")]
    Backend(#[source] BackendError),
}

#[derive(Debug)]
struct ActiveConnection<C> {
    profile: ConnectionProfile,
    handle: C,
}

#[derive(Debug)]
pub struct ConnectionManager<B: ConnectionBackend> {
    backend: B,
    active: Option<ActiveConnection<B::Connection>>,
    last_latency: Option<Duration>,
    last_health_check_at: Option<SystemTime>,
}

impl<B: ConnectionBackend> ConnectionManager<B> {
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            active: None,
            last_latency: None,
            last_health_check_at: None,
        }
    }

    #[must_use]
    pub fn status(&self) -> ConnectionStatus {
        ConnectionStatus {
            profile_name: self
                .active
                .as_ref()
                .map(|active| active.profile.name.clone()),
            is_connected: self.active.is_some(),
            last_latency: self.last_latency,
            last_health_check_at: self.last_health_check_at,
        }
    }

    #[must_use]
    pub fn active_profile(&self) -> Option<&ConnectionProfile> {
        self.active.as_ref().map(|active| &active.profile)
    }

    pub async fn connect(
        &mut self,
        profile: ConnectionProfile,
    ) -> Result<Duration, ConnectionManagerError> {
        if let Some(active) = &self.active {
            return Err(ConnectionManagerError::AlreadyConnected {
                active_profile: active.profile.name.clone(),
            });
        }

        let started_at = Instant::now();
        let mut handle = self
            .backend
            .connect(&profile)
            .await
            .map_err(ConnectionManagerError::Backend)?;
        self.backend
            .ping(&mut handle)
            .await
            .map_err(ConnectionManagerError::Backend)?;

        let latency = started_at.elapsed();
        self.last_latency = Some(latency);
        self.last_health_check_at = Some(SystemTime::now());
        self.active = Some(ActiveConnection { profile, handle });

        Ok(latency)
    }

    pub async fn health_check(&mut self) -> Result<Duration, ConnectionManagerError> {
        let active = self
            .active
            .as_mut()
            .ok_or(ConnectionManagerError::NotConnected)?;

        let started_at = Instant::now();
        self.backend
            .ping(&mut active.handle)
            .await
            .map_err(ConnectionManagerError::Backend)?;

        let latency = started_at.elapsed();
        self.last_latency = Some(latency);
        self.last_health_check_at = Some(SystemTime::now());

        Ok(latency)
    }

    pub async fn disconnect(&mut self) -> Result<(), ConnectionManagerError> {
        let Some(active) = self.active.take() else {
            return Ok(());
        };

        self.backend
            .disconnect(active.handle)
            .await
            .map_err(ConnectionManagerError::Backend)?;
        self.last_latency = None;
        self.last_health_check_at = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    use super::{
        BackendError, ConnectionBackend, ConnectionManager, ConnectionManagerError,
        ConnectionStatus,
    };
    use crate::profiles::ConnectionProfile;

    #[derive(Debug, Default)]
    struct FakeBackend {
        disconnect_calls: AtomicUsize,
        fail_connect: AtomicUsize,
        fail_ping: AtomicUsize,
        ping_calls: AtomicUsize,
    }

    #[derive(Debug)]
    struct FakeConnection {
        _state: Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl ConnectionBackend for FakeBackend {
        type Connection = FakeConnection;

        async fn connect(
            &self,
            _profile: &ConnectionProfile,
        ) -> Result<Self::Connection, BackendError> {
            if self.fail_connect.load(Ordering::Relaxed) > 0 {
                self.fail_connect.fetch_sub(1, Ordering::Relaxed);
                return Err(BackendError::new("connect failed"));
            }

            Ok(FakeConnection {
                _state: Mutex::new(0),
            })
        }

        async fn ping(&self, _connection: &mut Self::Connection) -> Result<(), BackendError> {
            self.ping_calls.fetch_add(1, Ordering::Relaxed);
            if self.fail_ping.load(Ordering::Relaxed) > 0 {
                self.fail_ping.fetch_sub(1, Ordering::Relaxed);
                return Err(BackendError::new("ping failed"));
            }
            Ok(())
        }

        async fn disconnect(&self, _connection: Self::Connection) -> Result<(), BackendError> {
            self.disconnect_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn sample_profile() -> ConnectionProfile {
        ConnectionProfile::new("local", "127.0.0.1", "root")
    }

    #[tokio::test]
    async fn connect_updates_status_and_profile() {
        let backend = FakeBackend::default();
        let mut manager = ConnectionManager::new(backend);

        let latency = manager
            .connect(sample_profile())
            .await
            .expect("connect should succeed");
        assert!(latency >= std::time::Duration::ZERO);

        let status = manager.status();
        assert!(status.is_connected);
        assert_eq!(status.profile_name.as_deref(), Some("local"));
        assert!(status.last_latency.is_some());
        assert!(status.last_health_check_at.is_some());
    }

    #[tokio::test]
    async fn health_check_requires_active_connection() {
        let backend = FakeBackend::default();
        let mut manager = ConnectionManager::new(backend);

        let err = manager
            .health_check()
            .await
            .expect_err("health check should fail when disconnected");
        assert!(matches!(err, ConnectionManagerError::NotConnected));
    }

    #[tokio::test]
    async fn connect_fails_when_already_connected() {
        let backend = FakeBackend::default();
        let mut manager = ConnectionManager::new(backend);
        manager
            .connect(sample_profile())
            .await
            .expect("first connect should succeed");

        let err = manager
            .connect(sample_profile())
            .await
            .expect_err("second connect should fail");
        assert!(matches!(
            err,
            ConnectionManagerError::AlreadyConnected { .. }
        ));
    }

    #[tokio::test]
    async fn disconnect_is_idempotent_and_clears_status() {
        let backend = FakeBackend::default();
        let mut manager = ConnectionManager::new(backend);
        manager
            .connect(sample_profile())
            .await
            .expect("connect should succeed");
        manager
            .disconnect()
            .await
            .expect("disconnect should succeed");
        manager
            .disconnect()
            .await
            .expect("disconnect should stay idempotent");

        let status = manager.status();
        assert_eq!(status, ConnectionStatus::disconnected());
    }

    #[tokio::test]
    async fn failed_connect_does_not_set_active_connection() {
        let backend = FakeBackend {
            fail_connect: AtomicUsize::new(1),
            ..FakeBackend::default()
        };
        let mut manager = ConnectionManager::new(backend);

        let err = manager
            .connect(sample_profile())
            .await
            .expect_err("connect should fail");
        assert!(matches!(err, ConnectionManagerError::Backend(_)));
        assert!(manager.active_profile().is_none());
    }
}
