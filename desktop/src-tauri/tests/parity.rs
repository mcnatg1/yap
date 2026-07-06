use std::sync::{Arc, Mutex};

use yap_desktop_lib::stt::crispasr::CrispasrBackend;
use yap_desktop_lib::stt::parity::parse_verbose_json_has_timestamps;
use yap_desktop_lib::stt::sidecar::CrispasrSidecar;

const MOCK_VERBOSE_JSON: &str = include_str!("fixtures/parity-contract.verbose.json");

fn parity_clip() -> std::path::PathBuf {
    std::env::var("YAP_PARITY_CLIP")
        .map(std::path::PathBuf::from)
        .expect("set YAP_PARITY_CLIP to run the real audio sidecar probe")
}

#[test]
fn crispasr_mock_verbose_json_contract_carries_timestamps() {
    assert!(
        parse_verbose_json_has_timestamps(MOCK_VERBOSE_JSON),
        "mock verbose_json contract must include segment or word timestamps"
    );
}

#[test]
#[ignore = "requires YAP_PARITY_CLIP and the local CrispASR sidecar"]
fn crispasr_transcribes_parity_clip() {
    let clip = parity_clip();

    let sidecar = Arc::new(Mutex::new(CrispasrSidecar::new()));
    let crispasr = CrispasrBackend::new(sidecar);
    let crispasr_text = crispasr
        .transcribe_with_progress(&clip, "en", None)
        .expect("crispasr transcription");
    assert!(!crispasr_text.trim().is_empty());
}

#[test]
#[ignore = "requires YAP_PARITY_CLIP and the local CrispASR sidecar"]
fn crispasr_verbose_json_carries_timestamps() {
    let clip = parity_clip();

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
