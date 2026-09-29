//! `user ...` and `config ...` subcommands.

use super::args::Parsed;
use crate::config::{self, Config, User};
use crate::error::{Error, Result};
use crate::term;
use crate::text::out;

fn load_for_edit(p: &Parsed) -> Result<(Config, std::path::PathBuf)> {
    let explicit = p.value("config");
    let path = config::path_for_write(explicit);
    let cfg = if explicit.is_some() && !path.exists() {
        Config::default()
    } else {
        Config::load_or_default(&path)?
    };
    Ok((cfg, path))
}

fn resolve_password(p: &Parsed, username: &str) -> Result<String> {
    if let Some(pw) = p.value("password") {
        return Ok(pw.to_string());
    }
    if let Ok(pw) = std::env::var("SRUN_PASSWORD") {
        if !pw.is_empty() {
            return Ok(pw);
        }
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
    let sub = p.positionals.first().map(|s| s.as_str()).unwrap_or("list");
    match sub {
        "add" => {
            let username = p
                .positionals
                .get(1)
                .ok_or_else(|| Error::usage("usage: srun user add USERNAME [--name ALIAS] [-p PASSWORD] [--ip IP | --ifname NAME] [--default] [--force]"))?
                .clone();
            let (mut cfg, path) = load_for_edit(p)?;
            let name = p.value("name").unwrap_or(&username).to_string();
            let exists = cfg
                .users
                .iter()
                .any(|u| u.name == name || u.username == username);
            if exists && !p.flag("force") {
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
                username: username.clone(),
                password,
                ip: p.value("ip").map(|s| s.to_string()),
                ifname: p.value("ifname").map(|s| s.to_string()),
            };
            if exists {
                // Replace the one matching entry in place (alias first, then username).
                let idx = cfg
                    .users
                    .iter()
                    .position(|u| u.name == name)
                    .or_else(|| cfg.users.iter().position(|u| u.username == username))
                    .expect("exists");
                cfg.users[idx] = user;
            } else {
                cfg.users.push(user);
            }
            if p.flag("default") || cfg.users.len() == 1 {
                cfg.default_user = Some(name.clone());
            }
            cfg.save(&path)?;
            crate::log_info!(
                "{} user '{name}' in {}",
                if exists { "updated" } else { "added" },
                path.display()
            );
            Ok(())
        }
        "remove" | "rm" => {
            let name = p
                .positionals
                .get(1)
                .ok_or_else(|| Error::usage("usage: srun user remove NAME"))?;
            let (mut cfg, path) = load_for_edit(p)?;
            let before = cfg.users.len();
            let removed_aliases: Vec<String> = cfg
                .users
                .iter()
                .filter(|u| &u.name == name || &u.username == name)
                .map(|u| u.name.clone())
                .collect();
            cfg.users.retain(|u| &u.name != name && &u.username != name);
            if cfg.users.len() == before {
                if p.flag("force") {
                    crate::log_warn!("user '{name}' not found, nothing to do");
                    return Ok(());
                }
                return Err(Error::config(format!("user '{name}' not found")));
            }
            if cfg
                .default_user
                .as_deref()
                .map(|d| d == name.as_str() || removed_aliases.iter().any(|a| a == d))
                .unwrap_or(false)
            {
                cfg.default_user = None;
            }
            cfg.save(&path)?;
            crate::log_info!("removed user '{name}'");
            Ok(())
        }
        "default" => {
            let name = p
                .positionals
                .get(1)
                .ok_or_else(|| Error::usage("usage: srun user default NAME"))?;
            let (mut cfg, path) = load_for_edit(p)?;
            if cfg.find_user(name).is_none() {
                return Err(Error::config(format!("user '{name}' not found")));
            }
            let name = cfg
                .find_user(name)
                .map(|u| u.name.clone())
                .unwrap_or_default();
            cfg.default_user = Some(name.clone());
            cfg.save(&path)?;
            crate::log_info!("default user is now '{name}'");
            Ok(())
        }
        "list" | "ls" => {
            let path = config::path_for_read(p.value("config"));
            let cfg = Config::load_or_default(&path)?;
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
        other => Err(Error::usage(format!(
            "unknown user subcommand '{other}' (add | remove | list | default)"
        ))),
    }
}

pub fn config_cmd(p: &Parsed) -> Result<()> {
    let sub = p.positionals.first().map(|s| s.as_str()).unwrap_or("show");
    match sub {
        "path" => {
            let path = config::path_for_read(p.value("config"));
            let state = if path.exists() { "exists" } else { "missing" };
            out(&format!("{} ({state})", path.display()));
            if p.value("config").is_none() {
                out("search order:");
                for c in config::paths::candidates() {
                    let state = if c.path.exists() { "exists" } else { "missing" };
                    out(&format!("  {} ({}, {state})", c.path.display(), c.source));
                }
            }
            Ok(())
        }
        "show" => {
            let path = config::path_for_read(p.value("config"));
            let mut cfg = Config::load_or_default(&path)?;
            for u in &mut cfg.users {
                if !u.password.is_empty() {
                    u.password = "***".to_string();
                }
            }
            let text = serde_json::to_string_pretty(&cfg)
                .map_err(|e| Error::internal(format!("serialize: {e}")))?;
            for line in text.lines() {
                out(line);
            }
            Ok(())
        }
        "obfuscate" => {
            let path = config::path_for_write(p.value("config"));
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
        "init" => {
            let path = config::path_for_write(p.value("config"));
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
        other => Err(Error::usage(format!(
            "unknown config subcommand '{other}' (show | path | init | obfuscate)"
        ))),
    }
}
