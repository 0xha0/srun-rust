//! Everything a command needs: parsed args, config, a portal client, and
//! helpers to pick users, IPs and the ac_id.

use super::args::Parsed;
use crate::config::{self, Acid, Config, User};
use crate::error::{Error, Result};
use crate::http::Url;
use crate::net::ifaces;
use crate::protocol::{Client, PasswordMode};
use std::path::PathBuf;

pub struct Session {
    pub p: Parsed,
    pub cfg: Config,
    pub cfg_path: PathBuf,
    pub client: Client,
    pub acid: Acid,
    pub strict_bind: bool,
    pub password_mode: PasswordMode,
    pub retry: u32,
    pub retry_delay_ms: u64,
}

impl Session {
    pub fn build(p: Parsed) -> Result<Session> {
        let explicit = p.value("config");
        let cfg_path = config::path_for_read(explicit);
        let cfg = if explicit.is_some() {
            Config::load(&cfg_path)?
        } else {
            Config::load_or_default(&cfg_path)?
        };
        crate::log_debug!("config {}", cfg_path.display());
        let cfg = decode_passwords(cfg)?;

        let server = p.value("server").unwrap_or(&cfg.server);
        let url = Url::parse(server)?;
        let mut client = Client::new(url);
        client.n = cfg.n;
        client.utype = cfg.utype;
        client.os = cfg.os.clone();
        client.name = cfg.name.clone();
        client.double_stack = cfg.double_stack || p.flag("double-stack");
        client.opts.tls_insecure = p.flag("tls-insecure");
        if let Some(v) = p.value("n") {
            client.n = parse_num(v, "n")?;
        }
        if let Some(v) = p.value("type") {
            client.utype = parse_num(v, "type")?;
        }
        if let Some(v) = p.value("os") {
            client.os = v.to_string();
        }
        if let Some(v) = p.value("name") {
            client.name = v.to_string();
        }
        let acid = match p.value("acid") {
            Some(v) => Acid::parse(v)?,
            None => cfg.acid.clone(),
        };
        let password_mode = match p.value("password-mode") {
            Some(v) => v.parse()?,
            None => cfg.password_mode,
        };
        let retry = match p.value("retry") {
            Some(v) => parse_num::<u32>(v, "retry")?,
            None => cfg.retry,
        };
        let retry_delay_ms = match p.value("retry-delay") {
            Some(v) => parse_num::<u64>(v, "retry-delay")?,
            None => cfg.retry_delay_ms,
        };
        let strict_bind = cfg.strict_bind || p.flag("strict-bind");
        Ok(Session {
            p,
            cfg,
            cfg_path,
            client,
            acid,
            strict_bind,
            password_mode,
            retry,
            retry_delay_ms,
        })
    }

    /// Was a user chosen explicitly on the command line?
    pub fn explicit_user(&self) -> bool {
        self.p.value("username").is_some() || self.p.value("user").is_some() || self.p.flag("all")
    }

    /// Users to act on: ad-hoc `-u`, `--all`, `--user NAME`, or the default.
    pub fn select_users(&self) -> Result<Vec<User>> {
        if let Some(username) = self.p.value("username") {
            let password = match self.p.value("password") {
                Some(pw) => pw.to_string(),
                None => std::env::var("SRUN_PASSWORD").unwrap_or_default(),
            };
            return Ok(vec![User {
                name: username.to_string(),
                username: username.to_string(),
                password,
                ip: self.p.value("ip").map(|s| s.to_string()),
                ifname: self.p.value("ifname").map(|s| s.to_string()),
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
            return self.select_users()?.into_iter().map(Ok).collect();
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
        if !user.password.is_empty() {
            return Ok(());
        }
        if let Ok(pw) = std::env::var("SRUN_PASSWORD") {
            if !pw.is_empty() {
                user.password = pw;
                return Ok(());
            }
        }
        if crate::term::stdin_is_tty() {
            user.password =
                crate::term::read_password(&format!("password for {}: ", user.username))?;
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
        if let Some(ip) = &user.ip {
            if !ip.is_empty() {
                return Ok(ip.clone());
            }
        }
        if let Some(name) = &user.ifname {
            if !name.is_empty() {
                return ip_of_interface(name);
            }
        }
        if self.p.flag("select-ip") {
            return select_ip_interactive();
        }
        if !self.p.flag("detect-ip") {
            crate::log_debug!("no ip given, using the address seen by the server");
        }
        Ok(String::new())
    }

    /// A client for this user, bound to the chosen ip when strict_bind is on.
    pub fn client_for(&self, ip: &str) -> Result<Client> {
        let mut c = self.client.clone();
        if self.strict_bind {
            if ip.is_empty() {
                return Err(Error::usage(
                    "strict_bind needs a concrete ip (-i, --ifname, or ip/ifname in config)",
                ));
            }
            let local = ip
                .parse()
                .map_err(|_| Error::usage(format!("not an ip address: {ip}")))?;
            c.opts.bind_ip = Some(local);
        }
        Ok(c)
    }

    pub fn resolve_acid(&self, client: &Client) -> i64 {
        if let Some(n) = self.acid.fixed() {
            return n;
        }
        match client.detect_acid() {
            Ok(n) => {
                crate::log_info!("acid detected: {n}");
                n
            }
            Err(e) => {
                crate::log_warn!(
                    "acid detection failed ({e}), using {}",
                    crate::protocol::client::DEFAULT_ACID
                );
                crate::protocol::client::DEFAULT_ACID
            }
        }
    }
}

/// Turn `obf1:` passwords back into plaintext for this process only.
pub fn decode_passwords(mut cfg: Config) -> Result<Config> {
    for u in cfg.users.iter_mut() {
        if crate::obf::is_obfuscated(&u.password) {
            match crate::obf::decode(&u.username, &u.password) {
                Ok(pw) => u.password = pw,
                Err(e) => {
                    // One broken entry must not lock every command out;
                    // ensure_password will ask for it or fail clearly later.
                    crate::log_warn!("{e}; treating the password as unset");
                    u.password.clear();
                }
            }
        }
    }
    Ok(cfg)
}

fn parse_num<T: std::str::FromStr>(v: &str, what: &str) -> Result<T> {
    v.parse::<T>()
        .map_err(|_| Error::usage(format!("{what} must be a number, got {v}")))
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
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            break;
        }
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
