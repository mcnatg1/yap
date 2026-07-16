use super::super::persisted_file::read_persisted_bytes;
#[cfg(windows)]
use super::super::platform::{
    windows_file_attributes_are_regular, windows_move_flags, windows_persisted_file_open_flags,
    windows_settings_lock_open_flags,
};
use super::*;

#[cfg(windows)]
#[test]
fn publication_uses_only_supported_move_replace_and_write_through_flags() {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};

    assert_eq!(
        windows_move_flags().0,
        (MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH).0
    );
}

#[cfg(windows)]
#[test]
fn persisted_reader_uses_no_follow_flags_and_rejects_reparse_attributes() {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    assert_eq!(
        windows_persisted_file_open_flags() & FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_FLAG_OPEN_REPARSE_POINT
    );
    assert_eq!(
        windows_settings_lock_open_flags() & FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_FLAG_OPEN_REPARSE_POINT
    );
    assert!(windows_file_attributes_are_regular(0));
    assert!(!windows_file_attributes_are_regular(
        FILE_ATTRIBUTE_DIRECTORY
    ));
    assert!(!windows_file_attributes_are_regular(
        FILE_ATTRIBUTE_REPARSE_POINT
    ));
}

#[test]
fn persisted_reader_rejects_a_link_at_open_time() {
    let dir = temp_dir("persisted-reader-link");
    let outside = dir.join("outside-settings.json");
    let path = dir.join("server-settings.json");
    std::fs::write(&outside, b"outside settings").unwrap();
    if let Err(error) = create_file_symlink(&outside, &path) {
        if test_symlink_is_unavailable(&error) {
            std::fs::remove_dir_all(dir).ok();
            return;
        }
        panic!("could not create test symlink: {error}");
    }

    let error = read_persisted_bytes(&path).unwrap_err();

    assert!(error.to_string().contains("regular file"), "{error}");
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside settings");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn settings_link_is_rejected_without_reading_its_target() {
    let dir = temp_dir("settings-link-load");
    let outside = dir.join("outside-settings.json");
    let path = dir.join("server-settings.json");
    let target = r#"{"schemaVersion":1,"enabled":false,"baseUrl":null}"#;
    std::fs::write(&outside, target).unwrap();
    if let Err(error) = create_file_symlink(&outside, &path) {
        if test_symlink_is_unavailable(&error) {
            std::fs::remove_dir_all(dir).ok();
            return;
        }
        panic!("could not create test symlink: {error}");
    }

    let error = load_from_path(&path, false).unwrap_err();

    assert!(error.to_string().contains("regular file"), "{error}");
    assert_eq!(std::fs::read_to_string(outside).unwrap(), target);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn save_does_not_replace_a_settings_link_or_modify_its_target() {
    let dir = temp_dir("settings-link-save");
    let outside = dir.join("outside-settings.json");
    let path = dir.join("server-settings.json");
    let target = r#"{"schemaVersion":1,"enabled":false,"baseUrl":null}"#;
    std::fs::write(&outside, target).unwrap();
    if let Err(error) = create_file_symlink(&outside, &path) {
        if test_symlink_is_unavailable(&error) {
            std::fs::remove_dir_all(dir).ok();
            return;
        }
        panic!("could not create test symlink: {error}");
    }

    let error = save_to_path(&ServerSettings::default(), &path, false).unwrap_err();

    assert!(error.to_string().contains("regular file"), "{error}");
    assert!(std::fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read_to_string(outside).unwrap(), target);
    assert_eq!(partial_files(&dir), Vec::<String>::new());
    assert_eq!(recovery_files(&dir), Vec::<PathBuf>::new());
    std::fs::remove_dir_all(dir).ok();
}
