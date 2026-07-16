mod artifact_io;
mod chunk;
mod preparation;
mod result;
mod spool;
mod wav;

pub(super) use chunk::read_prepared_chunk;
pub(super) use preparation::prepare_imported_pcm_wav;
pub(super) use result::{publish_remote_result, read_published_remote_transcript};
pub(super) use spool::reset_unattached_spool;

#[cfg(test)]
use result::{read_bounded_to_end, validate_published_result_contract};
#[cfg(test)]
use wav::{validate_pcm_data_bytes, MAX_WAV_CONTAINER_OVERHEAD_BYTES};

#[cfg(test)]
mod tests;
