//! Everything a command needs: the parsed args, the config with command-line
//! overrides applied, a portal client, and helpers to pick users, IPs and
//! the ac_id.

use super::args::Parsed;
use crate::config::{self, Acid, Config, User};
use crate::error::{Error, Result};
use crate::net::ifaces;
use crate::protocol::{client::DEFAULT_ACID, Client, LoginRequest};
use std::cell::Cell;
use std::path::PathBuf;

pub struct Session {
    pub p: Parsed,
    /// Config with `--acid`, `--retry`, `--strict-bind` ... already applied.
    pub cfg: Config,
    pub cfg_path: PathBuf,
    pub client: Client,
    /// Memoized ac_id; only a fixed or successfully detected value is kept.
    acid: Cell<Option<i64>>,
}

fn parse_num<T: std::str::FromStr>(v: &str, what: &str) -> Result<T> {
    v.parse::<T>()
        .map_err(|_| Error::usage(format!("{what} must be a number, got {v}")))
}

impl Session {
    pub fn build(p: Parsed) -> Result<Session> {
        let explicit = p.value("config");
        let cfg_path = config::paths::resolve(explicit);
        let mut cfg = if explicit.is_some() {
            Config::load(&cfg_path)?
        } else {
            Config::load_or_default(&cfg_path)?
        };
        crate::log_debug!("config {}", cfg_path.display());
        cfg.decode_passwords();

        // Command-line overrides.
        if let Some(v) = p.value("server") {
            cfg.server = v.to_string();
        }
        if let Some(v) = p.value("acid") {
            cfg.acid = Acid::parse(v)?;
        }
        if let Some(v) = p.value("password-mode") {
            cfg.password_mode = v.parse()?;
        }
        if let Some(v) = p.value("retry") {
            cfg.retry = parse_num(v, "retry")?;
        }
        if let Some(v) = p.value("retry-delay") {
            cfg.retry_delay_ms = parse_num(v, "retry-delay")?;
        }
        if let Some(v) = p.value("n") {
            cfg.n = parse_num(v, "n")?;
        }
        if let Some(v) = p.value("type") {
            cfg.utype = parse_num(v, "type")?;
        }
        if let Some(v) = p.value("os") {
            cfg.os = v.to_string();
        }
        if let Some(v) = p.value("name") {
            cfg.name = v.to_string();
        }
        cfg.strict_bind |= p.flag("strict-bind");
        cfg.double_stack |= p.flag("double-stack");

        let mut client = cfg.client(&cfg.server)?;
        client.opts.tls_insecure = p.flag("tls-insecure");
        Ok(Session {
            p,
            cfg,
            cfg_path,
            client,
            acid: Cell::new(None),
        })
    }

    /// Was a user chosen explicitly on the command line?
    pub fn explicit_user(&self) -> bool {
        self.p.value("username").is_some() || self.p.value("user").is_some() || self.p.flag("all")
    }

    /// Users to act on: ad-hoc `-u`, `--all`, `--user NAME`, or the default.
    pub fn select_users(&self) -> Result<Vec<User>> {
        if let Some(username) = self.p.value("username") {
            return Ok(vec![User {
                name: username.to_string(),
                username: username.to_string(),
                password: self
                    .p
                    .value("password")
                    .map(str::to_string)
                    .or_else(config::env_password)
                    .unwrap_or_default(),
                ip: self.p.value("ip").map(str::to_string),
                ifname: self.p.value("ifname").map(str::to_string),
            }]);
        }
        if self.p.flag("all") {
            if self.cfg.users.is_empty() {
                return Err(Error::config("no users in config"));
            }
            return Ok(self.cfg.users.clone());
        }
        if let Some(name) = self.p.value("user") {
            return self
                .cfg
                .find_user(name)
                .cloned()
                .map(|u| vec![u])
                .ok_or_else(|| {
                    Error::config(format!(
                        "user '{name}' not found in {}",
                        self.cfg_path.display()
                    ))
                });
        }
        match self.cfg.default_user() {
            Some(u) => Ok(vec![u.clone()]),
            None if self.cfg.users.is_empty() => Err(Error::config(
                "no user configured: run 'srun user add USERNAME' or pass -u USERNAME -p PASSWORD",
            )),
            None => Err(Error::config(
                "several users configured: pass --user NAME, --all, or set default_user",
            )),
        }
    }

    /// Candidates for login: an explicit selection, or the default user
    /// followed by every other configured user as fallbacks.
    pub fn login_candidates(&self) -> Result<Vec<User>> {
        if self.explicit_user() {
            return self.select_users();
        }
        let v = self.cfg.users_default_first();
        if v.is_empty() {
            return Err(Error::config(
                "no user configured: run 'srun user add USERNAME' or pass -u USERNAME -p PASSWORD",
            ));
        }
        Ok(v)
    }

