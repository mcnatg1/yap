use super::*;

#[test]
fn polished_path_writes_sibling_file() {
    let path = polished_path(std::path::Path::new("C:/recordings/take.txt")).unwrap();
    assert_eq!(path.file_name().unwrap(), "take.polished.txt");
}

#[test]
fn atomic_text_write_replaces_stale_partial_file() {
    let dir = temp_test_dir("atomic-polish-write");
    let output = dir.join("take.polished.txt");
    let partial = dir.join("take.polished.txt.part");
    std::fs::write(&partial, "stale").unwrap();

    write_text_atomically(&output, "polished").unwrap();

    assert_eq!(std::fs::read_to_string(&output).unwrap(), "polished");
    assert!(!partial.exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn atomic_text_write_replaces_existing_output() {
    let dir = temp_test_dir("atomic-polish-overwrite");
    let output = dir.join("take.polished.txt");
    std::fs::write(&output, "old").unwrap();

    write_text_atomically(&output, "new").unwrap();

    assert_eq!(std::fs::read_to_string(&output).unwrap(), "new");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn atomic_text_write_uses_unique_temps_for_concurrent_writes() {
    let dir = temp_test_dir("atomic-polish-concurrent");
    let output = dir.join("take.polished.txt");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let left_output = output.clone();
    let left_barrier = std::sync::Arc::clone(&barrier);
    let left = std::thread::spawn(move || {
        left_barrier.wait();
        write_text_atomically(&left_output, "left")
    });
    let right_output = output.clone();
    let right_barrier = std::sync::Arc::clone(&barrier);
    let right = std::thread::spawn(move || {
        right_barrier.wait();
        write_text_atomically(&right_output, "right")
    });

    left.join().unwrap().unwrap();
    right.join().unwrap().unwrap();

    let text = std::fs::read_to_string(&output).unwrap();
    assert!(text == "left" || text == "right");
    let leftovers = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "part")
        })
        .count();
    assert_eq!(leftovers, 0);
    std::fs::remove_dir_all(dir).ok();
}
