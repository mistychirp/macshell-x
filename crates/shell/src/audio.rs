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

/// A place sound can go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    id: u32,
    pub name: String,
    pub current: bool,
}

impl Output {
    /// Sends sound here.
    pub fn select(&self) {
        run(vec![String::from("set-default"), self.id.to_string()]);
    }
}

/// The sound outputs, from `wpctl status`.
pub fn outputs() -> Vec<Output> {
    let Ok(output) = Command::new("wpctl").arg("status").output() else {
        return Vec::new();
    };
    parse_outputs(&String::from_utf8_lossy(&output.stdout))
}

/// Reads the lines in the Audio section's list of sinks, like `│  *   55. Speakers [vol: 0.50]`.
fn parse_outputs(status: &str) -> Vec<Output> {
    let audio = status.split("Audio\n").nth(1).unwrap_or_default();
    let Some((_, sinks)) = audio.split_once("Sinks:\n") else {
        return Vec::new();
    };
    sinks
        .lines()
        .take_while(|line| !line.contains("─"))
        .filter_map(|line| {
            let line = line.trim_start_matches(|c: char| c == '│' || c.is_whitespace());
            let (current, line) = match line.strip_prefix('*') {
                Some(line) => (true, line.trim_start()),
                None => (false, line),
            };
            let (id, rest) = line.split_once(". ")?;
            let name = rest.split(" [vol:").next().unwrap_or(rest).trim();
            Some(Output {
                id: id.parse().ok()?,
                name: name.to_owned(),
                current,
            })
        })
        .collect()
}
