#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupState {
    Checking,
    FallbackMissing,
    FallbackInstalling,
    FallbackReady,
    FallbackDisabled,
    SetupError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerConnectorState {
    NotSet,
    Connecting,
    Ready,
    Offline,
    SignInRequired,
    Retrying,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    Idle,
    FallbackReady,
    FallbackRunning,
    ServerQueued,
    ServerUploading,
    LiveReady,
    LiveActive,
    BackgroundEnriching,
    DegradedBackground,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobRoute {
    LocalFallback,
    ServerBatch,
    ServerLive,
}
