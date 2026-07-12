use std::sync::Mutex;

use crate::server_connector::ServerCapabilities;

use super::state::{JobRoute, RuntimeState, ServerConnectorState, SetupState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    FallbackDisabled,
    RuntimeBusy,
    ServerUnavailable,
    SetupUnavailable,
    SetupRequired,
}

#[derive(Debug)]
pub struct RuntimeOrchestrator {
    setup: SetupState,
    server: ServerConnectorState,
    server_capabilities: ServerCapabilities,
    runtime: RuntimeState,
}

impl Default for RuntimeOrchestrator {
    fn default() -> Self {
        Self {
            setup: SetupState::Checking,
            server: ServerConnectorState::NotSet,
            server_capabilities: ServerCapabilities::default(),
            runtime: RuntimeState::Idle,
        }
    }
}

impl RuntimeOrchestrator {
    pub fn setup(&self) -> SetupState {
        self.setup
    }

    pub fn server(&self) -> ServerConnectorState {
        self.server
    }

    pub fn runtime(&self) -> RuntimeState {
        self.runtime
    }

    pub fn server_capabilities(&self) -> ServerCapabilities {
        self.server_capabilities
    }

    pub fn set_setup(&mut self, setup: SetupState) {
        self.setup = setup;
        match setup {
            SetupState::FallbackReady if self.runtime == RuntimeState::Idle => {
                self.runtime = RuntimeState::FallbackReady;
            }
            SetupState::FallbackReady => {}
            _ if matches!(
                self.runtime,
                RuntimeState::FallbackReady | RuntimeState::FallbackRunning
            ) =>
            {
                self.runtime = RuntimeState::Idle;
            }
            _ => {}
        }
    }

    pub fn set_server(&mut self, server: ServerConnectorState, capabilities: ServerCapabilities) {
        self.server = server;
        self.server_capabilities = capabilities;
    }

    pub fn route_recording(&self, _larger_recording: bool) -> Result<JobRoute, RuntimeError> {
        self.route_imported_recording()
    }

    pub fn route_imported_recording(&self) -> Result<JobRoute, RuntimeError> {
        if self.server == ServerConnectorState::Ready && self.server_capabilities.batch_jobs {
            Ok(JobRoute::ServerBatch)
        } else {
            Err(RuntimeError::ServerUnavailable)
        }
    }

    pub fn route_live(&self) -> Result<JobRoute, RuntimeError> {
        if self.server == ServerConnectorState::Ready && self.server_capabilities.live_streaming {
            return Ok(JobRoute::ServerLive);
        }
        match self.setup {
            SetupState::FallbackReady => Ok(JobRoute::LocalFallback),
            SetupState::FallbackDisabled => Err(RuntimeError::FallbackDisabled),
            _ => Err(RuntimeError::SetupRequired),
        }
    }

    pub fn start_fallback(&mut self) -> Result<(), RuntimeError> {
        match self.setup {
            SetupState::FallbackReady => {}
            SetupState::FallbackDisabled => return Err(RuntimeError::FallbackDisabled),
            SetupState::SetupError => return Err(RuntimeError::SetupUnavailable),
            _ => return Err(RuntimeError::SetupRequired),
        }
        if !matches!(
            self.runtime,
            RuntimeState::Idle | RuntimeState::FallbackReady
        ) {
            return Err(RuntimeError::RuntimeBusy);
        }
        self.runtime = RuntimeState::FallbackRunning;
        Ok(())
    }

    pub fn finish_active_work(&mut self) {
        self.runtime = match self.setup {
            SetupState::FallbackReady => RuntimeState::FallbackReady,
            _ => RuntimeState::Idle,
        };
    }
}

pub struct RuntimeOrchestratorState {
    orchestrator: Mutex<RuntimeOrchestrator>,
}

impl RuntimeOrchestratorState {
    pub fn new() -> Self {
        Self {
            orchestrator: Mutex::new(RuntimeOrchestrator::default()),
        }
    }

    pub fn with<T>(&self, update: impl FnOnce(&mut RuntimeOrchestrator) -> T) -> T {
        let mut orchestrator = self
            .orchestrator
            .lock()
            .expect("runtime orchestrator poisoned");
        update(&mut orchestrator)
    }
}

