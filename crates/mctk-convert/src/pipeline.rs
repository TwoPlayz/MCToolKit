//! Staged, multi-threaded chunk pipeline.
//!
//! UMT's `AreaProcessor` converts a world by turning every chunk into a series of small
//! tasks, one per stage, that run on a shared thread pool: Load, Parse, Prepare, Light,
//! Render, Convert and Clean (observed behaviour of `AreaProcessor::dispatchNextTask`).
//! Each time a stage finishes, the chunk's next stage becomes ready; when the user cancels,
//! the queue is cleared and the run ends as "Conversion Stopped".
//!
//! This module provides the same shape: a [`ChunkStages`] implementation supplies the work
//! of each stage and [`run_pipeline`] schedules the stages over a pool of worker threads.
//! Stages a converter has no use for keep their default no-op. Later stages are preferred
//! over starting new chunks, which keeps the number of chunks in flight (and so memory)
//! small.

use std::collections::BinaryHeap;
use std::sync::atomic::Ordering;
use std::sync::{Condvar, Mutex};

use crate::Error;
use crate::options::Progress;

/// One step of a chunk's life, in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    /// Read the chunk's stored bytes.
    Load = 1,
    /// Decode the bytes into the source edition's chunk structure.
    Parse = 2,
    /// Gather everything the later stages need (neighbours, lookup data).
    Prepare = 3,
    /// Recompute lighting.
    Light = 4,
    /// Draw the chunk for the live preview.
    Render = 5,
    /// Produce the output chunk.
    Convert = 6,
    /// Release the chunk's working memory and hand over its result.
    Clean = 7,
}

impl Stage {
    const ALL: [Stage; 7] = [
        Stage::Load,
        Stage::Parse,
        Stage::Prepare,
        Stage::Light,
        Stage::Render,
        Stage::Convert,
        Stage::Clean,
    ];

    fn next(self) -> Option<Stage> {
        Stage::ALL.get(self as usize).copied()
    }
}

/// What a stage tells the scheduler about its chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Go on with the next stage.
    Continue,
    /// The chunk cannot be converted: count it as invalid and skip to [`Stage::Clean`].
    Invalid,
    /// The chunk is deliberately left out (not counted as invalid): skip to [`Stage::Clean`].
    Skip,
}

/// The per-stage work of one conversion. `index` identifies the chunk (0-based, in the
/// order the caller listed them); `slot` carries the chunk's data from stage to stage.
pub trait ChunkStages: Sync {
    /// Working data of one chunk, empty until [`Stage::Load`] fills it.
    type Slot: Default + Send;
    /// What a finished chunk produces (collected by [`run_pipeline`]).
    type Output: Send;

    fn load(&self, _index: usize, _slot: &mut Self::Slot) -> Result<Flow, Error> {
        Ok(Flow::Continue)
    }
    fn parse(&self, _index: usize, _slot: &mut Self::Slot) -> Result<Flow, Error> {
        Ok(Flow::Continue)
    }
    fn prepare(&self, _index: usize, _slot: &mut Self::Slot) -> Result<Flow, Error> {
        Ok(Flow::Continue)
    }
    fn light(&self, _index: usize, _slot: &mut Self::Slot) -> Result<Flow, Error> {
        Ok(Flow::Continue)
    }
    fn render(&self, _index: usize, _slot: &mut Self::Slot) -> Result<Flow, Error> {
        Ok(Flow::Continue)
    }
    fn convert(&self, index: usize, slot: &mut Self::Slot) -> Result<Flow, Error>;
    /// Drops the slot's data and returns the chunk's result, if it has one (an invalid
    /// chunk, or one the converter chose to leave out, returns `None`).
    fn clean(&self, index: usize, slot: &mut Self::Slot) -> Option<Self::Output>;
}

struct Queue {
    /// Ready tasks; the heap pops the highest stage first, then the lowest chunk index.
    ready: BinaryHeap<(Stage, std::cmp::Reverse<usize>)>,
    next_chunk: usize,
    finished: usize,
    failed: Option<Error>,
}

