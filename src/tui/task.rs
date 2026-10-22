use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use polars::prelude::{DataFrame, InProcessQuery, LazyFrame};

use crate::QuiltError;

/// Shared cancellation state for one background TUI task.
///
/// A task may be cancelled before Polars has created its in-process query.
/// Keeping the flag separately from the query lets that cancellation be
/// observed as soon as query creation finishes.
#[derive(Clone, Default)]
pub(super) struct TaskControl {
    state: Arc<TaskState>,
}

#[derive(Default)]
struct TaskState {
    cancelled: AtomicBool,
    query: Mutex<Option<InProcessQuery>>,
}

impl TaskControl {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::SeqCst);

        // Never hold this lock while waiting for query completion. It is used
        // only to publish or cancel the short-lived Polars query handle.
        if let Ok(query) = self.state.query.lock() {
            if let Some(query) = query.as_ref() {
                query.cancel();
            }
        }
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
    }

    pub(super) fn check(&self) -> Result<(), QuiltError> {
        if self.is_cancelled() {
            Err(cancelled_error())
        } else {
            Ok(())
        }
    }

    pub(super) fn collect(&self, frame: LazyFrame) -> Result<DataFrame, QuiltError> {
        self.check()?;

        let query = frame
            .collect_concurrently()
            .map_err(|error| QuiltError::operation("lazy query", error.to_string()))?;

        {
            let mut registered = self
                .state
                .query
                .lock()
                .map_err(|_| QuiltError::operation("lazy query", "query lock poisoned"))?;
            *registered = Some(query.clone());

            // Cancellation may have happened while Polars was preparing the
            // query, before there was a handle for `cancel` to reach.
            if self.is_cancelled() {
                query.cancel();
            }
        }

        // Keep the local handle alive as well as the registered clone. Every
        // InProcessQuery drop cancels its token, so neither handle may be
        // dropped until fetch has completed.
        let result = query
            .fetch_blocking()
            .map_err(|error| QuiltError::operation("lazy query", error.to_string()));

        let mut registered = self
            .state
            .query
            .lock()
            .map_err(|_| QuiltError::operation("lazy query", "query lock poisoned"))?;
        registered.take();
        drop(registered);

        self.check()?;
        result
    }
}

fn cancelled_error() -> QuiltError {
    QuiltError::operation("lazy query", "operation cancelled")
}

#[cfg(test)]
mod tests {
    use super::TaskControl;
    use polars::prelude::{AllowedOptimizations, Column, DataFrame, IntoLazy};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn pre_cancelled_task_does_not_collect() {
        let control = TaskControl::new();
        control.cancel();

        let error = control.collect(DataFrame::empty().lazy()).unwrap_err();
        assert_eq!(
            error,
            crate::QuiltError::operation("lazy query", "operation cancelled")
        );
    }

    #[test]
    fn active_query_cancellation_waits_for_polars_to_finish() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let first_call = Arc::new(AtomicBool::new(true));
        let control = TaskControl::new();
        let query_control = control.clone();

        let frame = DataFrame::new_infer_height(vec![Column::new("value".into(), [1i64])])
            .unwrap()
            .lazy()
            .map(
                move |frame| {
                    if first_call.swap(false, Ordering::SeqCst) {
                        entered_tx.send(()).unwrap();
                        release_rx.lock().unwrap().recv().unwrap();
                    }
                    Ok(frame)
                },
                AllowedOptimizations::default(),
                None,
                Some("blocking cancellation test"),
            );

        let query = thread::spawn(move || query_control.collect(frame));
        if entered_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            let _ = release_tx.send(());
            let _ = query.join();
            panic!("Polars did not enter the lazy callback");
        }

        // `cancel` must be able to reach the registered query while
        // `fetch_blocking` is waiting for the deliberately held callback.
        control.cancel();
        release_tx.send(()).unwrap();

        let error = query.join().unwrap().unwrap_err();
        assert_eq!(
            error,
            crate::QuiltError::operation("lazy query", "operation cancelled")
        );
    }
}
