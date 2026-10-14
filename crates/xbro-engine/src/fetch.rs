//! Parallel execution of subresource loads.
//!
//! Two flavours:
//! * [`scoped_map`] runs jobs on scoped worker threads and waits for all of
//!   them (stylesheets: the page cannot be styled without them anyway, and
//!   each fetch is bounded by the fetcher's own timeout);
//! * [`detached_map`] runs jobs on detached workers and stops waiting at a
//!   deadline (images: a slow image server must not hold the page hostage;
//!   stragglers finish in the background and their results are dropped).
//!
//! Both catch panics in jobs (a job that panics yields `None`) and fall back
//! to running jobs on the calling thread when no thread can be spawned.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

/// Maximum number of concurrent subresource loads per page.
pub(crate) const MAX_WORKERS: usize = 8;

/// Which subresources a document may load: like browsers, only `file:`
/// documents may load `file:` subresources (a web page must not read local
/// files, even as images or stylesheets).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Policy {
    allow_file: bool,
}

impl Policy {
    /// The policy of a document loaded from `document_url`.
    pub(crate) fn for_document(document_url: &url::Url) -> Self {
        Self {
            allow_file: document_url.scheme() == "file",
        }
    }

    /// Why loading `url` is refused, if it is.
    pub(crate) fn refusal(&self, url: &url::Url) -> Option<&'static str> {
        (url.scheme() == "file" && !self.allow_file)
            .then_some("local files cannot be loaded by a web page")
    }
}

/// Maps `f` over `jobs` on up to `workers` threads (the calling thread
/// included) and returns the results in job order.
pub(crate) fn scoped_map<T, R, F>(jobs: &[T], workers: usize, f: F) -> Vec<Option<R>>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let n = jobs.len();
    if n == 0 {
        return Vec::new();
    }
    let results: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let work = || loop {
        let i = next.fetch_add(1, Ordering::Relaxed);
        let Some(job) = jobs.get(i) else { break };
        let result = catch_unwind(AssertUnwindSafe(|| f(job))).ok();
        if let Some(slot) = results.get(i) {
            if let Ok(mut guard) = slot.lock() {
                *guard = result;
            }
        }
    };
    let threads = workers.clamp(1, MAX_WORKERS).min(n);
    std::thread::scope(|scope| {
        for _ in 1..threads {
            // A failed spawn only reduces parallelism: the calling thread works too.
            let _ = std::thread::Builder::new()
                .name("xbro-fetch".into())
                .spawn_scoped(scope, work);
        }
        work();
    });
    results
        .into_iter()
        .map(|slot| slot.into_inner().ok().flatten())
        .collect()
}

/// Maps `f` over `jobs` on up to `workers` detached threads, collecting the
/// results that arrive before `deadline` (in job order; `None` for jobs that
/// were not finished in time). Workers stop picking new jobs after the deadline.
pub(crate) fn detached_map<T, R>(
    jobs: Vec<T>,
    workers: usize,
    deadline: Instant,
    f: Arc<dyn Fn(T) -> R + Send + Sync>,
) -> Vec<Option<R>>
where
    T: Send + 'static,
    R: Send + 'static,
{
    let n = jobs.len();
    let mut results: Vec<Option<R>> = (0..n).map(|_| None).collect();
    if n == 0 {
        return results;
    }
    let queue: Arc<Mutex<VecDeque<(usize, T)>>> =
        Arc::new(Mutex::new(jobs.into_iter().enumerate().collect()));
    let (tx, rx) = mpsc::channel::<(usize, Option<R>)>();
    let threads = workers.clamp(1, MAX_WORKERS).min(n);
    let mut spawned = 0;
    for _ in 0..threads {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        let f = Arc::clone(&f);
        let worker = move || run_queue(&queue, deadline, &*f, |i, r| tx.send((i, r)).is_ok());
        if std::thread::Builder::new()
            .name("xbro-images".into())
            .spawn(worker)
            .is_ok()
        {
            spawned += 1;
        }
    }
    drop(tx);
    if spawned == 0 {
        // No threads available: do the work here, still honouring the deadline.
        run_queue(&queue, deadline, &*f, |i, r| {
            if let Some(slot) = results.get_mut(i) {
                *slot = r;
            }
            true
        });
        return results;
    }
    let mut received = 0;
    while received < n {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match rx.recv_timeout(deadline - now) {
            Ok((i, r)) => {
                received += 1;
                if let Some(slot) = results.get_mut(i) {
                    *slot = r;
                }
            }
            // Timeout: the budget is spent. Disconnected: every worker is done.
            Err(_) => break,
        }
    }
    results
}

/// Worker loop: pops jobs until the queue is empty, the deadline passes or
/// `deliver` reports that nobody is listening any more.
fn run_queue<T, R>(
    queue: &Mutex<VecDeque<(usize, T)>>,
    deadline: Instant,
    f: &(dyn Fn(T) -> R + Send + Sync),
    mut deliver: impl FnMut(usize, Option<R>) -> bool,
) {
    loop {
        if Instant::now() >= deadline {
            break;
        }
        let job = match queue.lock() {
            Ok(mut q) => q.pop_front(),
            Err(_) => None,
        };
        let Some((i, job)) = job else { break };
        let result = catch_unwind(AssertUnwindSafe(|| f(job))).ok();
        if !deliver(i, result) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn file_subresources_need_a_file_document() {
        let url = |s: &str| url::Url::parse(s).unwrap();
        let web = Policy::for_document(&url("https://e.org/"));
        let local = Policy::for_document(&url("file:///tmp/a.html"));
        let data = Policy::for_document(&url("data:text/html,x"));
        let file = url("file:///etc/passwd");
        assert!(web.refusal(&file).is_some());
        assert!(data.refusal(&file).is_some());
        assert_eq!(local.refusal(&file), None);
        assert_eq!(web.refusal(&url("https://e.org/a.css")), None);
        assert_eq!(local.refusal(&url("https://e.org/a.png")), None);
    }

    #[test]
    fn scoped_map_keeps_order_and_survives_panics() {
        let jobs: Vec<u32> = (0..50).collect();
        let out = scoped_map(&jobs, 8, |&j| {
            if j == 7 {
                panic!("job 7 fails");
            }
            j * 2
        });
        assert_eq!(out.len(), 50);
        assert_eq!(out[7], None);
        assert_eq!(out[10], Some(20));
        assert_eq!(out[49], Some(98));
        assert!(scoped_map(&Vec::<u8>::new(), 4, |_| 0).is_empty());
    }

    #[test]
    fn detached_map_honours_the_deadline() {
        let jobs: Vec<u64> = vec![0, 1, 2, 3, 400];
        let f: Arc<dyn Fn(u64) -> u64 + Send + Sync> = Arc::new(|ms| {
            std::thread::sleep(Duration::from_millis(ms));
            ms
        });
        let start = Instant::now();
        let out = detached_map(jobs, 8, start + Duration::from_millis(150), f);
        assert!(start.elapsed() < Duration::from_millis(350));
        assert_eq!(&out[..4], &[Some(0), Some(1), Some(2), Some(3)]);
        assert_eq!(out[4], None);
    }

    #[test]
    fn detached_map_without_deadline_pressure_returns_everything() {
        let f: Arc<dyn Fn(String) -> usize + Send + Sync> = Arc::new(|s: String| s.len());
        let jobs = vec!["a".to_string(), "bb".into(), "ccc".into()];
        let out = detached_map(jobs, 2, Instant::now() + Duration::from_secs(5), f);
        assert_eq!(out, vec![Some(1), Some(2), Some(3)]);
    }
}
