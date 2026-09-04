//! Shared Logs Insights lifecycle. Local cancellation is not remote completion.
//! Query text, IDs and registry keys are never written to diagnostics or audit.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aws_sdk_cloudwatchlogs::operation::get_query_results::GetQueryResultsOutput;
use aws_sdk_cloudwatchlogs::types::QueryStatus;
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use parking_lot::Mutex;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use tokio::time::Instant;

use super::{coverage::Coverage, WidgetCtx};
use crate::scheduler::{ResourceKind, WorkFailure, WorkScope};

const REGISTRY_LIMIT: usize = 128;

pub(crate) struct QuerySpec<'a> {
    pub group: &'a str,
    pub query: &'a str,
    pub start: i64,
    pub end: i64,
    pub limit: i32,
}

#[derive(Clone, Copy)]
pub(crate) struct QueryTiming {
    pub start: Duration,
    pub poll_interval: Duration,
    pub total: Duration,
    pub max_polls: usize,
}
impl Default for QueryTiming {
    fn default() -> Self {
        Self {
            start: Duration::from_secs(10),
            poll_interval: Duration::from_millis(500),
            total: Duration::from_secs(45),
            max_polls: 50,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegistryState {
    Active,
    Unknown,
}
struct RegistryEntry {
    state: RegistryState,
    // A possibly running remote query keeps local capacity until deliberate
    // acknowledgement; automatic retries cannot silently accumulate scans.
    _unresolved_permit: Option<crate::scheduler::Permit>,
    authority: String,
    summary: Value,
}
#[derive(Default)]
pub(crate) struct QueryRegistry {
    entries: Mutex<HashMap<String, RegistryEntry>>,
    next_token: AtomicU64,
}

impl QueryRegistry {
    fn begin_for(
        self: &Arc<Self>,
        scope: &WorkScope,
        widget: &str,
        spec: &QuerySpec<'_>,
        acknowledge: bool,
        token: Option<&str>,
    ) -> Result<QueryGuard, &'static str> {
        if let Some(token) = token {
            if !acknowledge {
                return Err("recovery_mismatch");
            }
            self.acknowledge_selected(scope, token)?;
        }
        let key = registry_key(scope, widget, spec);
        let guard = self.begin(key.clone(), acknowledge && token.is_none())?;
        if let Some(entry) = self.entries.lock().get_mut(&key) {
            entry.authority = scope.recovery_authority_key.clone();
            entry.summary["group"] = json!(spec.group);
            entry.summary["window_seconds"] = json!(spec.end.saturating_sub(spec.start));
            entry.summary["query_digest"] = json!(hex::encode(Sha1::digest(spec.query.as_bytes())));
        }
        Ok(guard)
    }
    pub(crate) fn acknowledge_selected(
        &self,
        scope: &WorkScope,
        token: &str,
    ) -> Result<(), &'static str> {
        let mut entries = self.entries.lock();
        let key = entries
            .iter()
            .find(|(_, entry)| {
                entry.state == RegistryState::Unknown
                    && entry.authority == scope.recovery_authority_key
                    && entry.summary["token"].as_str() == Some(token)
            })
            .map(|(key, _)| key.clone())
            .ok_or("recovery_mismatch")?;
        entries.remove(&key);
        Ok(())
    }
    pub(crate) fn pending_for(&self, scope: &WorkScope) -> Vec<Value> {
        let mut pending: Vec<_> = self
            .entries
            .lock()
            .values()
            .filter(|entry| {
                entry.state == RegistryState::Unknown
                    && entry.authority == scope.recovery_authority_key
            })
            .map(|entry| entry.summary.clone())
            .collect();
        pending.sort_by_key(|entry| {
            entry["token"]
                .as_str()
                .and_then(|token| token.strip_prefix("q-"))
                .and_then(|token| token.parse::<u64>().ok())
                .unwrap_or(0)
        });
        pending.truncate(2);
        pending
    }
    fn begin(
        self: &Arc<Self>,
        key: String,
        acknowledge_unknown: bool,
    ) -> Result<QueryGuard, &'static str> {
        let mut entries = self.entries.lock();
        match entries.get(&key).map(|entry| entry.state) {
            Some(RegistryState::Active) => return Err("already_active"),
            Some(RegistryState::Unknown) if !acknowledge_unknown => {
                return Err("recovery_required")
            }
            None if entries
                .values()
                .filter(|entry| {
                    entry.state == RegistryState::Unknown && entry._unresolved_permit.is_some()
                })
                .count()
                >= 2 =>
            {
                return Err("query_capacity_unknown")
            }
            None if entries.len() >= REGISTRY_LIMIT => return Err("recovery_capacity"),
            _ => {}
        }
        entries.insert(
            key.clone(),
            RegistryEntry {
                state: RegistryState::Active,
                _unresolved_permit: None,
                authority:String::new(),
                summary:json!({"token":format!("q-{}",self.next_token.fetch_add(1,Ordering::Relaxed)+1)}),
            },
        );
        Ok(QueryGuard {
            registry: self.clone(),
            key,
            remote_possible: false,
            finished: false,
            permit: None,
        })
    }
}

