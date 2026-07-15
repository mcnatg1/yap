pub(super) const CRASH_CLAIM_BIT: u64 = 1 << 63;

pub(super) fn active_session_matches(active_session: u64, session: u64) -> bool {
    session != 0 && (active_session == session || active_session == session | CRASH_CLAIM_BIT)
}
