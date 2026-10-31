Output "demo/zoom-autostatus.gif"

Require cargo

Set Shell "bash"
Set FontFamily "JetBrainsMono Nerd Font"
Set FontSize 16
Set Width 900
Set Height 550
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 50ms

# Sandbox HOME for the LaunchAgent, logs, and state file. demo/bin goes first on PATH
# so pgrep, st, and launchctl are the fakes, and SHELL is the fake login shell.
Hide
Type "cargo build --release -q && export DEMO_HOME=$(mktemp -d) && mkdir -p $DEMO_HOME/Library/LaunchAgents"
Enter
Sleep 3s
# The meeting starts once the log is being followed on screen, and lasts one poll
Type "export HOME=$DEMO_HOME PATH=$PWD/demo/bin:$PWD/target/release:$PATH SHELL=$PWD/demo/bin/zsh SLACK_PAT=demo DEMO_MEETING_START=$(( $(date +%s) + 22 )) PS1='$ '"
Enter
# Show paths under HOME as ~ and the fake shell as /bin/zsh
Type `demo_paths() { sed -e "s|$SHELL|/bin/zsh|g" -e "s|$HOME|~|g"; }; zoom-autostatus() { command zoom-autostatus "$@" | demo_paths; }; tail() { command tail "$@" | demo_paths; }; clear`
Enter
Sleep 500ms
Show

Type "zoom-autostatus install"
Sleep 500ms
Enter
Wait+Screen /installed and loaded/
Sleep 2s

Type "zoom-autostatus status"
Sleep 500ms
Enter
Wait+Screen /Logs:/
Sleep 4s

Ctrl+L
Type "tail -f ~/Library/Logs/zoom-autostatus/stderr.log"
Sleep 500ms
Enter
Wait+Screen@40s /ok: st meeting/
Wait+Screen@30s /ok: st clear --if meeting/
Sleep 3s

Hide
Ctrl+C
Type "launchctl bootout x; rm -rf $DEMO_HOME"
Enter
