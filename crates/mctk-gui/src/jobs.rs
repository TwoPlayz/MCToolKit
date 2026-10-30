//! Background work: conversions, pruning and map rendering run on their own thread and
//! wake the UI when done.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Set when something the user is waiting on (a conversion, a prune, a save) has finished.
static FINISHED: AtomicBool = AtomicBool::new(false);

/// Records that a long task finished, so the app can flash its taskbar button if the window
/// is in the background.
pub fn finished() {
    FINISHED.store(true, Ordering::Relaxed);
}

/// True once per [`finished`] call.
pub fn take_finished() -> bool {
    FINISHED.swap(false, Ordering::Relaxed)
}

pub struct Job<T> {
    result: Arc<Mutex<Option<Result<T, String>>>>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(
        label: impl Into<String>,
        ctx: &eframe::egui::Context,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Self {
        Self::start(label.into(), ctx, false, work)
    }

    /// Like [`Job::spawn`], and flags the app to draw attention to itself when the job ends.
    pub fn spawn_notifying(
        label: impl Into<String>,
        ctx: &eframe::egui::Context,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Self {
        Self::start(label.into(), ctx, true, work)
    }

    fn start(
        label: String,
        ctx: &eframe::egui::Context,
        notify: bool,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Self {
        let result = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&result);
        let ctx = ctx.clone();
        let run = move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|_| Err("the task crashed".into()));
            *slot.lock().expect("job lock") = Some(r);
            if notify {
                finished();
            }
            ctx.request_repaint();
        };
        std::thread::Builder::new()
            .name(label)
            .spawn(run)
            .expect("spawn worker thread");
        Job { result }
    }

    /// The result, once; `None` while still running.
    pub fn take(&self) -> Option<Result<T, String>> {
        self.result.lock().expect("job lock").take()
    }
}