struct QueryGuard {
    registry: Arc<QueryRegistry>,
    key: String,
    remote_possible: bool,
    finished: bool,
    permit: Option<crate::scheduler::Permit>,
}
impl QueryGuard {
    fn finish(mut self, unknown: bool) {
        let mut entries = self.registry.entries.lock();
        if unknown {
            if let Some(entry) = entries.get_mut(&self.key) {
                entry.state = RegistryState::Unknown;
                entry._unresolved_permit = self.permit.take();
            }
        } else {
            entries.remove(&self.key);
        }
        self.finished = true;
    }
}
impl Drop for QueryGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let mut entries = self.registry.entries.lock();
        if self.remote_possible {
            if let Some(entry) = entries.get_mut(&self.key) {
                entry.state = RegistryState::Unknown;
                entry._unresolved_permit = self.permit.take();
            }
        } else {
            entries.remove(&self.key);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cleanup {
    NotNeeded,
    Stopped,
    NotConfirmed,
    Denied,
    Failed,
    Unknown,
}
impl Cleanup {
    pub fn status(self) -> &'static str {
        match self {
            Self::NotNeeded => "not_needed",
            Self::Stopped => "stopped",
            Self::NotConfirmed => "not_confirmed",
            Self::Denied => "denied",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
    pub fn remote_unknown(self) -> bool {
        !matches!(self, Self::NotNeeded | Self::Stopped)
    }
    pub fn json(self) -> Value {
        json!({"status":self.status(), "remote_stop_confirmed": self == Self::Stopped,
            "remote_queries_may_still_run":self.remote_unknown(),
            "recovery_required":self.remote_unknown()})
    }
}

pub(crate) struct QueryOutcome {
    pub state: &'static str,
    pub result: Option<GetQueryResultsOutput>,
    pub cleanup: Cleanup,
    pub polls: usize,
    pub recovery_pending: Vec<Value>,
}
impl QueryOutcome {
    pub(crate) fn failed(state: &'static str, cleanup: Cleanup, polls: usize) -> Self {
        Self {
            state,
            result: None,
            cleanup,
            polls,
            recovery_pending: Vec::new(),
        }
    }
    pub fn error_result(&self) -> Value {
        let message = match self.state {
            "recovery_required" => "A previous query has an unknown remote state. Review it before explicitly authorizing another query.",
            "recovery_mismatch" => "The selected recovery item does not belong to this verified context or is no longer pending. Refresh the recovery list and review it again.",
            "recovery_capacity" => "The query recovery registry is full. Review unresolved queries before starting more.",
            "query_capacity_unknown" => "Two unresolved remote queries occupy the query budget. Review those original queries before authorizing more work.",
            "already_active" => "An identical query is already active. Wait for it to finish before retrying.",
            "cancelled" => "The local query was cancelled.",
            "timed_out" => "The query reached its time or polling limit. Narrow the time range before retrying.",
            "start_failed" => "AWS did not accept the query. Check the selected account and query inputs.",
            "start_unknown" => "The query start could not be confirmed. A remote query may still be running.",
            "poll_failed" => "Query results could not be retrieved.",
            "failed" => "AWS reported that the query failed.",
            "denied" => "The query is blocked by the current local policy.",
            _ => "The query could not complete within the local work limits.",
        };
        let mut coverage = Coverage::unknown(0);
        coverage.count("polls", self.polls);
        coverage.failure("query_incomplete", message, false);
        coverage.attach(json!({"ok":false,"error_type":"QueryIncomplete","error":message,
            "query_state":self.state,"cleanup":self.cleanup.json(),
            "recovery_required": !self.recovery_pending.is_empty() || self.cleanup.remote_unknown() || matches!(self.state,"recovery_required" | "query_capacity_unknown" | "recovery_capacity"),
            "recovery_pending":self.recovery_pending,
            "can_acknowledge_unknown": !self.recovery_pending.is_empty(),
            "recovery_action":if !self.recovery_pending.is_empty() {"select_unknown_query"} else {"review_outstanding_queries"}}))
    }
}

fn registry_key(scope: &WorkScope, widget: &str, spec: &QuerySpec<'_>) -> String {
    // This is an in-memory lookup key, not an authorization token. Authority is
    // kept outside the digest; no key or query value is logged or persisted.
    let encoded = serde_json::to_vec(&(
        widget,
        spec.group,
        spec.query,
        spec.end.saturating_sub(spec.start),
        spec.limit,
    ))
    .expect("primitive query key serialization");
    format!(
        "{}:{}",
        scope.recovery_authority_key,
        hex::encode(Sha1::digest(encoded))
    )
}

pub(crate) async fn run(
    ctx: &WidgetCtx,
    spec: &QuerySpec<'_>,
    timing: QueryTiming,
) -> QueryOutcome {
    let mut outcome = run_inner(ctx, spec, timing).await;
    let fallback;
    let scope = if let Some(scope) = &ctx.runtime.work {
        scope
    } else {
        fallback = ctx.fallback_scope();
        &fallback
    };
    outcome.recovery_pending = ctx.runtime.queries.pending_for(scope);
    outcome
}

async fn run_inner(ctx: &WidgetCtx, spec: &QuerySpec<'_>, timing: QueryTiming) -> QueryOutcome {
    let fallback;
    let source_scope = if let Some(scope) = &ctx.runtime.work {
        scope
    } else {
        fallback = ctx.fallback_scope();
        &fallback
    };
    let mut scope = source_scope.clone();
    scope.deadline = scope.deadline.min(Instant::now() + timing.total);
    for operation in ["StartQuery", "GetQueryResults", "StopQuery"] {
        if ctx.preflight("logs", operation).is_some() {
            return QueryOutcome::failed("denied", Cleanup::NotNeeded, 0);
        }
    }
    let acknowledge = ctx
        .inputs
        .get("acknowledge_unknown")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut guard = match ctx.runtime.queries.begin_for(
        &scope,
        &ctx.widget_name,
        spec,
        acknowledge,
        ctx.inputs.get("acknowledge_query").and_then(Value::as_str),
    ) {
        Ok(guard) => guard,
        Err(state) => {
            return QueryOutcome::failed(
                state,
                if state == "recovery_required" {
                    Cleanup::Unknown
                } else {
                    Cleanup::NotNeeded
                },
                0,
            )
        }
    };
    let query_permit = match ctx
        .runtime
        .scheduler
        .acquire(&scope, "logs", ResourceKind::Query)
        .await
    {
        Ok(permit) => permit,
        Err(error) => {
            guard.finish(false);
            return QueryOutcome::failed(work_state(&error), Cleanup::NotNeeded, 0);
        }
    };
    guard.permit = Some(query_permit);
    if let Err(error) = scope.count_operation() {
        guard.finish(false);
        return QueryOutcome::failed(work_state(&error), Cleanup::NotNeeded, 0);
    }
    let start_permit = match ctx
        .runtime
        .scheduler
        .acquire(&scope, "logs", ResourceKind::Ordinary)
        .await
    {
        Ok(permit) => permit,
        Err(error) => {
            guard.finish(false);
            return QueryOutcome::failed(work_state(&error), Cleanup::NotNeeded, 0);
        }
    };
    if let Err(error) = scope.check() {
        guard.finish(false);
        return QueryOutcome::failed(work_state(&error), Cleanup::NotNeeded, 0);
    }
    if !authorize_start(ctx).await {
        guard.finish(false);
        return QueryOutcome::failed("denied", Cleanup::NotNeeded, 0);
    }
    if let Err(error) = scope.check() {
        guard.finish(false);
        return QueryOutcome::failed(work_state(&error), Cleanup::NotNeeded, 0);
    }
    let request_ctx = WidgetCtx {
        runtime: ctx.runtime.with_work(scope.clone()),
        ..ctx.clone()
    };
    let client = aws_sdk_cloudwatchlogs::Client::new(&ctx.sdk);
    let start = client
        .start_query()
        .log_group_name(spec.group)
        .query_string(spec.query)
        .start_time(spec.start)
        .end_time(spec.end)
        .limit(spec.limit)
        .customize()
        .config_override(
            aws_sdk_cloudwatchlogs::config::Builder::new()
                .retry_config(aws_config::retry::RetryConfig::standard().with_max_attempts(1)),
        )
        .send();
    guard.remote_possible = true;
    // Do not drop StartQuery on local cancellation: a late returned ID enables
    // cleanup. The independent bounded start wait may outlast local deadline.
    let started = tokio::time::timeout(timing.start, start).await;
    drop(start_permit);
    let query_id = match started {
        Ok(Ok(response)) => match response.query_id().filter(|id| !id.is_empty()) {
            Some(id) => id.to_string(),
            None => {
                guard.finish(true);
                return QueryOutcome::failed("start_unknown", Cleanup::Unknown, 0);
            }
        },
        Ok(Err(error))
            if matches!(
                error.code(),
                Some(
                    "AccessDeniedException"
                        | "AccessDenied"
                        | "InvalidParameterException"
                        | "MalformedQueryException"
                        | "ResourceNotFoundException"
                )
            ) =>
        {
            guard.finish(false);
            return QueryOutcome::failed("start_failed", Cleanup::NotNeeded, 0);
        }
        _ => {
            guard.finish(true);
            return QueryOutcome::failed("start_unknown", Cleanup::Unknown, 0);
        }
    };
    let mut polls = 0;
    let failure = loop {
        if let Err(error) = scope.check() {
            break work_state(&error);
        }
        if polls >= timing.max_polls {
            break "timed_out";
        }
        polls += 1;
        let response = request_ctx
            .send(
                "logs",
                "GetQueryResults",
                client.get_query_results().query_id(&query_id).send(),
            )
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error)
                if matches!(
                    error.code(),
                    Some("WorkCancelled" | "WorkDeadline" | "WorkOperationLimit")
                ) =>
            {
                break work_state(&error)
            }
            Err(_) => break "poll_failed",
        };
        match response.status() {
            Some(QueryStatus::Complete) => {
                guard.finish(false);
                return QueryOutcome {
                    state: "complete",
                    result: Some(response),
                    cleanup: Cleanup::NotNeeded,
                    polls,
                    recovery_pending: Vec::new(),
                };
            }
            Some(QueryStatus::Failed) => {
                guard.finish(false);
                return QueryOutcome::failed("failed", Cleanup::NotNeeded, polls);
            }
            Some(QueryStatus::Cancelled) => {
                guard.finish(false);
                return QueryOutcome::failed("cancelled", Cleanup::NotNeeded, polls);
            }
            Some(QueryStatus::Timeout) => {
                guard.finish(false);
                return QueryOutcome::failed("timed_out", Cleanup::NotNeeded, polls);
            }
            _ => {}
        }
        tokio::select! {
            biased;
            _ = scope.cancellation.cancelled() => break "cancelled",
            _ = tokio::time::sleep_until(scope.deadline) => break "timed_out",
            _ = tokio::time::sleep(timing.poll_interval) => {},
        }
    };
    let cleanup = stop(ctx, &scope, &client, &query_id).await;
    guard.finish(cleanup.remote_unknown());
    QueryOutcome::failed(failure, cleanup, polls)
}

