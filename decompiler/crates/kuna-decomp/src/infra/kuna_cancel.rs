//! (kuna cancel) A host's way to take back work it no longer wants.
//!
//! A program that embeds Kuna — an interactive viewer, where the user clicks
//! on to the next function, or closes the window while the binary is still
//! being analysed — has no use for the answer it asked for a moment ago, and
//! no way to stop the thread computing it: Kuna's state is not `Send`, so the
//! work cannot be moved away and abandoned. [`Cancel`] is the way in from the
//! outside. The host keeps one clone and runs the work under
//! [`CancelScope::enter`] with another; [`Cancel::cancel`] from any thread
//! makes the work stop at its next probe.
//!
//! The probes are the ones the decompile-all watchdog already polls
//! ([`crate::decompile_drive`]): a cancelled drive stops exactly where one
//! whose budget ran out stops, and fails with [`CANCELLED`] instead of the
//! budget error. That path is the one `decompile-all` takes in-process before
//! it goes on to the next function, so the program stays usable afterwards.
//! The analysis walk polls [`requested`] once per function it discovers and
//! stops early; whoever drives the analysis checks [`requested`] after it and
//! commits nothing from a cancelled walk.
//!
//! The scope is per thread. Work that fans out to threads of its own (the
//! parallel decode lanes) takes [`current`] on the thread that spawns them and
//! enters it on each.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The error text of work stopped by [`Cancel::cancel`].
pub const CANCELLED: &str = "cancelled by the host";

/// A cancellation flag shared between the host and the work it started.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the work running under this token to stop. Callable from any
    /// thread, any number of times; it cannot be undone.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Cancel>> = const { RefCell::new(None) };
}

/// Runs this thread's work under a token until dropped, then restores the
/// one before it.
#[must_use = "the token applies only while the scope is alive"]
pub struct CancelScope(Option<Cancel>);

impl CancelScope {
    pub fn enter(token: Option<Cancel>) -> Self {
        CancelScope(CURRENT.with(|current| current.replace(token)))
    }
}

impl Drop for CancelScope {
    fn drop(&mut self) {
        CURRENT.with(|current| *current.borrow_mut() = self.0.take());
    }
}

/// The token this thread's work runs under, to hand to threads it spawns.
pub fn current() -> Option<Cancel> {
    CURRENT.with(|current| current.borrow().clone())
}

/// Does this thread run under a token at all? A drive arms its probes for
/// one even when no budget or memory limit would.
pub fn armed() -> bool {
    CURRENT.with(|current| current.borrow().is_some())
}

/// Has the host cancelled the work this thread runs?
pub fn requested() -> bool {
    CURRENT.with(|current| current.borrow().as_ref().is_some_and(Cancel::is_cancelled))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_requested_outside_a_scope() {
        assert!(!armed());
        assert!(!requested());
    }

    #[test]
    fn a_cancel_reaches_the_thread_running_under_the_token() {
        let token = Cancel::new();
        let _scope = CancelScope::enter(Some(token.clone()));
        assert!(armed());
        assert!(!requested());
        std::thread::spawn({
            let token = token.clone();
            move || token.cancel()
        })
        .join()
        .expect("the cancelling thread");
        assert!(requested());
    }

    #[test]
    fn leaving_a_scope_restores_the_token_before_it() {
        let outer = Cancel::new();
        outer.cancel();
        let _outer = CancelScope::enter(Some(outer));
        {
            let _inner = CancelScope::enter(Some(Cancel::new()));
            assert!(!requested());
        }
        assert!(requested());
    }

    #[test]
    fn another_thread_does_not_see_this_threads_token() {
        let token = Cancel::new();
        token.cancel();
        let _scope = CancelScope::enter(Some(token));
        let seen = std::thread::spawn(requested)
            .join()
            .expect("the other thread");
        assert!(!seen);
    }
}
