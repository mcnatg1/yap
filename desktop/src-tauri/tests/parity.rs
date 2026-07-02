use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use desktop_lib::stt::backend::SttBackend;
use desktop_lib::stt::crispasr::CrispasrBackend;
use desktop_lib::stt::parity::{parse_verbose_json_has_timestamps, word_error_rate};
use desktop_lib::stt::python::PythonBackend;
use desktop_lib::stt::sidecar::CrispasrSidecar;

fn parity_clip() -> Option<PathBuf> {
    std::env::var("YAP_PARITY_CLIP").ok().map(PathBuf::from)
}

fn parity_root() -> PathBuf {
    std::env::var("YAP_PARITY_ROOT").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("../.."))
}

#[test]
fn crispasr_matches_python_within_wer_tolerance() {
    let Some(clip) = parity_clip() else {
        eprintln!("skipping parity: set YAP_PARITY_CLIP to a known audio clip");
        return;
    };

    let python = PythonBackend::new(parity_root());
    let python_text = python.transcribe(&clip, "en").expect("python transcription");

    let sidecar = Arc::new(Mutex::new(CrispasrSidecar::new()));
    let crispasr = CrispasrBackend::new(sidecar);
    let crispasr_text = crispasr.transcribe(&clip, "en").expect("crispasr transcription");

    let wer = word_error_rate(&python_text, &crispasr_text);
    println!("parity WER = {wer:.3}");
    assert!(wer <= 0.20, "WER {wer:.3} exceeds the 0.20 tolerance");
}

#[test]
fn crispasr_verbose_json_carries_timestamps() {
    let Some(clip) = parity_clip() else {
        eprintln!("skipping verbose_json probe: set YAP_PARITY_CLIP");
        return;
    };

    let sidecar = Arc::new(Mutex::new(CrispasrSidecar::new()));
    let endpoint = sidecar.lock().unwrap().ensure_ready().expect("sidecar ready");

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
