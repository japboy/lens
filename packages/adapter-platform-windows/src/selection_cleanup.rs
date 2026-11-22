//! Cleanup evidence for the cooperative fixture's selection lifecycle.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionPhase {
    #[default]
    Pending,
    /// The selector cancelled and both acquisition helpers were observed exited.
    Cancelled,
    /// The adapter installed a registration that explicit release must close.
    Registered,
}

impl SelectionPhase {
    pub fn confirms_release(self, released: bool) -> bool {
        released && matches!(self, Self::Cancelled | Self::Registered)
    }
}

/// Always attempt both owned endpoints; Drop is not a cleanup acknowledgment.
#[cfg(any(test, all(windows, target_arch = "x86_64")))]
pub(crate) fn close_cancelled<E>(
    worker: impl FnOnce() -> Result<(), E>,
    owner: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    let worker = worker();
    let owner = owner();
    worker.and(owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn empty_release_never_confirms_an_unsettled_selection() {
        for phase in [
            SelectionPhase::Pending,
            SelectionPhase::Cancelled,
            SelectionPhase::Registered,
        ] {
            assert!(!phase.confirms_release(false));
            assert_eq!(
                phase.confirms_release(true),
                phase != SelectionPhase::Pending
            );
        }
    }

    #[test]
    fn cancellation_requires_both_exit_observations_and_attempts_both_on_error() {
        for (worker_ok, owner_ok) in [(true, true), (false, true), (true, false), (false, false)] {
            let calls = Cell::new(0);
            let result = close_cancelled(
                || {
                    calls.set(calls.get() + 1);
                    worker_ok.then_some(()).ok_or("worker exit unconfirmed")
                },
                || {
                    calls.set(calls.get() + 1);
                    owner_ok.then_some(()).ok_or("owner exit unconfirmed")
                },
            );
            assert_eq!(calls.get(), 2);
            assert_eq!(result.is_ok(), worker_ok && owner_ok);
        }
    }
}
