//! login / logout / status / switch.

use super::session::Session;
use crate::config::User;
use crate::error::{Error, Result};
use crate::probe::{self, Probe};
use crate::protocol::{Client, LoginRequest, StatusResp};
use crate::text::{fmt_bytes, fmt_secs, out};
use std::thread;
use std::time::Duration;

pub fn is_already_online(e: &Error) -> bool {
    matches!(e, Error::Rejected { code, .. } if code == "ip_already_online_error" || code == "E2620")
}

/// Rejections the portal lifts by itself after a while: E2532 (two
/// authentications too close together) and E2533 (too many attempts).
pub fn is_transient(e: &Error) -> bool {
    matches!(e, Error::Rejected { code, .. } if code == "E2532" || code == "E2533")
}

/// Wait before retrying a transient rejection: 10x the network retry delay,
/// doubling each time (10s, 20s, 40s by default).
fn transient_delay(s: &Session, attempt: u32) -> Duration {
    let base = s.retry_delay_ms.max(100).saturating_mul(10);
    Duration::from_millis(base.saturating_mul(1u64 << attempt.min(6)))
}

pub fn login(s: &Session) -> Result<()> {
    let test = if s.p.flag("test") {
        Some(match s.p.value("probe") {
            Some(v) => Probe::parse(v)?,
            None => Probe::Server,
        })
    } else {
        None
    };
    if s.p.flag("all") {
        return login_all(s);
    }
    let candidates = s.login_candidates()?;
    let mut acid_cache: Option<i64> = None;
    let mut last: Option<Error> = None;
    let fallback = !s.explicit_user();
    for (i, mut user) in candidates.into_iter().enumerate() {
        // With fallbacks, a broken entry (bad ifname, unbindable ip, no
        // password on a non-tty) must not stop the others from being tried.
        let prepared = s
            .resolve_ip(&user)
            .and_then(|ip| s.client_for(&ip).map(|c| (ip, c)));
        let (ip, client) = match prepared {
            Ok(v) => v,
            Err(e) if fallback => {
                crate::log_warn!("user={}: {e}", user.username);
                last = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        };
        if let (0, Some(pr)) = (i, &test) {
            match probe::is_online(&client, pr) {
                Ok(true) => {
                    crate::log_info!("already online, skipping login");
                    return Ok(());
                }
                Ok(false) => {}
                Err(e) => crate::log_debug!("probe failed: {e}"),
            }
        }
        let acid = cached_acid(s, &client, &mut acid_cache);
        if let Err(e) = s.ensure_password(&mut user) {
            if fallback {
                crate::log_warn!("user={}: {e}", user.username);
                last = Some(e);
                continue;
            }
            return Err(e);
        }
        match login_one(s, &client, &user, &ip, acid) {
            Ok(()) => return Ok(()),
            Err(e) if s.explicit_user() || is_already_online(&e) => return Err(e),
            Err(e @ Error::Rejected { .. }) => {
                crate::log_warn!("user={}: {e}", user.username);
                last = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| Error::internal("no login candidates")))
}

fn login_all(s: &Session) -> Result<()> {
    let users = s.select_users()?;
    let mut acid_cache: Option<i64> = None;
    let mut failures = 0;
    for mut user in users.iter().cloned() {
        let ip = s.resolve_ip(&user)?;
        let client = s.client_for(&ip)?;
        let acid = cached_acid(s, &client, &mut acid_cache);
        if let Err(e) = s
            .ensure_password(&mut user)
            .and_then(|_| login_one(s, &client, &user, &ip, acid))
        {
            crate::log_error!("user={}: {e}", user.username);
            failures += 1;
        }
    }
    if failures > 0 {
        return Err(Error::rejected(
            "",
            format!("{failures} of {} logins failed", users.len()),
        ));
    }
    Ok(())
}

fn cached_acid(s: &Session, client: &Client, cache: &mut Option<i64>) -> i64 {
    match cache {
        Some(a) => *a,
        None => {
            let a = s.resolve_acid(client);
            *cache = Some(a);
            a
        }
    }
}

/// One user, with retries on network errors. "Already online" counts as
/// success when it is this user; otherwise it is reported with a hint.
fn login_one(s: &Session, client: &Client, user: &User, ip: &str, acid: i64) -> Result<()> {
    let req = LoginRequest {
        username: user.username.clone(),
        password: user.password.clone(),
        ip: ip.to_string(),
        acid,
        password_mode: s.password_mode,
    };
    let attempts = s.retry.max(1);
    for i in 1..=attempts {
        match client.login(&req) {
            Ok(outcome) => {
                crate::log_info!("login ok user={} ip={}", req.username, outcome.ip);
                return Ok(());
            }
            Err(e) if is_already_online(&e) => {
                let who = client.status().map(|st| st.user_name).unwrap_or_default();
                if who.is_empty() || who == user.username {
                    crate::log_warn!("already online as {}, nothing to do", user.username);
                    return Ok(());
                }
                return Err(Error::rejected(
                    "ip_already_online_error",
                    format!(
                        "already online as {who}; run 'srun switch {}' to change accounts",
                        user.username
                    ),
                ));
            }
            Err(e @ Error::Network(_)) if i < attempts => {
                crate::log_warn!("attempt {i}/{attempts}: {e}");
                thread::sleep(Duration::from_millis(s.retry_delay_ms));
            }
            Err(e) if is_transient(&e) && i < attempts => {
                let wait = transient_delay(s, i - 1);
                crate::log_warn!(
                    "attempt {i}/{attempts}: {e}; portal asks to wait, retrying in {}s",
                    wait.as_secs()
                );
                thread::sleep(wait);
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// Who the portal thinks is online on this connection.
fn online_user(client: &Client) -> Result<Option<StatusResp>> {
    let st = client.status()?;
    Ok(if st.is_online() { Some(st) } else { None })
}

pub fn logout(s: &Session) -> Result<()> {
    if s.p.flag("all") || s.p.value("user").is_some() || s.p.value("username").is_some() {
        let users = s.select_users()?;
        let mut acid_cache: Option<i64> = None;
        for user in &users {
            let ip = s.resolve_ip(user)?;
            let client = s.client_for(&ip)?;
            let acid = cached_acid(s, &client, &mut acid_cache);
            logout_one(&client, &user.username, &ip, acid)?;
        }
        return Ok(());
    }
    // No explicit user: log out whoever is online on this connection.
    let fallback = s.cfg.default_user().cloned().unwrap_or_default();
    let ip = s.resolve_ip(&fallback)?;
    let client = if ip.is_empty() {
        s.client.clone()
    } else {
        s.client_for(&ip)?
    };
    let Some(st) = online_user(&client)? else {
        crate::log_warn!("not online, nothing to do");
        return Ok(());
    };
    let acid = s.resolve_acid(&client);
    let ip = if ip.is_empty() {
        st.online_ip.clone()
    } else {
        ip
    };
    logout_one(&client, &st.user_name, &ip, acid)
}

fn logout_one(client: &Client, username: &str, ip: &str, acid: i64) -> Result<()> {
    match client.logout(username, ip, acid) {
        Ok(_) => {
            crate::log_info!("logout ok user={username}");
            Ok(())
        }
        Err(Error::Rejected { code, .. }) if code == "not_online_error" => {
            crate::log_warn!("user={username} was not online");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// `srun switch NAME`: end up online as NAME, whatever the current state,
/// and remember NAME as the default user. Idempotent.
pub fn switch(s: &Session) -> Result<()> {
    let name =
        s.p.positionals
            .first()
            .ok_or_else(|| Error::usage("usage: srun switch NAME"))?;
    let mut user = s.cfg.find_user(name).cloned().ok_or_else(|| {
        Error::config(format!(
            "user '{name}' not found in {}",
            s.cfg_path.display()
        ))
    })?;
    let ip = s.resolve_ip(&user)?;
    let client = s.client_for(&ip)?;
    let acid = s.resolve_acid(&client);
    if let Some(st) = online_user(&client)? {
        if st.user_name == user.username {
            crate::log_info!("already online as {}", user.username);
        } else {
            let old_ip = if ip.is_empty() {
                st.online_ip.clone()
            } else {
                ip.clone()
            };
            s.ensure_password(&mut user)?;
            logout_one(&client, &st.user_name, &old_ip, acid)?;
            if let Err(e) = login_one(s, &client, &user, &ip, acid) {
                // Do not leave the machine offline: put the old account back.
                let mut previous = s
                    .cfg
                    .users
                    .iter()
                    .find(|u| u.username == st.user_name)
                    .cloned();
                if let Some(prev) = previous.as_mut() {
                    if s.ensure_password(prev).is_err() {
                        previous = None;
                    }
                }
                return match previous {
                    Some(prev) => {
                        crate::log_warn!(
                            "login as {} failed ({e}); restoring {}",
                            user.username,
                            prev.username
                        );
                        match login_one(s, &client, &prev, &old_ip, acid) {
                            Ok(()) => {
                                let code = match &e {
                                    Error::Rejected { code, .. } => code.clone(),
                                    _ => String::new(),
                                };
                                Err(Error::rejected(
                                    code,
                                    format!("{e}; still online as {}", prev.username),
                                ))
                            }
                            Err(e2) => Err(Error::rejected(
                                "",
                                format!(
                                    "{e}; restoring {} also failed: {e2}; now offline",
                                    prev.username
                                ),
                            )),
                        }
                    }
                    _ => {
                        crate::log_warn!(
                            "login as {} failed and {} is not in the config; now offline",
                            user.username,
                            st.user_name
                        );
                        Err(e)
                    }
                };
            }
        }
    } else {
        s.ensure_password(&mut user)?;
        login_one(s, &client, &user, &ip, acid)?;
    }
    if s.cfg.default_user.as_deref() != Some(user.name.as_str()) {
        // Reload from disk: the in-memory config holds decoded passwords.
        let mut cfg = crate::config::Config::load(&s.cfg_path)?;
        cfg.default_user = Some(user.name.clone());
        cfg.save(&s.cfg_path)?;
        crate::log_info!("default user is now '{}'", user.name);
    }
    Ok(())
}

pub fn status(s: &Session) -> Result<()> {
    // The portal reports the session of the address the request comes from,
    // so -i / --ifname only make sense as a local bind address.
    let wants_bind = s.strict_bind || s.p.value("ip").is_some() || s.p.value("ifname").is_some();
    let client = if wants_bind {
        let user = s
            .select_users()
            .unwrap_or_default()
            .into_iter()
            .next()
            .unwrap_or_default();
        let ip = s.resolve_ip(&user)?;
        if ip.is_empty() {
            return Err(Error::usage(
                "status needs a local ip to bind to (-i IP or --ifname NAME)",
            ));
        }
        let mut c = s.client.clone();
        c.opts.bind_ip = Some(
            ip.parse()
                .map_err(|_| Error::usage(format!("not an ip address: {ip}")))?,
        );
        c
    } else {
        s.client.clone()
    };
    let st = client.status()?;
    print_status(&st);
    Ok(())
}

pub fn print_status(st: &StatusResp) {
    if !st.is_online() {
        out("online: no");
        if !st.online_ip.is_empty() {
            out(&format!("ip: {}", st.online_ip));
        }
        return;
    }
    out("online: yes");
    out(&format!("ip: {}", st.online_ip));
    out(&format!("username: {}", st.user_name));
    if !st.products_name.is_empty() {
        out(&format!("product: {}", st.products_name));
    }
    out(&format!("wallet: {:.2}", st.wallet_balance));
    out(&format!("balance: {:.2}", st.user_balance));
    out(&format!("used: {}", fmt_bytes(st.sum_bytes)));
    if st.remain_bytes > 0 {
        out(&format!("remaining: {}", fmt_bytes(st.remain_bytes)));
    }
    // add_time = when this session started, keepalive_time = last seen;
    // both are server clocks, so the difference is valid even when the
    // local clock is wrong. sum_seconds is the billing-period total.
    if st.keepalive_time >= st.add_time && st.add_time > 0 {
        out(&format!(
            "session: {}",
            fmt_secs(st.keepalive_time - st.add_time)
        ));
    }
    out(&format!("total time: {}", fmt_secs(st.sum_seconds)));
    if !st.user_mac.is_empty() {
        out(&format!("mac: {}", st.user_mac));
    }
}
