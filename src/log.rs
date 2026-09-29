//! Tiny leveled logger writing plain ASCII lines to stderr.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

pub fn set_level(level: Level) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Level::Error,
        1 => Level::Warn,
        2 => Level::Info,
        3 => Level::Debug,
        _ => Level::Trace,
    }
}

pub fn enabled(level: Level) -> bool {
    level <= self::level()
}

pub fn log(level: Level, args: std::fmt::Arguments<'_>) {
    if !enabled(level) {
        return;
    }
    let tag = match level {
        Level::Error => "error",
        Level::Warn => "warn",
        Level::Info => "info",
        Level::Debug => "debug",
        Level::Trace => "trace",
    };
    crate::text::err(&format!("{tag}: {args}"));
}

#[macro_export]
macro_rules! log_error { ($($t:tt)*) => { $crate::log::log($crate::log::Level::Error, format_args!($($t)*)) } }
#[macro_export]
macro_rules! log_warn { ($($t:tt)*) => { $crate::log::log($crate::log::Level::Warn, format_args!($($t)*)) } }
#[macro_export]
macro_rules! log_info { ($($t:tt)*) => { $crate::log::log($crate::log::Level::Info, format_args!($($t)*)) } }
#[macro_export]
macro_rules! log_debug { ($($t:tt)*) => { $crate::log::log($crate::log::Level::Debug, format_args!($($t)*)) } }
#[macro_export]
macro_rules! log_trace { ($($t:tt)*) => { $crate::log::log($crate::log::Level::Trace, format_args!($($t)*)) } }
