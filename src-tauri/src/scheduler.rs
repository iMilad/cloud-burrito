//! Local work budgets. Limits describe app resource use, never AWS quotas.
//! Queue entries contain only opaque authority and resource-class keys.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aws_smithy_types::error::metadata::{ErrorMetadata, ProvideErrorMetadata};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::process::ProcessCancellation;

const APP_LIMIT: usize = 8;
const SERVICE_LIMIT: usize = 4;
const CLI_LIMIT: usize = 2;
const QUERY_LIMIT: usize = 2;
const CLEANUP_LIMIT: usize = 2;
const QUEUE_LIMIT: usize = 128;
const CLEANUP_QUEUE_LIMIT: usize = 2;
const PRIORITY_BURST: usize = 4;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Priority {
    Interactive,
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceKind {
    Ordinary,
    Cli,
    Query,
    Cleanup,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkBudget {
    pub deadline: Duration,
    pub operations: usize,
    pub response_bytes: usize,
}

impl WorkBudget {
    pub fn for_widget(name: &str) -> Self {
        Self {
            deadline: Duration::from_secs(match name {
                "logs-insights" => 45,
                "errors-by-stack" => 60,
                _ => 30,
            }),
            // The legacy package path remains available for the reproducible
            // baseline. Progressive list/enrich modes enforce smaller inputs.
            operations: if name == "codeartifact-packages" {
                2500
            } else {
                50
            },
            response_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Clone)]
pub(crate) struct WorkScope {
    pub authority_key: String,
    pub recovery_authority_key: String,
    pub account: String,
    pub region: String,
    pub cancellation: ProcessCancellation,
    pub deadline: Instant,
    pub budget: WorkBudget,
    pub priority: Priority,
    operations: Arc<AtomicUsize>,
}

impl WorkScope {
    pub fn new(
        authority_key: String,
        account: String,
        region: String,
        cancellation: ProcessCancellation,
        budget: WorkBudget,
    ) -> Self {
        Self {
            recovery_authority_key: authority_key.clone(),
            authority_key,
            account,
            region,
            cancellation,
            deadline: Instant::now() + budget.deadline,
            budget,
            priority: Priority::Interactive,
            operations: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_recovery_authority_key(mut self, key: String) -> Self {
        self.recovery_authority_key = key;
        self
    }

    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Cleanup is tied to the original authority but may outlive local polling.
    /// Its independent two-slot resource pool cannot be starved by normal work.
    pub fn cleanup_scope(&self) -> Self {
        Self::new(
            self.authority_key.clone(),
            self.account.clone(),
            self.region.clone(),
            ProcessCancellation::new(),
            WorkBudget {
                deadline: Duration::from_secs(10),
                operations: 2,
                response_bytes: 64 * 1024,
            },
        )
    }

    pub fn check(&self) -> Result<(), WorkFailure> {
        if self.cancellation.is_cancelled() {
            Err(WorkFailure::new("WorkCancelled"))
        } else if Instant::now() >= self.deadline {
            Err(WorkFailure::new("WorkDeadline"))
        } else {
            Ok(())
        }
    }

    pub(crate) fn count_operation(&self) -> Result<(), WorkFailure> {
        self.operations
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count < self.budget.operations).then_some(count + 1)
            })
            .map(|_| ())
            .map_err(|_| WorkFailure::new("WorkOperationLimit"))
    }
}

/// Never retain the source error, message, request ID or response body.
#[derive(Debug)]
pub(crate) struct WorkFailure {
    metadata: ErrorMetadata,
}

impl WorkFailure {
    pub fn new(code: &'static str) -> Self {
        Self {
            metadata: ErrorMetadata::builder().code(code).build(),
        }
    }
    pub fn service(error: impl ProvideErrorMetadata) -> Self {
        let code = match error.code() {
            Some(
                "AccessDenied"
                | "AccessDeniedException"
                | "UnauthorizedException"
                | "UnauthorizedOperation",
            ) => "AccessDenied",
            Some(
                "ExpiredToken"
                | "ExpiredTokenException"
                | "InvalidClientTokenId"
                | "UnrecognizedClientException",
            ) => "ExpiredToken",
            Some(
                "Throttling"
                | "ThrottlingException"
                | "TooManyRequestsException"
                | "RequestLimitExceeded",
            ) => "Throttling",
            Some("ResourceNotFound" | "ResourceNotFoundException" | "NotFoundException") => {
                "ResourceNotFound"
            }
            Some(
                "InvalidParameterException"
                | "InvalidParameterValueException"
                | "ValidationException"
                | "ValidationError"
                | "MalformedQueryException",
            ) => "ValidationException",
            Some(
                "ServiceUnavailable"
                | "ServiceUnavailableException"
                | "InternalFailure"
                | "InternalServerException"
                | "InternalServerError",
            ) => "ServiceUnavailable",
            Some("ConflictException" | "ResourceInUseException") => "ConflictException",
            _ => "WorkServiceFailure",
        };
        Self::new(code)
    }
}
impl ProvideErrorMetadata for WorkFailure {
    fn meta(&self) -> &ErrorMetadata {
        &self.metadata
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ServiceKey {
    account: String,
    region: String,
    service: String,
}
#[derive(Clone)]
struct Waiter {
    id: u64,
    service: ServiceKey,
    kind: ResourceKind,
    priority: Priority,
}

#[derive(Default, Clone, Serialize)]
pub(crate) struct Snapshot {
    pub queued: usize,
    pub ordinary: usize,
    pub cli: usize,
    pub queries: usize,
    pub cleanup: usize,
    pub peak_queued: usize,
    pub peak_ordinary: usize,
    pub peak_service: usize,
    pub peak_cli: usize,
    pub peak_queries: usize,
    pub peak_cleanup: usize,
    /// One dispatched SDK operation may contain sequential SDK retry attempts.
    pub logical_operations: usize,
    pub cancelled_queued: usize,
    pub rejected_queue: usize,
}
#[derive(Default)]
struct State {
    next_id: u64,
    queue: VecDeque<Waiter>,
    service_active: HashMap<ServiceKey, usize>,
    counters: Snapshot,
    priority_burst: usize,
}

#[derive(Default)]
pub(crate) struct Scheduler {
    state: Mutex<State>,
    changed: Notify,
}

impl State {
    fn fits(&self, waiter: &Waiter) -> bool {
        match waiter.kind {
            ResourceKind::Ordinary => {
                self.counters.ordinary < APP_LIMIT
                    && self
                        .service_active
                        .get(&waiter.service)
                        .copied()
                        .unwrap_or(0)
                        < SERVICE_LIMIT
            }
            ResourceKind::Cli => self.counters.cli < CLI_LIMIT,
            ResourceKind::Query => self.counters.queries < QUERY_LIMIT,
            ResourceKind::Cleanup => self.counters.cleanup < CLEANUP_LIMIT,
        }
    }
    fn next_eligible(&self) -> Option<u64> {
        let oldest = self.queue.iter().find(|waiter| self.fits(waiter))?;
        if self.priority_burst >= PRIORITY_BURST {
            return Some(oldest.id);
        }
        self.queue
            .iter()
            .find(|waiter| waiter.priority == Priority::Interactive && self.fits(waiter))
            .or(Some(oldest))
            .map(|waiter| waiter.id)
    }
    fn enter(&mut self, waiter: &Waiter) {
        match waiter.kind {
            ResourceKind::Ordinary => {
                self.counters.ordinary += 1;
                self.counters.logical_operations += 1;
                let active = self
                    .service_active
                    .entry(waiter.service.clone())
                    .or_default();
                *active += 1;
                self.counters.peak_service = self.counters.peak_service.max(*active);
                self.counters.peak_ordinary =
                    self.counters.peak_ordinary.max(self.counters.ordinary);
            }
            ResourceKind::Cli => {
                self.counters.cli += 1;
                self.counters.peak_cli = self.counters.peak_cli.max(self.counters.cli);
            }
            ResourceKind::Query => {
                self.counters.queries += 1;
                self.counters.peak_queries = self.counters.peak_queries.max(self.counters.queries);
            }
            ResourceKind::Cleanup => {
                self.counters.cleanup += 1;
                self.counters.peak_cleanup = self.counters.peak_cleanup.max(self.counters.cleanup);
            }
        }
    }
}

impl Scheduler {
    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock();
        let mut snapshot = state.counters.clone();
        snapshot.queued = state.queue.len();
        snapshot
    }

    pub async fn acquire(
        self: &Arc<Self>,
        scope: &WorkScope,
        service: &str,
        kind: ResourceKind,
    ) -> Result<Permit, WorkFailure> {
        scope.check()?;
        let waiter = {
            let mut state = self.state.lock();
            let pending = state
                .queue
                .iter()
                .filter(|waiter| {
                    (waiter.kind == ResourceKind::Cleanup) == (kind == ResourceKind::Cleanup)
                })
                .count();
            let queue_limit = if kind == ResourceKind::Cleanup {
                CLEANUP_QUEUE_LIMIT
            } else {
                QUEUE_LIMIT
            };
            if pending >= queue_limit {
                state.counters.rejected_queue += 1;
                return Err(WorkFailure::new("WorkQueueFull"));
            }
            state.next_id += 1;
            let waiter = Waiter {
                id: state.next_id,
                service: ServiceKey {
                    account: scope.account.clone(),
                    region: scope.region.clone(),
                    service: service.into(),
                },
                kind,
                priority: scope.priority,
            };
            state.queue.push_back(waiter.clone());
            state.counters.peak_queued = state.counters.peak_queued.max(state.queue.len());
            waiter
        };
        let mut ticket = QueueTicket {
            scheduler: self.clone(),
            id: Some(waiter.id),
        };
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            scope.check()?;
            let acquired = {
                let mut state = self.state.lock();
                if state.next_eligible() == Some(waiter.id) {
                    let index = state
                        .queue
                        .iter()
                        .position(|pending| pending.id == waiter.id)
                        .expect("owned queue ticket");
                    let older_background = state.queue.iter().take(index).any(|pending| {
                        pending.priority == Priority::Background && state.fits(pending)
                    });
                    state.priority_burst = if older_background {
                        state.priority_burst + 1
                    } else {
                        0
                    };
                    state.queue.remove(index);
                    state.enter(&waiter);
                    true
                } else {
                    false
                }
            };
            if acquired {
                ticket.id = None;
                self.changed.notify_waiters();
                return Ok(Permit {
                    scheduler: self.clone(),
                    waiter,
                });
            }
            tokio::select! {
                biased;
                _ = scope.cancellation.cancelled() => return Err(WorkFailure::new("WorkCancelled")),
                _ = tokio::time::sleep_until(scope.deadline) => return Err(WorkFailure::new("WorkDeadline")),
                _ = &mut notified => {},
            }
        }
    }

    /// The future is not polled until both resource ceilings were acquired.
    /// A permit spans the SDK's bounded sequential retries and is released on
    /// error, timeout, cancellation or future drop.
    pub async fn run<T, F>(
        self: &Arc<Self>,
        scope: &WorkScope,
        service: &str,
        kind: ResourceKind,
        future: F,
    ) -> Result<T, WorkFailure>
    where
        F: Future<Output = T>,
    {
        scope.count_operation()?;
        let _permit = self.acquire(scope, service, kind).await?;
        scope.check()?;
        let deadline = scope.deadline.min(Instant::now() + OPERATION_TIMEOUT);
        tokio::select! {
            biased;
            _ = scope.cancellation.cancelled() => Err(WorkFailure::new("WorkCancelled")),
            _ = tokio::time::sleep_until(deadline) => Err(WorkFailure::new("WorkDeadline")),
            output = future => Ok(output),
        }
    }
}

struct QueueTicket {
    scheduler: Arc<Scheduler>,
    id: Option<u64>,
}
impl Drop for QueueTicket {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            let mut state = self.scheduler.state.lock();
            if let Some(index) = state.queue.iter().position(|waiter| waiter.id == id) {
                state.queue.remove(index);
                state.counters.cancelled_queued += 1;
            }
            drop(state);
            self.scheduler.changed.notify_waiters();
        }
    }
}
pub(crate) struct Permit {
    scheduler: Arc<Scheduler>,
    waiter: Waiter,
}
impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self.scheduler.state.lock();
        match self.waiter.kind {
            ResourceKind::Ordinary => {
                state.counters.ordinary -= 1;
                if let Some(active) = state.service_active.get_mut(&self.waiter.service) {
                    *active -= 1;
                    if *active == 0 {
                        state.service_active.remove(&self.waiter.service);
                    }
                }
            }
            ResourceKind::Cli => state.counters.cli -= 1,
            ResourceKind::Query => state.counters.queries -= 1,
            ResourceKind::Cleanup => state.counters.cleanup -= 1,
        }
        drop(state);
        self.scheduler.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope(account: &str) -> WorkScope {
        WorkScope::new(
            format!("synthetic-{account}"),
            account.into(),
            "fixture-region".into(),
            ProcessCancellation::new(),
            WorkBudget::for_widget("fixture"),
        )
    }
    #[tokio::test]
    async fn fifty_pins_share_atomic_app_and_service_budgets() {
        let scheduler = Arc::new(Scheduler::default());
        let futures = (0..50).map(|index| {
            let scheduler = scheduler.clone();
            async move {
                let scope = scope(if index % 2 == 0 { "demo-a" } else { "demo-b" });
                scheduler
                    .run(
                        &scope,
                        "logs",
                        ResourceKind::Ordinary,
                        tokio::time::sleep(Duration::from_millis(2)),
                    )
                    .await
                    .unwrap();
            }
        });
        futures::future::join_all(futures).await;
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.peak_ordinary, APP_LIMIT);
        assert_eq!(snapshot.peak_service, SERVICE_LIMIT);
        assert_eq!(snapshot.logical_operations, 50);
        assert_eq!((snapshot.ordinary, snapshot.queued), (0, 0));
    }
    #[tokio::test]
    async fn queue_cancel_and_drop_never_poll_work_and_recover_slots() {
        let scheduler = Arc::new(Scheduler::default());
        let blocker = scope("demo-a");
        let mut permits = Vec::new();
        for _ in 0..SERVICE_LIMIT {
            permits.push(
                scheduler
                    .acquire(&blocker, "logs", ResourceKind::Ordinary)
                    .await
                    .unwrap(),
            );
        }
        let waiting = scope("demo-a");
        let polled = AtomicBool::new(false);
        let run = scheduler.run(&waiting, "logs", ResourceKind::Ordinary, async {
            polled.store(true, Ordering::SeqCst);
        });
        tokio::pin!(run);
        assert!(futures::poll!(&mut run).is_pending());
        assert_eq!(scheduler.snapshot().queued, 1);
        waiting.cancellation.cancel();
        assert_eq!(run.await.unwrap_err().code(), Some("WorkCancelled"));
        assert!(!polled.load(Ordering::SeqCst));
        assert_eq!(scheduler.snapshot().queued, 0);
        drop(permits);
        assert_eq!(scheduler.snapshot().ordinary, 0);
    }
    use std::sync::atomic::AtomicBool;
    #[tokio::test]
    async fn queue_bound_and_reserved_cleanup_are_independent() {
        let scheduler = Arc::new(Scheduler::default());
        let scope = scope("demo-a");
        let mut permits = Vec::new();
        for _ in 0..SERVICE_LIMIT {
            permits.push(
                scheduler
                    .acquire(&scope, "logs", ResourceKind::Ordinary)
                    .await
                    .unwrap(),
            );
        }
        let mut queued = Vec::new();
        for _ in 0..QUEUE_LIMIT {
            let mut future = Box::pin(scheduler.acquire(&scope, "logs", ResourceKind::Ordinary));
            assert!(futures::poll!(&mut future).is_pending());
            queued.push(future);
        }
        assert_eq!(
            scheduler
                .acquire(&scope, "logs", ResourceKind::Ordinary)
                .await
                .err()
                .unwrap()
                .code(),
            Some("WorkQueueFull")
        );
        // Both execution capacity and two pending cleanup tickets are reserved.
        let cleanup = scheduler
            .acquire(&scope.cleanup_scope(), "logs", ResourceKind::Cleanup)
            .await
            .unwrap();
        assert_eq!(scheduler.snapshot().cleanup, 1);
        drop(cleanup);
        drop(queued);
        drop(permits);
        assert_eq!(
            (scheduler.snapshot().queued, scheduler.snapshot().ordinary),
            (0, 0)
        );
    }
    #[tokio::test]
    async fn operation_budget_and_deadline_are_total_and_recover_permits() {
        let scheduler = Arc::new(Scheduler::default());
        let mut scope = scope("demo-a");
        scope.budget.operations = 1;
        scheduler
            .run(&scope, "logs", ResourceKind::Ordinary, async {})
            .await
            .unwrap();
        assert_eq!(
            scheduler
                .run(&scope, "logs", ResourceKind::Ordinary, async {})
                .await
                .unwrap_err()
                .code(),
            Some("WorkOperationLimit")
        );
        let mut timed = scope.clone();
        timed.operations = Arc::new(AtomicUsize::new(0));
        timed.deadline = Instant::now() + Duration::from_millis(2);
        assert_eq!(
            scheduler
                .run(
                    &timed,
                    "logs",
                    ResourceKind::Ordinary,
                    std::future::pending::<()>()
                )
                .await
                .unwrap_err()
                .code(),
            Some("WorkDeadline")
        );
        assert_eq!(scheduler.snapshot().ordinary, 0);
    }
    #[tokio::test]
    async fn cli_query_and_cleanup_have_two_slots_each() {
        let scheduler = Arc::new(Scheduler::default());
        let scope = scope("demo-a");
        for kind in [
            ResourceKind::Cli,
            ResourceKind::Query,
            ResourceKind::Cleanup,
        ] {
            let first = scheduler.acquire(&scope, "logs", kind).await.unwrap();
            let second = scheduler.acquire(&scope, "logs", kind).await.unwrap();
            let mut third = Box::pin(scheduler.acquire(&scope, "logs", kind));
            assert!(futures::poll!(&mut third).is_pending());
            drop(first);
            let third = third.await.unwrap();
            drop(second);
            drop(third);
        }
        let snapshot = scheduler.snapshot();
        assert_eq!(
            (
                snapshot.peak_cli,
                snapshot.peak_queries,
                snapshot.peak_cleanup
            ),
            (2, 2, 2)
        );
        assert_eq!(
            (snapshot.cli, snapshot.queries, snapshot.cleanup),
            (0, 0, 0)
        );
    }
    #[tokio::test]
    async fn interactive_priority_cannot_starve_older_background_work() {
        let scheduler = Arc::new(Scheduler::default());
        let scope = scope("demo-a");
        let first = scheduler
            .acquire(&scope, "cli", ResourceKind::Cli)
            .await
            .unwrap();
        let second = scheduler
            .acquire(&scope, "cli", ResourceKind::Cli)
            .await
            .unwrap();
        let background = scope.clone().with_priority(Priority::Background);
        let mut oldest = Box::pin(scheduler.acquire(&background, "cli", ResourceKind::Cli));
        assert!(futures::poll!(&mut oldest).is_pending());
        let mut interactive = (0..6)
            .map(|_| Box::pin(scheduler.acquire(&scope, "cli", ResourceKind::Cli)))
            .collect::<Vec<_>>();
        for future in &mut interactive {
            assert!(futures::poll!(future).is_pending());
        }
        drop(first);
        for future in interactive.iter_mut().take(PRIORITY_BURST) {
            drop(future.await.unwrap());
        }
        let permitted = oldest.await.unwrap();
        drop(permitted);
        drop(interactive);
        drop(second);
        assert_eq!(scheduler.snapshot().queued, 0);
    }
}
