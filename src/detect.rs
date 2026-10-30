use std::process::{Command, Stdio};

pub trait MeetingDetector {
    fn in_meeting(&self) -> bool;
}

/// Zoom runs its CptHost helper only while you're in a meeting
pub struct ZoomDetector;

impl MeetingDetector for ZoomDetector {
    fn in_meeting(&self) -> bool {
        Command::new("pgrep")
            .args(["-x", "CptHost"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Needs macOS and no Zoom meeting in progress, so it's run by hand
    #[test]
    #[ignore]
    fn detector_reports_false_without_zoom() {
        assert!(!ZoomDetector.in_meeting());
    }
}
