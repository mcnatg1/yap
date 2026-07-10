//! Deterministic desktop-side audio preparation.
//! Heavy inference, diarization, enrichment, and team storage stay server-owned.

pub mod capture;
pub mod evidence;
pub mod frame;
pub mod manifest;
pub mod preprocess;
pub mod results;
pub mod session;
pub mod timeline;
pub mod vad;
