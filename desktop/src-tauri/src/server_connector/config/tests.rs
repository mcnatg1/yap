use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use super::*;

fn temp_dir(name: &str) -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "yap-server-settings-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
fn create_file_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(windows)]
fn create_file_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(source, destination)
}

fn test_symlink_is_unavailable(error: &std::io::Error) -> bool {
    cfg!(windows)
        && (error.kind() == std::io::ErrorKind::PermissionDenied
            || error.raw_os_error() == Some(1314))
}

fn partial_files(dir: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("server-settings.json") && name.ends_with(".part"))
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn recovery_files(dir: &Path) -> Vec<PathBuf> {
    let mut paths = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|name| {
                name.starts_with("server-settings.json.recovery.") && name.ends_with(".json")
            })
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn assert_one_recovery_with_contents(dir: &Path, expected: &[u8]) {
    let recovery = recovery_files(dir);
    assert_eq!(recovery.len(), 1, "expected one recovery artifact");
    assert_eq!(std::fs::read(&recovery[0]).unwrap(), expected);
}

const CROSS_PROCESS_CHILD_PATH: &str = "YAP_TEST_SETTINGS_CHILD_PATH";
const CROSS_PROCESS_READY_PATH: &str = "YAP_TEST_SETTINGS_READY_PATH";
const CROSS_PROCESS_RELEASE_PATH: &str = "YAP_TEST_SETTINGS_RELEASE_PATH";
const CROSS_PROCESS_FUTURE: &str = r#"{
  "schemaVersion": 2,
  "enabled": true,
  "baseUrl": "https://future-process.example",
  "futureField": "preserve-cross-process"
}"#;

fn wait_for_path(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn wait_for_child(mut child: std::process::Child, timeout: Duration) -> std::process::Output {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().unwrap() {
            Some(_) => return child.wait_with_output().unwrap(),
            None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            None => {
                child.kill().ok();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "settings child exceeded {:?}: stdout={} stderr={}",
                    timeout,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

mod bounds;
mod locking;
mod platform;
mod policy;
mod publication;
