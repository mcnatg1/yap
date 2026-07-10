//! Deterministic desktop-side audio preparation.
//! Heavy inference, diarization, enrichment, and team storage stay server-owned.

pub mod frame;
pub mod manifest;
pub mod preprocess;
pub mod session;
pub mod vad;
