//! When the game dies, what it says goes somewhere that outlives the console.
//! Nobody had the text of Andrew's crashes: his console closes with the window,
//! and Windows records nothing for a Rust panic. Every panic, on any thread,
//! now writes `game/out/crash/panic-<UTC time>.txt` with its message, where it
//! happened, the command line, the build and a backtrace. It prints as ever too.

use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};

/// `game/out/crash/`.
pub fn dir() -> PathBuf {
    PathBuf::from(format!("{}/out/crash", env!("CARGO_MANIFEST_DIR")))
}

/// Before anything can panic.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Nothing in here may panic. A failed write loses the file, not the text.
        if let Err(e) = write(&dir(), info) {
            eprintln!("could not write the crash file in {}: {e}", dir().display());
        }
        previous(info);
    }));
}

fn write(folder: &Path, info: &PanicHookInfo<'_>) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let path = free_path(folder);
    std::fs::write(&path, report(info))?;
    eprintln!("this crash is written down in {}", path.display());
    Ok(path)
}

/// A second panic in the same second, on another thread, keeps its own file.
fn free_path(folder: &Path) -> PathBuf {
    let stem = format!("panic-{}", crate::replay::utc_stamp());
    let first = folder.join(format!("{stem}.txt"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| folder.join(format!("{stem}-{n}.txt"))).find(|p| !p.exists()).unwrap_or(first)
}

fn report(info: &PanicHookInfo<'_>) -> String {
    let thread = std::thread::current();
    let args: Vec<String> = std::env::args().collect();
    format!(
        "Triangulum panicked, {} UTC\nbuild {}\ncommand line: {}\nthread {}\nat {}\n\n{}\n\nbacktrace:\n{:#}\n",
        crate::replay::utc_stamp(),
        crate::visual::BUILD,
        args.join(" "),
        thread.name().unwrap_or("?"),
        info.location().map_or("an unknown place".to_string(), |l| l.to_string()),
        info.payload_as_str().unwrap_or("(a panic with no text)"),
        Backtrace::force_capture()
    )
}

/// Does this run open the game's window for a person? Tools, tests, the harness
/// and `--no-focus` don't: they have a console, or nobody to tell.
fn a_person_is_waiting() -> bool {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let tool = ["--play", "--bench", "--shots", "--find-places", "--probe", "--sky-reel", "--landform-shots", "--terrain-map", "--terrain-stats", "--repro", "--no-focus"];
    !tool.iter().any(|t| has(t)) && (!has("--replay") || has("--watch"))
}

/// Say why the game won't start, in a box on the screen.
pub fn tell_the_player(msg: &str) {
    #[cfg(windows)]
    if a_person_is_waiting() {
        rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("Triangulum can't start").set_description(msg).set_buttons(rfd::MessageButtons::Ok).show();
    }
    #[cfg(not(windows))]
    let _ = msg;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The hook is the process's; tests that panic on purpose share it.
    static HOOK: Mutex<()> = Mutex::new(());

    #[test]
    fn a_panic_leaves_its_message_where_and_why_in_a_file() {
        let _one_at_a_time = HOOK.lock().unwrap();
        let folder = std::env::temp_dir().join(format!("triangulum-crash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let f2 = folder.clone();
        let this = std::thread::current().id();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Other tests are running: only this thread's panic is this test's.
            if std::thread::current().id() == this {
                write(&f2, info).unwrap();
            }
        }));
        let r = std::panic::catch_unwind(|| panic!("the depth attachment has another size"));
        std::panic::set_hook(previous);
        assert!(r.is_err());
        let files: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(files.len(), 1, "{files:?}");
        let text = std::fs::read_to_string(&files[0]).unwrap();
        assert!(text.contains("the depth attachment has another size") && text.contains("crash.rs") && text.contains("backtrace:") && text.contains("build "), "{text}");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_second_panic_keeps_its_own_file() {
        let folder = std::env::temp_dir().join(format!("triangulum-crash-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let a = free_path(&folder);
        std::fs::write(&a, "x").unwrap();
        let b = free_path(&folder);
        assert_ne!(a, b);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
