use std::{
    fs::{self, File},
    io::{Cursor, Write},
    time::{Duration, UNIX_EPOCH},
};

use crate::{
    audio::session::OwnerNamespace,
    server_connector::batch::{LanguageDecision, ModelRevision, TranscriptResultRevision},
};

use super::{
    prepare_imported_pcm_wav, publish_remote_result, read_bounded_to_end, read_prepared_chunk,
    read_published_remote_transcript, reset_unattached_spool, validate_pcm_data_bytes,
    validate_published_result_contract,
};

#[test]
fn result_reader_never_buffers_past_its_declared_limit() {
    let mut source = Cursor::new(vec![0_u8; 9]);

    let error = read_bounded_to_end(&mut source, 8).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn client_intake_matches_the_server_four_hour_pcm_ceiling() {
    let four_hours = 16_000_u64 * 2 * 4 * 60 * 60;
    assert!(validate_pcm_data_bytes(four_hours).is_ok());
    assert!(validate_pcm_data_bytes(four_hours + 2).is_err());
}

#[test]
fn wav_bytes_outside_declared_riff_are_rejected_before_spooling() {
    let root =
        std::env::temp_dir().join(format!("yap-phase5-riff-boundary-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let source_path = root.join("source.wav");
    write_pcm_wav(&source_path, &[0_u8; 320]);
    let mut append = fs::OpenOptions::new()
        .append(true)
        .open(&source_path)
        .unwrap();
    append.write_all(b"private trailing bytes").unwrap();
    append.sync_all().unwrap();
    drop(append);
    let mut source = File::open(&source_path).unwrap();
    let owner = OwnerNamespace::local("i-phase5-riff-boundary").unwrap();

    let error = prepare_imported_pcm_wav(
        "job-phase5-riff-boundary",
        "source.wav",
        &mut source,
        &root.join("spool"),
        &owner,
        UNIX_EPOCH + Duration::from_secs(1_720_000_000),
    )
    .err()
    .expect("trailing bytes must reject the imported WAV");

    assert_eq!(
        error,
        "imported WAV file length does not match its RIFF boundary"
    );
    assert!(!root.join("spool").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oversized_wav_container_metadata_is_rejected_before_spooling() {
    let root =
        std::env::temp_dir().join(format!("yap-phase5-riff-overhead-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let source_path = root.join("source.wav");
    write_pcm_wav_with_junk(
        &source_path,
        &[0_u8; 320],
        super::MAX_WAV_CONTAINER_OVERHEAD_BYTES as usize,
    );
    let mut source = File::open(&source_path).unwrap();
    let owner = OwnerNamespace::local("i-phase5-riff-overhead").unwrap();

    let error = prepare_imported_pcm_wav(
        "job-phase5-riff-overhead",
        "source.wav",
        &mut source,
        &root.join("spool"),
        &owner,
        UNIX_EPOCH + Duration::from_secs(1_720_000_000),
    )
    .err()
    .expect("oversized WAV metadata must be rejected");

    assert_eq!(error, "imported WAV container metadata is too large");
    assert!(!root.join("spool").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn canonical_pcm_wav_becomes_an_immutable_owned_upload_manifest() {
    let root = std::env::temp_dir().join(format!("yap-phase5-prepare-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let source_path = root.join("source.wav");
    let pcm = vec![0_u8; 320];
    write_pcm_wav(&source_path, &pcm);
    let original = fs::read(&source_path).unwrap();
    let mut source = File::open(&source_path).unwrap();
    let owner = OwnerNamespace::local("i-phase5-test").unwrap();

    let prepared = prepare_imported_pcm_wav(
        "job-phase5-test",
        "source.wav",
        &mut source,
        &root.join("spool"),
        &owner,
        UNIX_EPOCH + Duration::from_secs(1_720_000_000),
    )
    .unwrap();

    assert_eq!(prepared.request.route, "server_batch");
    assert_eq!(
        prepared.request.metadata.origin,
        crate::audio::session::SessionOrigin::ImportedFile
    );
    assert_eq!(
        prepared.request.metadata.preferred_languages_bcp47,
        ["en-US"]
    );
    assert_eq!(prepared.request.tracks.len(), 1);
    assert_eq!(prepared.request.chunks.len(), 1);
    assert_eq!(prepared.chunks.len(), 1);
    assert_eq!(fs::read(&prepared.chunks[0].artifact_path).unwrap(), pcm);
    assert!(prepared.capture_manifest_path.is_file());
    assert_eq!(
        fs::metadata(&prepared.capture_manifest_path).unwrap().len(),
        prepared.request.capture_manifest.byte_length
    );
    assert_eq!(fs::read(source_path).unwrap(), original);
    assert_eq!(prepared.owner_namespace, owner.as_str());
    assert_eq!(
        read_prepared_chunk(
            &prepared.chunks[0].artifact_path,
            &root.join("spool"),
            &prepared.chunks[0].reference,
        )
        .unwrap(),
        pcm
    );

    let durable = prepared.into_ledger_state().unwrap();
    let durable_request: serde_json::Value =
        serde_json::from_str(&durable.create_request_json).unwrap();
    assert_eq!(durable_request["route"], "server_batch");
    assert_eq!(durable.chunks.len(), 1);
    assert_eq!(durable.chunks[0].content_byte_length, 320);
    assert_eq!(durable.chunks[0].sequence_start, 0);
    assert_eq!(durable.chunks[0].sequence_end, 159);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_removes_only_exact_owned_job_staging_shapes_after_a_crash() {
    let root =
        std::env::temp_dir().join(format!("yap-phase5-staging-cleanup-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let spool = root.join("remote-jobs");
    fs::create_dir_all(&spool).unwrap();
    let abandoned_prepare = spool.join(".job-stale-4242-7.part");
    let abandoned_quarantine = spool.join(".job-stale-orphan-4242-8");
    let unrelated = spool.join(".job-stale-user-data");
    for directory in [&abandoned_prepare, &abandoned_quarantine, &unrelated] {
        fs::create_dir(directory).unwrap();
        fs::write(directory.join("private.pcm"), b"private bytes").unwrap();
    }

    reset_unattached_spool("job-stale", &spool).unwrap();

    assert!(!abandoned_prepare.exists());
    assert!(!abandoned_quarantine.exists());
    assert!(unrelated.is_dir());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn published_remote_transcript_is_reopened_only_through_its_result_revision() {
    let root = std::env::temp_dir().join(format!("yap-phase5-result-open-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let spool = root.join("remote-jobs");
    let job_id = "job-phase5-result-open";
    fs::create_dir_all(spool.join(job_id)).unwrap();
    let result = TranscriptResultRevision {
        session_id: "s-phase5-result-open".into(),
        revision: 1,
        authority: "server_authoritative".into(),
        created_at_utc: "2026-07-14T21:00:02Z".into(),
        capture_manifest_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .into(),
        previous_result_sha256: None,
        status: "complete".into(),
        language: Some(LanguageDecision {
            language_bcp47: "en-US".into(),
            confidence: Some(0.98),
        }),
        transcript: "Private result.".into(),
        aligned_words: Vec::new(),
        model_provenance: vec![ModelRevision {
            model_id: "CohereLabs/cohere-transcribe-03-2026".into(),
            revision: "b1eacc2686a3d08ceaae5f24a88b1d519620bc09".into(),
            calibration_revision: "asr-not-applicable".into(),
        }],
    };

    let output = publish_remote_result(job_id, &spool, &result).unwrap();
    let reopened = read_published_remote_transcript(&output, &spool).unwrap();
    assert_eq!(reopened.text, "Private result.\n");
    assert_eq!(reopened.result, result);

    let mut empty = result.clone();
    empty.transcript = " \n\t".into();
    assert!(validate_published_result_contract(&empty, 1).is_err());
    assert!(publish_remote_result(job_id, &spool, &empty).is_err());
    let mut offset_timestamp = result.clone();
    offset_timestamp.created_at_utc = "2026-07-14T16:00:02-05:00".into();
    assert!(validate_published_result_contract(&offset_timestamp, 1).is_err());

    fs::write(&output, "tampered\n").unwrap();
    assert!(read_published_remote_transcript(&output, &spool).is_err());
    assert!(read_published_remote_transcript(
        &spool
            .join(job_id)
            .join("result-00000000000000000001/../transcript.txt"),
        &spool,
    )
    .is_err());

    fs::remove_dir_all(root).unwrap();
}

fn write_pcm_wav(path: &std::path::Path, pcm: &[u8]) {
    let mut file = File::create(path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36_u32 + pcm.len() as u32).to_le_bytes())
        .unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&16_000_u32.to_le_bytes()).unwrap();
    file.write_all(&32_000_u32.to_le_bytes()).unwrap();
    file.write_all(&2_u16.to_le_bytes()).unwrap();
    file.write_all(&16_u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&(pcm.len() as u32).to_le_bytes()).unwrap();
    file.write_all(pcm).unwrap();
    file.sync_all().unwrap();
}

fn write_pcm_wav_with_junk(path: &std::path::Path, pcm: &[u8], junk_bytes: usize) {
    let file_bytes = 52_u64 + junk_bytes as u64 + pcm.len() as u64;
    let mut file = File::create(path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&u32::try_from(file_bytes - 8).unwrap().to_le_bytes())
        .unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&16_000_u32.to_le_bytes()).unwrap();
    file.write_all(&32_000_u32.to_le_bytes()).unwrap();
    file.write_all(&2_u16.to_le_bytes()).unwrap();
    file.write_all(&16_u16.to_le_bytes()).unwrap();
    file.write_all(b"JUNK").unwrap();
    file.write_all(&u32::try_from(junk_bytes).unwrap().to_le_bytes())
        .unwrap();
    file.write_all(&vec![0_u8; junk_bytes]).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&u32::try_from(pcm.len()).unwrap().to_le_bytes())
        .unwrap();
    file.write_all(pcm).unwrap();
    file.sync_all().unwrap();
}
