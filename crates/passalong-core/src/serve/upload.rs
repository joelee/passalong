//! Sending one queued job, retrying until it succeeds or `serve` stops.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use tokio::sync::watch;
use tracing::Instrument;

use crate::config::AfterSend;
use crate::model::{ContentHasher, ItemId, NewItem};
use crate::random::StdRandom;
use crate::serve::drop_watcher::unique_target;
use crate::serve::retry::Backoff;
use crate::serve::{Job, StoreOpener, stopped};
use crate::store::{Store, StoreError};
use crate::telemetry;

/// What happened to a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// Stored as a new item.
    Sent(ItemId),
    /// Identical content was already stored; nothing was uploaded.
    AlreadyPresent,
    /// Could not be sent for a local reason, such as an unreadable file.
    Skipped,
    /// `serve` was stopped before the job finished.
    Abandoned,
}

enum Failure {
    /// A problem on this machine; retrying will not help.
    Local(String),
    /// A problem with the store; retry with a fresh connection.
    Store(StoreError),
}

/// Sends jobs to the store, reconnecting between retries.
pub struct Uploader {
    store: Box<dyn Store>,
    open_store: StoreOpener,
    device: String,
    after_send: AfterSend,
    sent_dir: PathBuf,
    rng: StdRandom,
}

impl Uploader {
    /// An uploader starting with `store`; `open_store` replaces it before
    /// each retry.
    pub fn new(
        store: Box<dyn Store>,
        open_store: StoreOpener,
        device: String,
        after_send: AfterSend,
        sent_dir: PathBuf,
    ) -> Self {
        Self {
            store,
            open_store,
            device,
            after_send,
            sent_dir,
            rng: StdRandom::new(),
        }
    }

    /// Sends one job. Store failures are retried after 1, 2, 4 … 60 seconds
    /// with a freshly opened store, until the job succeeds or `shutdown`
    /// fires. Local failures skip the job.
    pub async fn handle(&mut self, job: Job, shutdown: &mut watch::Receiver<bool>) -> JobOutcome {
        let span = telemetry::op_span("serve", &mut self.rng);
        self.handle_with_retries(job, shutdown)
            .instrument(span)
            .await
    }

