use super::*;

#[test]
fn corrupt_final_intent_is_quarantined_only_before_deletion_has_started() {
    let dir = test_dir("corrupt-intent-recovery");
    let session = SessionId::new("s-corrupt-intent-recovery").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let intent_name = deletion_intent_name(&session);
    std::fs::write(dir.join(&intent_name), b"{\"truncated\"").unwrap();

    delete_saved_session_for_test(&dir, &session).unwrap();

    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    assert!(!dir.join(&intent_name).exists());
    assert!(!std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .contains("deletion.v1.json.delete-")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn reconciliation_collects_only_old_foreign_private_deletion_leftovers() {
    let dir = test_dir("private-deletion-leftovers");
    let stale_staging = dir.join(".live-s-stale-leftover.deletion.v1.999999-0.part");
    let stale_quarantine = dir.join(".live-s-stale-leftover.deletion.v1.json.delete-999999-0");
    let active_staging = dir.join(format!(
        ".live-s-active-leftover.deletion.v1.{}-0.part",
        std::process::id()
    ));
    let unknown = dir.join(".live-s-unknown-leftover.deletion.v1.invalid.part");
    for path in [&stale_staging, &stale_quarantine, &active_staging, &unknown] {
        std::fs::write(path, b"leftover").unwrap();
        set_old_modified_time(path);
    }

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    assert!(!stale_staging.exists());
    assert!(!stale_quarantine.exists());
    assert!(active_staging.is_file());
    assert!(unknown.is_file());
    assert!(catalog
        .maintenance_warnings
        .iter()
        .any(|warning| warning.contains("Unknown private deletion artifact")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn reconciliation_collects_old_generic_artifact_quarantines_and_retains_invalid_evidence() {
    let dir = test_dir("generic-private-deletion-leftovers");
    let stale = [
        ".live-s-generic-cleanup.wav.delete-999999-0",
        ".live-s-generic-cleanup.capture.json.delete-999999-1",
        ".live-s-generic-cleanup.txt.delete-999999-2",
        ".live-s-generic-cleanup.transcript.r1.json.delete-999999-3",
        ".live-s-generic-cleanup.commit.json.delete-999999-4",
        ".live-s-generic-cleanup.capture.journal.part.delete-999999-5",
        ".live-s-generic-cleanup.deletion.v1.json.delete-999999-6",
    ];
    for name in stale {
        let path = dir.join(name);
        std::fs::write(&path, b"leftover").unwrap();
        set_old_modified_time(&path);
    }
    let nested = dir.join("..live-s-generic-cleanup.wav.delete-999999-0.delete-999999-7");
    let malformed = dir.join(".live-s-generic-cleanup.wav.delete-999999-extra-8");
    let active = dir.join(format!(
        ".live-s-generic-cleanup.wav.delete-{}-9",
        std::process::id()
    ));
    let recent = dir.join(".live-s-generic-cleanup.capture.json.delete-999999-10");
    let nonregular = dir.join(".live-s-generic-cleanup.txt.delete-999999-11");
    for path in [&nested, &malformed] {
        std::fs::write(path, b"evidence").unwrap();
        set_old_modified_time(path);
    }
    std::fs::write(&active, b"active evidence").unwrap();
    set_old_modified_time(&active);
    std::fs::write(&recent, b"recent evidence").unwrap();
    std::fs::create_dir(&nonregular).unwrap();

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    for name in stale {
        assert!(!dir.join(name).exists(), "{name}");
    }
    assert!(nested.is_file());
    assert!(malformed.is_file());
    assert!(active.is_file());
    assert!(recent.is_file());
    assert!(nonregular.is_dir());
    assert!(catalog
        .maintenance_warnings
        .iter()
        .any(|warning| warning.contains("Unknown private deletion artifact")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn private_cleanup_filters_before_its_budget_and_progresses_across_batches() {
    let dir = test_dir("private-deletion-cleanup-budget");
    for index in 0..256 {
        std::fs::write(dir.join(format!("unrelated-{index:03}.tmp")), b"keep").unwrap();
    }
    let leftovers = (1..=129)
        .map(|revision| {
            format!(".live-s-cleanup-budget.transcript.r{revision}.json.delete-999999-{revision}")
        })
        .collect::<Vec<_>>();
    for name in &leftovers {
        let path = dir.join(name);
        std::fs::write(&path, b"leftover").unwrap();
        set_old_modified_time(&path);
    }

    list_session_catalog_from_dir(&dir).unwrap();
    assert_eq!(
        leftovers
            .iter()
            .filter(|name| dir.join(name).exists())
            .count(),
        1
    );
    assert!(dir.join("unrelated-000.tmp").is_file());

    list_session_catalog_from_dir(&dir).unwrap();
    assert!(leftovers.iter().all(|name| !dir.join(name).exists()));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn private_cleanup_rotation_advances_past_a_failed_full_batch() {
    let names = (0..=MAX_PRIVATE_DELETION_LEFTOVERS)
        .map(|index| format!("candidate-{index:03}"))
        .collect::<Vec<_>>();

    let mut first = RotatingDeletionCandidates::new(None, MAX_PRIVATE_DELETION_LEFTOVERS);
    for name in &names {
        first.push(name.clone());
    }
    let (first_batch, _, cursor) = first.finish();
    assert_eq!(first_batch.len(), MAX_PRIVATE_DELETION_LEFTOVERS);
    assert!(!first_batch.contains(names.last().unwrap()));

    let mut second = RotatingDeletionCandidates::new(cursor, MAX_PRIVATE_DELETION_LEFTOVERS);
    for name in &names {
        second.push(name.clone());
    }
    let (second_batch, _, _) = second.finish();

    assert!(second_batch.contains(names.last().unwrap()));
}

#[test]
fn pending_intent_reconciliation_rotates_past_a_failed_full_batch() {
    let dir = test_dir("pending-intent-rotation");
    for index in 0..MAX_PRIVATE_DELETION_LEFTOVERS {
        let session = SessionId::new(format!("s-pending-intent-{index:03}")).unwrap();
        let audio = format!("live-{session}.wav");
        let intent = DeletionIntent {
            schema_version: DELETION_INTENT_SCHEMA_VERSION,
            session_id: session.clone(),
            reason: "manual".into(),
            commit_file: format!("live-{session}.commit.json"),
            commit_sha256: "0".repeat(64),
            commit_file_identity: None,
            artifacts: vec![DeletionArtifact {
                name: audio.clone(),
                sha256: "0".repeat(64),
                file_identity: None,
            }],
        };
        std::fs::write(dir.join(audio), b"retained evidence").unwrap();
        std::fs::write(
            dir.join(deletion_intent_name(&session)),
            format!("{}\n", serde_json::to_string(&intent).unwrap()),
        )
        .unwrap();
    }
    let session = SessionId::new("s-pending-intent-999").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_name = deletion_intent_name(&session);
    write_deletion_intent(&dir.join(&intent_name), &intent).unwrap();

    reconcile_pending_deletion_intents(&dir);
    assert!(dir.join(&intent_name).is_file());
    reconcile_pending_deletion_intents(&dir);

    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    assert!(!dir.join(intent_name).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn corrupt_intent_retries_remove_each_verified_quarantine() {
    let dir = test_dir("corrupt-intent-retry-cleanup");
    let session = SessionId::new("s-corrupt-intent-retry-cleanup").unwrap();
    let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    capture.append_pcm16(&[1, 0]).unwrap();
    capture.finalize().unwrap();
    let committed = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &committed, "manual").unwrap();
    let intent_path = dir.join(deletion_intent_name(&session));

    for _ in 0..3 {
        std::fs::write(&intent_path, b"{corrupt").unwrap();
        write_deletion_intent(&intent_path, &intent).unwrap();
        assert!(!std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("deletion.v1.json.delete-")));
        std::fs::remove_file(&intent_path).unwrap();
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn repeated_post_publication_failures_keep_one_intent_evidence_quarantine() {
    let dir = test_dir("corrupt-intent-post-publication-retries");
    let session = SessionId::new("s-corrupt-intent-post-publication-retries").unwrap();
    let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    capture.append_pcm16(&[1, 0]).unwrap();
    capture.finalize().unwrap();
    let committed = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &committed, "manual").unwrap();
    let intent_path = dir.join(deletion_intent_name(&session));

    for _ in 0..3 {
        std::fs::write(&intent_path, b"{corrupt").unwrap();
        let replacement = intent_path.clone();
        assert!(write_deletion_intent_with_publication_barrier(
            &intent_path,
            &intent,
            move |published| {
                if published {
                    std::fs::remove_file(&replacement).unwrap();
                    std::fs::write(&replacement, b"replacement intent").unwrap();
                }
            }
        )
        .is_err());
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .contains("deletion.v1.json.delete-")
                })
                .count(),
            1
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn missing_intent_restores_the_newest_verified_quarantine_before_replacement() {
    let dir = test_dir("missing-intent-quarantine-recovery");
    let session = SessionId::new("s-missing-intent-quarantine-recovery").unwrap();
    let intent_name = deletion_intent_name(&session);
    let older = format!(".{intent_name}.delete-999999-1");
    let newer = format!(".{intent_name}.delete-999999-2");
    std::fs::write(dir.join(&older), b"{older").unwrap();
    std::fs::write(dir.join(&newer), b"{newer").unwrap();
    set_old_modified_time(&dir.join(&older));
    set_old_modified_time(&dir.join(&newer));

    reconcile_intent_evidence_quarantines(&dir, &intent_name).unwrap();

    assert_eq!(std::fs::read(dir.join(&intent_name)).unwrap(), b"{newer");
    assert!(!dir.join(&older).exists());
    assert!(!dir.join(&newer).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn fresh_foreign_intent_quarantine_is_retained_during_reconciliation() {
    let dir = test_dir("fresh-foreign-intent-quarantine");
    let session = SessionId::new("s-fresh-foreign-intent-quarantine").unwrap();
    let intent_name = deletion_intent_name(&session);
    let quarantine = format!(".{intent_name}.delete-999999-0");
    std::fs::write(dir.join(&quarantine), b"foreign in-flight intent").unwrap();

    reconcile_intent_evidence_quarantines(&dir, &intent_name).unwrap();

    assert!(!dir.join(&intent_name).exists());
    assert!(dir.join(&quarantine).is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn current_pid_intent_quarantine_is_reconciled_during_catalog_maintenance() {
    let dir = test_dir("current-pid-intent-quarantine");
    let session = SessionId::new("s-current-pid-intent-quarantine").unwrap();
    let intent_name = deletion_intent_name(&session);
    let quarantine = format!(".{intent_name}.delete-{}-0", std::process::id());
    std::fs::write(dir.join(&quarantine), b"prior failed intent").unwrap();

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    assert!(dir.join(&intent_name).is_file());
    assert!(!dir.join(&quarantine).exists());
    assert!(catalog
        .maintenance_warnings
        .iter()
        .any(|warning| warning.contains("pending")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn damaged_commit_warnings_take_priority_when_maintenance_warning_cap_is_full() {
    let dir = test_dir("damaged-warning-priority");
    let session = SessionId::new("s-damaged-warning-priority").unwrap();
    let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    capture.append_pcm16(&[1, 0]).unwrap();
    capture.finalize().unwrap();
    std::fs::write(dir.join(format!("live-{session}.commit.json")), b"{damaged").unwrap();
    for index in 0..MAX_MAINTENANCE_WARNINGS {
        std::fs::write(
            dir.join(format!(".live-s-warning-{index}.deletion.v1.invalid.part")),
            b"evidence",
        )
        .unwrap();
    }

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    assert_eq!(catalog.maintenance_warnings.len(), MAX_MAINTENANCE_WARNINGS);
    assert!(catalog.maintenance_warnings[0].contains("Damaged live recording"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn truncated_final_intent_after_progress_is_retained_as_a_catalog_warning() {
    let dir = test_dir("truncated-intent-after-progress");
    let session = SessionId::new("s-truncated-intent-after-progress").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_name = deletion_intent_name(&session);
    write_deletion_intent(&dir.join(&intent_name), &intent).unwrap();
    let audio = &intent.artifacts[0];
    recording::remove_regular_artifact_if_hash(&dir, &audio.name, &audio.sha256).unwrap();
    std::fs::write(dir.join(&intent_name), b"{\"truncated\"").unwrap();

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    assert!(catalog.sessions.is_empty());
    assert!(catalog
        .maintenance_warnings
        .iter()
        .any(|warning| warning.contains("pending")));
    assert!(dir.join(&intent_name).is_file());
    assert!(dir.join(format!("live-{session}.capture.json")).is_file());
    std::fs::remove_dir_all(dir).ok();
}
