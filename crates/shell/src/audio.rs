//! The default speaker's volume, through WirePlumber's `wpctl`.

use std::process::Command;
use std::thread;

const SINK: &str = "@DEFAULT_AUDIO_SINK@";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Volume {
    /// From 0 to 1; WirePlumber allows more, which shows as full.
    pub level: f32,
    pub muted: bool,
}

/// The default speaker's volume, or `None` without WirePlumber or a speaker.
pub fn volume() -> Option<Volume> {
    let output = Command::new("wpctl")
        .args(["get-volume", SINK])
        .output()
        .inspect_err(|err| log::warn!("can't run wpctl: {err}"))
        .ok()?;
    // Like "Volume: 0.45" or "Volume: 0.45 [MUTED]".
    let text = String::from_utf8(output.stdout).ok()?;
    let mut words = text.split_whitespace();
    if words.next() != Some("Volume:") {
        return None;
    }
    let level: f32 = words.next()?.parse().ok()?;
    Some(Volume {
        level: level.clamp(0.0, 1.0),
        muted: words.any(|word| word == "[MUTED]"),
    })
}

pub fn set_level(level: f32) {
    run(vec![
        String::from("set-volume"),
        String::from(SINK),
        format!("{:.3}", level.clamp(0.0, 1.0)),
    ]);
}

pub fn set_muted(muted: bool) {
    run(vec![
        String::from("set-mute"),
        String::from(SINK),
        String::from(if muted { "1" } else { "0" }),
    ]);
}

/// Runs `wpctl` off the UI thread; it reports its own failures.
fn run(args: Vec<String>) {
    thread::spawn(move || {
        if let Err(err) = Command::new("wpctl").args(&args).status() {
            log::warn!("can't run wpctl: {err}");
        }
    });
}
