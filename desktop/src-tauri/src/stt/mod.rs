//! STT backends: dispatcher, error contract, Python fallback, and the CrispASR
//! HTTP sidecar. Spec: docs/superpowers/specs/2026-06-30-crispasr-stt-sidecar-design.md

pub mod backend;
pub mod error;
pub mod model;
pub mod pin;
