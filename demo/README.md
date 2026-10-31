# Demo

The README GIF is rendered with [VHS](https://github.com/charmbracelet/vhs) from `zoom-autostatus.tape`.

## Requirements

- `vhs`
- A Rust toolchain (the tape builds `target/release/zoom-autostatus`)
- JetBrainsMono Nerd Font

## Rendering

From the repo root:

```shell
vhs demo/zoom-autostatus.tape
```

This overwrites `demo/zoom-autostatus.gif`.

## How it works

No Zoom meeting or Slack token is needed, and the real launchd, `st`, and shell startup files are never touched.

- The tape puts `demo/bin` first on `PATH`, so the CLI runs the fakes in it.
- `bin/pgrep` reports Zoom's `CptHost` as running on the first check at or after `DEMO_MEETING_START` (epoch seconds) and as gone on every check after that, so the meeting lasts one 5 second poll. The tape starts it 22 seconds after setup, so it lands while the log is followed on screen.
- `bin/st` prints what `st` prints and adds a dimmed `slack:` line to the log to show the status change.
- `bin/zsh` stands in for the login shell (`SHELL` points at it), so `-lic` runs the command without loading real startup files or tokens. `SLACK_PAT` is set to a placeholder so the install token check passes.
- `bin/launchctl` handles `bootstrap` by running `zoom-autostatus start` in the background with output going to the usual log files, and answers `list` and `bootout` for that process.
- The tape points `HOME` at a temp directory first, so the LaunchAgent plist, logs, and state file all live there. Hidden shell functions rewrite that temp path to `~` and the fake shell's path to `/bin/zsh` in the output.
