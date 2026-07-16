use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

static DELETION_CLEANUP_CURSORS: OnceLock<Mutex<HashMap<PathBuf, DeletionCleanupCursors>>> =
    OnceLock::new();

const MAX_DELETION_CLEANUP_CURSOR_DIRS: usize = 64;

#[derive(Default)]
struct DeletionCleanupCursors {
    private_leftovers: Option<String>,
    pending_intents: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) enum DeletionCleanupCursor {
    PrivateLeftovers,
    PendingIntents,
}

pub(super) fn deletion_cleanup_cursor(dir: &Path, kind: DeletionCleanupCursor) -> Option<String> {
    let cursors = DELETION_CLEANUP_CURSORS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cursors = cursors.get(dir)?;
    match kind {
        DeletionCleanupCursor::PrivateLeftovers => cursors.private_leftovers.clone(),
        DeletionCleanupCursor::PendingIntents => cursors.pending_intents.clone(),
    }
}

pub(super) fn update_deletion_cleanup_cursor(
    dir: &Path,
    kind: DeletionCleanupCursor,
    cursor: Option<String>,
) {
    let Some(cursor) = cursor else {
        return;
    };
    let mut cursors = DELETION_CLEANUP_CURSORS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cursors.len() >= MAX_DELETION_CLEANUP_CURSOR_DIRS && !cursors.contains_key(dir) {
        cursors.clear();
    }
    let cursors = cursors.entry(dir.to_path_buf()).or_default();
    match kind {
        DeletionCleanupCursor::PrivateLeftovers => cursors.private_leftovers = Some(cursor),
        DeletionCleanupCursor::PendingIntents => cursors.pending_intents = Some(cursor),
    }
}

pub(in crate::live::recordings) struct RotatingDeletionCandidates {
    cursor: Option<String>,
    after: BTreeSet<String>,
    before: BTreeSet<String>,
    overflow: bool,
    limit: usize,
}

impl RotatingDeletionCandidates {
    pub(in crate::live::recordings) fn new(cursor: Option<String>, limit: usize) -> Self {
        Self {
            cursor,
            after: BTreeSet::new(),
            before: BTreeSet::new(),
            overflow: false,
            limit,
        }
    }

    pub(in crate::live::recordings) fn push(&mut self, name: String) {
        let before_cursor = self.cursor.as_ref().is_some_and(|cursor| name <= *cursor);
        let target = if before_cursor {
            &mut self.before
        } else {
            &mut self.after
        };
        self.overflow |= push_bounded_candidate(target, name, self.limit);
    }

    pub(in crate::live::recordings) fn finish(self) -> (BTreeSet<String>, bool, Option<String>) {
        let mut selected = self.after;
        let mut wrapped_last = None;
        let remaining = self.limit.saturating_sub(selected.len());
        for name in self.before.into_iter().take(remaining) {
            wrapped_last = Some(name.clone());
            selected.insert(name);
        }
        let next_cursor = wrapped_last.or_else(|| selected.last().cloned());
        (selected, self.overflow, next_cursor)
    }
}

fn push_bounded_candidate(candidates: &mut BTreeSet<String>, name: String, limit: usize) -> bool {
    if candidates.len() < limit {
        candidates.insert(name);
        return false;
    }
    let Some(last) = candidates.last().cloned() else {
        return false;
    };
    if name < last {
        candidates.remove(&last);
        candidates.insert(name);
    }
    true
}
