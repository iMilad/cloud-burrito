//! Bounded request ownership and in-flight sharing. Keys never enter diagnostics.
use crate::process::ProcessCancellation;
use futures::FutureExt;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

const CAPACITY: usize = 128;
const TOMBSTONES: usize = 256;
const DISPLAY_DEADLINE: Duration = Duration::from_secs(85);

#[derive(Default)]
pub(crate) struct WorkRegistry {
    state: Mutex<RegistryState>,
}
#[derive(Default)]
struct RegistryState {
    requests: HashMap<String, ProcessCancellation>,
    cancelled: HashMap<String, Instant>,
    jobs: HashMap<String, Arc<Job>>,
}
struct Job {
    cancel: ProcessCancellation,
    subscribers: AtomicUsize,
    result: watch::Sender<Option<Arc<Value>>>,
}
pub(crate) struct Registration {
    registry: Arc<WorkRegistry>,
    id: String,
    pub(crate) cancellation: ProcessCancellation,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.registry.state.lock().requests.remove(&self.id);
    }
}
struct Subscription {
    registry: Arc<WorkRegistry>,
    job: Arc<Job>,
    attached: bool,
}
impl Subscription {
    /// Joining and last-subscriber cancellation use the same mutex. A new
    /// subscriber either joins before this decrement or sees a cancelled job.
    fn detach(&mut self) -> bool {
        if !self.attached {
            return false;
        }
        let _state = self.registry.state.lock();
        self.attached = false;
        let last = self.job.subscribers.fetch_sub(1, Ordering::SeqCst) == 1;
        if last {
            self.job.cancel.cancel();
        }
        last
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.detach();
    }
}

pub(crate) fn cancelled_result() -> Value {
    json!({"render":"raw_json","ok":false,"error_type":"Cancelled","data":{"error":"Request cancelled locally"},
        "cleanup":{"status":"pending","message":"Cancellation was requested. Remote cleanup, when applicable, is still owned by the original request."}})
}
fn busy_result() -> Value {
    json!({"render":"raw_json","ok":false,"error_type":"QueueFull","data":{"error":"The request queue is full. Try again after current work finishes."}})
}

impl WorkRegistry {
    pub(crate) fn register(self: &Arc<Self>, id: String) -> Result<Registration, Value> {
        let mut state = self.state.lock();
        state
            .cancelled
            .retain(|_, at| at.elapsed() < Duration::from_secs(60));
        if state.requests.len() >= CAPACITY || state.requests.contains_key(&id) {
            return Err(busy_result());
        }
        let cancellation = ProcessCancellation::new();
        if state.cancelled.remove(&id).is_some() {
            cancellation.cancel();
        }
        state.requests.insert(id.clone(), cancellation.clone());
        Ok(Registration {
            registry: self.clone(),
            id,
            cancellation,
        })
    }

    /// Includes bounded short-lived tombstones for cancellation overtaking dispatch.
    pub(crate) fn cancel(&self, id: &str) {
        let mut state = self.state.lock();
        if let Some(token) = state.requests.get(id) {
            token.cancel();
            return;
        }
        state
            .cancelled
            .retain(|_, at| at.elapsed() < Duration::from_secs(60));
        if state.cancelled.len() >= TOMBSTONES {
            if let Some(oldest) = state
                .cancelled
                .iter()
                .min_by_key(|(_, at)| *at)
                .map(|(id, _)| id.clone())
            {
                state.cancelled.remove(&oldest);
            }
        }
        state.cancelled.insert(id.into(), Instant::now());
    }

    pub(crate) fn cancel_all(&self) {
        let state = self.state.lock();
        for cancel in state.requests.values() {
            cancel.cancel();
        }
        for job in state.jobs.values() {
            job.cancel.cancel();
        }
    }

    pub(crate) fn cancel_jobs(&self) {
        for job in self.state.lock().jobs.values() {
            job.cancel.cancel();
        }
    }

