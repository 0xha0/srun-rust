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

struct Cmd {
    name: &'static str,
    help: &'static str,
    usage: &'static str,
    opts: &'static [&'static [OptSpec]],
}

const COMMANDS: &[Cmd] = &[
    Cmd {
        name: "login",
        help: "authenticate (default when no command given)",
        usage: "srun login [OPTIONS]",
        opts: &[GLOBAL_OPTS, USER_OPTS, IP_OPTS, LOGIN_OPTS],
    },
    Cmd {
        name: "logout",
        help: "de-authenticate",
        usage: "srun logout [OPTIONS]",
        opts: &[GLOBAL_OPTS, USER_OPTS, IP_OPTS],
    },
    Cmd {
        name: "status",
        help: "show online info (alias: info)",
        usage: "srun status [OPTIONS]",
        opts: &[GLOBAL_OPTS, USER_OPTS, IP_OPTS],
    },
    Cmd {
        name: "switch",
        help: "log out whoever is online and log in as NAME",
        usage: "srun switch NAME [OPTIONS]",
        opts: &[GLOBAL_OPTS, IP_OPTS, LOGIN_OPTS],
    },
    Cmd {
        name: "daemon",
        help: "stay online: check and re-login in a loop",
        usage: "srun daemon [OPTIONS]",
        opts: &[GLOBAL_OPTS, USER_OPTS, IP_OPTS, LOGIN_OPTS, DAEMON_OPTS],
    },
    Cmd {
        name: "user",
        help: "manage users in config: add | remove | list | default",
        usage: "srun user add USERNAME [--name ALIAS] [-p PASSWORD] [--ip IP | --ifname NAME] [--default] [--force]\n       srun user remove NAME [--force] | srun user list | srun user default NAME",
        opts: &[GLOBAL_OPTS, MANAGE_USER_OPTS],
    },
    Cmd {
        name: "config",
        help: "show | path | init | obfuscate",
        usage: "srun config show | path | init [--force] | obfuscate",
        opts: &[GLOBAL_OPTS, MANAGE_CONFIG_OPTS],
    },
    Cmd {
        name: "version",
        help: "print version and target",
        usage: "srun version",
        opts: &[GLOBAL_OPTS],
    },
];

/// Index of the command token: the first token that is neither an option
/// nor the value of a value-taking global option. Only global options may
/// precede the command.
fn find_command_index(args: &[String]) -> Option<usize> {
    let takes_value = |long: &str, short: Option<char>| {
        GLOBAL_OPTS
            .iter()
            .any(|o| o.takes_value && (o.long == long || (short.is_some() && o.short == short)))
    };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            return None;
        }
        if let Some(long) = a.strip_prefix("--") {
            let name = long.split('=').next().unwrap_or("");
            if takes_value(name, None) && !long.contains('=') {
                i += 1;
            }
        } else if a.starts_with('-') && a.len() > 1 {
            if takes_value("", a.chars().last()) {
                i += 1;
            }
        } else {
            return Some(i);
        }
        i += 1;
    }
    None
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
    let idx = find_command_index(args);
    if idx.is_none() && args.iter().any(|a| a == "-h" || a == "--help") {
        print_root_help();
        return Ok(());
    }
    let name = match idx.map(|i| args[i].as_str()).unwrap_or("login") {
        "info" => "status",
        other => other,
    };
    let cmd = COMMANDS
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| Error::usage(format!("unknown command '{name}' (try: srun --help)")))?;
    let mut rest: Vec<String> = args.to_vec();
    if let Some(pos) = idx {
        rest.remove(pos);
    }
    let opts: Vec<OptSpec> = cmd.opts.iter().flat_map(|p| p.iter().copied()).collect();
    let p = args::parse(&rest, &opts)?;
    apply_global(&p);
    if p.flag("help") {
        out(&format!("Usage: {}", cmd.usage));
        out("");
        out("Options:");
        out(&args::usage_block(&opts));
        return Ok(());
    }
    match cmd.name {
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
        other => unreachable!("command table lists {other}"),
    }
}

pub fn apply_global(p: &Parsed) {
    let env_ascii = std::env::var("SRUN_ASCII")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    crate::text::set_utf8(!(p.flag("ascii") || env_ascii));
    let level = if p.flag("quiet") {
        Level::Error
    } else {
        match p.count("verbose") {
            0 => Level::Info,
            1 => Level::Debug,
            _ => Level::Trace,
        }
    };
    crate::log::set_level(level);
}

pub fn print_root_help() {
    out("Usage: srun [OPTIONS] [COMMAND] [COMMAND OPTIONS]");
    out("");
    out("Commands:");
    let width = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0);
    for c in COMMANDS {
        out(&format!("  {:<width$}  {}", c.name, c.help));
    }
    out("");
    out("Global options:");
    out(&args::usage_block(GLOBAL_OPTS));
    out("");
    out("Run 'srun COMMAND --help' for command options.");
}
