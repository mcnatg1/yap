#[cfg(test)]
use super::loss_buffer::LossSnapshot;
use super::{
    clock::TimelineError,
    loss_buffer::{LossSlot, TryDrain, REGISTRATION_TICKET_CAPACITY},
    loss_registration::{LossWriter, RegistrationTicket},
};
use crate::audio::frame::GapCause;
#[cfg(test)]
use std::hint::spin_loop;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, TryLockError};

#[derive(Debug)]
struct PendingDrain {
    generation: u64,
    registration_floor: u64,
    registration_target: u64,
    registration_invalid: bool,
    counter_exhausted: bool,
}

#[derive(Debug, Default)]
pub(super) struct LossDrainCoordinator {
    pending: Option<PendingDrain>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistrationReadiness {
    Ready,
    Pending,
    Invalid,
}

#[derive(Debug)]
pub struct LossAccumulator {
    // The full handoff generation, rather than only its low slot bit, prevents ABA.
    pub(super) active_generation: AtomicU64,
    pub(super) registration_started: AtomicU64,
    pub(super) registration_drained: AtomicU64,
    pub(super) registration_completion_tickets: [AtomicU64; REGISTRATION_TICKET_CAPACITY],
    pub(super) registration_aborted_tickets: [AtomicU64; REGISTRATION_TICKET_CAPACITY],
    pub(super) invalid: AtomicBool,
    pub(super) registration_counter_exhausted: AtomicBool,
    pub(super) generation_exhausted: AtomicBool,
    pub(super) terminal_invalid_reported: AtomicBool,
    pub(super) coordinator: Mutex<LossDrainCoordinator>,
    pub(super) slots: [LossSlot; 2],
}

impl LossAccumulator {
    pub const fn new() -> Self {
        Self {
            active_generation: AtomicU64::new(0),
            registration_started: AtomicU64::new(0),
            registration_drained: AtomicU64::new(0),
            registration_completion_tickets: [const { AtomicU64::new(0) };
                REGISTRATION_TICKET_CAPACITY],
            registration_aborted_tickets: [const { AtomicU64::new(0) };
                REGISTRATION_TICKET_CAPACITY],
            invalid: AtomicBool::new(false),
            registration_counter_exhausted: AtomicBool::new(false),
            generation_exhausted: AtomicBool::new(false),
            terminal_invalid_reported: AtomicBool::new(false),
            coordinator: Mutex::new(LossDrainCoordinator { pending: None }),
            slots: [LossSlot::new(), LossSlot::new()],
        }
    }

    pub fn record(&self, source_position_frames: u64, dropped_frames: u64, cause: GapCause) {
        self.record_inner(source_position_frames, dropped_frames, cause, || {}, || {});
    }

    pub fn invalidate(&self) {
        self.invalid.store(true, Ordering::SeqCst);
    }

    fn record_inner<F, G>(
        &self,
        source_position_frames: u64,
        dropped_frames: u64,
        cause: GapCause,
        after_entrant_registered: F,
        after_generation_read: G,
    ) where
        F: FnOnce(),
        G: FnOnce(),
    {
        if dropped_frames == 0 || self.generation_exhausted.load(Ordering::SeqCst) {
            return;
        }
        let Some(started) = self.reserve_registration_ticket() else {
            return;
        };
        let mut registration = RegistrationTicket::new(self, started);
        after_entrant_registered();
        let generation = self.active_generation.load(Ordering::SeqCst);
        after_generation_read();
        let slot = &self.slots[(generation & 1) as usize];
        let writer = LossWriter::new(slot);
        slot.record(source_position_frames, dropped_frames, cause);
        drop(writer);
        registration.commit();
    }

    fn reserve_registration_ticket(&self) -> Option<u64> {
        loop {
            let started = self.registration_started.load(Ordering::SeqCst);
            let registration_floor = self.registration_drained.load(Ordering::SeqCst);
            let Some(outstanding_registrations) = started.checked_sub(registration_floor) else {
                self.invalidate();
                return None;
            };
            if started == u64::MAX {
                self.registration_counter_exhausted
                    .store(true, Ordering::SeqCst);
                self.invalidate();
                return None;
            }
            if outstanding_registrations >= REGISTRATION_TICKET_CAPACITY as u64 {
                self.invalidate();
                return None;
            }
            if self
                .registration_started
                .compare_exchange_weak(started, started + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Some(started);
            }
        }
    }

