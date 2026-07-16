mod clock;
mod loss_accumulator;
mod loss_buffer;
mod loss_registration;
mod track;

pub use clock::{
    ClockMappingRevision, RecordingInput, RecordingRevisionTransition, SessionClock, TimelineError,
};
pub use loss_accumulator::LossAccumulator;
pub use loss_buffer::{LossSnapshot, TryDrain};
pub use track::Timeline;

#[cfg(test)]
use loss_buffer::{LOSS_RUN_CAPACITY, REGISTRATION_TICKET_CAPACITY};
#[cfg(test)]
use track::merge_gap;

#[cfg(test)]
mod tests;
