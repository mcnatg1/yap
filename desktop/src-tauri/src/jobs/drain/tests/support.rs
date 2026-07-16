use super::*;

pub(super) fn queued_job(job_id: &str, source: std::path::PathBuf) -> NewRecordingJob {
    NewRecordingJob {
        job_id: job_id.into(),
        session_mode: SessionMode::Meeting,
        session_origin: SessionOrigin::ImportedFile,
        source_path: Some(source),
        source_ownership: SourceOwnership::External,
        output_path: None,
        display_name: "source.wav".into(),
        status: RecordingJobStatus::QueuedServer,
        route: Some(RecordingRoute::ServerBatch),
        attempt_count: 0,
        next_attempt_at_ms: None,
        cancellation_requested: false,
        capture_commit_path: None,
        capture_manifest_sha256: None,
        error_code: None,
        error_message: None,
        created_at_ms: 1_720_000_000_000,
        updated_at_ms: 1_720_000_000_000,
        expires_at_ms: Some(1_720_604_800_000),
    }
}

pub(super) fn start_json_server(
    responses: Vec<(u16, serde_json::Value)>,
) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let server_observed = Arc::clone(&observed);
    let server = thread::spawn(move || {
        for (status, response) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let expected = loop {
                let read = stream.read(&mut buffer).unwrap();
                assert_ne!(read, 0, "request ended before headers");
                request.extend_from_slice(&buffer[..read]);
                if let Some(split) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..split]);
                    let content_length = head
                        .lines()
                        .find_map(|line| {
                            line.split_once(':').and_then(|(name, value)| {
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                        })
                        .unwrap_or(0);
                    break split + 4 + content_length;
                }
            };
            while request.len() < expected {
                let read = stream.read(&mut buffer).unwrap();
                assert_ne!(read, 0, "request body ended early");
                request.extend_from_slice(&buffer[..read]);
            }
            server_observed
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&request[..expected]).into_owned());
            let body = serde_json::to_vec(&response).unwrap();
            let reason = match status {
                200 => "OK",
                201 => "Created",
                202 => "Accepted",
                _ => "Error",
            };
            write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        }
    });
    (format!("http://{address}"), observed, server)
}

pub(super) fn write_pcm_wav(path: &std::path::Path, pcm: &[u8]) {
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

pub(super) fn temp_dir(label: &str) -> std::path::PathBuf {
    let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "yap-phase5-drain-{label}-{}-{nonce}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}
