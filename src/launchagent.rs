use crate::detect::{MeetingDetector, ZoomDetector};
use crate::st::{friendly_error, login_shell, run_shell, ShellRun};
use crate::watcher::default_state_dir;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const LABEL: &str = "com.wassimk.zoom-autostatus";
const TOKEN_CHECK_TIMEOUT: Duration = Duration::from_secs(30);

fn plist_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join("Library/LaunchAgents/com.wassimk.zoom-autostatus.plist")
}

fn log_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join("Library/Logs/zoom-autostatus")
}

fn current_uid() -> String {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .expect("failed to run id -u");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

// SHELL is the installer's shell, so the agent loads the same startup files as their terminal
pub fn generate_plist(binary_path: &str, shell: &str) -> String {
    let log_dir = log_dir();
    let stdout_log = log_dir.join("stdout.log");
    let stderr_log = log_dir.join("stderr.log");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary_path}</string>
        <string>start</string>
    </array>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{stdout}</string>
    <key>StandardErrorPath</key>
    <string>{stderr}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
        <key>SHELL</key>
        <string>{shell}</string>
    </dict>
</dict>
</plist>"#,
        stdout = stdout_log.display(),
        stderr = stderr_log.display(),
    )
}

/// True when the login shell exports a non-empty SLACK_PAT. Never prints the token.
pub fn token_check(shell: &str) -> bool {
    matches!(
        run_shell(shell, r#"[ -n "$SLACK_PAT" ]"#, TOKEN_CHECK_TIMEOUT),
        ShellRun::Finished { success: true, .. }
    )
}

pub fn install_launchagent(force: bool) -> Result<()> {
    let plist = plist_path();

    if plist.exists() && !force {
        anyhow::bail!(
            "LaunchAgent already installed at {}\nUse --force to overwrite.",
            plist.display()
        );
    }

    let binary_path = std::env::current_exe()
        .context("Failed to determine binary path")?
        .to_string_lossy()
        .to_string();
    let shell = login_shell();

    // Unload existing agent if overwriting
    if plist.exists() {
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("gui/{}/{LABEL}", current_uid())])
            .output();
    }

    std::fs::create_dir_all(log_dir()).context("Failed to create log directory")?;

    let content = generate_plist(&binary_path, &shell);
    std::fs::write(&plist, content)
        .with_context(|| format!("Failed to write plist to {}", plist.display()))?;

    let output = Command::new("launchctl")
        .args([
            "bootstrap",
            &format!("gui/{}", current_uid()),
            &plist.to_string_lossy(),
        ])
        .output()
        .context("Failed to run launchctl bootstrap")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("launchctl bootstrap failed: {}", stderr.trim());
    }

    println!("LaunchAgent installed and loaded.");
    println!("  Plist: {}", plist.display());
    println!("  Logs:  {}", log_dir().display());

    if !token_check(&shell) {
        println!();
        println!("Warning: {}", friendly_error("SLACK_PAT not set"));
    }

    Ok(())
}

pub fn uninstall_launchagent() -> Result<()> {
    // Unload (ignore errors if not loaded)
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("gui/{}/{LABEL}", current_uid())])
        .output();

    let plist = plist_path();
    if plist.exists() {
        std::fs::remove_file(&plist)
            .with_context(|| format!("Failed to remove {}", plist.display()))?;
        println!("LaunchAgent uninstalled.");
    } else {
        println!("LaunchAgent was not installed.");
    }

    Ok(())
}

pub fn stop_launchagent() -> Result<()> {
    let target = format!("gui/{}/{LABEL}", current_uid());

    let output = Command::new("launchctl")
        .args(["bootout", &target])
        .output()
        .context("Failed to run launchctl bootout")?;

    if output.status.success() {
        println!("Service stopped.");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No such process") || stderr.contains("Could not find service") {
            println!("Service is not running.");
        } else {
            anyhow::bail!("Failed to stop service: {}", stderr.trim());
        }
    }

    Ok(())
}

pub fn restart_launchagent() -> Result<()> {
    let target = format!("gui/{}/{LABEL}", current_uid());

    let output = Command::new("launchctl")
        .args(["kickstart", "-k", &target])
        .output()
        .context("Failed to run launchctl kickstart")?;

    if output.status.success() {
        println!("Service restarted.");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to restart service: {}", stderr.trim());
    }

    Ok(())
}

pub fn show_status() -> Result<()> {
    let output = Command::new("launchctl")
        .args(["list", LABEL])
        .output()
        .context("Failed to run launchctl list")?;

    let running = output.status.success();
    let plist = plist_path();
    let installed = plist.exists();
    let state_dir = default_state_dir();
    let status_set = state_dir.join("active").exists();
    let last_result = std::fs::read_to_string(state_dir.join("last-result"))
        .map(|result| result.trim().to_string())
        .unwrap_or_else(|_| "none yet".to_string());
    let yes_no = |value: bool| if value { "yes" } else { "no" };

    println!("Service:          {LABEL}");
    println!("Installed:        {}", yes_no(installed));
    println!("Running:          {}", yes_no(running));
    println!();
    println!("In meeting:       {}", yes_no(ZoomDetector.in_meeting()));
    println!("Status set by us: {}", yes_no(status_set));
    println!("Last st result:   {last_result}");
    println!();
    println!("Paths:");
    println!("  Plist: {}", plist.display());
    println!("  State: {}", state_dir.display());
    println!("  Logs:  {}", log_dir().display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_runs_start_with_keepalive() {
        let plist = generate_plist("/usr/local/bin/zoom-autostatus", "/bin/zsh");
        for expected in [
            "<string>com.wassimk.zoom-autostatus</string>",
            "<string>/usr/local/bin/zoom-autostatus</string>",
            "<string>start</string>",
            "<key>KeepAlive</key>",
        ] {
            assert!(plist.contains(expected), "missing {expected:?}");
        }
    }

    #[test]
    fn plist_passes_shell() {
        let plist = generate_plist("/x", "/opt/homebrew/bin/fish");
        let shell_key = plist.find("<key>SHELL</key>").expect("no SHELL key");
        let after = &plist[shell_key..];
        let value = after
            .find("<string>/opt/homebrew/bin/fish</string>")
            .expect("no SHELL value");
        assert!(
            after[..value].lines().count() <= 2,
            "SHELL value doesn't follow its key"
        );
    }
}