    pub(crate) async fn coalesce<F, Fut>(
        self: &Arc<Self>,
        key: String,
        subscriber: ProcessCancellation,
        make: F,
    ) -> Value
    where
        F: FnOnce(ProcessCancellation) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Value> + Send + 'static,
    {
        if subscriber.is_cancelled() {
            return cancelled_result();
        }
        let (job, launch) = {
            let mut state = self.state.lock();
            if let Some(job) = state.jobs.get(&key) {
                if !job.cancel.is_cancelled() {
                    job.subscribers.fetch_add(1, Ordering::SeqCst);
                    (job.clone(), false)
                } else {
                    return busy_result();
                }
            } else {
                if state.jobs.len() >= CAPACITY {
                    return busy_result();
                }
                let job = Arc::new(Job {
                    cancel: ProcessCancellation::new(),
                    subscribers: AtomicUsize::new(1),
                    result: watch::channel(None).0,
                });
                state.jobs.insert(key.clone(), job.clone());
                (job, true)
            }
        };
        let mut subscription = Subscription {
            registry: self.clone(),
            job: job.clone(),
            attached: true,
        };
        let mut receiver = job.result.subscribe();
        if launch {
            let registry = self.clone();
            let worker = job.clone();
            tokio::spawn(async move {
                let result = std::panic::AssertUnwindSafe(make(worker.cancel.clone())).catch_unwind().await
                    .unwrap_or_else(|_| json!({"render":"raw_json","ok":false,"error_type":"WorkerFailed","data":{"error":"The request worker failed."}}));
                worker.result.send_replace(Some(Arc::new(result)));
                let mut state = registry.state.lock();
                if state
                    .jobs
                    .get(&key)
                    .is_some_and(|current| Arc::ptr_eq(current, &worker))
                {
                    state.jobs.remove(&key);
                }
            });
        }
        let mut cancelled_last = false;
        let display_deadline = tokio::time::Instant::now() + DISPLAY_DEADLINE;
        loop {
            if let Some(value) = receiver.borrow_and_update().as_ref() {
                return value.as_ref().clone();
            }
            tokio::select! {
                biased;
                _ = subscriber.cancelled(), if !cancelled_last => {
                    if subscription.detach() {
                        cancelled_last = true;
                    } else {
                        let mut result = cancelled_result();
                        result["cleanup"] = json!({"status":"not_requested","message":"This subscriber detached. Shared work is still needed by another request."});
                        return result;
                    }
                },
                _ = tokio::time::sleep_until(display_deadline) => {
                    let last = subscription.detach() || cancelled_last;
                    let mut result = cancelled_result();
                    result["error_type"] = json!("WorkDeadline");
                    result["data"] = json!({"error":"The local display deadline was reached."});
                    result["cleanup"] = if last {
                        json!({"status":"unknown","message":"Cleanup has not completed within the display deadline. The original worker still owns it."})
                    } else {
                        json!({"status":"not_requested","message":"This subscriber detached at its display deadline. Shared work is still needed by another request."})
                    };
                    return result;
                },
                changed = receiver.changed() => if changed.is_err() { return busy_result(); },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn identical_work_is_shared_and_one_cancel_does_not_cancel_other_subscribers() {
        let registry = Arc::new(WorkRegistry::default());
        let one = registry.register("one".into()).unwrap();
        let two = registry.register("two".into()).unwrap();
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let first = tokio::spawn({
            let registry = registry.clone();
            let token = one.cancellation.clone();
            let started = started.clone();
            let release = release.clone();
            async move {
                registry
                    .coalesce(
                        "verified-a:operation".into(),
                        token,
                        move |cancel| async move {
                            started.notify_one();
                            release.notified().await;
                            assert!(!cancel.is_cancelled());
                            json!({"ok":true,"synthetic":1})
                        },
                    )
                    .await
            }
        });
        started.notified().await;
        let second = tokio::spawn({
            let registry = registry.clone();
            let token = two.cancellation.clone();
            async move {
                registry
                    .coalesce("verified-a:operation".into(), token, |_| async {
                        panic!("duplicate operation dispatched")
                    })
                    .await
            }
        });
        while registry
            .state
            .lock()
            .jobs
            .values()
            .next()
            .unwrap()
            .subscribers
            .load(Ordering::SeqCst)
            != 2
        {
            tokio::task::yield_now().await;
        }
        registry.cancel("one");
        assert_eq!(first.await.unwrap()["error_type"], "Cancelled");
        release.notify_one();
        assert_eq!(second.await.unwrap()["synthetic"], 1);
        drop((one, two));
        assert!(registry.state.lock().requests.is_empty());
    }
    #[tokio::test]
    async fn cancel_before_register_never_dispatches_and_capacity_is_recovered() {
        let registry = Arc::new(WorkRegistry::default());
        registry.cancel("late");
        let late = registry.register("late".into()).unwrap();
        let result = registry
            .coalesce("key".into(), late.cancellation.clone(), |_| async {
                panic!("cancelled work dispatched")
            })
            .await;
        assert_eq!(result["error_type"], "Cancelled");
        drop(late);
        let requests: Vec<_> = (0..CAPACITY)
            .map(|i| registry.register(format!("synthetic-{i}")).unwrap())
            .collect();
        assert!(registry.register("overflow".into()).is_err());
        drop(requests);
        assert!(registry.register("recovered".into()).is_ok());
    }
    #[tokio::test]
    async fn distinct_authority_keys_never_share_and_last_subscriber_cancels_worker() {
        let registry = Arc::new(WorkRegistry::default());
        let a = registry.register("a".into()).unwrap();
        let b = registry.register("b".into()).unwrap();
        let started = Arc::new(tokio::sync::Notify::new());
        let stopped = Arc::new(tokio::sync::Notify::new());
        let first = tokio::spawn({
            let registry = registry.clone();
            let token = a.cancellation.clone();
            let started = started.clone();
            let stopped = stopped.clone();
            async move {
                registry
                    .coalesce("verified-a".into(), token, move |cancel| async move {
                        started.notify_one();
                        cancel.cancelled().await;
                        stopped.notify_one();
                        cancelled_result()
                    })
                    .await
            }
        });
        started.notified().await;
        assert_eq!(
            registry
                .coalesce("verified-b".into(), b.cancellation.clone(), |_| async {
                    json!({"identity":"b"})
                })
                .await["identity"],
            "b"
        );
        registry.cancel("a");
        first.await.unwrap();
        stopped.notified().await;
    }
    #[tokio::test]
    async fn both_cancels_detach_once_and_only_the_last_subscriber_waits_for_cleanup() {
        let registry = Arc::new(WorkRegistry::default());
        let one = registry.register("first-cancel".into()).unwrap();
        let two = registry.register("second-cancel".into()).unwrap();
        let release = Arc::new(tokio::sync::Notify::new());
        let worker_release = release.clone();
        let first=registry.coalesce("shared-cleanup".into(),one.cancellation.clone(),move|_|async move {
            worker_release.notified().await;
            json!({"ok":false,"error_type":"CliCleanupFailed","error":"Synthetic child exit was not confirmed"})
        });
        tokio::pin!(first);
        assert!(futures::poll!(&mut first).is_pending());
        let second = registry.coalesce(
            "shared-cleanup".into(),
            two.cancellation.clone(),
            |_| async { panic!("duplicate worker") },
        );
        tokio::pin!(second);
        assert!(futures::poll!(&mut second).is_pending());
        let job = registry.state.lock().jobs["shared-cleanup"].clone();
        assert_eq!(job.subscribers.load(Ordering::SeqCst), 2);
        registry.cancel("first-cancel");
        registry.cancel("second-cancel");
        assert_eq!(first.await["cleanup"]["status"], "not_requested");
        assert!(futures::poll!(&mut second).is_pending());
        assert_eq!(job.subscribers.load(Ordering::SeqCst), 0);
        assert!(job.cancel.is_cancelled());
        release.notify_one();
        assert_eq!(second.await["error_type"], "CliCleanupFailed");
        assert_eq!(job.subscribers.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn concurrent_join_and_last_detach_never_cancel_work_with_a_live_joiner() {
        for _ in 0..128 {
            let registry = Arc::new(WorkRegistry::default());
            let job = Arc::new(Job {
                cancel: ProcessCancellation::new(),
                subscribers: AtomicUsize::new(1),
                result: watch::channel(None).0,
            });
            registry
                .state
                .lock()
                .jobs
                .insert("race".into(), job.clone());
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let joiner = std::thread::spawn({
                let registry = registry.clone();
                let job = job.clone();
                let barrier = barrier.clone();
                move || {
                    barrier.wait();
                    let _state = registry.state.lock();
                    if job.cancel.is_cancelled() {
                        return false;
                    }
                    job.subscribers.fetch_add(1, Ordering::SeqCst);
                    true
                }
            });
            let mut departing = Subscription {
                registry: registry.clone(),
                job: job.clone(),
                attached: true,
            };
            barrier.wait();
            let last = departing.detach();
            let joined = joiner.join().unwrap();
            assert_eq!(last, !joined);
            assert_eq!(job.cancel.is_cancelled(), !joined);
            assert_eq!(job.subscribers.load(Ordering::SeqCst), usize::from(joined));
            drop(departing);
            assert_eq!(job.subscribers.load(Ordering::SeqCst), usize::from(joined));
            if joined {
                let mut remaining = Subscription {
                    registry: registry.clone(),
                    job: job.clone(),
                    attached: true,
                };
                assert!(remaining.detach());
                assert!(job.cancel.is_cancelled());
            }
        }
    }
}
