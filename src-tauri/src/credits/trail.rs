//! Which stage of work was in flight when the process died.
//!
//! The app has crashed five times on 2026-11-08 with 0xc0000409 (a native fault,
//! not a Rust panic — `panic.log` never gets written), each time at a different
//! fault offset. A crash dump needs admin rights to enable, so instead we leave a
//! breadcrumb: every long-running stage stamps what it is doing into a small file
//! that is fsynced on write. After a crash the file still holds the last stage,
//! which narrows the fault to one region of code.
//!
//! Cost is one tiny append per stage transition, not per request, so it stays off
//! the hot path.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn trail_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ai-toolkit").join("crash-trail.log"))
}

/// Record that `stage` is starting. Returns a guard whose drop records completion,
/// so a crash between the two leaves the stage open in the file.
#[allow(dead_code)]
pub(crate) fn enter(stage: &'static str) -> StageGuard {
    write_line(&format!("BEGIN {stage}"));
    StageGuard { stage }
}

#[allow(dead_code)]
pub(crate) struct StageGuard {
    stage: &'static str,
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        write_line(&format!("END   {}", self.stage));
    }
}

/// One-shot note with no matching END line, rate-limited to once every 2s.
///
/// Use for "we are about to do X" inside a hot loop. Called per streamed chunk it
/// would otherwise fsync thousands of times per response; the cooldown keeps the
/// trail readable and the IO negligible while still recording the last activity
/// before a fault.
pub(crate) fn mark(stage: &str) {
    use std::sync::atomic::AtomicI64;
    static LAST_MS: AtomicI64 = AtomicI64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let prev = LAST_MS.load(Ordering::Relaxed);
    if now - prev < 2_000 {
        return;
    }
    if LAST_MS
        .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        write_line(&format!("MARK  {stage}"));
    }
}

fn write_line(line: &str) {
    let Some(path) = trail_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Keep the file small: it only needs the recent past, and unbounded growth
    // would itself become a problem on a process meant to run for days.
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 512 * 1024 {
            let _ = std::fs::remove_file(&path);
        }
    }
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(fh, "{stamp} #{seq:<6} {line}");
        // Flush explicitly: a crash kills the process without running destructors,
        // so anything still sitting in the buffer is lost — which is exactly the
        // line we need.
        let _ = fh.flush();
    }
}

/// Read the last few trail entries, newest last. Used by the UI's diagnostics.
#[allow(dead_code)]
pub(crate) fn tail(lines: usize) -> Vec<String> {
    let Some(path) = trail_path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|s| s.to_string())
        .collect()
}
