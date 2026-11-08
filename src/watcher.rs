use crate::st::{FocusLine, StAction, StOutcome, StReport, StRunner};
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

    // Focus counts too, so a meeting where only Do Not Disturb turned on still gets cleared
    fn start_meeting(&self) {
        let report = self.run_with_retry(StAction::Meeting);
        if report.slack == StOutcome::Success || matches!(report.focus, FocusLine::Ok(_)) {
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

    // Only a Slack failure retries, since a retry reruns the whole st command
    fn run_with_retry(&self, action: StAction) -> StReport {
        let mut report = self.runner.run(action);
        if matches!(report.slack, StOutcome::Failed(_)) {
            std::thread::sleep(self.retry_delay);
            report = self.runner.run(action);
        }

        let mut result = match &report.slack {
            StOutcome::Success => format!("ok: {}", action.command()),
            StOutcome::Failed(msg) => format!("failed: {}: {msg}", action.command()),
        };
        match &report.focus {
            FocusLine::Ok(detail) | FocusLine::Skipped(detail) => {
                result.push_str(&format!("\nfocus: {detail}"));
            }
            FocusLine::Failed(detail) => result.push_str(&format!("\nfocus failed: {detail}")),
            FocusLine::Missing => {}
        }
        eprintln!("{result}");
        let _ = std::fs::create_dir_all(&self.state_dir)
            .and_then(|()| std::fs::write(self.last_result_path(), format!("{result}\n")));

        report
    }
}

/// The log line for a change in meeting state, if there was one
pub fn meeting_change(was_in_meeting: Option<bool>, in_meeting: bool) -> Option<&'static str> {
    match (was_in_meeting, in_meeting) {
        (None | Some(false), true) => Some("Zoom meeting started"),
        (Some(true), false) => Some("Zoom meeting ended"),
        _ => None,
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
        reports: RefCell<VecDeque<StReport>>,
    }

    impl FakeRunner {
        fn with_outcomes(outcomes: Vec<StOutcome>) -> Self {
            Self::with_reports(
                outcomes
                    .into_iter()
                    .map(|slack| StReport {
                        slack,
                        focus: FocusLine::Missing,
                    })
                    .collect(),
            )
        }

        fn with_reports(reports: Vec<StReport>) -> Self {
            Self {
                calls: RefCell::default(),
                reports: RefCell::new(reports.into()),
            }
        }
    }

    impl StRunner for &FakeRunner {
        fn run(&self, action: StAction) -> StReport {
            self.calls.borrow_mut().push(action);
            self.reports.borrow_mut().pop_front().unwrap_or(StReport {
                slack: StOutcome::Success,
                focus: FocusLine::Missing,
            })
        }
    }

    fn report(slack: StOutcome, focus: FocusLine) -> StReport {
        StReport { slack, focus }
    }

    #[test]
    fn focus_success_alone_still_marks_the_meeting() {
        let runner = FakeRunner::with_reports(vec![
            report(failed(), FocusLine::Ok("Do Not Disturb on".into())),
            report(failed(), FocusLine::Ok("Do Not Disturb on".into())),
        ]);
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true]);
        assert!(w.active_path().exists());
        run_ticks(&mut w, &[false]);
        assert_eq!(runner.calls.borrow().last(), Some(&StAction::Clear));
    }

    #[test]
    fn focus_failure_alone_does_not_retry() {
        let runner = FakeRunner::with_reports(vec![report(
            StOutcome::Success,
            FocusLine::Failed("boom".into()),
        )]);
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true]);
        assert_eq!(*runner.calls.borrow(), vec![StAction::Meeting]);
        assert!(w.active_path().exists());
    }

    #[test]
    fn last_result_includes_the_focus_line() {
        let runner = FakeRunner::with_reports(vec![report(
            StOutcome::Success,
            FocusLine::Ok("Do Not Disturb on".into()),
        )]);
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true]);
        let last = std::fs::read_to_string(w.last_result_path()).unwrap();
        assert_eq!(
            last.trim(),
            "ok: st meeting --focus\nfocus: Do Not Disturb on"
        );
    }

    #[test]
    fn last_result_shows_a_focus_failure() {
        let runner = FakeRunner::with_reports(vec![report(
            StOutcome::Success,
            FocusLine::Failed("The st Focus shortcut isn't installed. Run: st setup-focus".into()),
        )]);
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true]);
        let last = std::fs::read_to_string(w.last_result_path()).unwrap();
        assert_eq!(
            last.trim(),
            "ok: st meeting --focus\nfocus failed: The st Focus shortcut isn't installed. Run: st setup-focus"
        );
    }

    #[test]
    fn last_result_shows_a_focus_skip() {
        let runner = FakeRunner::with_reports(vec![
            report(
                StOutcome::Success,
                FocusLine::Ok("Do Not Disturb on".into()),
            ),
            report(
                StOutcome::Success,
                FocusLine::Skipped("Skipped, Slack status wasn't cleared".into()),
            ),
        ]);
        let mut w = watcher(&runner, temp_state_dir());
        run_ticks(&mut w, &[false, true, false]);
        let last = std::fs::read_to_string(w.last_result_path()).unwrap();
        assert_eq!(
            last.trim(),
            "ok: st clear --if meeting --focus\nfocus: Skipped, Slack status wasn't cleared"
        );
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
    fn meeting_change_logs_start_and_end() {
        assert_eq!(
            meeting_change(Some(false), true),
            Some("Zoom meeting started")
        );
        assert_eq!(
            meeting_change(Some(true), false),
            Some("Zoom meeting ended")
        );
        assert_eq!(meeting_change(None, true), Some("Zoom meeting started"));
    }

    #[test]
    fn meeting_change_is_quiet_without_a_change() {
        assert_eq!(meeting_change(Some(true), true), None);
        assert_eq!(meeting_change(Some(false), false), None);
        assert_eq!(meeting_change(None, false), None);
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
        assert_eq!(last.trim(), "ok: st meeting --focus");
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
