use std::{
    io::{Cursor, Write},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::media_protocol::server::{stream_exact_range, StreamOutcome, STREAM_BUFFER_BYTES};

struct CappedWriter {
    fail_after: usize,
    largest_write: usize,
    written: usize,
}

impl Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.largest_write = self.largest_write.max(bytes.len());
        if self.written >= self.fail_after {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "client disconnected",
            ));
        }
        self.written += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn large_copy_uses_bounded_chunks_and_stops_on_disconnect_or_revoke() {
    let payload = vec![7_u8; STREAM_BUFFER_BYTES * 4];
    let mut reader = Cursor::new(payload.clone());
    let mut writer = CappedWriter {
        fail_after: STREAM_BUFFER_BYTES,
        largest_write: 0,
        written: 0,
    };
    let revoked = AtomicBool::new(false);

    let error =
        stream_exact_range(&mut reader, &mut writer, payload.len() as u64, &revoked).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    assert!(writer.largest_write <= STREAM_BUFFER_BYTES);
    assert!(reader.position() <= (STREAM_BUFFER_BYTES * 2) as u64);

    let mut reader = Cursor::new(payload);
    let mut sink = Vec::new();
    revoked.store(true, Ordering::Release);
    let outcome = stream_exact_range(&mut reader, &mut sink, u64::MAX, &revoked).unwrap();
    assert_eq!(outcome, StreamOutcome::Revoked);
    assert_eq!(reader.position(), 0);
    assert!(sink.is_empty());
}
