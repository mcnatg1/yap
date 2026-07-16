use super::*;

#[test]
fn native_drop_queue_bounds_payload_and_backlog() {
    let (batches, _receiver) = native_import_channel();
    let batch = vec![std::path::PathBuf::from("recording.wav")];

    queue_native_import_batch(&batches, batch.clone()).unwrap();
    let busy = queue_native_import_batch(&batches, batch).unwrap_err();
    assert_eq!(busy.code, "IMPORT_BUSY");

    let oversized = vec![std::path::PathBuf::from("recording.wav"); MAX_RECORDING_JOBS + 1];
    let rejected = queue_native_import_batch(&batches, oversized).unwrap_err();
    assert_eq!(rejected.code, "JOB_LIMIT_EXCEEDED");
}
