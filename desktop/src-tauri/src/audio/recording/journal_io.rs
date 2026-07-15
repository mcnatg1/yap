use super::*;

pub(super) fn write_wav_header(file: &mut File, data_bytes: u64) -> Result<(), String> {
    let data_bytes = u32::try_from(data_bytes)
        .map_err(|_| "Live recording exceeds the WAV 32-bit data-length limit".to_string())?;
    let riff_bytes = 36u32
        .checked_add(data_bytes)
        .ok_or_else(|| "Live recording exceeds the WAV 32-bit data-length limit".to_string())?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("Failed to seek live audio: {error}"))?;
    file.write_all(b"RIFF")
        .and_then(|_| file.write_all(&riff_bytes.to_le_bytes()))
        .and_then(|_| file.write_all(b"WAVEfmt "))
        .and_then(|_| file.write_all(&16u32.to_le_bytes()))
        .and_then(|_| file.write_all(&1u16.to_le_bytes()))
        .and_then(|_| file.write_all(&1u16.to_le_bytes()))
        .and_then(|_| file.write_all(&16_000u32.to_le_bytes()))
        .and_then(|_| file.write_all(&32_000u32.to_le_bytes()))
        .and_then(|_| file.write_all(&2u16.to_le_bytes()))
        .and_then(|_| file.write_all(&16u16.to_le_bytes()))
        .and_then(|_| file.write_all(b"data"))
        .and_then(|_| file.write_all(&data_bytes.to_le_bytes()))
        .map_err(|error| format!("Failed to write live audio header: {error}"))?;
    file.seek(SeekFrom::End(0))
        .map_err(|error| format!("Failed to seek live audio data: {error}"))?;
    Ok(())
}

pub(super) fn serialize_journal_record(record: &JournalRecord) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(record)
        .map_err(|error| format!("Failed to serialize recording journal: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn write_journal_record(file: &mut File, record: &JournalRecord) -> Result<u64, String> {
    let bytes = serialize_journal_record(record)?;
    file.write_all(&bytes)
        .map_err(|error| format!("Failed to write recording journal: {error}"))?;
    Ok(bytes.len() as u64)
}

pub(super) fn read_journal_append_log(
    directory: &Path,
    name: &str,
) -> Result<CaptureJournal, String> {
    let mut file = open_regular_artifact(directory, name)?;
    let text = read_open_file(&mut file)?;
    parse_journal_append_log(&text)
}

pub(super) fn parse_journal_append_log(text: &str) -> Result<CaptureJournal, String> {
    if let Ok(snapshot) = serde_json::from_str::<CaptureJournal>(text) {
        validate_timeline_control_metadata(
            &snapshot.session_id,
            snapshot.tracks.values(),
            &snapshot.track_configurations,
            &snapshot.clock_mappings,
            &snapshot.timeline_gaps,
        )?;
        validate_initial_sequence_coverage(&snapshot.sequence_coverage)?;
        return Ok(snapshot);
    }
    let mut journal = None;
    let lines = text.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let record = match serde_json::from_str::<JournalRecord>(line) {
            Ok(record) => record,
            Err(_) if index + 1 == lines.len() && !text.ends_with('\n') => break,
            Err(error) => return Err(format!("Failed to parse recording journal: {error}")),
        };
        match record {
            JournalRecord::Header { journal: header } => {
                if journal.is_some() {
                    return Err("recording journal has multiple headers".into());
                }
                journal = Some(header);
            }
            JournalRecord::Delta { delta } => {
                let Some(recovered) = journal.as_mut() else {
                    return Err("recording journal delta has no header".into());
                };
                apply_journal_delta(recovered, delta)?;
            }
            JournalRecord::Overflow { session_id, .. } => {
                let Some(recovered) = journal.as_ref() else {
                    return Err("recording journal overflow has no header".into());
                };
                if recovered.session_id != session_id {
                    return Err("recording journal overflow session does not match".into());
                }
            }
        }
    }
    let journal = journal.ok_or_else(|| "recording journal has no valid header".to_string())?;
    validate_timeline_control_metadata(
        &journal.session_id,
        journal.tracks.values(),
        &journal.track_configurations,
        &journal.clock_mappings,
        &journal.timeline_gaps,
    )?;
    validate_initial_sequence_coverage(&journal.sequence_coverage)?;
    Ok(journal)
}

pub(crate) fn parse_journal_for_session(
    text: &str,
    session_id: &SessionId,
) -> Result<bool, String> {
    Ok(parse_journal_append_log(text)?.session_id == *session_id)
}

pub(super) fn apply_journal_delta(
    journal: &mut CaptureJournal,
    delta: JournalDelta,
) -> Result<(), String> {
    if delta.schema_version != CAPTURE_SCHEMA_VERSION || delta.session_id != journal.session_id {
        return Err("recording journal delta does not match the session".into());
    }
    validate_initial_sequence_coverage(&delta.sequence_coverage)?;
    for track in delta.tracks {
        journal.tracks.insert(track.track_id.clone(), track);
    }
    for transition in delta.revision_transitions {
        journal.observe_revision_transition(transition)?;
    }
    if delta.timeline_gap_start_index > journal.timeline_gaps.len() {
        return Err("recording journal timeline-gap delta is out of order".into());
    }
    journal
        .timeline_gaps
        .truncate(delta.timeline_gap_start_index);
    journal.timeline_gaps.extend(delta.timeline_gaps);
    for coverage in delta.sequence_coverage {
        if let Some(existing) = journal
            .sequence_coverage
            .iter_mut()
            .find(|existing| existing.track_id == coverage.track_id)
        {
            *existing = coverage;
        } else {
            journal.sequence_coverage.push(coverage);
        }
    }
    if delta.gap_start_index != journal.sequence_gaps.len().saturating_sub(1) {
        return Err("recording journal gap delta is out of order".into());
    }
    journal.sequence_gaps.truncate(delta.gap_start_index);
    journal.sequence_gaps.extend(delta.sequence_gaps);
    journal.sequence_gap_overflow = delta.sequence_gap_overflow;
    journal.sink_degraded |= delta.sink_degraded;
    Ok(())
}

#[cfg(test)]
pub(super) fn read_journal_snapshot(path: &Path) -> Result<CaptureJournal, String> {
    let directory = path
        .parent()
        .ok_or_else(|| "recording journal has no parent directory".to_string())?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "recording journal has no file name".to_string())?;
    read_journal_append_log(directory, name)
}

pub(super) fn write_json_file_open<T: serde::Serialize>(
    path: &Path,
    value: &T,
    label: &str,
) -> Result<File, String> {
    let mut file = create_new(path, label)?;
    serde_json::to_writer(&mut file, value)
        .map_err(|error| format!("Failed to write {label}: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("Failed to write {label}: {error}"))?;
    Ok(file)
}
