//! Memory-only result reuse. A key is supplied only after current verification.
//! Neither keys nor cached resource data are written to diagnostics or disk.
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const MAX_ENTRIES: usize = 16;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 512 * 1024;
const MAX_AGE: Duration = Duration::from_secs(15);

struct Entry {
    key: String,
    value: Value,
    bytes: usize,
    inserted: Instant,
    captured_at: f64,
}
#[derive(Default)]
struct Contents {
    entries: VecDeque<Entry>,
    bytes: usize,
}
#[derive(Default)]
pub(crate) struct ResultCache {
    contents: Mutex<Contents>,
}

pub(crate) fn cacheable(name: &str) -> bool {
    matches!(
        name,
        "cfn-stack-detail" | "pipeline-execution-detail" | "codeartifact-package-version-history"
    )
}

impl ResultCache {
    pub(crate) fn clear(&self) {
        *self.contents.lock() = Contents::default();
    }

    /// Verification, current configuration and policy checks belong to the caller.
    pub(crate) fn get(&self, key: &str) -> Option<Value> {
        self.get_at(key, Instant::now())
    }

    fn get_at(&self, key: &str, now: Instant) -> Option<Value> {
        let mut cache = self.contents.lock();
        expire(&mut cache, now);
        let index = cache.entries.iter().position(|entry| entry.key == key)?;
        let entry = cache.entries.remove(index)?;
        let mut value = entry.value.clone();
        value["_cache"] =
            json!({"hit":true,"captured_at":entry.captured_at,"max_age_seconds":MAX_AGE.as_secs()});
        cache.entries.push_back(entry);
        Some(value)
    }

    pub(crate) fn insert(&self, key: String, value: &Value, captured_at: f64) {
        self.insert_at(key, value, captured_at, Instant::now());
    }

    fn insert_at(&self, key: String, value: &Value, captured_at: f64, now: Instant) {
        // Partial evidence/errors and request-specific envelopes must never become
        // a new successful response under a different subscriber's request ID.
        if !captured_at.is_finite()
            || crate::request::outcome(value) != "succeeded"
            || value.get("_request").is_some()
            || value.get("cleanup").is_some()
            || value
                .get("coverage")
                .is_some_and(|coverage| coverage["completeness"] != "complete")
        {
            return;
        }
        let mut stored = value.clone();
        if let Some(object) = stored.as_object_mut() {
            object.remove("_budget");
            object.remove("_diagnostics");
        }
        let value = &stored;
        let Some(value_bytes) = bounded_size(value, MAX_ENTRY_BYTES) else {
            return;
        };
        if key.len() > 64 * 1024 {
            return;
        }
        // This is an encoded-data budget including keys. Value/container and
        // allocator overhead are separate; no native RSS ceiling is claimed.
        let bytes = value_bytes + key.len();
        let mut cache = self.contents.lock();
        expire(&mut cache, now);
        if let Some(index) = cache.entries.iter().position(|entry| entry.key == key) {
            let old = cache.entries.remove(index).unwrap();
            cache.bytes -= old.bytes;
        }
        while cache.entries.len() >= MAX_ENTRIES || cache.bytes + bytes > MAX_BYTES {
            if let Some(old) = cache.entries.pop_front() {
                cache.bytes -= old.bytes;
            } else {
                break;
            }
        }
        cache.bytes += bytes;
        cache.entries.push_back(Entry {
            key,
            value: value.clone(),
            bytes,
            inserted: now,
            captured_at,
        });
    }
}

fn expire(cache: &mut Contents, now: Instant) {
    // LRU order is independent of insertion age after a hit.
    cache
        .entries
        .retain(|entry| now.saturating_duration_since(entry.inserted) < MAX_AGE);
    cache.bytes = cache.entries.iter().map(|entry| entry.bytes).sum();
}

fn bounded_size(value: &Value, limit: usize) -> Option<usize> {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            if data.len() > self.limit.saturating_sub(self.bytes) {
                return Err(std::io::Error::other("cache value limit"));
            }
            self.bytes += data.len();
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut count, value).ok()?;
    Some(count.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authority_generation_operation_and_inputs_are_separate_keys() {
        let cache = ResultCache::default();
        cache.insert(
            "verified-a:policy1:detail:resource-one".into(),
            &json!({"render":"table","rows":[{"synthetic":"a"}]}),
            10.0,
        );
        assert!(cache
            .get("verified-b:policy1:detail:resource-one")
            .is_none());
        assert!(cache
            .get("verified-a:policy2:detail:resource-one")
            .is_none());
        assert!(cache
            .get("verified-a:policy1:detail:resource-two")
            .is_none());
        assert_eq!(
            cache.get("verified-a:policy1:detail:resource-one").unwrap()["_cache"]["captured_at"],
            10.0
        );
        cache.clear();
        assert!(cache
            .get("verified-a:policy1:detail:resource-one")
            .is_none());
    }
    #[test]
    fn hits_do_not_extend_expiry_and_partial_data_is_not_promoted() {
        let cache = ResultCache::default();
        let now = Instant::now();
        cache.insert_at("complete".into(), &json!({"ok":true}), 1.0, now);
        cache.insert_at(
            "partial".into(),
            &json!({"partial":true,"rows":[1]}),
            1.0,
            now,
        );
        assert!(cache.get_at("partial", now).is_none());
        assert!(cache
            .get_at("complete", now + Duration::from_secs(14))
            .is_some());
        assert!(cache
            .get_at("complete", now + Duration::from_secs(15))
            .is_none());
    }
    #[test]
    fn concurrent_inserts_obey_entry_and_serialized_byte_bounds() {
        let cache = std::sync::Arc::new(ResultCache::default());
        let workers: Vec<_> = (0..32)
            .map(|n| {
                let cache = cache.clone();
                std::thread::spawn(move || {
                    cache.insert(
                        format!("synthetic-{n}"),
                        &json!({"data":"x".repeat(400*1024)}),
                        1.0,
                    )
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let contents = cache.contents.lock();
        assert!(contents.entries.len() <= MAX_ENTRIES);
        assert!(contents.bytes <= MAX_BYTES);
        assert_eq!(
            contents.bytes,
            contents.entries.iter().map(|e| e.bytes).sum::<usize>()
        );
        drop(contents);
        cache.insert(
            "oversized".into(),
            &json!({"data":"x".repeat(MAX_ENTRY_BYTES)}),
            1.0,
        );
        assert!(cache.get("oversized").is_none());
    }
}
