//! A bounded FIFO configuration transaction boundary, independent of runtime progress.
use crate::{
    app_state::{next_revision, AppState},
    model::{AppConfig, AppSnapshot},
};
use std::sync::{Condvar, Mutex};

const MAX_CONFIGURATION_TRANSACTIONS: u64 = 16;

#[derive(Debug, Default)]
pub(crate) struct ConfigurationWriter {
    sequence: Mutex<(u64, u64)>,
    ready: Condvar,
}

pub(crate) struct ConfigurationTransaction<'a> {
    writer: &'a ConfigurationWriter,
}

impl ConfigurationWriter {
    pub(crate) fn begin(&self) -> Result<ConfigurationTransaction<'_>, String> {
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| "Settings writer is unavailable")?;
        if sequence.0 - sequence.1 >= MAX_CONFIGURATION_TRANSACTIONS {
            return Err(
                "Settings save queue is full. Retry after the pending save completes.".into(),
            );
        }
        let ticket = sequence.0;
        sequence.0 = sequence
            .0
            .checked_add(1)
            .ok_or("Settings transaction sequence exhausted")?;
        while sequence.1 != ticket {
            sequence = self
                .ready
                .wait(sequence)
                .map_err(|_| "Settings writer is unavailable")?;
        }
        Ok(ConfigurationTransaction { writer: self })
    }
}

impl Drop for ConfigurationTransaction<'_> {
    fn drop(&mut self) {
        if let Ok(mut sequence) = self.writer.sequence.lock() {
            sequence.1 += 1;
            self.writer.ready.notify_all();
        }
    }
}

