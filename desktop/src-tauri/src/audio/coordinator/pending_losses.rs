use std::collections::VecDeque;

use crate::audio::timeline::LossSnapshot;

const PENDING_LOSS_CAPACITY: usize = 64;
pub(super) const LOSS_DRAIN_ATTEMPT_LIMIT: usize = PENDING_LOSS_CAPACITY;

pub(super) struct PendingLosses {
    snapshots: VecDeque<LossSnapshot>,
}

impl PendingLosses {
    pub(super) fn new() -> Self {
        Self {
            snapshots: VecDeque::with_capacity(PENDING_LOSS_CAPACITY),
        }
    }

    pub(super) fn push(&mut self, loss: LossSnapshot) -> bool {
        if self.snapshots.len() < PENDING_LOSS_CAPACITY {
            self.snapshots.push_back(loss);
            return true;
        }
        let Some(previous) = self.snapshots.back_mut() else {
            return false;
        };
        let Some(previous_end) = previous
            .first_source_position_frames
            .checked_add(previous.dropped_frames)
        else {
            return false;
        };
        let Some(merged_frames) = previous.dropped_frames.checked_add(loss.dropped_frames) else {
            return false;
        };
        if previous.cause != loss.cause
            || previous_end != loss.first_source_position_frames
            || loss.generation <= previous.generation
        {
            return false;
        }
        previous.dropped_frames = merged_frames;
        previous.generation = loss.generation;
        true
    }

    pub(super) fn front(&self) -> Option<&LossSnapshot> {
        self.snapshots.front()
    }

    pub(super) fn pop_front(&mut self) -> Option<LossSnapshot> {
        self.snapshots.pop_front()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub(super) fn clear(&mut self) {
        self.snapshots.clear();
    }
}
