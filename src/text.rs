//! Output helpers. Everything the program prints goes through `ascii()`.
//! Our own strings are plain ASCII by construction; text that comes from
//! the portal (product names) is shown as is, best effort, unless
//! `--ascii` / `SRUN_ASCII=1` asks for `\uXXXX` escapes (busybox, serial
//! consoles). Control characters and ANSI escapes are always stripped.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static ALLOW_UTF8: AtomicBool = AtomicBool::new(true);

/// `false` escapes non-ASCII characters as `\uXXXX` (`--ascii` / `SRUN_ASCII=1`).
pub fn set_utf8(allow: bool) {
    ALLOW_UTF8.store(allow, Ordering::Relaxed);
}

pub fn utf8_allowed() -> bool {
    ALLOW_UTF8.load(Ordering::Relaxed)
}

/// Sanitize one string for the terminal: tabs become two spaces, `\r`, ESC
/// and other control characters are dropped or escaped, `\n` is kept, and
/// non-ASCII text is kept (default) or escaped as `\uXXXX` (ascii mode).
pub fn ascii(s: &str) -> String {
    let utf8 = utf8_allowed();
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\n' => out.push('\n'),
            '\t' => out.push_str("  "),
            '\r' | '\u{1b}' => {}
            ' '..='~' => out.push(ch),
            c if utf8 && !c.is_control() => out.push(c),
            _ => {
                let cp = ch as u32;
                if cp > 0xFFFF {
                    let v = cp - 0x10000;
                    let hi = 0xD800 + (v >> 10);
                    let lo = 0xDC00 + (v & 0x3FF);
                    out.push_str(&format!("\\u{hi:04X}\\u{lo:04X}"));
                } else {
                    out.push_str(&format!("\\u{cp:04X}"));
                }
            }
        }
    }
    out
}

/// Print one sanitized line to stdout. Broken pipes are ignored so that
/// `srun status | head -1` does not turn into an error.
pub fn out(s: &str) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(ascii(s).as_bytes());
    let _ = lock.write_all(b"\n");
    let _ = lock.flush();
}

/// Print one sanitized line to stderr.
pub fn err(s: &str) {
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    let _ = lock.write_all(ascii(s).as_bytes());
    let _ = lock.write_all(b"\n");
    let _ = lock.flush();
}

/// Human readable byte count, e.g. `1.23 GB`.
pub fn fmt_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let f = n as f64;
    if f >= TB {
        format!("{:.2} TB", f / TB)
    } else if f >= GB {
        format!("{:.2} GB", f / GB)
    } else if f >= MB {
        format!("{:.1} MB", f / MB)
    } else if f >= KB {
        format!("{:.0} KB", f / KB)
    } else {
        format!("{n} B")
    }
}

/// Seconds as `HH:MM:SS` (hours may exceed two digits).
pub fn fmt_secs(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_passthrough() {
        assert_eq!(ascii("hello, world 123 ~"), "hello, world 123 ~");
    }

    #[test]
    fn ascii_mode_escapes_non_ascii() {
        set_utf8(false);
        assert_eq!(ascii("a\u{4e2d}b"), "a\\u4E2Db");
        assert_eq!(ascii("\u{1F600}"), "\\uD83D\\uDE00");
        set_utf8(true);
    }

    #[test]
    fn ascii_controls() {
        assert_eq!(ascii("a\tb\r\n\u{1b}[31m"), "a  b\n[31m");
    }

    #[test]
    fn ascii_mode_property_all_outputs_printable() {
        set_utf8(false);
        let sample = "\u{5b66}\u{751f}-10\u{5143}\u{542b}300GB \u{0}\u{7f}\u{80}";
        for b in ascii(sample).bytes() {
            assert!(b == b'\n' || (0x20..=0x7E).contains(&b), "byte {b:#x}");
        }
        set_utf8(true);
    }

    #[test]
    fn default_keeps_text_but_strips_controls() {
        set_utf8(true);
        assert_eq!(ascii("a\u{4e2d}b\u{1b}[31m\u{7f}"), "a\u{4e2d}b[31m\\u007F");
        assert_eq!(ascii("\u{1F600} ok"), "\u{1F600} ok");
    }

    #[test]
    fn bytes_and_secs() {
        assert_eq!(fmt_bytes(0), "0 B");
        assert_eq!(fmt_bytes(2048), "2 KB");
        assert_eq!(fmt_bytes(225_715_260_616), "210.21 GB");
        assert_eq!(fmt_secs(6_824_184), "1895:36:24");
        assert_eq!(fmt_secs(3723), "01:02:03");
    }
}
