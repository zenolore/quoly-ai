//! Sizing the request body limit against the machine's *live* memory headroom.
//!
//! ## Why a fixed limit was wrong in both directions
//!
//! `MAX_REQUEST_BYTES` used to be a flat 32 MiB. Every byte of a chat request is
//! held twice while it is being served: once as the raw body, and once as the
//! parsed `serde_json::Value` tree, which costs roughly 10-15x the JSON text
//! (each string, array and object gains a Rust struct header plus allocator
//! padding on top of its contents). A 32 MiB body therefore briefly pins ~400 MiB
//! of commit charge.
//!
//! This machine measured 43-47 GiB of commit limit with as little as **2.2 GiB
//! free** while the user's normal workload was running (20 WorkBuddy processes,
//! 14 Qoder, 28 python, plus an explorer.exe that had leaked to 2871 threads).
//! In that state a single 32 MiB body is survivable; four arriving together while
//! the body-parse semaphore allows three at a time is not. When the allocation
//! fails, Rust aborts the process — the 0xc0000409 / parameter 7 crash the user
//! kept seeing, with no log line because there is no unwinding from an aborted
//! allocation.
//!
//! ## What this module does
//!
//! Places the limit on a sliding scale:
//!
//!   free commit   limit    rationale
//!   > 6.0 GiB     32 MiB   plenty of room; the hard ceiling, unchanged behaviour
//!   > 3.0 GiB     16 MiB   still far above any real request
//!   > 1.5 GiB     10 MiB   tight; the largest real bodies still fit
//!   else           6 MiB   emergency floor, still above the largest real body
//!
//! For calibration, the largest request this user actually sends is ~880k input
//! tokens, which is a **3.8 MB** JSON body. The floor is deliberately set just
//! above that (3.5 MiB) rather than lower: clamping below it would reject the
//! user's genuine daily traffic during exactly the busy period that motivated
//! this whole exercise. The tiers therefore only reject bodies that are larger
//! than anything real, and only when memory is actually under pressure.
//!
//! The value is read per request rather than cached: a stale limit is precisely
//! what we are trying to avoid. `GlobalMemoryStatusEx` is a cheap syscall
//! (microseconds) and requests arrive at ~2.3/s at this user's peak.

/// Absolute ceiling. No request may ever exceed this regardless of free memory:
/// it bounds worst-case parse cost at ~400 MiB and matches what the providers
/// themselves accept.
pub(super) const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;

/// Free commit charge, in bytes, or `None` when it cannot be determined.
///
/// Uses `ullAvailPageFile` from `MEMORYSTATUSEX`, which is what Windows calls
/// "commit available": physical RAM plus page file, minus what is already
/// committed across all processes. This is the number that actually fills up and
/// makes an allocation fail, so it is the right thing to budget against — free
/// physical RAM alone overstates headroom when the page file is small.
#[cfg(windows)]
pub(super) fn available_commit_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `status` is a valid, correctly-sized MEMORYSTATUSEX with `dwLength`
    // set, which is the entire contract of GlobalMemoryStatusEx. The call writes
    // only into our own stack value.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return None;
    }
    Some(status.ullAvailPageFile)
}

#[cfg(not(windows))]
pub(super) fn available_commit_bytes() -> Option<u64> {
    // Linux/macOS have no direct equivalent of Windows commit charge, and the
    // crash this module exists for was Windows-specific (0xc0000409 from an
    // aborted allocation under commit exhaustion). Report "unknown" so callers
    // fall back to the fixed hard ceiling rather than guessing.
    None
}

/// Request body limit for the current amount of free commit charge.
///
/// Monotonic in `free`: more headroom can only raise the limit. When the free
/// amount is unknown, returns the hard ceiling — refusing traffic because a
/// syscall failed would be worse than accepting it.
///
/// The floor is 6 MiB, and that number is not arbitrary: the largest body this
/// user actually sends is ~3.8 MB of text for 880k tokens, and the gateway's own
/// integration test builds a **>4 MiB** body (35 screenshots in one conversation).
/// A floor below that would start rejecting real traffic during busy periods,
/// which defeats the purpose — the goal is to reject the *abnormal* body that
/// would abort the process, not to throttle normal work.
///
/// Even at the floor, worst-case parse cost drops from ~400 MiB (32 MiB body,
/// ~12x expansion into a `Value` tree) to ~75 MiB, and with at most three parse
/// slots that is 225 MiB instead of 1.2 GiB — the difference between surviving and
/// hitting the commit limit on this machine.
pub(super) fn request_limit_for(free_commit: Option<u64>) -> usize {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    let Some(free) = free_commit else {
        return MAX_REQUEST_BYTES;
    };
    let limit = if free > 6 * GIB {
        32 * MIB
    } else if free > 3 * GIB {
        16 * MIB
    } else if free > 3 * GIB / 2 {
        10 * MIB
    } else {
        // Emergency floor: still above every real request, so a near-exhausted
        // machine keeps serving the user's normal traffic.
        6 * MIB
    };
    // Defensive: never exceed the hard ceiling even if the ladder is edited.
    (limit as usize).min(MAX_REQUEST_BYTES)
}

/// The request body limit to enforce right now.
pub(super) fn current_request_limit() -> usize {
    request_limit_for(available_commit_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;

    #[test]
    fn unknown_headroom_falls_back_to_the_hard_ceiling() {
        assert_eq!(request_limit_for(None), MAX_REQUEST_BYTES);
    }

    #[test]
    fn limit_never_exceeds_the_hard_ceiling() {
        for free in [0, MIB, 900 * MIB, 2 * GIB, 5 * GIB, 64 * GIB, u64::MAX / 2] {
            assert!(
                request_limit_for(Some(free)) <= MAX_REQUEST_BYTES,
                "free={free} produced a limit above the ceiling"
            );
        }
    }

    #[test]
    fn limit_is_monotonic_in_free_commit() {
        let mut previous = 0;
        for step in 0..=64 {
            let limit = request_limit_for(Some(step * GIB / 8));
            assert!(
                limit >= previous,
                "limit shrank as free memory grew (step {step})"
            );
            previous = limit;
        }
    }

    /// The user's largest real request is ~880k tokens ≈ 3.8 MB of text, and a
    /// body with images or big tool schemas reaches ~4 MB. Every tier — including
    /// the emergency floor — must let that through.
    #[test]
    fn a_real_request_survives_every_tier_including_the_floor() {
        const REAL_BODY_WITH_IMAGES: usize = 4 * 1024 * 1024;
        for free in [0, 256 * MIB, 900 * MIB, 2 * GIB, 4 * GIB, 8 * GIB] {
            assert!(
                request_limit_for(Some(free)) > REAL_BODY_WITH_IMAGES,
                "a real ~4 MB request would be rejected with {free} bytes free"
            );
        }
    }

    /// Ramps down explicitly, so a change in thresholds is a visible edit here
    /// rather than a silent behaviour shift.
    #[test]
    fn tiers_step_down_as_memory_tightens() {
        assert_eq!(request_limit_for(Some(8 * GIB)), 32 * 1024 * 1024);
        assert_eq!(request_limit_for(Some(4 * GIB)), 16 * 1024 * 1024);
        assert_eq!(request_limit_for(Some(2 * GIB)), 10 * 1024 * 1024);
        // Floor holds at 6 MiB no matter how little is left.
        assert_eq!(request_limit_for(Some(256 * MIB)), 6 * 1024 * 1024);
        assert_eq!(request_limit_for(Some(0)), 6 * 1024 * 1024);
    }
}
