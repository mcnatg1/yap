use std::io::Write;

pub(crate) fn write(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing file name")
        })?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    std::fs::remove_file(path.with_file_name(format!("{file_name}.part"))).ok();
    for attempt in 0..32 {
        let tmp = path.with_file_name(format!("{file_name}.{pid}.{nonce}.{attempt}.part"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(mut file) => {
                let write_result = file.write_all(text.as_bytes());
                drop(file);
                let result = write_result.and_then(|_| std::fs::rename(&tmp, path));
                if result.is_err() {
                    std::fs::remove_file(&tmp).ok();
                }
                return result;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not reserve temporary transcript path",
    ))
}
