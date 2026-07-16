use super::*;

pub(super) fn create_new(path: &Path, label: &str) -> Result<File, String> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .read(true)
        .open(path)
        .map_err(|error| format!("Failed to create {label}: {error}"))
}

pub(crate) fn publish_no_replace(
    source: &Path,
    destination: &Path,
    owned_staging: &File,
    label: &str,
) -> Result<File, String> {
    publish_no_replace_with_after_link(source, destination, owned_staging, label, || {})
}

#[cfg(test)]
pub(crate) fn publish_no_replace_with_after_link_for_test<F>(
    source: &Path,
    destination: &Path,
    owned_staging: &File,
    label: &str,
    after_link: F,
) -> Result<File, String>
where
    F: FnOnce(),
{
    publish_no_replace_with_after_link(source, destination, owned_staging, label, after_link)
}

fn publish_no_replace_with_after_link<F>(
    source: &Path,
    destination: &Path,
    owned_staging: &File,
    label: &str,
    after_link: F,
) -> Result<File, String>
where
    F: FnOnce(),
{
    let opened_staging = open_regular_path(source)?;
    if !same_file_identity(owned_staging, &opened_staging)? {
        return Err(format!(
            "Refused to {label}: staging path no longer names the owned file"
        ));
    }
    drop(opened_staging);
    fs::hard_link(source, destination).map_err(|error| format!("Failed to {label}: {error}"))?;
    after_link();
    let destination_file = open_regular_path(destination)?;
    if !same_file_identity(owned_staging, &destination_file)? {
        return Err(format!(
            "Refused to {label}: published destination does not name the owned file"
        ));
    }
    remove_owned_staging(source, owned_staging, label);
    Ok(destination_file)
}

pub(crate) fn remove_owned_staging(source: &Path, owned_staging: &File, label: &str) {
    let cleanup_warning = match open_regular_path(source) {
        Ok(current_staging) => match same_file_identity(owned_staging, &current_staging) {
            Ok(true) => fs::remove_file(source)
                .err()
                .map(|error| format!("Published {label}, but staging cleanup is pending: {error}")),
            Ok(false) => Some(format!(
                "Published {label}, but staging cleanup is pending: staging path no longer names the owned file"
            )),
            Err(error) => Some(format!("Published {label}, but staging cleanup is pending: {error}")),
        },
        Err(error) => Some(format!("Published {label}, but staging cleanup is pending: {error}")),
    };
    if let Some(warning) = cleanup_warning {
        crate::stt::log_yap(&warning);
    }
}
