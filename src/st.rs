use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StAction {
    Meeting,
    Clear,
}

impl StAction {
    pub fn command(&self) -> &'static str {
        match self {
            StAction::Meeting => "st meeting",
            StAction::Clear => "st clear",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StOutcome {
    Success,
    Failed(String),
}

const SLACK_OK: &str = "Slack   \u{2713}";
const SLACK_FAILED: &str = "Slack   \u{2717}";

// st exits 0 even when Slack fails, so its output is the only reliable signal
pub fn parse_st_output(stdout: &str, stderr: &str) -> StOutcome {
    let lines = || stdout.lines().chain(stderr.lines());

    if lines().any(|line| line.contains(SLACK_OK)) {
        return StOutcome::Success;
    }

    if let Some(line) = lines().find(|line| line.contains(SLACK_FAILED)) {
        let raw = line.split(SLACK_FAILED).nth(1).unwrap_or(line).trim();
        return StOutcome::Failed(friendly_error(raw));
    }

    if lines().any(|line| line.contains("command not found: st")) {
        return StOutcome::Failed(friendly_error("command not found: st"));
    }

    StOutcome::Failed(friendly_error("st printed no Slack result"))
}

pub fn friendly_error(raw: &str) -> String {
    if raw.contains("SLACK_PAT not set") {
        return "st couldn't find SLACK_PAT. The background service runs st with \
                `$SHELL -lic`, so export SLACK_PAT from a file that shell reads \
                (~/.zshrc or ~/.zprofile for zsh). Run `zoom-autostatus start --verbose` to test."
            .to_string();
    }

    if raw.contains("command not found: st") {
        return "st isn't installed or isn't on your shell's PATH. \
                Install it with: brew install wassimk/tap/st"
            .to_string();
    }

    format!("st failed: {raw}")
}

pub fn login_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string())
}

pub trait StRunner {
    fn run(&self, action: StAction) -> StOutcome;
}

pub struct ShellStRunner {
    pub shell: String,
    pub timeout: Duration,
    pub verbose: bool,
}

impl StRunner for ShellStRunner {
    fn run(&self, action: StAction) -> StOutcome {
        run_with_timeout(&self.shell, action.command(), self.timeout, self.verbose)
    }
}

pub enum ShellRun {
    Finished {
        success: bool,
        stdout: String,
        stderr: String,
    },
    TimedOut,
    SpawnFailed(String),
}

/// Runs `command` in an interactive login shell so it sees the user's exported tokens
pub fn run_shell(shell: &str, command: &str, timeout: Duration) -> ShellRun {
    let mut child = match Command::new(shell)
        .args(["-lic", command])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Its own process group, so anything the startup files leave running can be killed with it
        .process_group(0)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return ShellRun::SpawnFailed(format!("Failed to start {shell}: {e}")),
    };
    let process_group = child.id();

    // Read the pipes on their own threads so a chatty shell can't fill them and stall
    let stdout = read_in_background(child.stdout.take());
    let stderr = read_in_background(child.stderr.take());

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                kill_process_group(process_group);
                let _ = child.wait();
                return ShellRun::TimedOut;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return ShellRun::SpawnFailed(format!("Failed waiting for {shell}: {e}")),
        }
    };

    // A background process started by the shell can keep the pipes open after the shell exits
    kill_process_group(process_group);
    let deadline = Instant::now() + OUTPUT_GRACE;

    ShellRun::Finished {
        success: status.success(),
        stdout: collect_output(&stdout, deadline),
        stderr: collect_output(&stderr, deadline),
    }
}