/// Runs every chunk (`0..count`) through the stages on `threads` workers (`0` = one per
/// core) and returns the results in chunk order. Stops early on the first error or when
/// `progress` is cancelled.
pub fn run_pipeline<S: ChunkStages>(
    stages: &S,
    count: usize,
    threads: usize,
    progress: &Progress,
) -> Result<Vec<Option<S::Output>>, Error> {
    let threads = if threads == 0 {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        threads
    }
    .min(count.max(1));

    let slots: Vec<Mutex<S::Slot>> = (0..count).map(|_| Mutex::default()).collect();
    let results: Vec<Mutex<Option<S::Output>>> = (0..count).map(|_| Mutex::new(None)).collect();
    let queue = Mutex::new(Queue {
        ready: BinaryHeap::new(),
        next_chunk: 0,
        finished: 0,
        failed: None,
    });
    let wake = Condvar::new();

    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    // Take the next task, or wait until one appears or the run is over.
                    let (index, stage) = {
                        let mut q = queue.lock().expect("pipeline queue");
                        loop {
                            if q.failed.is_none() && progress.is_cancelled() {
                                q.failed = Some(Error::Invalid("conversion stopped".into()));
                            }
                            if q.failed.is_some() || q.finished == count {
                                wake.notify_all();
                                return;
                            }
                            if let Some((stage, std::cmp::Reverse(index))) = q.ready.pop() {
                                break (index, stage);
                            }
                            if q.next_chunk < count {
                                q.next_chunk += 1;
                                break (q.next_chunk - 1, Stage::Load);
                            }
                            q = wake.wait(q).expect("pipeline queue");
                        }
                    };

                    let mut slot = slots[index].lock().expect("chunk slot");
                    let outcome = match stage {
                        Stage::Load => stages.load(index, &mut slot),
                        Stage::Parse => stages.parse(index, &mut slot),
                        Stage::Prepare => stages.prepare(index, &mut slot),
                        Stage::Light => stages.light(index, &mut slot),
                        Stage::Render => stages.render(index, &mut slot),
                        Stage::Convert => stages.convert(index, &mut slot),
                        Stage::Clean => {
                            *results[index].lock().expect("chunk result") =
                                stages.clean(index, &mut slot);
                            Ok(Flow::Continue)
                        }
                    };
                    drop(slot);

                    let mut q = queue.lock().expect("pipeline queue");
                    match outcome {
                        Err(e) => {
                            q.failed.get_or_insert(e);
                        }
                        Ok(flow) => {
                            let after = match (flow, stage.next()) {
                                (_, None) => None,
                                (Flow::Invalid, Some(_)) => {
                                    progress.invalid_chunks.fetch_add(1, Ordering::Relaxed);
                                    Some(Stage::Clean)
                                }
                                (Flow::Skip, Some(_)) => Some(Stage::Clean),
                                (Flow::Continue, next) => next,
                            };
                            match after {
                                Some(next) => q.ready.push((next, std::cmp::Reverse(index))),
                                None => q.finished += 1,
                            }
                        }
                    }
                    wake.notify_all();
                }
            });
        }
    });

    let q = queue.into_inner().expect("pipeline queue");
    if let Some(e) = q.failed {
        return Err(e);
    }
    Ok(results
        .into_iter()
        .map(|r| r.into_inner().expect("chunk result"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// Records each chunk's stage trail; chunk 3 is invalid; Light/Render left at default.
    struct Trail {
        parsed: AtomicUsize,
        fail_at: Option<usize>,
    }

    impl ChunkStages for Trail {
        type Slot = Vec<Stage>;
        type Output = Vec<Stage>;

        fn load(&self, _: usize, s: &mut Vec<Stage>) -> Result<Flow, Error> {
            s.push(Stage::Load);
            Ok(Flow::Continue)
        }
        fn parse(&self, i: usize, s: &mut Vec<Stage>) -> Result<Flow, Error> {
            self.parsed.fetch_add(1, Ordering::Relaxed);
            if self.fail_at == Some(i) {
                return Err(Error::Invalid("boom".into()));
            }
            s.push(Stage::Parse);
            Ok(if i == 3 { Flow::Invalid } else { Flow::Continue })
        }
        fn convert(&self, _: usize, s: &mut Vec<Stage>) -> Result<Flow, Error> {
            s.push(Stage::Convert);
            Ok(Flow::Continue)
        }
        fn clean(&self, _: usize, s: &mut Vec<Stage>) -> Option<Vec<Stage>> {
            let trail = std::mem::take(s);
            (trail.last() == Some(&Stage::Convert)).then_some(trail)
        }
    }

    #[test]
    fn stages_run_in_order_and_results_keep_chunk_order() {
        let t = Trail {
            parsed: AtomicUsize::new(0),
            fail_at: None,
        };
        let progress = Progress::default();
        let out = run_pipeline(&t, 40, 4, &progress).unwrap();
        assert_eq!(out.len(), 40);
        for (i, r) in out.iter().enumerate() {
            if i == 3 {
                assert!(r.is_none());
            } else {
                assert_eq!(
                    r.as_deref(),
                    Some(&[Stage::Load, Stage::Parse, Stage::Convert][..])
                );
            }
        }
        assert_eq!(progress.invalid_chunks.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn first_error_stops_the_run() {
        let t = Trail {
            parsed: AtomicUsize::new(0),
            fail_at: Some(5),
        };
        let err = run_pipeline(&t, 1000, 2, &Progress::default()).unwrap_err();
        assert!(err.to_string().contains("boom"));
        assert!(t.parsed.load(Ordering::Relaxed) < 1000);
    }

    #[test]
    fn cancelling_before_the_run_does_no_work() {
        let t = Trail {
            parsed: AtomicUsize::new(0),
            fail_at: None,
        };
        let progress = Progress::default();
        progress.cancel();
        let err = run_pipeline(&t, 100, 4, &progress).unwrap_err();
        assert!(err.to_string().contains("conversion stopped"));
        assert_eq!(t.parsed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn empty_input_is_fine() {
        let t = Trail {
            parsed: AtomicUsize::new(0),
            fail_at: None,
        };
        assert!(run_pipeline(&t, 0, 0, &Progress::default()).unwrap().is_empty());
    }
}
