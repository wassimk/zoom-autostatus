use anyhow::Result;
use clap::{Parser, Subcommand};

#[allow(dead_code)] // wired into the watcher in a later commit
mod st;
#[allow(dead_code)] // wired into the start command in a later commit
mod watcher;

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
        Cmd::Start { .. } => todo!(),
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