impl Default for RuntimeOrchestratorState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn larger_recording_requires_server_ready() {
        let mut runtime = RuntimeOrchestrator::default();
        runtime.set_setup(SetupState::FallbackReady);
        assert_eq!(
            runtime.route_recording(true),
            Err(RuntimeError::ServerUnavailable)
        );
        runtime.set_server(
            ServerConnectorState::Ready,
            ServerCapabilities {
                batch_jobs: true,
                ..ServerCapabilities::default()
            },
        );
        assert_eq!(runtime.route_recording(true), Ok(JobRoute::ServerBatch));
    }

    #[test]
    fn ready_without_batch_capability_never_routes_imports_locally() {
        let mut runtime = RuntimeOrchestrator::default();
        runtime.set_setup(SetupState::FallbackReady);
        runtime.set_server(ServerConnectorState::Ready, ServerCapabilities::default());

        assert_eq!(
            runtime.route_imported_recording(),
            Err(RuntimeError::ServerUnavailable)
        );
        assert_eq!(
            runtime.route_recording(true),
            Err(RuntimeError::ServerUnavailable)
        );
    }

    #[test]
    fn live_server_route_requires_ready_and_live_streaming() {
        let mut runtime = RuntimeOrchestrator::default();
        runtime.set_setup(SetupState::FallbackReady);
        runtime.set_server(ServerConnectorState::Ready, ServerCapabilities::default());
        assert_eq!(runtime.route_live(), Ok(JobRoute::LocalFallback));

        runtime.set_server(
            ServerConnectorState::Ready,
            ServerCapabilities {
                live_streaming: true,
                ..ServerCapabilities::default()
            },
        );
        assert_eq!(runtime.route_live(), Ok(JobRoute::ServerLive));

        runtime.set_server(
            ServerConnectorState::Offline,
            ServerCapabilities {
                live_streaming: true,
                ..ServerCapabilities::default()
            },
        );
        assert_eq!(runtime.route_live(), Ok(JobRoute::LocalFallback));
    }

    #[test]
    fn fallback_requires_setup_ready() {
        let mut runtime = RuntimeOrchestrator::default();
        assert_eq!(runtime.start_fallback(), Err(RuntimeError::SetupRequired));
        runtime.set_setup(SetupState::FallbackDisabled);
        assert_eq!(
            runtime.start_fallback(),
            Err(RuntimeError::FallbackDisabled)
        );
        runtime.set_setup(SetupState::SetupError);
        assert_eq!(
            runtime.start_fallback(),
            Err(RuntimeError::SetupUnavailable)
        );
        runtime.set_setup(SetupState::FallbackReady);
        assert_eq!(runtime.start_fallback(), Ok(()));
        assert_eq!(runtime.runtime(), RuntimeState::FallbackRunning);
    }

    #[test]
    fn finish_returns_to_fallback_ready_when_setup_is_ready() {
        let mut runtime = RuntimeOrchestrator::default();
        runtime.set_setup(SetupState::FallbackReady);
        runtime.start_fallback().unwrap();
        runtime.finish_active_work();
        assert_eq!(runtime.runtime(), RuntimeState::FallbackReady);
    }

    #[test]
    fn setup_loss_demotes_fallback_runtime() {
        let mut runtime = RuntimeOrchestrator::default();
        runtime.set_setup(SetupState::FallbackReady);
        assert_eq!(runtime.runtime(), RuntimeState::FallbackReady);
        runtime.set_setup(SetupState::FallbackMissing);
        assert_eq!(runtime.runtime(), RuntimeState::Idle);
    }

    #[test]
    fn fallback_start_rejects_existing_work() {
        let mut runtime = RuntimeOrchestrator::default();
        runtime.set_setup(SetupState::FallbackReady);
        runtime.start_fallback().unwrap();
        assert_eq!(runtime.start_fallback(), Err(RuntimeError::RuntimeBusy));
        assert_eq!(runtime.runtime(), RuntimeState::FallbackRunning);
    }
}