    async fn handle_with_retries(
        &mut self,
        job: Job,
        shutdown: &mut watch::Receiver<bool>,
    ) -> JobOutcome {
        let mut backoff = Backoff::default();
        let mut attempt: u64 = 1;
        loop {
            let result = tokio::select! {
                result = self.attempt(&job) => result,
                () = stopped(shutdown) => {
                    tracing::info!("stopping: upload abandoned");
                    return JobOutcome::Abandoned;
                }
            };
            match result {
                Ok(outcome) => return outcome,
                Err(Failure::Local(message)) => {
                    tracing::warn!(
                        error = message.as_str(),
                        "cannot send; skipping until serve restarts"
                    );
                    return JobOutcome::Skipped;
                }
                Err(Failure::Store(err)) => {
                    let delay = backoff.next_delay();
                    tracing::warn!(attempt, error = %err, "upload failed; retrying in {} s", delay.as_secs());
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {}
                        () = stopped(shutdown) => return JobOutcome::Abandoned,
                    }
                    match (self.open_store)().await {
                        Ok(store) => self.store = store,
                        Err(err) => tracing::warn!(attempt, error = %err, "reconnecting failed"),
                    }
                    attempt += 1;
                }
            }
        }
    }

    async fn attempt(&self, job: &Job) -> Result<JobOutcome, Failure> {
        match job {
            Job::Text(text) => self.send_text(text).await,
            Job::File(path) => self.send_file(path).await,
        }
    }

    async fn send_text(&self, text: &str) -> Result<JobOutcome, Failure> {
        // Text that `load` just put on the clipboard is already stored;
        // checking first avoids uploading it again.
        let mut hasher = ContentHasher::new();
        hasher.update(text.as_bytes());
        let key = hasher.finalize().content_key();
        if let Some(existing) = self
            .store
            .find_by_content_key(&key)
            .await
            .map_err(Failure::Store)?
        {
            tracing::debug!(id = %existing.id, "clipboard text is already stored");
            return Ok(JobOutcome::AlreadyPresent);
        }
        let content = Box::new(Cursor::new(text.as_bytes().to_vec()));
        let outcome = self
            .store
            .put(NewItem::text(self.device.clone()), content)
            .await
            .map_err(Failure::Store)?;
        tracing::info!(id = %outcome.meta.id, size = outcome.meta.size, "sent clipboard text");
        Ok(if outcome.created {
            JobOutcome::Sent(outcome.meta.id)
        } else {
            JobOutcome::AlreadyPresent
        })
    }

    async fn send_file(&self, path: &Path) -> Result<JobOutcome, Failure> {
        let shown = path.display();
        let local = |message: String| Failure::Local(format!("{shown}: {message}"));
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| local("no file name".to_owned()))?;
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|err| local(err.to_string()))?;
        let outcome = match self
            .store
            .put(NewItem::file(name, self.device.clone()), Box::new(file))
            .await
        {
            Ok(outcome) => outcome,
            // Reading the local file failed part-way.
            Err(StoreError::Content(message)) => return Err(local(message)),
            Err(err) => return Err(Failure::Store(err)),
        };
        tracing::info!(id = %outcome.meta.id, size = outcome.meta.size, path = %shown, "sent file");
        self.finish_file(path).await;
        Ok(if outcome.created {
            JobOutcome::Sent(outcome.meta.id)
        } else {
            JobOutcome::AlreadyPresent
        })
    }

    /// Moves a sent file into `sent/`, or deletes it.
    async fn finish_file(&self, path: &Path) {
        let result = match self.after_send {
            AfterSend::Move => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let target = unique_target(&self.sent_dir, &name);
                tokio::fs::rename(path, &target)
                    .await
                    .map(|()| Some(target))
            }
            AfterSend::Delete => tokio::fs::remove_file(path).await.map(|()| None),
        };
        match result {
            Ok(Some(target)) => {
                tracing::debug!(path = %target.display(), "moved to the sent folder")
            }
            Ok(None) => tracing::debug!(path = %path.display(), "deleted after sending"),
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "sent, but could not move or delete the file")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SystemClock;
    use crate::config::AfterSend;
    use crate::fs::{BoxRead, LocalFs};
    use crate::model::{ContentKey, ItemId, ItemMeta, NewItem};
    use crate::random::StdRandom;
    use crate::serve::{Job, StoreOpener};
    use crate::store::{FsStore, PutOutcome, Store, StoreError};
    use async_trait::async_trait;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::sync::watch;

    /// A local store whose `put` fails while `failures` is above zero.
    struct Flaky {
        inner: FsStore<LocalFs>,
        failures: Arc<AtomicUsize>,
        puts: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Store for Flaky {
        async fn put(&self, item: NewItem, content: BoxRead) -> Result<PutOutcome, StoreError> {
            self.puts.fetch_add(1, Ordering::SeqCst);
            if self.failures.load(Ordering::SeqCst) > 0 {
                self.failures.fetch_sub(1, Ordering::SeqCst);
                return Err(StoreError::Backend("server went away".into()));
            }
            self.inner.put(item, content).await
        }
        async fn list(&self) -> Result<Vec<ItemMeta>, StoreError> {
            self.inner.list().await
        }
        async fn get(&self, id: &ItemId) -> Result<(ItemMeta, BoxRead), StoreError> {
            self.inner.get(id).await
        }
        async fn exists(&self, id: &ItemId) -> Result<bool, StoreError> {
            self.inner.exists(id).await
        }
        async fn find_by_content_key(
            &self,
            key: &ContentKey,
        ) -> Result<Option<ItemMeta>, StoreError> {
            self.inner.find_by_content_key(key).await
        }
        async fn resolve(&self, input: &str) -> Result<ItemId, StoreError> {
            self.inner.resolve(input).await
        }
        async fn delete(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
            self.inner.delete(id).await
        }
        async fn clean_staging(&self, older_than: Duration) -> Result<usize, StoreError> {
            self.inner.clean_staging(older_than).await
        }
    }

    struct Rig {
        dir: TempDir,
        failures: Arc<AtomicUsize>,
        puts: Arc<AtomicUsize>,
        opens: Arc<AtomicUsize>,
    }

    impl Rig {
        fn new(failures: usize) -> Self {
            let dir = TempDir::new().unwrap();
            std::fs::create_dir_all(dir.path().join("store")).unwrap();
            std::fs::create_dir_all(dir.path().join("drop/sent")).unwrap();
            Self {
                dir,
                failures: Arc::new(AtomicUsize::new(failures)),
                puts: Arc::new(AtomicUsize::new(0)),
                opens: Arc::new(AtomicUsize::new(0)),
            }
        }
        fn flaky(&self) -> Box<dyn Store> {
            Box::new(Flaky {
                inner: FsStore::new(
                    LocalFs::new(self.dir.path().join("store")),
                    Arc::new(SystemClock),
                    Box::new(StdRandom::new()),
                ),
                failures: self.failures.clone(),
                puts: self.puts.clone(),
            })
        }
        fn opener(&self) -> StoreOpener {
            let (root, failures, puts, opens) = (
                self.dir.path().join("store"),
                self.failures.clone(),
                self.puts.clone(),
                self.opens.clone(),
            );
            Arc::new(move || -> crate::store::BackendFuture<'static> {
                opens.fetch_add(1, Ordering::SeqCst);
                let store: Box<dyn Store> = Box::new(Flaky {
                    inner: FsStore::new(
                        LocalFs::new(root.clone()),
                        Arc::new(SystemClock),
                        Box::new(StdRandom::new()),
                    ),
                    failures: failures.clone(),
                    puts: puts.clone(),
                });
                Box::pin(async move { Ok(store) })
            })
        }
        fn uploader(&self, after_send: AfterSend) -> Uploader {
            Uploader::new(
                self.flaky(),
                self.opener(),
                "box".into(),
                after_send,
                self.drop("sent"),
            )
        }
        fn drop(&self, name: &str) -> PathBuf {
            self.dir.path().join("drop").join(name)
        }
        async fn items(&self) -> Vec<ItemMeta> {
            self.flaky().list().await.unwrap()
        }
    }

    fn quiet() -> (watch::Sender<bool>, watch::Receiver<bool>) {
        watch::channel(false)
    }

    #[tokio::test(start_paused = true)]
    async fn failed_uploads_are_retried_with_a_fresh_store() {
        let rig = Rig::new(2);
        let mut uploader = rig.uploader(AfterSend::Move);
        let (_tx, mut rx) = quiet();
        let started = tokio::time::Instant::now();
        let outcome = uploader.handle(Job::Text("hello".into()), &mut rx).await;
        assert!(matches!(outcome, JobOutcome::Sent(_)), "{outcome:?}");
        assert_eq!(
            rig.opens.load(Ordering::SeqCst),
            2,
            "one reconnect per retry"
        );
        assert_eq!(rig.puts.load(Ordering::SeqCst), 3);
        assert_eq!(
            started.elapsed(),
            Duration::from_secs(3),
            "waited 1 s then 2 s"
        );
        assert_eq!(rig.items().await.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_reconnect_keeps_retrying() {
        let rig = Rig::new(1);
        let opens = Arc::new(AtomicUsize::new(0));
        let good = rig.opener();
        let counted = opens.clone();
        let opener: StoreOpener = Arc::new(move || -> crate::store::BackendFuture<'static> {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                Box::pin(async { Err(StoreError::Backend("still down".into())) })
            } else {
                good()
            }
        });
        let mut uploader = Uploader::new(
            rig.flaky(),
            opener,
            "box".into(),
            AfterSend::Move,
            rig.drop("sent"),
        );
        let (_tx, mut rx) = quiet();
        assert!(matches!(
            uploader.handle(Job::Text("x".into()), &mut rx).await,
            JobOutcome::Sent(_)
        ));
        assert_eq!(
            opens.load(Ordering::SeqCst),
            1,
            "the first reconnect failed; the old store was retried"
        );
    }

    #[tokio::test]
    async fn text_already_on_the_server_is_not_uploaded_again() {
        let rig = Rig::new(0);
        let mut uploader = rig.uploader(AfterSend::Move);
        let (_tx, mut rx) = quiet();
        assert!(matches!(
            uploader.handle(Job::Text("hello".into()), &mut rx).await,
            JobOutcome::Sent(_)
        ));
        assert!(matches!(
            uploader.handle(Job::Text("hello".into()), &mut rx).await,
            JobOutcome::AlreadyPresent
        ));
        assert_eq!(rig.puts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sent_files_move_into_sent() {
        let rig = Rig::new(0);
        std::fs::write(rig.drop("a.txt"), b"body").unwrap();
        std::fs::write(rig.drop("sent/a.txt"), b"older").unwrap();
        let mut uploader = rig.uploader(AfterSend::Move);
        let (_tx, mut rx) = quiet();
        assert!(matches!(
            uploader.handle(Job::File(rig.drop("a.txt")), &mut rx).await,
            JobOutcome::Sent(_)
        ));
        assert!(!rig.drop("a.txt").exists());
        assert_eq!(std::fs::read(rig.drop("sent/a (1).txt")).unwrap(), b"body");
        assert_eq!(rig.items().await[0].name.as_deref(), Some("a.txt"));
    }

    #[tokio::test]
    async fn sent_files_can_be_deleted_instead() {
        let rig = Rig::new(0);
        std::fs::write(rig.drop("b.txt"), b"body").unwrap();
        let mut uploader = rig.uploader(AfterSend::Delete);
        let (_tx, mut rx) = quiet();
        assert!(matches!(
            uploader.handle(Job::File(rig.drop("b.txt")), &mut rx).await,
            JobOutcome::Sent(_)
        ));
        assert!(!rig.drop("b.txt").exists());
        assert!(!rig.drop("sent/b.txt").exists());
    }

    #[tokio::test]
    async fn duplicate_files_are_still_moved_out_of_the_drop_folder() {
        let rig = Rig::new(0);
        std::fs::write(rig.drop("c.txt"), b"same").unwrap();
        std::fs::write(rig.drop("d.txt"), b"same").unwrap();
        let mut uploader = rig.uploader(AfterSend::Move);
        let (_tx, mut rx) = quiet();
        uploader.handle(Job::File(rig.drop("c.txt")), &mut rx).await;
        assert!(matches!(
            uploader.handle(Job::File(rig.drop("d.txt")), &mut rx).await,
            JobOutcome::AlreadyPresent
        ));
        assert!(rig.drop("sent/d.txt").exists());
    }

    #[tokio::test]
    async fn vanished_files_are_skipped_without_retrying() {
        let rig = Rig::new(5);
        let mut uploader = rig.uploader(AfterSend::Move);
        let (_tx, mut rx) = quiet();
        let outcome = uploader
            .handle(Job::File(rig.drop("gone.txt")), &mut rx)
            .await;
        assert!(matches!(outcome, JobOutcome::Skipped), "{outcome:?}");
        assert_eq!(rig.opens.load(Ordering::SeqCst), 0);
        assert_eq!(rig.puts.load(Ordering::SeqCst), 0);
        let _: &Path = rig.dir.path();
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_interrupts_retrying() {
        let rig = Rig::new(usize::MAX);
        let mut uploader = rig.uploader(AfterSend::Move);
        let (tx, mut rx) = quiet();
        let stopper = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(100)).await;
            tx.send(true).unwrap();
            tx
        });
        let outcome = uploader.handle(Job::Text("never".into()), &mut rx).await;
        assert!(matches!(outcome, JobOutcome::Abandoned), "{outcome:?}");
        drop(stopper.await.unwrap());
        assert!(rig.items().await.is_empty());
    }
}
