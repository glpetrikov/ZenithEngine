/// Desktop launchers (double-clicking in a file manager, systemd `app.slice`
/// scopes, etc.) commonly start processes with the OS-default 8 MiB
/// `RLIMIT_STACK`, while an interactive terminal session frequently has a
/// larger limit configured via shell/login setup. The .NET runtime's
/// exception unwinding through generic-method dispatch and native/managed
/// interop can need more stack than that, and a stack overflow there
/// corrupts CoreCLR's own SIGSEGV-based fault handling instead of being
/// caught cleanly, crashing the whole process. Raising `RLIMIT_STACK` and
/// re-executing ourselves (`execve` keeps the same PID, so process
/// supervisors/systemd scopes are unaffected) makes the main thread's stack
/// size consistent regardless of how the process was launched.
#[cfg(target_os = "linux")]
pub fn ensure_large_main_thread_stack() {
	const DESIRED_STACK_BYTES: libc::rlim_t = 64 * 1024 * 1024;
	const REEXEC_GUARD_VAR: &str = "__ZE_STACK_REEXEC_DONE";

	if std::env::var_os(REEXEC_GUARD_VAR).is_some() {
		return;
	}

	let mut rlim = libc::rlimit {
		rlim_cur: 0,
		rlim_max: 0,
	};
	// SAFETY: `rlim` is a valid, appropriately-sized out-parameter for getrlimit.
	if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut rlim) } != 0 {
		return;
	}

	if rlim.rlim_cur == libc::RLIM_INFINITY || rlim.rlim_cur >= DESIRED_STACK_BYTES {
		return;
	}

	rlim.rlim_cur = if rlim.rlim_max == libc::RLIM_INFINITY {
		DESIRED_STACK_BYTES
	} else {
		DESIRED_STACK_BYTES.min(rlim.rlim_max)
	};
	// SAFETY: `rlim` holds a valid rlimit struct with rlim_cur <= rlim_max.
	if unsafe { libc::setrlimit(libc::RLIMIT_STACK, &rlim) } != 0 {
		return;
	}

	let Ok(exe) = std::env::current_exe() else {
		return;
	};

	// Re-exec so the kernel maps a fresh main-thread stack under the new
	// limit; a raised rlimit alone does not reliably grow a stack region
	// that the kernel already mapped at process start.
	use std::os::unix::process::CommandExt;
	let error = std::process::Command::new(exe)
		.args(std::env::args_os().skip(1))
		.env(REEXEC_GUARD_VAR, "1")
		.exec();
	eprintln!("failed to re-exec for a larger main-thread stack: {error}");
}

#[cfg(not(target_os = "linux"))]
pub fn ensure_large_main_thread_stack() {}
