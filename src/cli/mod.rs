//! Command dispatch. Every subcommand returns `Result<()>`; `run` maps
//! errors to exit codes and prints them through the ASCII sanitizer.

pub mod args;
pub mod commands;
pub mod manage;
pub mod session;

use crate::error::{Error, Result};
use crate::log::Level;
use crate::text::out;
use args::{OptSpec, Parsed};
use session::Session;

pub const GLOBAL_OPTS: &[OptSpec] = &[
    OptSpec::value(Some('c'), "config", "PATH", "config file path"),
    OptSpec::value(
        Some('s'),
        "server",
        "URL",
        "auth server, e.g. http://10.0.0.55",
    ),
    OptSpec::flag(Some('v'), "verbose", "more output (repeat for more)"),
    OptSpec::flag(Some('q'), "quiet", "errors only"),
    OptSpec::value(None, "acid", "N", "override ac_id (default: auto)"),
    OptSpec::flag(None, "tls-insecure", "skip certificate verification"),
    OptSpec::flag(
        None,
        "ascii",
        "escape non-ascii server text as \\uXXXX (also SRUN_ASCII=1)",
    ),
    OptSpec::flag(Some('h'), "help", "print help"),
];

pub const USER_OPTS: &[OptSpec] = &[
    OptSpec::value(
        Some('u'),
        "username",
        "NAME",
        "ad-hoc username (with -p or SRUN_PASSWORD)",
    ),
    OptSpec::value(Some('p'), "password", "PASS", "ad-hoc password"),
    OptSpec::value(None, "user", "NAME", "named user from config"),
    OptSpec::flag(None, "all", "every user in config"),
];

pub const IP_OPTS: &[OptSpec] = &[
    OptSpec::value(Some('i'), "ip", "IP", "authorize this ip"),
    OptSpec::flag(
        Some('d'),
        "detect-ip",
        "use the ip the server sees (default)",
    ),
    OptSpec::flag(None, "select-ip", "choose an ip from local interfaces"),
    OptSpec::value(None, "ifname", "NAME", "use the ipv4 of this interface"),
    OptSpec::flag(None, "strict-bind", "bind the connection to the chosen ip"),
];

pub const LOGIN_OPTS: &[OptSpec] = &[
    OptSpec::value(None, "retry", "N", "attempts on network errors (default 3)"),
    OptSpec::value(
        None,
        "retry-delay",
        "MS",
        "delay between attempts (default 1000)",
    ),
    OptSpec::flag(None, "test", "skip login when already online"),
    OptSpec::value(
        None,
        "probe",
        "MODE",
        "online check: server (default), none, or HOST:PORT",
    ),
    OptSpec::flag(None, "double-stack", "send double_stack=1"),
    OptSpec::value(None, "password-mode", "MODE", "real (default) or empty"),
    OptSpec::value(None, "os", "STR", "os field"),
    OptSpec::value(None, "name", "STR", "name field"),
    OptSpec::value(None, "n", "N", "n field (default 200)"),
    OptSpec::value(None, "type", "N", "type field (default 1)"),
];

pub const MANAGE_USER_OPTS: &[OptSpec] = &[
    OptSpec::value(
        None,
        "name",
        "ALIAS",
        "alias for this user (default: the username)",
    ),
    OptSpec::value(
        Some('p'),
        "password",
        "PASS",
        "password (else SRUN_PASSWORD, else prompt)",
    ),
    OptSpec::flag(None, "password-stdin", "read the password from stdin"),
    OptSpec::flag(None, "no-password", "store no password; ask at login time"),
    OptSpec::flag(None, "plain", "store the password without obfuscation"),
    OptSpec::value(None, "ip", "IP", "fixed ip for this user"),
    OptSpec::value(None, "ifname", "NAME", "interface whose ipv4 to use"),
    OptSpec::flag(None, "default", "make this the default user"),
    OptSpec::flag(
        None,
        "force",
        "update an existing user / ignore a missing one",
    ),
];

pub const DAEMON_OPTS: &[OptSpec] = &[
    OptSpec::value(
        None,
        "interval",
        "SECS",
        "seconds between checks (default 60)",
    ),
    OptSpec::flag(None, "logout-on-exit", "log out when stopped"),
];

pub const MANAGE_CONFIG_OPTS: &[OptSpec] =
    &[OptSpec::flag(None, "force", "overwrite an existing file")];

const COMMANDS: &[(&str, &str)] = &[
    ("login", "authenticate (default when no command given)"),
    ("logout", "de-authenticate"),
    ("status", "show online info (alias: info)"),
    ("switch", "log out whoever is online and log in as NAME"),
    ("daemon", "stay online: check and re-login in a loop"),
    (
        "user",
        "manage users in config: add | remove | list | default",
    ),
    ("config", "show | path | init | obfuscate"),
    ("version", "print version and target"),
];