    /// Fill in a missing password from `SRUN_PASSWORD` or an interactive prompt.
    pub fn ensure_password(&self, user: &mut User) -> Result<()> {
        if user.password.is_empty() {
            if let Some(pw) = config::env_password() {
                user.password = pw;
            } else if crate::term::stdin_is_tty() {
                user.password =
                    crate::term::read_password(&format!("password for {}: ", user.username))?;
            }
        }
        if user.password.is_empty() {
            return Err(Error::usage(format!(
                "no password for {} (use -p, SRUN_PASSWORD, or store it with 'srun user add')",
                user.username
            )));
        }
        Ok(())
    }

    /// The IP to authorize for `user`; empty means "let the server decide".
    pub fn resolve_ip(&self, user: &User) -> Result<String> {
        if let Some(ip) = self.p.value("ip") {
            return Ok(ip.to_string());
        }
        if let Some(name) = self.p.value("ifname") {
            return ip_of_interface(name);
        }
        if let Some(ip) = user.ip.as_deref().filter(|s| !s.is_empty()) {
            return Ok(ip.to_string());
        }
        if let Some(name) = user.ifname.as_deref().filter(|s| !s.is_empty()) {
            return ip_of_interface(name);
        }
        if self.p.flag("select-ip") {
            return select_ip_interactive();
        }
        if !self.p.flag("detect-ip") {
            crate::log_debug!("no ip given, using the address seen by the server");
        }
        Ok(String::new())
    }

    /// A client whose connections come from `ip`.
    pub fn bound_client(&self, ip: &str) -> Result<Client> {
        let mut c = self.client.clone();
        c.opts.bind_ip = Some(
            ip.parse()
                .map_err(|_| Error::usage(format!("not an ip address: {ip}")))?,
        );
        Ok(c)
    }

    /// A client for this user: bound to `ip` when strict_bind is on.
    pub fn client_for(&self, ip: &str) -> Result<Client> {
        if !self.cfg.strict_bind {
            return Ok(self.client.clone());
        }
        if ip.is_empty() {
            return Err(Error::usage(
                "strict_bind needs a concrete ip (-i, --ifname, or ip/ifname in config)",
            ));
        }
        self.bound_client(ip)
    }

    /// Resolve the ip for `user` and build its client in one step.
    pub fn prepare(&self, user: &User) -> Result<(String, Client)> {
        let ip = self.resolve_ip(user)?;
        let client = self.client_for(&ip)?;
        Ok((ip, client))
    }

    /// The ac_id: fixed in config, or detected once through the portal's
    /// redirect chain. A failed detection (WAN still down) uses the default
    /// for this attempt only and is retried next time.
    pub fn acid(&self, client: &Client) -> i64 {
        if let Some(a) = self.acid.get() {
            return a;
        }
        if let Some(n) = self.cfg.acid.fixed() {
            self.acid.set(Some(n));
            return n;
        }
        match client.detect_acid() {
            Ok(n) => {
                crate::log_info!("acid detected: {n}");
                self.acid.set(Some(n));
                n
            }
            Err(e) => {
                crate::log_warn!(
                    "acid detection failed ({e}), using {DEFAULT_ACID} for this attempt"
                );
                DEFAULT_ACID
            }
        }
    }

    pub fn login_request(&self, user: &User, ip: &str, acid: i64) -> LoginRequest {
        LoginRequest {
            username: user.username.clone(),
            password: user.password.clone(),
            ip: ip.to_string(),
            acid,
            password_mode: self.cfg.password_mode,
        }
    }
}

fn ip_of_interface(name: &str) -> Result<String> {
    ifaces::ipv4_by_name(name)
        .map(|ip| ip.to_string())
        .ok_or_else(|| Error::usage(format!("no ipv4 address on interface matching '{name}'")))
}

fn select_ip_interactive() -> Result<String> {
    let ips = ifaces::list();
    if ips.is_empty() {
        return Err(Error::usage("no network interfaces with an address"));
    }
    if ips.len() == 1 {
        return Ok(ips[0].1.to_string());
    }
    crate::text::err("select the ip to authorize:");
    for (i, (name, ip)) in ips.iter().enumerate() {
        crate::text::err(&format!("  {}. {ip} ({name})", i + 1));
    }
    for attempt in 1..=3 {
        let Ok(line) = crate::term::read_line_stdin() else {
            break;
        };
        if let Ok(n) = line.trim().parse::<usize>() {
            if n >= 1 && n <= ips.len() {
                let ip = ips[n - 1].1.to_string();
                crate::log_info!("selected {ip}");
                return Ok(ip);
            }
        }
        crate::text::err(&format!("not a valid number ({attempt}/3)"));
    }
    Err(Error::usage("no ip selected"))
}
