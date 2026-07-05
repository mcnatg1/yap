use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use desktop_lib::stt::crispasr::CrispasrBackend;
use desktop_lib::stt::parity::parse_verbose_json_has_timestamps;
use desktop_lib::stt::sidecar::CrispasrSidecar;

const MOCK_VERBOSE_JSON: &str = r#"{
  "text": "hello from the parity contract",
  "segments": [
    { "start": 0.0, "end": 1.2, "text": "hello from the parity contract" }
  ],
  "words": [
    { "word": "hello", "start": 0.0, "end": 0.3 }
  ]
}"#;

fn parity_clip(test_name: &str) -> Option<PathBuf> {
    match std::env::var("YAP_PARITY_CLIP").ok().map(PathBuf::from) {
        Some(path) => Some(path),
        None => {
            eprintln!(
                "skipping {test_name}: set YAP_PARITY_CLIP to run the real audio sidecar probe"
            );
            None
        }
    }
}

#[test]
fn crispasr_mock_verbose_json_contract_carries_timestamps() {
    assert!(
        parse_verbose_json_has_timestamps(MOCK_VERBOSE_JSON),
        "mock verbose_json contract must include segment or word timestamps"
    );
}

#[test]
fn crispasr_transcribes_parity_clip() {
    let Some(clip) = parity_clip("crispasr_transcribes_parity_clip") else {
        return;
    };

    let sidecar = Arc::new(Mutex::new(CrispasrSidecar::new()));
    let crispasr = CrispasrBackend::new(sidecar);
    let crispasr_text = crispasr
        .transcribe_with_progress(&clip, "en", None)
        .expect("crispasr transcription");
    assert!(!crispasr_text.trim().is_empty());
}

#[test]
fn crispasr_verbose_json_carries_timestamps() {
    let Some(clip) = parity_clip("crispasr_verbose_json_carries_timestamps") else {
        return;
    };

    let sidecar = Arc::new(Mutex::new(CrispasrSidecar::new()));
    let endpoint = sidecar
        .lock()
        .unwrap()
        .ensure_ready()
        .expect("sidecar ready");

    let client = reqwest::blocking::Client::new();
    let form = reqwest::blocking::multipart::Form::new()
        .file("file", &clip)
        .expect("clip file")
        .text("language", "en")
        .text("response_format", "verbose_json");
    let body = client
        .post(format!("{}/v1/audio/transcriptions", endpoint.url))
        .bearer_auth(&endpoint.api_key)
        .multipart(form)
        .send()
        .expect("verbose_json request")
        .text()
        .expect("verbose_json body");

    assert!(
        parse_verbose_json_has_timestamps(&body),
        "verbose_json response lacked segment/word timing: {body}"
    );
    sidecar.lock().unwrap().shutdown();
}
