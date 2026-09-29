//! Terminal helpers: no-echo password prompt and stop signals.

use crate::error::{Error, Result};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};

pub static STOP: AtomicBool = AtomicBool::new(false);

pub fn stop_requested() -> bool {
    STOP.load(Ordering::SeqCst)
}

pub fn stdin_is_tty() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: isatty on fd 0 has no preconditions.
        unsafe { libc::isatty(0) == 1 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE};
        // SAFETY: console query on the process stdin handle.
        unsafe {
            let mut mode = 0u32;
            GetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), &mut mode) != 0
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Read one line from stdin, hiding the echo when stdin is a terminal.
pub fn read_password(prompt: &str) -> Result<String> {
    let stderr = std::io::stderr();
    let _ = stderr
        .lock()
        .write_all(crate::text::ascii(prompt).as_bytes());
    let _ = stderr.lock().flush();
    let _guard = NoEcho::enable();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| Error::internal(format!("read stdin: {e}")))?;
    if _guard.is_some() {
        let _ = stderr.lock().write_all(b"\n");
    }
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}

/// Read one line from stdin as-is (for `--password-stdin`).
pub fn read_line_stdin() -> Result<String> {
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| Error::internal(format!("read stdin: {e}")))?;
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}

struct NoEcho {
    #[cfg(unix)]
    saved: libc::termios,
    #[cfg(windows)]
    saved: u32,
}

#[cfg(unix)]
impl NoEcho {
    fn enable() -> Option<NoEcho> {
        // SAFETY: plain libc calls on fd 0 with a zeroed termios out-param.
        unsafe {
            if libc::isatty(0) == 0 {
                return None;
            }
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) != 0 {
                return None;
            }
            let saved = t;
            t.c_lflag &= !libc::ECHO;
            if libc::tcsetattr(0, libc::TCSANOW, &t) != 0 {
                return None;
            }
            Some(NoEcho { saved })
        }
    }
}

#[cfg(unix)]
impl Drop for NoEcho {
    fn drop(&mut self) {
        // SAFETY: restoring the attributes we read earlier.
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.saved);
        }
    }
}

#[cfg(windows)]
impl NoEcho {
    fn enable() -> Option<NoEcho> {
        use windows_sys::Win32::System::Console::{
            GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_ECHO_INPUT, STD_INPUT_HANDLE,
        };
        // SAFETY: console API calls on the process stdin handle.
        unsafe {
            let h = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode: u32 = 0;
            if GetConsoleMode(h, &mut mode) == 0 {
                return None;
            }
            if SetConsoleMode(h, mode & !ENABLE_ECHO_INPUT) == 0 {
                return None;
            }
            Some(NoEcho { saved: mode })
        }
    }
}

#[cfg(windows)]
impl Drop for NoEcho {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Console::{GetStdHandle, SetConsoleMode, STD_INPUT_HANDLE};
        // SAFETY: restoring the mode we read earlier.
        unsafe {
            SetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), self.saved);
        }
    }
}

#[cfg(not(any(unix, windows)))]
impl NoEcho {
    fn enable() -> Option<NoEcho> {
        None
    }
}

/// Install SIGTERM/SIGINT (unix) or console control (windows) handlers that
/// only set `STOP`. std retries reads interrupted by a signal, so a stop
/// during an HTTP call is noticed when that call finishes or times out
/// (io_timeout, 10 s); the sleep between checks is sliced and stops within
/// half a second.
pub fn install_stop_handlers() {
    #[cfg(unix)]
    {
        extern "C" fn on_signal(_sig: libc::c_int) {
            STOP.store(true, Ordering::SeqCst);
        }
        // SAFETY: sigaction with a handler that only touches an atomic.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as *const () as usize;
            sa.sa_flags = 0;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
            libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
        unsafe extern "system" fn on_ctrl(_kind: u32) -> i32 {
            STOP.store(true, Ordering::SeqCst);
            1
        }
        // SAFETY: registering a handler that only touches an atomic.
        unsafe {
            SetConsoleCtrlHandler(Some(on_ctrl), 1);
        }
    }
}

/// Sleep in small slices so a stop request is noticed within ~500 ms.
pub fn sleep_interruptible(total: std::time::Duration) -> bool {
    let slice = std::time::Duration::from_millis(500);
    let start = std::time::Instant::now();
    while start.elapsed() < total {
        if stop_requested() {
            return false;
        }
        let left = total - start.elapsed();
        std::thread::sleep(left.min(slice));
    }
    !stop_requested()
}
