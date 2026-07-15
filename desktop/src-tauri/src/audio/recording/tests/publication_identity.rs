use super::*;

#[test]
fn partial_lineage_uses_the_owned_partial_receipt_not_a_colliding_complete_sidecar() {
    let dir = tempfile_dir("partial-receipt-collision");
    let session = SessionId::new("s-partial-receipt-collision").unwrap();
    let paths = RecordingPaths::new(&dir, session.clone());
    fs::write(&paths.sidecar, b"attacker complete sidecar").unwrap();
    let mut recording = StreamingRecording::create(&dir, session).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();

    let result = recording.finalize().unwrap();

    let lineage = result.partial_lineage.expect("owned partial receipt");
    assert_eq!(
        lineage.capture_sidecar_file,
        paths.partial_sidecar_file_name()
    );
    assert_ne!(
        lineage.capture_sidecar_sha256,
        sha256_file(&paths.sidecar).unwrap()
    );
    assert_eq!(
        fs::read(&paths.sidecar).unwrap(),
        b"attacker complete sidecar"
    );
}

#[test]
fn publication_replacement_barriers_fail_closed_without_deleting_unowned_artifacts() {
    for artifact in [
        PublicationArtifact::CompleteSidecar,
        PublicationArtifact::PartialSidecar,
        PublicationArtifact::Commit,
    ] {
        for barrier in [
            PublicationBarrier::BeforeHardLink,
            PublicationBarrier::AfterHardLink,
        ] {
            let dir = tempfile_dir(&format!("publication-{artifact:?}-{barrier:?}"));
            let session = SessionId::new("s-publication-replacement").unwrap();
            let unowned_path = Arc::new(Mutex::new(None));
            let hook_path = Arc::clone(&unowned_path);
            let mut recording = StreamingRecording::create_with_publication_hook(
                &dir,
                session.clone(),
                if artifact == PublicationArtifact::PartialSidecar {
                    Some(CommitFaultPoint::AudioSync)
                } else {
                    None
                },
                move |published, reached, paths| {
                    if published != artifact || reached != barrier {
                        return;
                    }
                    let target = paths.path_for_publication(artifact, barrier);
                    let displaced =
                        target.with_extension(format!("displaced-{:?}-{:?}", artifact, barrier));
                    fs::rename(&target, &displaced).unwrap();
                    fs::write(&target, b"unowned replacement").unwrap();
                    *hook_path.lock().unwrap() = Some(target);
                },
            )
            .unwrap();
            recording.append_pcm16(&[1, 0]).unwrap();

            let result = recording.finalize().unwrap();
            let replacement = unowned_path
                .lock()
                .unwrap()
                .clone()
                .expect("replacement barrier ran");

            assert_eq!(
                result.status,
                CaptureStatus::Partial,
                "{artifact:?} {barrier:?}"
            );
            assert!(result.committed.is_none(), "{artifact:?} {barrier:?}");
            assert_eq!(fs::read(replacement).unwrap(), b"unowned replacement");
            assert!(scan_recordings(&dir).unwrap().complete.is_empty());
        }
    }
}
