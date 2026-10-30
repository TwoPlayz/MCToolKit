//! Background work: conversions, pruning and map rendering run on their own thread and
//! wake the UI when done.

use std::sync::{Arc, Mutex};

pub struct Job<T> {
    result: Arc<Mutex<Option<Result<T, String>>>>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(
        label: impl Into<String>,
        ctx: &eframe::egui::Context,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Self {
        let result = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&result);
        let ctx = ctx.clone();
        let run = move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|_| Err("the task crashed".into()));
            *slot.lock().expect("job lock") = Some(r);
            ctx.request_repaint();
        };
        std::thread::Builder::new()
            .name(label.into())
            .spawn(run)
            .expect("spawn worker thread");
        Job { result }
    }

    /// The result, once; `None` while still running.
    pub fn take(&self) -> Option<Result<T, String>> {
        self.result.lock().expect("job lock").take()
    }
}
