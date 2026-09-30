//! `user ...` and `config ...` subcommands.

use super::args::Parsed;
use crate::config::{self, Config, User};
use crate::error::{Error, Result};
use crate::term;
use crate::text::out;
use std::path::PathBuf;

pub const USER_ADD_USAGE: &str = "srun user add USERNAME [--name ALIAS] [-p PASSWORD] [--ip IP | --ifname NAME] [--default] [--force]";

fn load(p: &Parsed) -> Result<(Config, PathBuf)> {
    let path = config::paths::resolve(p.value("config"));
    Ok((Config::load_or_default(&path)?, path))
}

fn positional<'a>(p: &'a Parsed, i: usize, usage: &str) -> Result<&'a String> {
    p.positionals
        .get(i)
        .ok_or_else(|| Error::usage(format!("usage: {usage}")))
}

fn resolve_password(p: &Parsed, username: &str) -> Result<String> {
    if let Some(pw) = p.value("password") {
        return Ok(pw.to_string());
    }
    if let Some(pw) = config::env_password() {
        return Ok(pw);
    }
    if p.flag("password-stdin") {
        return term::read_line_stdin();
    }
    let pw = term::read_password(&format!("password for {username}: "))?;
    if pw.is_empty() {
        return Err(Error::usage("empty password"));
    }
    Ok(pw)
}

pub fn user(p: &Parsed) -> Result<()> {
    match p.positionals.first().map(String::as_str).unwrap_or("list") {
        "add" => user_add(p),
        "remove" | "rm" => user_remove(p),
        "default" => user_default(p),
        "list" | "ls" => user_list(p),
        other => Err(Error::usage(format!(
            "unknown user subcommand '{other}' (add | remove | list | default)"
        ))),
    }
}

fn user_add(p: &Parsed) -> Result<()> {
    let username = positional(p, 1, USER_ADD_USAGE)?.clone();
    let (mut cfg, path) = load(p)?;
    let name = p.value("name").unwrap_or(&username).to_string();
    let existing = cfg
        .users
        .iter()
        .position(|u| u.name == name)
        .or_else(|| cfg.users.iter().position(|u| u.username == username));
    if existing.is_some() && !p.flag("force") {
        return Err(Error::config(format!(
            "user '{name}' already exists (use --force to update it)"
        )));
    }
    let mut password = if p.flag("no-password") {
        String::new()
    } else {
        resolve_password(p, &username)?
    };
    if !password.is_empty() && !p.flag("plain") {
        password = crate::obf::encode(&username, &password);
    }
    let user = User {
        name: name.clone(),
        username,
        password,
        ip: p.value("ip").map(str::to_string),
        ifname: p.value("ifname").map(str::to_string),
    };
    match existing {
        Some(i) => cfg.users[i] = user,
        None => cfg.users.push(user),
    }
    if p.flag("default") || cfg.users.len() == 1 {
        cfg.default_user = Some(name.clone());
    }
    cfg.save(&path)?;
    crate::log_info!(
        "{} user '{name}' in {}",
        if existing.is_some() {
            "updated"
        } else {
            "added"
        },
        path.display()
    );
    Ok(())
}

fn user_remove(p: &Parsed) -> Result<()> {
    let name = positional(p, 1, "srun user remove NAME [--force]")?;
    let (mut cfg, path) = load(p)?;
    let before = cfg.users.len();
    cfg.users.retain(|u| &u.name != name && &u.username != name);
    if cfg.users.len() == before {
        if p.flag("force") {
            crate::log_warn!("user '{name}' not found, nothing to do");
            return Ok(());
        }
        return Err(Error::config(format!("user '{name}' not found")));
    }
    // A default that no longer resolves (by alias or username) is dropped.
    if cfg
        .default_user
        .as_deref()
        .is_some_and(|d| cfg.find_user(d).is_none())
    {
        cfg.default_user = None;
    }
    cfg.save(&path)?;
    crate::log_info!("removed user '{name}'");
    Ok(())
}

