use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vigia_lib::commands::{CommandError, ErrorKind};
use vigia_lib::updates::{
    run_schedule, PendingUpdate, ProgressSink, UpdateError, UpdateInfo, UpdateManager,
    UpdateProgress, UpdateSource, CHECK_INTERVAL, FIRST_CHECK_DELAY,
};

fn release(version: &str) -> UpdateInfo {
    UpdateInfo {
        version: version.into(),
        notes: Some("Example notes".into()),
    }
}

struct FakeUpdate {
    info: UpdateInfo,
    installs: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl PendingUpdate for FakeUpdate {
    fn info(&self) -> UpdateInfo {
        self.info.clone()
    }

    async fn download_and_install(&self, progress: ProgressSink<'_>) -> Result<(), UpdateError> {
        progress(UpdateProgress {
            downloaded: 50,
            total: Some(100),
        });
        progress(UpdateProgress {
            downloaded: 100,
            total: Some(100),
        });
        self.installs.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Answers each check with the next scripted result and counts the checks.
#[derive(Default)]
struct FakeSource {
    answers: Mutex<Vec<Result<Option<UpdateInfo>, UpdateError>>>,
    checks: AtomicUsize,
    installs: Arc<AtomicUsize>,
}

impl FakeSource {
    fn answering(answers: Vec<Result<Option<UpdateInfo>, UpdateError>>) -> Arc<FakeSource> {
        let mut answers = answers;
        answers.reverse();
        Arc::new(FakeSource {
            answers: Mutex::new(answers),
            ..Default::default()
        })
    }
}

#[async_trait::async_trait]
impl UpdateSource for FakeSource {
    async fn check(&self) -> Result<Option<Arc<dyn PendingUpdate>>, UpdateError> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        let answer = self.answers.lock().unwrap().pop().unwrap_or(Ok(None));
        let found = answer?;
        Ok(found.map(|info| {
            Arc::new(FakeUpdate {
                info,
                installs: Arc::clone(&self.installs),
            }) as Arc<dyn PendingUpdate>
        }))
    }
}

type Published = Arc<Mutex<Vec<Option<UpdateInfo>>>>;

fn manager(source: &Arc<FakeSource>) -> (Arc<UpdateManager>, Published) {
    let published: Published = Arc::default();
    let sink = Arc::clone(&published);
    let manager = UpdateManager::new(
        Arc::clone(source) as Arc<dyn UpdateSource>,
        Arc::new(move |update| sink.lock().unwrap().push(update)),
    );
    (Arc::new(manager), published)
}

#[tokio::test]
async fn a_check_hands_its_result_to_the_sink_and_a_failure_keeps_the_pending_update() {
    let source = FakeSource::answering(vec![
        Ok(Some(release("9.9.9"))),
        Err(UpdateError::Network("offline".into())),
        Ok(None),
    ]);
    let (manager, published) = manager(&source);

    assert_eq!(manager.check_now().await, Ok(Some(release("9.9.9"))));
    assert_eq!(manager.pending(), Some(release("9.9.9")));

    let failed = manager.check_now().await;
    assert_eq!(failed, Err(UpdateError::Network("offline".into())));
    assert_eq!(manager.pending(), Some(release("9.9.9")));

    assert_eq!(manager.check_now().await, Ok(None));
    assert_eq!(manager.pending(), None);
    assert_eq!(
        *published.lock().unwrap(),
        vec![Some(release("9.9.9")), None]
    );
}

#[tokio::test]
async fn installing_without_a_pending_update_is_invalid_input() {
    let source = FakeSource::answering(Vec::new());
    let (manager, _) = manager(&source);
    let error = manager.install(&|_| {}).await.unwrap_err();
    assert_eq!(error, UpdateError::NothingPending);
    assert_eq!(CommandError::from(error).kind, ErrorKind::InvalidInput);
    assert_eq!(source.installs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn installing_the_pending_update_reports_progress() {
    let source = FakeSource::answering(vec![Ok(Some(release("9.9.9")))]);
    let (manager, _) = manager(&source);
    manager.check_now().await.unwrap();
    let progress = Mutex::new(Vec::new());
    let record = |p: UpdateProgress| progress.lock().unwrap().push(p.downloaded);
    manager.install(&record).await.unwrap();
    assert_eq!(*progress.lock().unwrap(), vec![50, 100]);
    assert_eq!(source.installs.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn scheduled_checks_run_a_minute_after_launch_then_daily_while_enabled() {
    let source = FakeSource::answering(Vec::new());
    let (manager, _) = manager(&source);
    let enabled = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&enabled);
    let schedule = tokio::spawn(run_schedule(manager, move || flag.load(Ordering::SeqCst)));
    let checks = || source.checks.load(Ordering::SeqCst);
    let second = Duration::from_secs(1);

    tokio::time::sleep(FIRST_CHECK_DELAY - second).await;
    assert_eq!(checks(), 0);
    tokio::time::sleep(second * 2).await;
    assert_eq!(checks(), 1);

    tokio::time::sleep(CHECK_INTERVAL - second * 2).await;
    assert_eq!(checks(), 1);
    tokio::time::sleep(second * 2).await;
    assert_eq!(checks(), 2);

    // Turned off, the daily tick checks nothing; turned back on, the next tick checks again.
    enabled.store(false, Ordering::SeqCst);
    tokio::time::sleep(CHECK_INTERVAL).await;
    assert_eq!(checks(), 2);
    enabled.store(true, Ordering::SeqCst);
    tokio::time::sleep(CHECK_INTERVAL).await;
    assert_eq!(checks(), 3);
    schedule.abort();
}

#[tokio::test(start_paused = true)]
async fn scheduled_checks_never_run_while_disabled() {
    let source = FakeSource::answering(Vec::new());
    let (manager, _) = manager(&source);
    let schedule = tokio::spawn(run_schedule(manager, || false));
    tokio::time::sleep(FIRST_CHECK_DELAY + CHECK_INTERVAL * 3).await;
    assert_eq!(source.checks.load(Ordering::SeqCst), 0);
    schedule.abort();
}
