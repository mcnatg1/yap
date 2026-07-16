use super::*;

impl StreamingRecording {
    pub(super) fn revalidate_sidecar_receipt(&self) -> Result<(), String> {
        self.sidecar_receipt
            .as_ref()
            .ok_or_else(|| "capture sidecar receipt is unavailable".to_string())?
            .revalidate()
    }

    pub(super) fn hit_fault(&self, point: CommitFaultPoint) -> Result<(), String> {
        #[cfg(test)]
        if self.fault == Some(point) {
            return Err(format!("injected recording fault at {point:?}"));
        }
        let _ = point;
        Ok(())
    }

    pub(super) fn persist_journal(&mut self) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if self.journal_growth_stopped {
            return self.fail("recording journal durability is unavailable".into());
        }
        let record = JournalRecord::Delta {
            delta: self.journal_durable.delta(&self.journal),
        };
        let bytes = match serialize_journal_record(&record) {
            Ok(bytes) => bytes,
            Err(error) => return self.fail(error),
        };
        if bytes.len() as u64 > MAX_JOURNAL_RECORD_BYTES
            || self
                .journal_bytes
                .saturating_add(bytes.len() as u64)
                .saturating_add(MAX_JOURNAL_TERMINAL_BYTES)
                > MAX_JOURNAL_BYTES
        {
            return self.stop_journal_growth("journal size limit reached");
        }
        #[cfg(test)]
        if let Some(attempts) = &self.journal_write_attempts {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if let Err(error) = self.hit_fault(CommitFaultPoint::JournalAppend) {
            return self.fail(error);
        }
        let write_result = self
            .journal_file
            .as_mut()
            .ok_or_else(|| "recording journal handle is unavailable".to_string())
            .and_then(|file| {
                file.write_all(&bytes)
                    .map_err(|error| format!("Failed to append recording journal: {error}"))
            });
        if let Err(error) = write_result {
            return self.fail(error);
        }
        if let Err(error) = self.hit_fault(CommitFaultPoint::JournalSync) {
            return self.fail(error);
        }
        let sync_result = self
            .journal_file
            .as_mut()
            .expect("recording journal was checked before sync")
            .sync_data()
            .map_err(|error| format!("Failed to sync recording journal: {error}"));
        if let Err(error) = sync_result {
            return self.fail(error);
        }
        self.journal_bytes = self.journal_bytes.saturating_add(bytes.len() as u64);
        self.journal_durable = DurableJournalState::from_journal(&self.journal);
        Ok(())
    }

    pub(super) fn stop_journal_growth(&mut self, reason: &str) -> Result<(), String> {
        self.journal.sink_degraded = true;
        if !self.journal_terminal_written {
            let bytes = match serialize_journal_record(&JournalRecord::Overflow {
                session_id: self.paths.session_id.clone(),
                reason: reason.to_string(),
            }) {
                Ok(bytes) => bytes,
                Err(error) => return self.fail(error),
            };
            if self.journal_bytes.saturating_add(bytes.len() as u64) <= MAX_JOURNAL_BYTES {
                #[cfg(test)]
                if let Some(attempts) = &self.journal_write_attempts {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                if let Err(error) = self.hit_fault(CommitFaultPoint::JournalAppend) {
                    return self.fail(error);
                }
                let write_result = self
                    .journal_file
                    .as_mut()
                    .ok_or_else(|| "recording journal handle is unavailable".to_string())
                    .and_then(|file| {
                        file.write_all(&bytes).map_err(|error| {
                            format!("Failed to append recording journal overflow: {error}")
                        })
                    });
                if let Err(error) = write_result {
                    return self.fail(error);
                }
                if let Err(error) = self.hit_fault(CommitFaultPoint::JournalSync) {
                    return self.fail(error);
                }
                let sync_result = self
                    .journal_file
                    .as_mut()
                    .expect("recording journal was checked before overflow sync")
                    .sync_data()
                    .map_err(|error| format!("Failed to sync recording journal overflow: {error}"));
                if let Err(error) = sync_result {
                    return self.fail(error);
                }
                self.journal_bytes = self.journal_bytes.saturating_add(bytes.len() as u64);
                self.journal_terminal_written = true;
            }
        }
        self.journal_growth_stopped = true;
        self.fail(format!("recording journal durability stopped: {reason}"))
    }

    pub(super) fn publish_owned(
        &mut self,
        source: &Path,
        destination: &Path,
        owned_staging: &File,
        label: &str,
        artifact: PublicationArtifact,
        cleanup_fault: CommitFaultPoint,
    ) -> Result<File, String> {
        self.publication_barrier(artifact, PublicationBarrier::BeforeHardLink);
        let opened_staging = open_regular_path(source)?;
        if !same_file_identity(owned_staging, &opened_staging)? {
            return Err(format!(
                "Refused to {label}: staging path no longer names the owned file"
            ));
        }
        drop(opened_staging);

        fs::hard_link(source, destination)
            .map_err(|error| format!("Failed to {label}: {error}"))?;
        self.publication_barrier(artifact, PublicationBarrier::AfterHardLink);

        let destination_file = open_regular_path(destination)?;
        if !same_file_identity(owned_staging, &destination_file)? {
            return Err(format!(
                "Refused to {label}: published destination does not name the owned file"
            ));
        }

        let cleanup_warning = match open_regular_path(source) {
            Ok(current_staging) if same_file_identity(owned_staging, &current_staging)? => {
                #[cfg(test)]
                if self.fault == Some(cleanup_fault) {
                    Some(format!(
                        "Published {label}, but staging cleanup is pending: injected post-link cleanup failure at {cleanup_fault:?}"
                    ))
                } else {
                    fs::remove_file(source)
                        .err()
                        .map(|error| format!("Published {label}, but staging cleanup is pending: {error}"))
                }
                #[cfg(not(test))]
                {
                    let _ = cleanup_fault;
                    fs::remove_file(source)
                        .err()
                        .map(|error| format!("Published {label}, but staging cleanup is pending: {error}"))
                }
            }
            Ok(_) => Some(format!(
                "Published {label}, but staging cleanup is pending: staging path no longer names the owned file"
            )),
            Err(error) => Some(format!(
                "Published {label}, but staging cleanup is pending: {error}"
            )),
        };
        if let Some(warning) = cleanup_warning {
            crate::stt::log_yap(&warning);
        }
        Ok(destination_file)
    }

    pub(super) fn publication_barrier(
        &mut self,
        artifact: PublicationArtifact,
        barrier: PublicationBarrier,
    ) {
        #[cfg(test)]
        if let Some(mut hook) = self.publication_hook.take() {
            hook(artifact, barrier, &self.paths);
            self.publication_hook = Some(hook);
        }
        let _ = (artifact, barrier);
    }

    #[cfg(test)]
    pub(super) fn set_data_limit_for_test(&mut self, data_limit: u64) {
        self.data_limit = data_limit;
    }

    #[cfg(test)]
    pub(super) fn journal_for_test(&self) -> &CaptureJournal {
        &self.journal
    }

    #[cfg(test)]
    pub(crate) fn journal_path_for_test(&self) -> &Path {
        &self.paths.journal_part
    }

    #[cfg(test)]
    pub(super) fn persist_journal_for_test(&mut self) -> Result<(), String> {
        self.persist_journal()
    }

    #[cfg(test)]
    pub(super) fn journal_growth_stopped_for_test(&self) -> bool {
        self.journal_growth_stopped
    }
}
