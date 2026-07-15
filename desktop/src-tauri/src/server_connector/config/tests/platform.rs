#[cfg(windows)]
use super::super::platform::windows_move_flags;

#[cfg(windows)]
#[test]
fn publication_uses_only_supported_move_replace_and_write_through_flags() {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};

    assert_eq!(
        windows_move_flags().0,
        (MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH).0
    );
}
