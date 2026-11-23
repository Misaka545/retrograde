//! glibc allocator tuning applied once at startup.
//!
//! glibc gives each thread that allocates its own arena (up to 8 per core), and freed memory
//! in those arenas is rarely returned to the OS. Retrograde runs ~20 threads (tokio, mpv,
//! PipeWire, notify, ...) that mostly allocate small, short-lived buffers, so a couple of
//! shared arenas cost nothing measurable in speed and keep resident memory lower.

/// Number of malloc arenas Retrograde asks glibc for.
pub const MALLOC_ARENAS: i32 = 2;

/// Caps glibc's malloc arenas at [`MALLOC_ARENAS`], unless the user already set
/// `MALLOC_ARENA_MAX` or `GLIBC_TUNABLES=glibc.malloc.arena_max=…` themselves. Returns `true`
/// when the cap was applied.
///
/// Call it at the very start of `main`, before any other thread exists. It does nothing on
/// non-glibc targets (musl has a single allocator lock and no `M_ARENA_MAX`).
pub fn limit_malloc_arenas() -> bool {
    let user_tunable = std::env::var("GLIBC_TUNABLES").is_ok_and(|t| t.contains("malloc.arena_max"));
    if user_tunable || std::env::var_os("MALLOC_ARENA_MAX").is_some() {
        return false;
    }
    apply_arena_max(MALLOC_ARENAS)
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn apply_arena_max(arenas: i32) -> bool {
    use std::ffi::c_int;

    // From <malloc.h>.
    const M_ARENA_MAX: c_int = -8;

    unsafe extern "C" {
        fn mallopt(param: c_int, value: c_int) -> c_int;
    }

    // SAFETY: `mallopt` only adjusts allocator parameters; it is safe to call at any time,
    // and glibc returns 1 on success, 0 on error.
    unsafe { mallopt(M_ARENA_MAX, arenas) == 1 }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn apply_arena_max(_arenas: i32) -> bool {
    false
}
