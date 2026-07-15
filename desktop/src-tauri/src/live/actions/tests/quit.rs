use std::cell::RefCell;

use super::super::quit::{run_quit_with, QuitClaim, QuitCoordinator};

#[test]
fn quit_does_not_exit_when_finalization_fails() {
    let events = RefCell::new(Vec::new());

    let result = run_quit_with(
        || {
            events.borrow_mut().push("finalize");
            Err("save failed".to_string())
        },
        || events.borrow_mut().push("exit"),
    );

    assert_eq!(result, Err("save failed".into()));
    assert_eq!(events.into_inner(), vec!["finalize"]);
}

#[test]
fn quit_exits_only_after_successful_finalization() {
    let events = RefCell::new(Vec::new());

    let result = run_quit_with(
        || {
            events.borrow_mut().push("finalize");
            Ok(())
        },
        || events.borrow_mut().push("exit"),
    );

    assert_eq!(result, Ok(()));
    assert_eq!(events.into_inner(), vec!["finalize", "exit"]);
}

#[test]
fn repeated_quit_coalesces_and_cannot_bypass_an_unacknowledged_save_failure() {
    let quit = QuitCoordinator::new();

    assert_eq!(quit.claim(), QuitClaim::Finalize);
    assert_eq!(quit.claim(), QuitClaim::Coalesced);
    quit.finish(Err("save failed".into()));

    assert_eq!(quit.claim(), QuitClaim::Blocked("save failed".to_string()));
    assert!(!quit.exit_authorized());
}

#[test]
fn successful_quit_authorizes_only_the_semantic_exit_it_started() {
    let quit = QuitCoordinator::new();

    assert_eq!(quit.claim(), QuitClaim::Finalize);
    quit.finish(Ok(()));

    assert!(quit.exit_authorized());
    assert_eq!(quit.claim(), QuitClaim::ExitAuthorized);
}
