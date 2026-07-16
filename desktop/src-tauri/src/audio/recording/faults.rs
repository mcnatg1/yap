#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitFaultPoint {
    Append,
    PeriodicFlush,
    JournalAppend,
    JournalSync,
    WavHeaderPatch,
    AudioSync,
    SidecarSync,
    FinalArtifactRename,
    CommitSync,
    CommitRename,
    AudioStagingCleanup,
    SidecarStagingCleanup,
    CommitStagingCleanup,
}

impl CommitFaultPoint {
    #[cfg(test)]
    pub(super) const ALL: [Self; 10] = [
        Self::Append,
        Self::PeriodicFlush,
        Self::JournalAppend,
        Self::JournalSync,
        Self::WavHeaderPatch,
        Self::AudioSync,
        Self::SidecarSync,
        Self::FinalArtifactRename,
        Self::CommitSync,
        Self::CommitRename,
    ];
}