fn user_default(p: &Parsed) -> Result<()> {
    let name = positional(p, 1, "srun user default NAME")?;
    let (mut cfg, path) = load(p)?;
    let alias = cfg
        .find_user(name)
        .map(|u| u.name.clone())
        .ok_or_else(|| Error::config(format!("user '{name}' not found")))?;
    cfg.default_user = Some(alias.clone());
    cfg.save(&path)?;
    crate::log_info!("default user is now '{alias}'");
    Ok(())
}

fn user_list(p: &Parsed) -> Result<()> {
    let (cfg, _) = load(p)?;
    if cfg.users.is_empty() {
        out("no users");
        return Ok(());
    }
    let default = cfg.default_user().map(|u| u.name.clone());
    for u in &cfg.users {
        let mark = if default.as_deref() == Some(u.name.as_str()) {
            " (default)"
        } else {
            ""
        };
        let addr = match (&u.ip, &u.ifname) {
            (Some(ip), _) => format!(" ip={ip}"),
            (None, Some(n)) => format!(" ifname={n}"),
            _ => String::new(),
        };
        let pw = if u.password.is_empty() {
            " (no stored password)"
        } else if crate::obf::is_obfuscated(&u.password) {
            ""
        } else {
            " (plaintext password)"
        };
        out(&format!(
            "{} username={}{addr}{pw}{mark}",
            u.name, u.username
        ));
    }
    Ok(())
}

pub fn config_cmd(p: &Parsed) -> Result<()> {
    match p.positionals.first().map(String::as_str).unwrap_or("show") {
        "path" => config_path(p),
        "show" => config_show(p),
        "init" => config_init(p),
        "obfuscate" => config_obfuscate(p),
        other => Err(Error::usage(format!(
            "unknown config subcommand '{other}' (show | path | init | obfuscate)"
        ))),
    }
}

fn state(path: &std::path::Path) -> &'static str {
    if path.exists() {
        "exists"
    } else {
        "missing"
    }
}

fn config_path(p: &Parsed) -> Result<()> {
    let path = config::paths::resolve(p.value("config"));
    out(&format!("{} ({})", path.display(), state(&path)));
    if p.value("config").is_none() {
        out("search order:");
        for c in config::paths::candidates() {
            out(&format!(
                "  {} ({}, {})",
                c.path.display(),
                c.source,
                state(&c.path)
            ));
        }
    }
    Ok(())
}

fn config_show(p: &Parsed) -> Result<()> {
    let (mut cfg, _) = load(p)?;
    for u in &mut cfg.users {
        if !u.password.is_empty() {
            u.password = "***".to_string();
        }
    }
    let text = serde_json::to_string_pretty(&cfg)
        .map_err(|e| Error::internal(format!("serialize: {e}")))?;
    text.lines().for_each(out);
    Ok(())
}

fn config_init(p: &Parsed) -> Result<()> {
    let path = config::paths::resolve(p.value("config"));
    if path.exists() && !p.flag("force") {
        return Err(Error::config(format!(
            "{} already exists (use --force to overwrite)",
            path.display()
        )));
    }
    Config::default().save(&path)?;
    crate::log_info!("wrote {}", path.display());
    Ok(())
}

fn config_obfuscate(p: &Parsed) -> Result<()> {
    let path = config::paths::resolve(p.value("config"));
    let mut cfg = Config::load(&path)?;
    let mut changed = 0;
    for u in cfg.users.iter_mut() {
        if !u.password.is_empty() && !crate::obf::is_obfuscated(&u.password) {
            u.password = crate::obf::encode(&u.username, &u.password);
            changed += 1;
        }
    }
    if changed == 0 {
        crate::log_info!("nothing to do");
        return Ok(());
    }
    cfg.save(&path)?;
    crate::log_info!("obfuscated {changed} password(s) in {}", path.display());
    Ok(())
}
