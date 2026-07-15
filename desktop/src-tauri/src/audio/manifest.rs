mod envelope;
mod validation;
mod window_support;
mod windows;

pub use envelope::{
    classify_replay, AudioChunkEnvelopeBuilder, AudioSessionEnvelope, AudioSessionEnvelopeBuilder,
    ChunkWindowConfig, ReplayConflict, ReplayDecision, MANIFEST_SCHEMA_VERSION,
};
pub use windows::build_manifest_windows;

#[cfg(test)]
mod tests;