/// Index of the command token: the first token that is neither an option
/// nor the value of a value-taking global option.
fn find_command_index(args: &[String]) -> Option<usize> {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            return None;
        }
        if let Some(long) = a.strip_prefix("--") {
            let name = long.split('=').next().unwrap_or("");
            let takes = GLOBAL_OPTS.iter().any(|o| o.long == name && o.takes_value);
            if takes && !long.contains('=') {
                i += 1;
            }
        } else if a.starts_with('-') && a.len() > 1 {
            let last = a.chars().last().unwrap_or(' ');
            let takes = GLOBAL_OPTS
                .iter()
                .any(|o| o.short == Some(last) && o.takes_value);
            if takes {
                i += 1;
            }
        } else {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_command(args: &[String]) -> Option<&str> {
    find_command_index(args).map(|i| args[i].as_str())
}

fn specs(parts: &[&[OptSpec]]) -> Vec<OptSpec> {
    parts.iter().flat_map(|p| p.iter().copied()).collect()
}

pub fn run(args: Vec<String>) -> i32 {
    match run_inner(&args) {
        Ok(()) => 0,
        Err(e) => {
            crate::log_error!("{e}");
            e.exit_code()
        }
    }
}

fn run_inner(args: &[String]) -> Result<()> {
    let explicit = find_command(args);
    if explicit.is_none() && args.iter().any(|a| a == "-h" || a == "--help") {
        print_root_help();
        return Ok(());
    }
    let cmd = explicit.unwrap_or("login");
    let cmd = match cmd {
        "info" => "status",
        other => other,
    };
    if !COMMANDS.iter().any(|(c, _)| *c == cmd) {
        return Err(Error::usage(format!(
            "unknown command '{cmd}' (try: srun --help)"
        )));
    }
    let mut rest: Vec<String> = args.to_vec();
    if let Some(pos) = find_command_index(args) {
        rest.remove(pos);
    }
    let (usage, opts): (&str, Vec<OptSpec>) = match cmd {
        "login" => (
            "srun login [OPTIONS]",
            specs(&[GLOBAL_OPTS, USER_OPTS, IP_OPTS, LOGIN_OPTS]),
        ),
        "logout" => ("srun logout [OPTIONS]", specs(&[GLOBAL_OPTS, USER_OPTS, IP_OPTS])),
        "daemon" => (
            "srun daemon [OPTIONS]",
            specs(&[GLOBAL_OPTS, USER_OPTS, IP_OPTS, LOGIN_OPTS, DAEMON_OPTS]),
        ),
        "status" => ("srun status [OPTIONS]", specs(&[GLOBAL_OPTS, USER_OPTS, IP_OPTS])),
        "switch" => ("srun switch NAME [OPTIONS]", specs(&[GLOBAL_OPTS, IP_OPTS, LOGIN_OPTS])),
        "user" => (
            "srun user add USERNAME [--name ALIAS] [-p PASSWORD] [--ip IP | --ifname NAME] [--default] [--force]\n       srun user remove NAME [--force] | srun user list | srun user default NAME",
            specs(&[GLOBAL_OPTS, MANAGE_USER_OPTS]),
        ),
        "config" => ("srun config show | path | init [--force] | obfuscate", specs(&[GLOBAL_OPTS, MANAGE_CONFIG_OPTS])),
        _ => (cmd, specs(&[GLOBAL_OPTS])),
    };
    let p = args::parse(&rest, &opts)?;
    apply_global(&p);
    if p.flag("help") {
        out(&format!("Usage: {usage}"));
        out("");
        out("Options:");
        out(&args::usage_block(&opts));
        return Ok(());
    }
    match cmd {
        "version" => {
            out(&format!("srun {} ({})", crate::VERSION, crate::TARGET));
            Ok(())
        }
        "login" => commands::login(&Session::build(p)?),
        "logout" => commands::logout(&Session::build(p)?),
        "status" => commands::status(&Session::build(p)?),
        "switch" => commands::switch(&Session::build(p)?),
        "daemon" => crate::daemon::run(&Session::build(p)?),
        "user" => manage::user(&p),
        "config" => manage::config_cmd(&p),
        _ => Err(Error::internal(format!(
            "command '{cmd}' not implemented yet"
        ))),
    }
}

pub fn apply_global(p: &Parsed) {
    let env_ascii = std::env::var("SRUN_ASCII")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    crate::text::set_utf8(!(p.flag("ascii") || env_ascii));
    if p.flag("quiet") {
        crate::log::set_level(Level::Error);
    } else {
        match p.count("verbose") {
            0 => crate::log::set_level(Level::Info),
            1 => crate::log::set_level(Level::Debug),
            _ => crate::log::set_level(Level::Trace),
        }
    }
}

pub fn print_root_help() {
    out("Usage: srun [OPTIONS] [COMMAND] [COMMAND OPTIONS]");
    out("");
    out("Commands:");
    let width = COMMANDS.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
    for (c, h) in COMMANDS {
        out(&format!("  {c:<width$}  {h}"));
    }
    out("");
    out("Global options:");
    out(&args::usage_block(GLOBAL_OPTS));
    out("");
    out("Run 'srun COMMAND --help' for command options.");
}
