use super::*;

#[test]
fn catalog_reconciliation_cannot_resume_a_manual_deletion_after_publication() {
    let dir = test_dir("manual-deletion-catalog-ownership");
    let session = SessionId::new("s-manual-deletion-catalog-ownership").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent_name = deletion_intent_name(&session);

    let (published_tx, published_rx) = std::sync::mpsc::channel();
    let (release_manual_tx, release_manual_rx) = std::sync::mpsc::channel();
    let manual_dir = dir.clone();
    let manual = std::thread::spawn(move || {
        delete_committed_session_in_dir_with_publication_barrier(
            &manual_dir,
            &capture,
            "manual",
            move |published| {
                if published {
                    published_tx.send(()).unwrap();
                    release_manual_rx.recv().unwrap();
                }
            },
        )
    });
    published_rx.recv().unwrap();
    assert!(dir.join(&intent_name).is_file());

    let catalog_dir = dir.clone();
    let (catalog_started_tx, catalog_started_rx) = std::sync::mpsc::channel();
    let (catalog_finished_tx, catalog_finished_rx) = std::sync::mpsc::channel();
    let catalog = std::thread::spawn(move || {
        catalog_started_tx.send(()).unwrap();
        catalog_finished_tx
            .send(list_session_catalog_from_dir(&catalog_dir))
            .unwrap();
    });
    catalog_started_rx.recv().unwrap();

    assert!(catalog_finished_rx
        .recv_timeout(Duration::from_millis(100))
        .is_err());
    release_manual_tx.send(()).unwrap();
    assert!(manual.join().unwrap().is_ok());
    assert!(catalog_finished_rx.recv().unwrap().is_ok());
    catalog.join().unwrap();
    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    assert!(!dir.join(intent_name).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn deletion_intent_publication_rejects_injected_pre_and_post_publication_replacements() {
    let dir = test_dir("intent-publication-barriers");
    let session = SessionId::new("s-intent-publication-barriers").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_path = dir.join(deletion_intent_name(&session));

    let before = intent_path.clone();
    assert!(
        write_deletion_intent_with_publication_barrier(&intent_path, &intent, move |after| {
            if !after {
                std::fs::write(&before, b"competing intent").unwrap();
            }
        })
        .is_err()
    );
    assert_eq!(std::fs::read(&intent_path).unwrap(), b"competing intent");
    std::fs::remove_file(&intent_path).unwrap();

    let after = intent_path.clone();
    assert!(write_deletion_intent_with_publication_barrier(
        &intent_path,
        &intent,
        move |published| {
            if published {
                std::fs::remove_file(&after).unwrap();
                std::fs::write(&after, b"replacement intent").unwrap();
            }
        }
    )
    .is_err());
    assert_eq!(std::fs::read(&intent_path).unwrap(), b"replacement intent");
    assert!(dir.join(format!("live-{session}.commit.json")).is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn concurrent_intent_replacements_do_not_reconcile_an_active_quarantine() {
    let dir = test_dir("concurrent-intent-replacements");
    let session = SessionId::new("s-concurrent-intent-replacements").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_path = dir.join(deletion_intent_name(&session));
    std::fs::write(&intent_path, b"{corrupt").unwrap();

    let (first_quarantined_tx, first_quarantined_rx) = std::sync::mpsc::channel();
    let (release_first_tx, release_first_rx) = std::sync::mpsc::channel();
    let first_path = intent_path.clone();
    let first_intent = intent.clone();
    let first = std::thread::spawn(move || {
        write_deletion_intent_with_publication_barrier(
            &first_path,
            &first_intent,
            move |published| {
                if !published {
                    first_quarantined_tx.send(()).unwrap();
                    release_first_rx.recv().unwrap();
                }
            },
        )
    });

    first_quarantined_rx.recv().unwrap();
    assert!(!intent_path.exists());
    assert_eq!(intent_quarantine_count(&dir), 1);

    let (contender_started_tx, contender_started_rx) = std::sync::mpsc::channel();
    let (contender_finished_tx, contender_finished_rx) = std::sync::mpsc::channel();
    let contender_path = intent_path.clone();
    let contender_intent = intent.clone();
    let contender = std::thread::spawn(move || {
        contender_started_tx.send(()).unwrap();
        let result = write_deletion_intent(&contender_path, &contender_intent);
        contender_finished_tx.send(result).unwrap();
    });

    contender_started_rx.recv().unwrap();
    assert!(!intent_path.exists());
    assert_eq!(intent_quarantine_count(&dir), 1);

    release_first_tx.send(()).unwrap();
    assert!(first.join().unwrap().is_ok());
    assert!(contender_finished_rx.recv().unwrap().is_ok());
    contender.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(&intent_path).unwrap(),
        serde_json::to_string(&intent).unwrap() + "\n"
    );
    assert_eq!(intent_quarantine_count(&dir), 0);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn forged_deletion_intent_cannot_name_another_session_or_arbitrary_file() {
    let dir = test_dir("forged-deletion-intent");
    let session = SessionId::new("s-forged-intent").unwrap();
    let arbitrary = dir.join("keep-me.txt");
    std::fs::write(&arbitrary, "keep").unwrap();
    let intent_name = deletion_intent_name(&session);
    let forged = DeletionIntent {
        schema_version: DELETION_INTENT_SCHEMA_VERSION,
        session_id: session,
        reason: "manual".into(),
        commit_file: "live-s-forged-intent.commit.json".into(),
        commit_sha256: "0".repeat(64),
        commit_file_identity: None,
        artifacts: vec![
            DeletionArtifact {
                name: "live-s-forged-intent.wav".into(),
                sha256: "0".repeat(64),
                file_identity: None,
            },
            DeletionArtifact {
                name: "live-s-forged-intent.capture.json".into(),
                sha256: "0".repeat(64),
                file_identity: None,
            },
            DeletionArtifact {
                name: "keep-me.txt".into(),
                sha256: recording::sha256_file(&arbitrary).unwrap(),
                file_identity: None,
            },
        ],
    };
    std::fs::write(
        dir.join(&intent_name),
        format!("{}\n", serde_json::to_string(&forged).unwrap()),
    )
    .unwrap();

    let warnings = reconcile_pending_deletion_intents(&dir);

    assert!(arbitrary.is_file());
    assert!(dir.join(intent_name).is_file());
    assert!(warnings.session_warnings.contains_key("s-forged-intent"));
    std::fs::remove_dir_all(dir).ok();
}