pub(crate) struct ConfigurationAdmission {
    operation: Option<uuid::Uuid>,
    view: (
        uuid::Uuid,
        Option<uuid::Uuid>,
        crate::session_view::ViewPhase,
    ),
}
impl ConfigurationTransaction<'_> {
    /// Writer authority precedes short session admission. No admission survives preparation.
    pub(crate) fn prepare_admitted(
        &self,
        state: &AppState,
        allow_current_loading: bool,
    ) -> Result<(AppSnapshot, ConfigurationAdmission), String> {
        let _admission = state
            .session_view
            .admission
            .lock()
            .map_err(|_| "Session admission is unavailable")?;
        if !allow_current_loading {
            state.session_view.ensure_not_loading()?;
        }
        let snapshot = state.snapshot()?;
        let authority = ConfigurationAdmission {
            operation: snapshot.lens.operation_id,
            view: state.session_view.configuration_authority()?,
        };
        Ok((snapshot, authority))
    }

    pub(crate) fn commit<T>(
        &self,
        state: &AppState,
        previous: &AppConfig,
        next: AppConfig,
        apply: impl FnOnce(&mut AppSnapshot) -> Result<T, String>,
    ) -> Result<(AppSnapshot, T), String> {
        self.commit_with(
            state,
            previous,
            next,
            |config| state.store.save(config).map_err(|error| error.to_string()),
            apply,
        )
    }

    pub(crate) fn commit_admitted<'s, T>(
        &self,
        state: &'s AppState,
        previous: &AppConfig,
        next: AppConfig,
        authority: ConfigurationAdmission,
        apply: impl FnOnce(&mut AppSnapshot) -> Result<T, String>,
    ) -> Result<(AppSnapshot, T, std::sync::MutexGuard<'s, ()>), String> {
        self.commit_impl(
            state,
            previous,
            next,
            Some(authority),
            |config| state.store.save(config).map_err(|error| error.to_string()),
            apply,
        )
        .map(|(snapshot, value, admission)| {
            (
                snapshot,
                value,
                admission.expect("admitted transaction owns admission"),
            )
        })
    }

    fn commit_with<T>(
        &self,
        state: &AppState,
        previous: &AppConfig,
        next: AppConfig,
        save: impl FnMut(&AppConfig) -> Result<(), String>,
        apply: impl FnOnce(&mut AppSnapshot) -> Result<T, String>,
    ) -> Result<(AppSnapshot, T), String> {
        self.commit_impl(state, previous, next, None, save, apply)
            .map(|(snapshot, value, _)| (snapshot, value))
    }

    fn commit_impl<'s, T>(
        &self,
        state: &'s AppState,
        previous: &AppConfig,
        next: AppConfig,
        authority: Option<ConfigurationAdmission>,
        mut save: impl FnMut(&AppConfig) -> Result<(), String>,
        apply: impl FnOnce(&mut AppSnapshot) -> Result<T, String>,
    ) -> Result<(AppSnapshot, T, Option<std::sync::MutexGuard<'s, ()>>), String> {
        save(&next)?;
        let result = (|| {
            let admission = if authority.is_some() {
                Some(
                    state
                        .session_view
                        .admission
                        .lock()
                        .map_err(|_| "Session admission is unavailable")?,
                )
            } else {
                None
            };
            if let Some(authority) = &authority {
                if state.session_view.configuration_authority()? != authority.view {
                    return Err("Session view changed while saving settings".into());
                }
            }
            let mut latest = state
                .runtime
                .write()
                .map_err(|_| "Application state is unavailable")?;
            if latest.config != *previous {
                return Err("Settings changed before the saved change could be applied".into());
            }
            if authority
                .as_ref()
                .is_some_and(|authority| latest.lens.operation_id != authority.operation)
            {
                return Err("Lens operation changed while saving settings".into());
            }
            let revision = next_revision(&latest)?;
            let mut committed = latest.clone();
            let value = apply(&mut committed)?;
            committed.config = next;
            committed.revision = revision;
            *latest = committed.clone();
            Ok((committed, value, admission))
        })();
        if let Err(error) = result.as_ref() {
            save(previous).map_err(|rollback| {
                format!("{error}; restoring previous settings failed: {rollback}")
            })?;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentKind;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    #[test]
    fn slow_save_preserves_concurrent_progress_and_orders_latest_configuration() {
        let state = Arc::new(crate::test_support::state());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_state = state.clone();
        let first = std::thread::spawn(move || {
            let transaction = first_state.store.writer.begin().unwrap();
            let previous = first_state.config().unwrap();
            let mut next = previous.clone();
            next.working_directory = "/saved-first".into();
            transaction
                .commit_with(
                    &first_state,
                    &previous,
                    next,
                    |config| {
                        entered_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                        first_state
                            .store
                            .save(config)
                            .map_err(|error| error.to_string())
                    },
                    |_| Ok(()),
                )
                .unwrap();
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let second_state = state.clone();
        let second = std::thread::spawn(move || {
            let transaction = second_state.store.writer.begin().unwrap();
            let previous = second_state.config().unwrap();
            let mut next = previous.clone();
            next.agent = AgentKind::Codex;
            transaction
                .commit(&second_state, &previous, next, |_| Ok(()))
                .unwrap();
        });
        // The storage worker is stalled, but the runtime lock remains immediately available.
        {
            let mut latest = state
                .runtime
                .try_write()
                .expect("storage must not own runtime authority");
            latest.revision += 17;
            latest.lens.error = Some("concurrent progress marker".into());
        }
        release_tx.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();
        let current = state.snapshot().unwrap();
        assert_eq!(
            current.config.working_directory,
            std::path::PathBuf::from("/saved-first")
        );
        assert_eq!(current.config.agent, AgentKind::Codex);
        assert_eq!(
            current.lens.error.as_deref(),
            Some("concurrent progress marker")
        );
        assert_eq!(current.revision, 19);
        assert_eq!(state.store.try_load().unwrap(), current.config);
    }

    #[test]
    fn disk_failure_never_applies_or_cancels_running_work() {
        let state = crate::test_support::state();
        let original = state.snapshot().unwrap();
        let run = state
            .agent_control
            .begin_validation(uuid::Uuid::new_v4())
            .unwrap();
        let transaction = state.store.writer.begin().unwrap();
        let mut next = original.config.clone();
        next.agent = AgentKind::Codex;
        let result = transaction.commit_with(
            &state,
            &original.config,
            next,
            |_| Err("injected fsync failure".into()),
            |_| {
                state.agent_control.cancel_active()?;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(state.snapshot().unwrap(), original);
        assert!(!*run.cancellation.borrow());
    }

    #[test]
    fn rejected_apply_restores_disk_and_preserves_latest_runtime() {
        let state = crate::test_support::state();
        let original = state.snapshot().unwrap();
        let transaction = state.store.writer.begin().unwrap();
        let mut next = original.config.clone();
        next.agent = AgentKind::Codex;
        let disk = Mutex::new(original.config.clone());
        let result: Result<(AppSnapshot, ()), String> = transaction.commit_with(
            &state,
            &original.config,
            next,
            |config| {
                *disk.lock().unwrap() = config.clone();
                Ok(())
            },
            |latest| {
                latest.lens.error = Some("must roll back".into());
                Err("stale selection".into())
            },
        );
        assert!(result.unwrap_err().contains("stale selection"));
        assert_eq!(*disk.lock().unwrap(), original.config);
        assert_eq!(state.snapshot().unwrap(), original);
    }

    #[test]
    fn stale_selection_during_storage_cannot_replace_current_authority() {
        let state = crate::test_support::state();
        let original = state.snapshot().unwrap();
        let transaction = state.store.writer.begin().unwrap();
        let mut next = original.config.clone();
        next.agent = AgentKind::Codex;
        let current_id = uuid::Uuid::new_v4();
        let disk = Mutex::new(original.config.clone());
        let result: Result<(AppSnapshot, ()), String> = transaction.commit_with(
            &state,
            &original.config,
            next,
            |config| {
                *disk.lock().unwrap() = config.clone();
                let mut latest = state.runtime.write().unwrap();
                latest.agent_selection.operation_id = Some(current_id);
                latest.revision = 4;
                Ok(())
            },
            |latest| {
                if latest.agent_selection != original.agent_selection {
                    return Err("stale selection".into());
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(*disk.lock().unwrap(), original.config);
        let latest = state.snapshot().unwrap();
        assert_eq!(latest.agent_selection.operation_id, Some(current_id));
        assert_eq!(latest.revision, 4);
    }

    #[test]
    fn stopped_operation_during_storage_has_free_admission_and_rejects_stale_commit() {
        let state = crate::test_support::state();
        state.runtime.write().unwrap().lens.operation_id = Some(uuid::Uuid::new_v4());
        let transaction = state.store.writer.begin().unwrap();
        let (original, authority) = transaction.prepare_admitted(&state, false).unwrap();
        let mut next = original.config.clone();
        next.working_directory = "/new-directory".into();
        let disk = Mutex::new(original.config.clone());
        let mut writes = 0;
        let result: Result<(AppSnapshot, (), Option<std::sync::MutexGuard<'_, ()>>), String> =
            transaction.commit_impl(
                &state,
                &original.config,
                next,
                Some(authority),
                |config| {
                    *disk.lock().unwrap() = config.clone();
                    writes += 1;
                    if writes == 1 {
                        let _admission = state
                            .session_view
                            .admission
                            .try_lock()
                            .expect("storage must not reserve session admission");
                        state.runtime.write().unwrap().lens.operation_id = None;
                        state.session_view.clear().unwrap();
                    }
                    Ok(())
                },
                |_| panic!("superseded operation must not be cancelled or applied"),
            );
        assert!(result.is_err());
        assert_eq!(*disk.lock().unwrap(), original.config);
        assert_eq!(state.config().unwrap(), original.config);
        assert_eq!(state.lens().unwrap().operation_id, None);
    }
}