async fn authorize_start(ctx: &WidgetCtx) -> bool {
    // One bounded fresh snapshot covers the full lifecycle after queue admission.
    // A Stop-only revocation while queued must prevent starting remote work.
    let paths = ctx.runtime.paths.clone();
    let current = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::task::spawn_blocking(move || crate::runtime::read_current_policy(&paths, false)),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_else(|| Err("policy unavailable".into()));
    for operation in ["StartQuery", "GetQueryResults", "StopQuery"] {
        if crate::aws::policy::gate(&current, "logs", operation).is_err()
            || matches!(
                crate::aws::policy::credential_preflight(&current, "logs", operation),
                crate::aws::policy::CredentialPreflight::Denied { .. }
            )
        {
            ctx.audit_call(
                "logs",
                operation,
                "aws-blocked",
                Some("Policy changed before query start"),
            );
            return false;
        }
    }
    true
}

fn work_state(error: &WorkFailure) -> &'static str {
    if error.code() == Some("WorkCancelled") {
        "cancelled"
    } else {
        "timed_out"
    }
}

async fn stop(
    ctx: &WidgetCtx,
    source_scope: &WorkScope,
    client: &aws_sdk_cloudwatchlogs::Client,
    query_id: &str,
) -> Cleanup {
    let scope = source_scope.cleanup_scope();
    let result = ctx
        .runtime
        .scheduler
        .run(&scope, "logs", ResourceKind::Cleanup, async {
            // Initial and current policies must both permit cleanup. Reading here
            // occurs after the reserved permit, so queue delay cannot use a stale
            // policy snapshot. Missing/invalid policy never creates or upgrades it.
            if ctx.preflight("logs", "StopQuery").is_some() {
                return Cleanup::Denied;
            }
            let paths = ctx.runtime.paths.clone();
            let policy = tokio::task::spawn_blocking(move || {
                crate::runtime::read_current_policy(&paths, false)
            })
            .await
            .unwrap_or_else(|_| Err("policy unavailable".into()));
            if crate::aws::policy::gate(&policy, "logs", "StopQuery").is_err()
                || matches!(
                    crate::aws::policy::credential_preflight(&policy, "logs", "StopQuery"),
                    crate::aws::policy::CredentialPreflight::Denied { .. }
                )
            {
                return Cleanup::Denied;
            }
            match client.stop_query().query_id(query_id).send().await {
                Ok(output) if output.success() => Cleanup::Stopped,
                Ok(_) => Cleanup::NotConfirmed,
                Err(error)
                    if matches!(
                        error.code(),
                        Some(
                            "AccessDenied"
                                | "AccessDeniedException"
                                | "UnauthorizedException"
                                | "UnauthorizedOperation"
                        )
                    ) =>
                {
                    Cleanup::Denied
                }
                Err(_) => Cleanup::Failed,
            }
        })
        .await;
    let outcome = result.unwrap_or(Cleanup::Failed);
    ctx.log("query cleanup", json!({"cleanup_status":outcome.status()}));
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_aws::{ExpectedRequest, ScriptedHttp},
        test_support::TestDir,
    };
    const START: &str = "Logs_20140328.StartQuery";
    const POLL: &str = "Logs_20140328.GetQueryResults";
    const STOP: &str = "Logs_20140328.StopQuery";
    fn spec() -> QuerySpec<'static> {
        QuerySpec {
            group: "/synthetic/query-group",
            query: "fields @message",
            start: 100,
            end: 160,
            limit: 100,
        }
    }
    fn timing() -> QueryTiming {
        QueryTiming {
            start: Duration::from_millis(50),
            poll_interval: Duration::from_millis(1),
            total: Duration::from_millis(100),
            max_polls: 3,
        }
    }
    fn fixture(requests: Vec<ExpectedRequest>) -> (TestDir, ScriptedHttp, WidgetCtx) {
        let dir = TestDir::new();
        crate::aws::policy::write_text(
            &dir.paths(),
            "statements:\n - effect: Allow\n   action: ['*']\n",
        )
        .unwrap();
        let transport = ScriptedHttp::new(requests);
        let mut ctx = transport.context(&dir, "logs-insights", json!({}));
        ctx.runtime = ctx.runtime.with_work(ctx.fallback_scope());
        (dir, transport, ctx)
    }
    fn start() -> ExpectedRequest {
        ExpectedRequest::json(START, json!({}), json!({"queryId":"synthetic-query-id"}))
    }
    fn poll(status: &str) -> ExpectedRequest {
        ExpectedRequest::json(
            POLL,
            json!({"queryId":"synthetic-query-id"}),
            json!({"status":status,"results":[]}),
        )
    }
    fn stop_response(success: bool) -> ExpectedRequest {
        ExpectedRequest::json(
            STOP,
            json!({"queryId":"synthetic-query-id"}),
            json!({"success":success}),
        )
    }
    #[tokio::test]
    async fn stop_revoked_while_queued_prevents_any_remote_query_start() {
        let (dir, http, ctx) = fixture(vec![]);
        let scope = ctx.runtime.work.as_ref().unwrap();
        let mut permits = Vec::new();
        for _ in 0..4 {
            permits.push(
                ctx.runtime
                    .scheduler
                    .acquire(scope, "logs", ResourceKind::Ordinary)
                    .await
                    .unwrap(),
            );
        }
        let request = tokio::spawn({
            let ctx = ctx.clone();
            async move { run(&ctx, &spec(), QueryTiming::default()).await }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while ctx.runtime.scheduler.snapshot().queued == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        crate::aws::policy::write_text(&dir.paths(),
            "statements:\n - effect: Allow\n   action: ['*']\n - effect: Deny\n   action: ['logs:StopQuery']\n"
        ).unwrap();
        drop(permits);
        let outcome = request.await.unwrap();
        assert_eq!(outcome.state, "denied");
        assert_eq!(outcome.cleanup, Cleanup::NotNeeded);
        assert_eq!(ctx.runtime.scheduler.snapshot().queries, 0);
        assert_eq!(http.calls(), 0);
        http.assert_finished();
    }

    #[tokio::test]
    async fn completed_query_releases_capacity_without_stopping() {
        let (_dir, http, ctx) = fixture(vec![start(), poll("Complete")]);
        let outcome = run(&ctx, &spec(), timing()).await;
        assert_eq!(outcome.state, "complete");
        assert!(outcome.result.is_some());
        assert_eq!(outcome.cleanup, Cleanup::NotNeeded);
        http.assert_finished();
        assert_eq!(ctx.runtime.scheduler.snapshot().queries, 0);
        assert!(ctx.runtime.queries.entries.lock().is_empty());
    }
    #[tokio::test]
    async fn cancelled_slow_start_keeps_id_then_stops_with_original_authority() {
        let (_dir, http, ctx) = fixture(vec![
            start().delay(Duration::from_millis(15)),
            stop_response(true),
        ]);
        let cancel = ctx.runtime.work.as_ref().unwrap().cancellation.clone();
        let cancel_later = async {
            tokio::time::sleep(Duration::from_millis(3)).await;
            cancel.cancel();
        };
        let query_spec = spec();
        let (outcome, ()) = tokio::join!(run(&ctx, &query_spec, timing()), cancel_later);
        assert_eq!(outcome.state, "cancelled");
        assert_eq!(outcome.cleanup, Cleanup::Stopped);
        assert_eq!(outcome.polls, 0);
        http.assert_finished();
        assert_eq!(ctx.runtime.scheduler.snapshot().queries, 0);
    }
    #[tokio::test]
    async fn lost_start_blocks_automatic_replacement_until_explicit_acknowledgement() {
        let (_dir, http, mut ctx) = fixture(vec![
            ExpectedRequest::json(START, json!({}), json!({})),
            start(),
            poll("Complete"),
        ]);
        let first = run(&ctx, &spec(), timing()).await;
        assert_eq!(first.state, "start_unknown");
        assert!(first.error_result()["recovery_required"].as_bool().unwrap());
        let second = run(&ctx, &spec(), timing()).await;
        assert_eq!(second.state, "recovery_required");
        assert_eq!(http.calls(), 1);
        ctx.inputs["acknowledge_unknown"] = json!(true);
        assert_eq!(run(&ctx, &spec(), timing()).await.state, "complete");
        http.assert_finished();
    }
    #[tokio::test]
    async fn polling_failure_attempts_stop_and_preserves_unknown_when_not_confirmed() {
        for confirmed in [true, false] {
            let (_dir, http, ctx) = fixture(vec![
                start(),
                ExpectedRequest::json(
                    POLL,
                    json!({}),
                    json!({"__type":"ServiceUnavailableException"}),
                )
                .status(500),
                stop_response(confirmed),
            ]);
            let outcome = run(&ctx, &spec(), timing()).await;
            assert_eq!(outcome.state, "poll_failed");
            assert_eq!(
                outcome.cleanup,
                if confirmed {
                    Cleanup::Stopped
                } else {
                    Cleanup::NotConfirmed
                }
            );
            assert_eq!(outcome.cleanup.remote_unknown(), !confirmed);
            http.assert_finished();
        }
    }
    #[tokio::test]
    async fn changed_policy_denies_cleanup_without_using_an_old_allow() {
        let (dir, http, ctx) = fixture(vec![start().delay(Duration::from_millis(15))]);
        let cancel = ctx.runtime.work.as_ref().unwrap().cancellation.clone();
        let revoke = async {
            tokio::time::sleep(Duration::from_millis(3)).await;
            crate::aws::policy::write_text(
                &dir.paths(),
                "statements:\n - effect: Deny\n   action: ['logs:StopQuery']\n",
            )
            .unwrap();
            cancel.cancel();
        };
        let query_spec = spec();
        let (outcome, ()) = tokio::join!(run(&ctx, &query_spec, timing()), revoke);
        assert_eq!(outcome.cleanup, Cleanup::Denied);
        http.assert_finished();
        assert_eq!(ctx.runtime.queries.entries.lock().len(), 1);
    }
    #[tokio::test]
    async fn polling_limit_and_failed_stop_remain_explicitly_unknown() {
        let (_dir, http, ctx) = fixture(vec![
            start(),
            poll("Running"),
            ExpectedRequest::json(
                STOP,
                json!({}),
                json!({"__type":"ServiceUnavailableException"}),
            )
            .status(500),
        ]);
        let outcome = run(
            &ctx,
            &spec(),
            QueryTiming {
                max_polls: 1,
                ..timing()
            },
        )
        .await;
        assert_eq!(outcome.state, "timed_out");
        assert_eq!(outcome.cleanup, Cleanup::Failed);
        http.assert_finished();
    }
    #[tokio::test]
    async fn hung_start_has_a_bound_and_does_not_retry_or_invent_an_id() {
        let (_dir, http, ctx) = fixture(vec![start().delay(Duration::from_millis(30))]);
        let outcome = run(
            &ctx,
            &spec(),
            QueryTiming {
                start: Duration::from_millis(2),
                ..timing()
            },
        )
        .await;
        assert_eq!(outcome.state, "start_unknown");
        assert_eq!(http.calls(), 1);
        http.assert_finished();
        assert_eq!(ctx.runtime.scheduler.snapshot().queries, 1);
    }
    #[test]
    fn unresolved_registry_has_a_fixed_capacity_and_never_evicts_unknown_entries() {
        let registry = Arc::new(QueryRegistry::default());
        for index in 0..REGISTRY_LIMIT {
            registry
                .begin(format!("synthetic-key-{index}"), false)
                .unwrap_or_else(|_| panic!("capacity"))
                .finish(true);
        }
        assert_eq!(
            registry.begin("one-too-many".into(), false).err(),
            Some("recovery_capacity")
        );
        assert_eq!(
            registry.begin("synthetic-key-0".into(), false).err(),
            Some("recovery_required")
        );
        registry
            .begin("synthetic-key-0".into(), true)
            .unwrap_or_else(|_| panic!("acknowledgement"))
            .finish(false);
        assert!(registry.begin("one-too-many".into(), false).is_ok());
    }
    #[tokio::test]
    async fn two_unknown_remote_queries_reserve_capacity_and_block_more_dispatch() {
        let (_dir, http, ctx) = fixture(vec![
            ExpectedRequest::json(START, json!({}), json!({})),
            ExpectedRequest::json(START, json!({}), json!({})),
        ]);
        let first = spec();
        assert_eq!(run(&ctx, &first, timing()).await.state, "start_unknown");
        let second = QuerySpec {
            group: "/synthetic/query-second",
            ..spec()
        };
        assert_eq!(run(&ctx, &second, timing()).await.state, "start_unknown");
        assert_eq!(ctx.runtime.scheduler.snapshot().queries, 2);
        let third = QuerySpec {
            group: "/synthetic/query-third",
            ..spec()
        };
        let blocked = run(&ctx, &third, timing()).await;
        assert_eq!(blocked.state, "query_capacity_unknown");
        assert_eq!(blocked.error_result()["recovery_required"], true);
        assert_eq!(blocked.error_result()["can_acknowledge_unknown"], true);
        assert_eq!(http.calls(), 2);
        http.assert_finished();
    }
    #[tokio::test]
    async fn aws_stop_denial_is_not_reported_as_remote_completion() {
        let (_dir,http,ctx)=fixture(vec![start(),poll("Running"),ExpectedRequest::json(STOP,json!({}),json!({"__type":"AccessDeniedException","message":"SYNTHETIC_PRIVATE_STOP_MESSAGE"})).status(400)]);
        let outcome = run(
            &ctx,
            &spec(),
            QueryTiming {
                max_polls: 1,
                ..timing()
            },
        )
        .await;
        assert_eq!(outcome.cleanup, Cleanup::Denied);
        assert!(!outcome
            .error_result()
            .to_string()
            .contains("SYNTHETIC_PRIVATE"));
        assert_eq!(ctx.runtime.scheduler.snapshot().queries, 1);
        http.assert_finished();
    }
    #[tokio::test]
    async fn explicit_selected_token_releases_one_same_principal_unknown_for_changed_query() {
        let (_dir, http, mut ctx) = fixture(vec![
            ExpectedRequest::json(START, json!({}), json!({})),
            start(),
            poll("Complete"),
        ]);
        let first = run(&ctx, &spec(), timing()).await;
        let token = first.recovery_pending[0]["token"]
            .as_str()
            .unwrap()
            .to_string();
        ctx.inputs = json!({"acknowledge_unknown":true,"acknowledge_query":token});
        let changed = QuerySpec {
            query: "fields @timestamp",
            ..spec()
        };
        assert_eq!(run(&ctx, &changed, timing()).await.state, "complete");
        assert_eq!(ctx.runtime.scheduler.snapshot().queries, 0);
        http.assert_finished();
    }
    #[tokio::test]
    async fn selecting_one_unknown_cannot_implicitly_acknowledge_a_second_unknown() {
        let (_dir, http, mut ctx) = fixture(vec![
            ExpectedRequest::json(START, json!({}), json!({})),
            ExpectedRequest::json(START, json!({}), json!({})),
        ]);
        let first = run(&ctx, &spec(), timing()).await;
        let token = first.recovery_pending[0]["token"].clone();
        let changed = QuerySpec {
            query: "fields @timestamp",
            ..spec()
        };
        let second = run(&ctx, &changed, timing()).await;
        assert_eq!(second.recovery_pending.len(), 2);
        assert_eq!(ctx.runtime.scheduler.snapshot().queries, 2);
        ctx.inputs = json!({"acknowledge_unknown":true,"acknowledge_query":token});
        let result = run(&ctx, &changed, timing()).await;
        assert_eq!(result.state, "recovery_required");
        assert_eq!(result.recovery_pending.len(), 1);
        assert_eq!(ctx.runtime.scheduler.snapshot().queries, 1);
        assert_eq!(http.calls(), 2);
        http.assert_finished();
    }

    #[tokio::test]
    async fn recovery_token_cannot_release_another_verified_principals_hold() {
        let (_dir, http, ctx) = fixture(vec![ExpectedRequest::json(START, json!({}), json!({}))]);
        let first = run(&ctx, &spec(), timing()).await;
        let token = first.recovery_pending[0]["token"].clone();
        let mut other = ctx.clone();
        let mut scope = other.runtime.work.clone().unwrap();
        scope.recovery_authority_key = "different-synthetic-principal".into();
        other.runtime = other.runtime.with_work(scope);
        other.inputs = json!({"acknowledge_unknown":true,"acknowledge_query":token});
        let denied = run(&other, &spec(), timing()).await;
        assert_eq!(denied.state, "recovery_mismatch");
        assert!(denied.recovery_pending.is_empty());
        assert_eq!(ctx.runtime.scheduler.snapshot().queries, 1);
        assert_eq!(http.calls(), 1);
        http.assert_finished();
    }
}