fn kill_process_group(process_group: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", &format!("-{process_group}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

// The shell has exited, so its own output is already in the pipe. Anything still holding the
// pipe open after the grace period is a stray process we don't wait for.
const OUTPUT_GRACE: Duration = Duration::from_millis(500);

fn collect_output(chunks: &Receiver<Vec<u8>>, deadline: Instant) -> String {
    let mut output = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match chunks.recv_timeout(remaining) {
            Ok(chunk) => output.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn run_with_timeout(shell: &str, command: &str, timeout: Duration, verbose: bool) -> StOutcome {
    match run_shell(shell, command, timeout) {
        ShellRun::Finished { stdout, stderr, .. } => {
            if verbose {
                eprintln!("[st stdout]\n{}", stdout.trim_end());
                eprintln!("[st stderr]\n{}", stderr.trim_end());
            }
            parse_st_output(&stdout, &stderr)
        }
        ShellRun::TimedOut => StOutcome::Failed(format!(
            "`{command}` timed out after {}s. Something in your shell startup \
             may be waiting for input.",
            timeout.as_secs()
        )),
        ShellRun::SpawnFailed(msg) => StOutcome::Failed(msg),
    }
}

fn read_in_background<R: Read + Send + 'static>(pipe: Option<R>) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let Some(mut pipe) = pipe else { return };
        let mut buffer = [0u8; 4096];
        while let Ok(read) = pipe.read(&mut buffer) {
            if read == 0 || sender.send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn failed_message(outcome: StOutcome) -> String {
        match outcome {
            StOutcome::Failed(msg) => msg,
            StOutcome::Success => panic!("expected Failed, got Success"),
        }
    }

    #[test]
    fn success_when_slack_check_line_present() {
        let stdout = "  Slack   \u{2713} In a meeting :virtual-meeting:\n  GitHub  - No change\n";
        assert_eq!(parse_st_output(stdout, ""), StOutcome::Success);
    }

    #[test]
    fn github_failure_is_ignored() {
        let stdout = "  Slack   \u{2713} Cleared (DND off)\n";
        let stderr = "  GitHub  \u{2717} GITHUB_PAT not set\n";
        assert_eq!(parse_st_output(stdout, stderr), StOutcome::Success);
    }

    #[test]
    fn missing_slack_token_is_friendly() {
        let msg = failed_message(parse_st_output(
            "",
            "  Slack   \u{2717} SLACK_PAT not set\n",
        ));
        for expected in [
            "SLACK_PAT",
            "-lic",
            "~/.zshrc",
            "zoom-autostatus start --verbose",
        ] {
            assert!(msg.contains(expected), "missing {expected:?} in {msg:?}");
        }
    }

    #[test]
    fn st_not_found_is_friendly() {
        let msg = failed_message(parse_st_output("", "zsh:1: command not found: st\n"));
        assert!(msg.contains("brew install wassimk/tap/st"), "{msg:?}");
    }

    #[test]
    fn unknown_slack_error_passes_through() {
        let msg = failed_message(parse_st_output("", "  Slack   \u{2717} invalid_auth\n"));
        assert!(msg.contains("invalid_auth"), "{msg:?}");
    }

    #[test]
    fn no_slack_line_at_all_is_failure() {
        assert!(matches!(parse_st_output("", ""), StOutcome::Failed(_)));
    }

    #[test]
    fn shell_noise_without_tty_is_ignored() {
        let stdout = "  Slack   \u{2713} In a meeting :virtual-meeting:\n";
        let stderr = "(eval):1: can't change option: zle\n(eval):1: can't change option: zle\n";
        assert_eq!(parse_st_output(stdout, stderr), StOutcome::Success);
    }

    #[test]
    fn runner_times_out() {
        let started = Instant::now();
        let msg = failed_message(run_with_timeout(
            "/bin/sh",
            "sleep 5",
            Duration::from_millis(200),
            false,
        ));
        assert!(msg.contains("timed out"), "{msg:?}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn background_child_holding_output_does_not_block() {
        let started = Instant::now();
        let run = run_shell("/bin/sh", "sleep 5 & echo hi", Duration::from_secs(3));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
        match run {
            ShellRun::Finished { stdout, .. } => assert!(stdout.contains("hi"), "{stdout:?}"),
            _ => panic!("expected the shell to finish"),
        }
    }

    #[test]
    fn action_commands() {
        assert_eq!(StAction::Meeting.command(), "st meeting");
        assert_eq!(StAction::Clear.command(), "st clear");
    }
}
