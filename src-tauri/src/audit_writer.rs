//! Ordered bounded audit writes outside async request workers.
//! Enqueue sanitizes before retaining a record. Overflow/failure is sticky.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;

use crate::{audit, paths::AppPaths};

const QUEUE_CAPACITY: usize = 512;
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

enum Message {
    Record(Vec<u8>),
    Flush(oneshot::Sender<Result<(), ()>>),
}

pub(crate) struct AuditWriter {
    sender: Option<mpsc::SyncSender<Message>>,
    paths: AppPaths,
    retention: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
}

impl AuditWriter {
    pub fn new(paths: AppPaths, retention: Arc<AtomicBool>, failed: Arc<AtomicBool>) -> Self {
        Self::start(paths, retention, failed, QUEUE_CAPACITY, None)
    }

    fn start(
        paths: AppPaths,
        retention: Arc<AtomicBool>,
        failed: Arc<AtomicBool>,
        capacity: usize,
        paused: Option<mpsc::Receiver<()>>,
    ) -> Self {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let writer_paths = paths.clone();
        let writer_retention = retention.clone();
        let writer_failed = failed.clone();
        let launched = std::thread::Builder::new()
            .name("cloud-burrito-audit".into())
            .spawn(move || {
                if let Some(paused) = paused {
                    let _ = paused.recv();
                }
                while let Ok(message) = receiver.recv() {
                    match message {
                        Message::Record(line) => {
                            if audit::append_record(&writer_paths, &line, &writer_retention)
                                .is_err()
                            {
                                writer_failed.store(true, Ordering::SeqCst);
                            }
                        }
                        Message::Flush(reply) => {
                            // Each completed append has closed its file. A barrier
                            // waits for those writes and requests local data sync.
                            let result = audit::sync_history(&writer_paths);
                            if result.is_err() {
                                writer_failed.store(true, Ordering::SeqCst);
                            }
                            let _ = reply.send(result);
                        }
                    }
                }
            });
        let sender = if launched.is_ok() {
            Some(sender)
        } else {
            failed.store(true, Ordering::SeqCst);
            None
        };
        Self {
            sender,
            paths,
            retention,
            failed,
        }
    }

    pub fn enqueue(&self, entry: &Value, timestamp: f64) {
        let result =
            audit::encoded(entry, timestamp).and_then(|line| self.send(Message::Record(line)));
        if result.is_err() {
            self.failed.store(true, Ordering::SeqCst);
        }
    }

    fn send(&self, message: Message) -> Result<(), ()> {
        self.sender
            .as_ref()
            .ok_or(())?
            .try_send(message)
            .map_err(|_| ())
    }

    pub async fn flush(&self) -> bool {
        let (sender, receiver) = oneshot::channel();
        if self.send(Message::Flush(sender)).is_err() {
            self.failed.store(true, Ordering::SeqCst);
            return false;
        }
        let complete = matches!(
            tokio::time::timeout(FLUSH_TIMEOUT, receiver).await,
            Ok(Ok(Ok(())))
        );
        if !complete {
            self.failed.store(true, Ordering::SeqCst);
        }
        complete && !self.failed.load(Ordering::SeqCst)
    }

    pub fn set_retention(&self, mode: audit::RetentionMode) -> Result<(), ()> {
        audit::set_retention(&self.paths, &self.retention, mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use serde_json::json;

    #[tokio::test]
    async fn ordered_records_are_sanitized_before_queueing_and_flushed() {
        let dir = TestDir::new();
        let failed = Arc::new(AtomicBool::new(false));
        let writer = AuditWriter::new(
            dir.paths(),
            Arc::new(AtomicBool::new(false)),
            failed.clone(),
        );
        for index in 0..100 {
            writer.enqueue(&json!({"kind":"request","attempt":index,"message":"SYNTHETIC_PRIVATE_QUEUED_DATA","credentials":{"value":"SYNTHETIC_PRIVATE_QUEUED_DATA"}}),index as f64);
        }
        assert!(writer.flush().await);
        let records = audit::try_tail(&dir.paths(), 100).unwrap();
        assert_eq!(records.len(), 100);
        assert_eq!(records.first().unwrap()["attempt"], 0);
        assert_eq!(records.last().unwrap()["attempt"], 99);
        assert!(!serde_json::to_string(&records)
            .unwrap()
            .contains("SYNTHETIC_PRIVATE"));
        assert!(!failed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_full_queue_is_reported_as_sticky_failure_without_blocking_enqueue() {
        let dir = TestDir::new();
        let failed = Arc::new(AtomicBool::new(false));
        let (resume, paused) = mpsc::sync_channel(1);
        let writer = AuditWriter::start(
            dir.paths(),
            Arc::new(AtomicBool::new(false)),
            failed.clone(),
            2,
            Some(paused),
        );
        writer.enqueue(&json!({"attempt":1}), 1.0);
        writer.enqueue(&json!({"attempt":2}), 2.0);
        writer.enqueue(&json!({"attempt":3}), 3.0);
        assert!(failed.load(Ordering::SeqCst));
        resume.send(()).unwrap();
        // Let the intentionally paused synthetic writer drain, then use its
        // barrier before destroying the disposable directory.
        for _ in 0..100 {
            if audit::try_tail(&dir.paths(), 10).unwrap().len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(!writer.flush().await);
        assert_eq!(audit::try_tail(&dir.paths(), 10).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn disk_write_failure_is_visible_when_the_command_flushes() {
        let dir = TestDir::new();
        std::fs::create_dir_all(audit::log_path(&dir.paths())).unwrap();
        let failed = Arc::new(AtomicBool::new(false));
        let writer = AuditWriter::new(
            dir.paths(),
            Arc::new(AtomicBool::new(false)),
            failed.clone(),
        );
        writer.enqueue(&json!({"kind":"request"}), 1.0);
        assert!(!writer.flush().await);
        assert!(failed.load(Ordering::SeqCst));
    }
}