    pub fn try_drain(&self) -> Result<TryDrain, TimelineError> {
        if self.generation_exhausted.load(Ordering::SeqCst) {
            return Err(TimelineError::InvalidTiming);
        }

        let mut coordinator = match self.coordinator.try_lock() {
            Ok(coordinator) => coordinator,
            Err(TryLockError::WouldBlock) => return Ok(TryDrain::Pending),
            Err(TryLockError::Poisoned(poisoned)) => {
                self.invalidate();
                poisoned.into_inner()
            }
        };

        if self.generation_exhausted.load(Ordering::SeqCst) {
            return Err(TimelineError::InvalidTiming);
        }

        if self.terminal_invalid_reported.load(Ordering::SeqCst) && coordinator.pending.is_none() {
            return Err(TimelineError::InvalidTiming);
        }

        if coordinator.pending.is_none() {
            let generation = self.advance_generation()?;
            let registration_floor = self.registration_drained.load(Ordering::SeqCst);
            let registration_target = self.registration_started.load(Ordering::SeqCst);
            let registration_invalid = registration_target
                .checked_sub(registration_floor)
                .is_none_or(|pending| pending > REGISTRATION_TICKET_CAPACITY as u64);
            if registration_invalid {
                self.invalidate();
            }
            coordinator.pending = Some(PendingDrain {
                generation,
                registration_floor,
                registration_target,
                registration_invalid,
                counter_exhausted: self.registration_counter_exhausted.load(Ordering::SeqCst),
            });
        }

        let pending = coordinator
            .pending
            .as_ref()
            .expect("pending drain is initialized before polling");
        let registration_readiness = if pending.registration_invalid {
            RegistrationReadiness::Invalid
        } else {
            self.registrations_are_ready(pending)
        };
        if registration_readiness == RegistrationReadiness::Pending {
            return Ok(TryDrain::Pending);
        }
        let slot = &self.slots[(pending.generation & 1) as usize];
        if slot.writers.load(Ordering::Acquire) != 0 {
            return Ok(TryDrain::Pending);
        }

        let pending = coordinator
            .pending
            .take()
            .expect("pending drain remains owned by the coordinator");
        let snapshot = slot.take(pending.generation);
        let counter_exhausted =
            pending.counter_exhausted || self.registration_counter_exhausted.load(Ordering::SeqCst);
        if pending.registration_invalid || registration_readiness == RegistrationReadiness::Invalid
        {
            self.registration_drained
                .store(pending.registration_target, Ordering::SeqCst);
            self.invalid.store(false, Ordering::SeqCst);
            if counter_exhausted {
                self.terminal_invalid_reported.store(true, Ordering::SeqCst);
            }
            return Err(TimelineError::InvalidTiming);
        }

        self.registration_drained
            .store(pending.registration_target, Ordering::SeqCst);
        if counter_exhausted {
            self.terminal_invalid_reported.store(true, Ordering::SeqCst);
            return Err(TimelineError::InvalidTiming);
        }
        if self.invalid.swap(false, Ordering::SeqCst) {
            Err(TimelineError::InvalidTiming)
        } else {
            match snapshot? {
                Some(snapshot) => Ok(TryDrain::Snapshot(snapshot)),
                None => Ok(TryDrain::Empty),
            }
        }
    }

    #[cfg(test)]
    pub fn drain(&self) -> Result<Option<LossSnapshot>, TimelineError> {
        loop {
            match self.try_drain()? {
                TryDrain::Pending => spin_loop(),
                TryDrain::Empty => return Ok(None),
                TryDrain::Snapshot(snapshot) => return Ok(Some(snapshot)),
            }
        }
    }

    fn registrations_are_ready(&self, pending: &PendingDrain) -> RegistrationReadiness {
        for ticket in pending.registration_floor..pending.registration_target {
            let expected = ticket + 1;
            let index = ticket as usize % REGISTRATION_TICKET_CAPACITY;
            let completion = &self.registration_completion_tickets[index];
            if completion.load(Ordering::SeqCst) == expected {
                continue;
            }
            if self.registration_aborted_tickets[index].load(Ordering::SeqCst) == expected {
                return RegistrationReadiness::Invalid;
            }
            return RegistrationReadiness::Pending;
        }
        RegistrationReadiness::Ready
    }

    fn advance_generation(&self) -> Result<u64, TimelineError> {
        let generation = self.active_generation.load(Ordering::SeqCst);
        match generation.checked_add(1) {
            Some(next_generation) => {
                self.active_generation
                    .store(next_generation, Ordering::SeqCst);
                Ok(generation)
            }
            None => {
                self.generation_exhausted.store(true, Ordering::SeqCst);
                self.invalidate();
                Err(TimelineError::InvalidTiming)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn record_with_registration_hooks<F, G>(
        &self,
        source_position_frames: u64,
        dropped_frames: u64,
        cause: GapCause,
        after_started: F,
        after_generation_read: G,
    ) where
        F: FnOnce(),
        G: FnOnce(),
    {
        self.record_inner(
            source_position_frames,
            dropped_frames,
            cause,
            after_started,
            after_generation_read,
        );
    }
}

impl Default for LossAccumulator {
    fn default() -> Self {
        Self::new()
    }
}
