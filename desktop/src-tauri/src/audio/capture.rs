mod adapter;
mod callback;

pub use adapter::CaptureAdapter;
pub use callback::{CapturePacket, CapturePorts};

#[cfg(test)]
pub(crate) use callback::new_callback_boundary;

#[cfg(test)]
mod tests;
