use super::*;

impl StreamingRecording {
    fn append_prepared(&mut self, frame: &PreparedFrame) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if frame.metadata.session_id != self.journal.session_id {
            return self.fail("recording prepared frame session does not match".into());
        }
        if frame.metadata.sequence != 0
            && !self
                .journal
                .sequence_coverage
                .iter()
                .any(|coverage| coverage.track_id == frame.metadata.track_id.as_str())
        {
            return self.fail("recording track sequence must start at zero".into());
        }
        self.observe_frame_metadata(
            frame.metadata.track_id.as_str(),
            frame.metadata.sample_rate_hz,
            frame.metadata.channels,
            frame.metadata.sequence,
            frame.metadata.start_ms,
            frame.metadata.duration_ms,
        );
        self.write_pcm16(&f32_to_i16_le_bytes(&frame.samples))
    }

    pub(super) fn append_input(&mut self, input: RecordingInput) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        match input {
            RecordingInput::PreparedFrame(frame) => self.append_prepared(&frame),
            RecordingInput::RevisionTransition(transition) => {
                self.journal.observe_revision_transition(transition)?;
                self.persist_journal()
            }
            RecordingInput::Gap(gap) => {
                self.journal.observe_gap(gap)?;
                self.persist_journal()
            }
        }
    }

    fn write_pcm16(&mut self, pcm: &[u8]) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if !pcm.len().is_multiple_of(2) {
            return self.fail("PCM16 append has an odd byte length".into());
        }
        let added =
            u64::try_from(pcm.len()).map_err(|_| "PCM16 append is too large".to_string())?;
        if self
            .data_bytes
            .checked_add(added)
            .is_none_or(|total| total > self.data_limit)
        {
            return self.fail("Live recording exceeds the WAV 32-bit data-length limit".into());
        }
        #[cfg(test)]
        if let Some(attempts) = &self.append_write_attempts {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if let Err(error) = self.hit_fault(CommitFaultPoint::Append) {
            return self.fail(error);
        }
        let write_result = self
            .audio
            .as_mut()
            .ok_or_else(|| "Live recording is already finalized".to_string())
            .and_then(|audio| {
                audio
                    .write_all(pcm)
                    .map_err(|error| format!("Failed to append live audio: {error}"))
            });
        if let Err(error) = write_result {
            return self.fail(error);
        }
        self.data_bytes += added;
        self.samples_since_sync += added / PCM16_BYTES_PER_SAMPLE;
        if self.samples_since_sync >= self.sync_interval_samples {
            if let Err(error) = self.hit_fault(CommitFaultPoint::PeriodicFlush) {
                return self.fail(error);
            }
            let sync_result = self
                .audio
                .as_mut()
                .expect("recording audio was checked before append")
                .sync_data();
            if let Err(error) = sync_result {
                return self.fail(format!("Failed to flush live audio: {error}"));
            }
            if let Err(error) = self.persist_journal() {
                return self.fail(error);
            }
            self.samples_since_sync = 0;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn append_pcm16(&mut self, pcm: &[u8]) -> Result<(), String> {
        const TEST_TRACK: &str = "test-raw-pcm";

        if !pcm.is_empty() && pcm.len().is_multiple_of(2) {
            let metadata_is_empty = self.journal.tracks.is_empty()
                && self.journal.track_configurations.is_empty()
                && self.journal.clock_mappings.is_empty()
                && self.journal.sequence_coverage.is_empty();
            if metadata_is_empty {
                let track = crate::audio::session::TrackId::new(TEST_TRACK).unwrap();
                self.journal.observe_revision_transition(
                    RecordingRevisionTransition::new(
                        TrackConfigurationRevision::new(track.clone(), 1, 0, 16_000).unwrap(),
                        ClockMappingRevision::new(track, 1, 0, 0).unwrap(),
                    )
                    .unwrap(),
                )?;
            }
            if self
                .journal
                .track_configurations
                .iter()
                .any(|configuration| configuration.track_id.as_str() == TEST_TRACK)
            {
                let sequence = self
                    .journal
                    .sequence_coverage
                    .iter()
                    .find(|coverage| coverage.track_id == TEST_TRACK)
                    .map_or(0, |coverage| coverage.last_sequence.saturating_add(1));
                self.journal
                    .observe_frame(TEST_TRACK, 16_000, 1, sequence, sequence);
            }
        }
        self.write_pcm16(pcm)
    }

    pub(super) fn observe_frame_metadata(
        &mut self,
        track_id: &str,
        sample_rate_hz: u32,
        channels: u16,
        sequence: u64,
        start_ms: u64,
        _duration_ms: u32,
    ) {
        self.journal
            .observe_frame(track_id, sample_rate_hz, channels, sequence, start_ms);
    }
}
