use crate::st::{StAction, StOutcome, StRunner};
use std::path::PathBuf;
use std::time::Duration;

pub struct Watcher<R: StRunner> {
    runner: R,
    state_dir: PathBuf,
    retry_delay: Duration,
    was_in_meeting: Option<bool>,
}

impl<R: StRunner> Watcher<R> {
    pub fn new(runner: R, state_dir: PathBuf, retry_delay: Duration) -> Self {
        Self {
            runner,
            state_dir,
            retry_delay,
            was_in_meeting: None,
        }
    }

    pub fn active_path(&self) -> PathBuf {
        self.state_dir.join("active")
    }

    pub fn last_result_path(&self) -> PathBuf {
        self.state_dir.join("last-result")
    }

    pub fn tick(&mut self, in_meeting: bool) {
        let was_in_meeting = self.was_in_meeting.replace(in_meeting);
        let we_set_status = self.active_path().exists();

        match (was_in_meeting, in_meeting) {
            // First tick: reconcile with whatever the last run left behind
            (None, true) if we_set_status => {}
            (None, true) | (Some(false), true) => self.start_meeting(),
            (None, false) | (Some(true), false) if we_set_status => self.end_meeting(),
            _ => {}
        }
    }

    fn start_meeting(&self) {
        if self.run_with_retry(StAction::Meeting) {
            if let Err(e) = std::fs::create_dir_all(&self.state_dir)
                .and_then(|()| std::fs::write(self.active_path(), ""))
            {
                eprintln!("Failed to write {}: {e}", self.active_path().display());
            }
        }
    }

    // A failed clear still drops the state file so the next meeting starts clean
    fn end_meeting(&self) {
        self.run_with_retry(StAction::Clear);
        let _ = std::fs::remove_file(self.active_path());
    }

    fn run_with_retry(&self, action: StAction) -> bool {
        let mut outcome = self.runner.run(action);
        if matches!(outcome, StOutcome::Failed(_)) {
            std::thread::sleep(self.retry_delay);
            outcome = self.runner.run(action);
        }

        let result = match &outcome {
            StOutcome::Success => format!("ok: {}", action.command()),
            StOutcome::Failed(msg) => format!("failed: {}: {msg}", action.command()),
        };
        eprintln!("{result}");
        let _ = std::fs::create_dir_all(&self.state_dir)
            .and_then(|()| std::fs::write(self.last_result_path(), format!("{result}\n")));

        outcome == StOutcome::Success
    }
}

pub fn default_state_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join("Library/Application Support/zoom-autostatus")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct FakeRunner {
        calls: RefCell<Vec<StAction>>,
        outcomes: RefCell<VecDeque<StOutcome>>,
    }

    impl FakeRunner {
        fn with_outcomes(outcomes: Vec<StOutcome>) -> Self {
            Self {
                calls: RefCell::default(),
                outcomes: RefCell::new(outcomes.into()),
            }
        }
    }

    impl StRunner for &FakeRunner {
        fn run(&self, action: StAction) -> StOutcome {
            self.calls.borrow_mut().push(action);
            self.outcomes
                .borrow_mut()
                .pop_front()
                .unwrap_or(StOutcome::Success)
        }
    }

    fn temp_state_dir() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "zoom-autostatus-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn watcher(runner: &FakeRunner, state_dir: PathBuf) -> Watcher<&FakeRunner> {
        Watcher::new(runner, state_dir, Duration::ZERO)
    }

    fn failed() -> StOutcome {
        StOutcome::Failed("boom".to_string())
    }

    fn run_ticks(watcher: &mut Watcher<&FakeRunner>, ticks: &[bool]) {
        for &in_meeting in ticks {
            watcher.tick(in_meeting);
        }
    }

    #[test]
    fn meeting_start_sets_status_and_creates_state_file() {
        let runner = FakeRunner::default();
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true]);
        assert_eq!(*runner.calls.borrow(), vec![StAction::Meeting]);
        assert!(w.active_path().exists());
    }

    #[test]
    fn meeting_end_clears_and_removes_state_file() {
        let runner = FakeRunner::default();
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true, false]);
        assert_eq!(
            *runner.calls.borrow(),
            vec![StAction::Meeting, StAction::Clear]
        );
        assert!(!w.active_path().exists());
    }

    #[test]
    fn no_meeting_never_touches_status() {
        let runner = FakeRunner::default();
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, false, false]);
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn steady_meeting_calls_st_once() {
        let runner = FakeRunner::default();
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true, true, true]);
        assert_eq!(*runner.calls.borrow(), vec![StAction::Meeting]);
    }

    #[test]
    fn startup_with_leftover_state_and_no_meeting_clears() {
        let runner = FakeRunner::default();
        let dir = temp_state_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("active"), "").unwrap();
        let mut w = watcher(&runner, dir);
        run_ticks(&mut w, &[false]);
        assert_eq!(*runner.calls.borrow(), vec![StAction::Clear]);
        assert!(!w.active_path().exists());
    }

    #[test]
    fn startup_mid_meeting_with_state_does_nothing() {
        let runner = FakeRunner::default();
        let dir = temp_state_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("active"), "").unwrap();
        let mut w = watcher(&runner, dir);
        run_ticks(&mut w, &[true]);
        assert!(runner.calls.borrow().is_empty());
        assert!(w.active_path().exists());
    }

    #[test]
    fn startup_mid_meeting_without_state_sets_status() {
        let runner = FakeRunner::default();
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[true]);
        assert_eq!(*runner.calls.borrow(), vec![StAction::Meeting]);
        assert!(w.active_path().exists());
    }

    #[test]
    fn failed_meeting_retries_once_then_gives_up() {
        let runner = FakeRunner::with_outcomes(vec![failed(), failed()]);
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true, true]);
        assert_eq!(
            *runner.calls.borrow(),
            vec![StAction::Meeting, StAction::Meeting]
        );
        assert!(!w.active_path().exists());
        let last = std::fs::read_to_string(w.last_result_path()).unwrap();
        assert!(last.starts_with("failed: st meeting"), "{last:?}");
    }

    #[test]
    fn retry_success_counts() {
        let runner = FakeRunner::with_outcomes(vec![failed(), StOutcome::Success]);
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true]);
        assert_eq!(
            *runner.calls.borrow(),
            vec![StAction::Meeting, StAction::Meeting]
        );
        assert!(w.active_path().exists());
    }

    #[test]
    fn failed_clear_still_removes_state_file() {
        let runner = FakeRunner::with_outcomes(vec![StOutcome::Success, failed(), failed()]);
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true, false]);
        assert_eq!(
            *runner.calls.borrow(),
            vec![StAction::Meeting, StAction::Clear, StAction::Clear]
        );
        assert!(!w.active_path().exists());
    }

    #[test]
    fn records_successful_result() {
        let runner = FakeRunner::default();
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true]);
        let last = std::fs::read_to_string(w.last_result_path()).unwrap();
        assert_eq!(last.trim(), "ok: st meeting");
    }

    #[test]
    fn creates_missing_state_dir() {
        let runner = FakeRunner::default();
        let dir = temp_state_dir().join("nested/deeper");
        let mut w = watcher(&runner, dir);
        run_ticks(&mut w, &[false, true]);
        assert!(w.active_path().exists());
    }
}
