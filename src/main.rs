use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use detect::{MeetingDetector, ZoomDetector};
use st::{login_shell, ShellStRunner};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use watcher::{default_state_dir, Watcher};

mod detect;
mod st;
mod watcher;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_secs(5);
const ST_TIMEOUT: Duration = Duration::from_secs(30);

// --- Watcher ---

fn run_watcher(verbose: bool) -> Result<()> {
    let shell = login_shell();
    let runner = ShellStRunner {
        shell: shell.clone(),
        timeout: ST_TIMEOUT,
        verbose,
    };
    let mut watcher = Watcher::new(runner, default_state_dir(), RETRY_DELAY);
    let detector = ZoomDetector;

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_flag = shutdown.clone();

    ctrlc::set_handler(move || {
        shutdown_flag.store(true, Ordering::SeqCst);
    })
    .context("Failed to set signal handler")?;

    eprintln!("Watching for Zoom meetings using {shell}...");

    let mut last_seen = None;
    while !shutdown.load(Ordering::SeqCst) {
        let in_meeting = detector.in_meeting();
        if verbose && last_seen != Some(in_meeting) {
            eprintln!("Zoom meeting: {}", if in_meeting { "yes" } else { "no" });
        }
        last_seen = Some(in_meeting);

        watcher.tick(in_meeting);

        // Sleep in short slices so Ctrl-C stops the loop quickly
        let mut slept = Duration::ZERO;
        while slept < POLL_INTERVAL && !shutdown.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(250));
            slept += Duration::from_millis(250);
        }
    }

    eprintln!("Shutting down.");
    Ok(())
}

// --- CLI ---

#[derive(Parser, Debug)]
#[command(
    name = "zoom-autostatus",
    about = "Automatically set your Slack status while you're in a Zoom meeting",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the Zoom meeting watcher in the foreground
    Start {
        #[arg(short, long, help = "Log every meeting change and the full st output")]
        verbose: bool,
    },
    /// Install the LaunchAgent for automatic startup
    Install {
        #[arg(short, long, help = "Overwrite existing LaunchAgent")]
        force: bool,
    },
    /// Uninstall the LaunchAgent
    Uninstall,
    /// Stop the background service
    Stop,
    /// Restart the background service
    Restart,
    /// Show running state, meeting state, last st result, and log paths
    Status,
}

// --- main ---

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Cmd::Start { verbose } => run_watcher(verbose),
        Cmd::Install { .. } => todo!(),
        Cmd::Uninstall => todo!(),
        Cmd::Stop => todo!(),
        Cmd::Restart => todo!(),
        Cmd::Status => {
            println!("not implemented");
            Ok(())
        }
    }
}
