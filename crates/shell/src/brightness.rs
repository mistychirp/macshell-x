//! The monitor's brightness, over DDC/CI with `ddcutil`. It takes most of a second per call, so
//! everything happens off the UI thread, and only the latest level set gets sent.

use std::process::Command;
use std::sync::Mutex;
use std::thread;

use iced_futures::futures::channel::oneshot;

/// The level waiting to be sent, and whether a thread is sending.
static PENDING: Mutex<(Option<u8>, bool)> = Mutex::new((None, false));

/// The brightness from 0 to 1, or `None` without a monitor that can tell.
pub async fn level() -> Option<f32> {
    let (sender, receiver) = oneshot::channel();
    thread::spawn(move || {
        let _ = sender.send(read());
    });
    receiver.await.ok().flatten()
}

fn read() -> Option<f32> {
    let output = Command::new("ddcutil")
        .args(["getvcp", "10", "--brief"])
        .output()
        .ok()?;
    // Like "VCP 10 C 50 100": the feature, its type, the current value and the maximum.
    let text = String::from_utf8(output.stdout).ok()?;
    let line = text.lines().find(|line| line.starts_with("VCP 10 "))?;
    let mut fields = line.split_whitespace().skip(3);
    let current: f32 = fields.next()?.parse().ok()?;
    let max: f32 = fields.next()?.parse().ok()?;
    (max > 0.0).then_some(current / max)
}

/// Asks for a brightness from 0 to 1. While one is being sent, only the latest request is kept.
pub fn set_level(level: f32) {
    let percent = (level.clamp(0.0, 1.0) * 100.0).round() as u8;
    let mut pending = PENDING.lock().unwrap();
    pending.0 = Some(percent);
    if pending.1 {
        return;
    }
    pending.1 = true;
    thread::spawn(|| {
        loop {
            let next = {
                let mut pending = PENDING.lock().unwrap();
                let next = pending.0.take();
                pending.1 = next.is_some();
                next
            };
            let Some(percent) = next else {
                return;
            };
            if let Err(err) = Command::new("ddcutil")
                .args(["setvcp", "10", &percent.to_string()])
                .status()
            {
                log::warn!("can't run ddcutil: {err}");
            }
        }
    });
}
