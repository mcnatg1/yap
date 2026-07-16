use super::{
    loss_accumulator::LossAccumulator,
    loss_buffer::{LossSlot, REGISTRATION_TICKET_CAPACITY},
};
use std::sync::atomic::Ordering;

pub(super) struct RegistrationTicket<'a> {
    losses: &'a LossAccumulator,
    ticket: u64,
    committed: bool,
}

impl<'a> RegistrationTicket<'a> {
    pub(super) fn new(losses: &'a LossAccumulator, ticket: u64) -> Self {
        Self {
            losses,
            ticket,
            committed: false,
        }
    }

    pub(super) fn commit(&mut self) {
        let completed_ticket = self.ticket + 1;
        self.losses.registration_completion_tickets
            [self.ticket as usize % REGISTRATION_TICKET_CAPACITY]
            .store(completed_ticket, Ordering::SeqCst);
        self.committed = true;
    }
}

impl Drop for RegistrationTicket<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let aborted_ticket = self.ticket + 1;
            self.losses.registration_aborted_tickets
                [self.ticket as usize % REGISTRATION_TICKET_CAPACITY]
                .store(aborted_ticket, Ordering::SeqCst);
            self.losses.invalidate();
        }
    }
}

pub(super) struct LossWriter<'a> {
    slot: &'a LossSlot,
}

impl<'a> LossWriter<'a> {
    pub(super) fn new(slot: &'a LossSlot) -> Self {
        slot.writers.fetch_add(1, Ordering::Relaxed);
        Self { slot }
    }
}

impl Drop for LossWriter<'_> {
    fn drop(&mut self) {
        self.slot.writers.fetch_sub(1, Ordering::Release);
    }
}
