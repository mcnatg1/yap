use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use crate::{
    audio::session::{OwnerNamespace, SessionId},
    paths,
};

#[allow(dead_code)]
const INSTALL_ID_FILE: &str = "install-id";

#[allow(dead_code)]
pub(crate) fn load_or_create() -> Result<OwnerNamespace, String> {
    load_or_create_at(&paths::app_data_dir())
}

#[allow(dead_code)]
pub(crate) fn load_or_create_at(directory: &Path) -> Result<OwnerNamespace, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("failed to create install identity directory: {error}"))?;
    let path = directory.join(INSTALL_ID_FILE);

    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            let generated = SessionId::generate()?;
            let install_id = format!("i-{}", &generated.as_str()[2..]);
            file.write_all(install_id.as_bytes())
                .map_err(|error| format!("failed to write install identity: {error}"))?;
            file.flush()
                .map_err(|error| format!("failed to flush install identity: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("failed to sync install identity: {error}"))?;
            OwnerNamespace::local(install_id)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_existing(&path),
        Err(error) => Err(format!("failed to create install identity: {error}")),
    }
}

#[allow(dead_code)]
fn read_existing(path: &Path) -> Result<OwnerNamespace, String> {
    let install_id = fs::read_to_string(path)
        .map_err(|error| format!("failed to read install identity: {error}"))?;
    OwnerNamespace::local(&install_id)
        .map_err(|error| format!("invalid install identity at {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::load_or_create_at;

    #[test]
    fn install_identity_is_stable_across_reopen_and_never_silently_rotates() {
        let directory =
            std::env::temp_dir().join(format!("yap-install-identity-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);

        let first = load_or_create_at(&directory).unwrap();
        let reopened = load_or_create_at(&directory).unwrap();
        assert_eq!(first, reopened);

        std::fs::write(directory.join("install-id"), "invalid/value").unwrap();
        assert!(load_or_create_at(&directory).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }
}
