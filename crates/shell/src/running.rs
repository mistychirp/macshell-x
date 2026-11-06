//! Which apps are running, windows or not. A closed window doesn't mean a closed app: plenty of them
//! carry on in the tray. Once an app has shown a window, KWin has told us its process, and it stays
//! running until that process ends. Pinned apps that started without a window, like ones that open
//! at login into the tray, are found by the name of their program.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Programs that say nothing about which app is running them.
const GENERIC_PROGRAMS: [&str; 14] = [
    "sh",
    "bash",
    "fish",
    "zsh",
    "env",
    "python",
    "python3",
    "perl",
    "ruby",
    "node",
    "java",
    "flatpak",
    "snap",
    "steam-run",
];

/// A process, told apart from a later one that gets the same id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Process {
    pid: u32,
    started: u64,
}

impl Process {
    /// The process with this id, if there is one.
    pub fn of(pid: u32) -> Option<Self> {
        let status = Status::read(pid)?;
        Some(Self {
            pid,
            started: status.started,
        })
    }

    fn is_alive(self) -> bool {
        Self::of(self.pid) == Some(self)
    }

    /// Asks it to quit, if it's still the same process.
    pub fn terminate(self) {
        if !self.is_alive() {
            return;
        }
        let Ok(pid) = i32::try_from(self.pid) else {
            return;
        };
        // SAFETY: `kill` takes plain integers, and only affects the process checked just above.
        if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
            log::warn!(
                "can't quit process {}: {}",
                self.pid,
                std::io::Error::last_os_error()
            );
        }
    }
}

struct Status {
    parent: u32,
    started: u64,
}

impl Status {
    /// What `/proc/<pid>/stat` says, unless the process is gone or only waiting to be reaped.
    fn read(pid: u32) -> Option<Self> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        Self::parse(&stat)
    }

    fn parse(stat: &str) -> Option<Self> {
        // The name in parentheses can hold anything, spaces and parentheses too.
        let (_, rest) = stat.rsplit_once(')')?;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if matches!(*fields.first()?, "Z" | "X") {
            return None;
        }
        Some(Self {
            parent: fields.get(1)?.parse().ok()?,
            started: fields.get(19)?.parse().ok()?,
        })
    }
}

/// A program's name without its path, or the wrappers and suffixes packaging adds, like NixOS's
/// `.Telegram-wrapped` for `Telegram`, or `vivaldi-bin` for `vivaldi`.
fn normalized(path: &str) -> String {
    let name = path
        .trim_end_matches(" (deleted)")
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_start_matches('.')
        .to_lowercase();
    let name = name.strip_suffix("-wrapped").unwrap_or(&name);
    name.strip_suffix("-bin").unwrap_or(name).to_owned()
}

/// The name to look for an app's processes by, if its command gives a useful one.
pub fn program_of(command: &[String]) -> Option<String> {
    let name = normalized(command.first()?);
    (!name.is_empty() && !GENERIC_PROGRAMS.contains(&name.as_str())).then_some(name)
}

/// The processes of these programs, without ours and without those started by another of them.
fn find_programs(programs: &[(String, String)]) -> BTreeMap<String, Vec<Process>> {
    let mut found = BTreeMap::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return found;
    };
    // The Settings app is this program too, and asking to quit it mustn't reach the shell, whichever
    // copy of the shell that is.
    let own = fs::read_link("/proc/self/exe")
        .ok()
        .map(|exe| normalized(&exe.to_string_lossy()));
    let mut by_program: BTreeMap<String, Vec<(u32, Status)>> = BTreeMap::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        // Only readable for our own processes, which are the only ones of interest.
        let Ok(exe) = fs::read_link(Path::new("/proc").join(pid.to_string()).join("exe")) else {
            continue;
        };
        let name = normalized(&exe.to_string_lossy());
        if own.as_deref() == Some(name.as_str()) {
            continue;
        }
        if programs.iter().any(|(_, program)| *program == name)
            && let Some(status) = Status::read(pid)
        {
            by_program.entry(name).or_default().push((pid, status));
        }
    }
    for (app_id, program) in programs {
        let Some(processes) = by_program.get(program) else {
            continue;
        };
        let pids: HashSet<u32> = processes.iter().map(|(pid, _)| *pid).collect();
        let roots: Vec<Process> = processes
            .iter()
            .filter(|(_, status)| !pids.contains(&status.parent))
            .map(|(pid, status)| Process {
                pid: *pid,
                started: status.started,
            })
            .collect();
        if !roots.is_empty() {
            found.insert(app_id.clone(), roots);
        }
    }
    found
}

/// What's still running of what was, along with `programs`, by app id and program name, for apps
/// that aren't known to be running otherwise. It reads a lot of files, so it's for another thread.
pub fn check(
    mut running: BTreeMap<String, Vec<Process>>,
    programs: Vec<(String, String)>,
) -> BTreeMap<String, Vec<Process>> {
    running.retain(|_, processes| {
        processes.retain(|process| process.is_alive());
        !processes.is_empty()
    });
    let unknown: Vec<(String, String)> = programs
        .into_iter()
        .filter(|(app_id, _)| !running.contains_key(app_id))
        .collect();
    if !unknown.is_empty() {
        running.extend(find_programs(&unknown));
    }
    running
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_through_packaging() {
        assert_eq!(
            normalized("/nix/store/abc-telegram/bin/.Telegram-wrapped"),
            "telegram"
        );
        assert_eq!(normalized("/opt/vivaldi/vivaldi-bin"), "vivaldi");
        assert_eq!(normalized("/usr/bin/gone (deleted)"), "gone");
    }

    #[test]
    fn knows_which_commands_say_nothing() {
        let command = |parts: &[&str]| parts.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            program_of(&command(&["Telegram", "--"])).as_deref(),
            Some("telegram")
        );
        assert_eq!(program_of(&command(&["/usr/bin/env", "FOO=1", "x"])), None);
        assert_eq!(program_of(&command(&["flatpak", "run", "x"])), None);
        assert_eq!(program_of(&[]), None);
    }

    #[test]
    fn reads_a_stat_line_with_an_awkward_name() {
        let tail = "S 7 8 9 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 12345 0 0";
        let status = Status::parse(&format!("42 (a b) (c)) {tail}")).unwrap();
        assert_eq!(status.parent, 7);
        assert_eq!(status.started, 12345);
        assert!(Status::parse("42 (gone) Z 7 8").is_none());
    }

    #[test]
    fn this_process_is_alive_and_the_same() {
        let process = Process::of(std::process::id()).unwrap();
        assert!(process.is_alive());
        let other = Process {
            started: process.started + 1,
            ..process
        };
        assert!(!other.is_alive());
    }

    #[test]
    fn a_process_that_ended_is_dropped() {
        let dead = Process {
            pid: u32::MAX,
            started: 1,
        };
        let running = BTreeMap::from([(String::from("a"), vec![dead])]);
        assert!(check(running, Vec::new()).is_empty());
    }
}
