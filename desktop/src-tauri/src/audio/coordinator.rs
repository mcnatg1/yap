mod core;
mod lifecycle;
mod pending_losses;
mod receiver;
mod sink;
mod sink_types;

pub use core::Coordinator;
#[cfg(test)]
pub use core::RevisionEvent;
pub(crate) use sink_types::SinkDegradeResult;
pub use sink_types::{
    bounded_sink, BoundedReceiver, BoundedSink, CoordinatorPorts, SinkKind, SinkOutcome,
    SinkSendError,
};

pub const RECORDING_QUEUE_CAPACITY: usize = 128;
pub const LOCAL_ASR_QUEUE_CAPACITY: usize = 64;
pub const EVIDENCE_QUEUE_CAPACITY: usize = 32;
pub const SERVER_TRANSPORT_QUEUE_CAPACITY: usize = 64;
pub(super) const TARGET_SAMPLE_RATE_HZ: u32 = 16_000;

#[cfg(test)]
mod tests;
