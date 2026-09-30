//! login / logout / status / switch.

use super::session::Session;
use crate::config::User;
use crate::error::{Error, Result};
use crate::probe::{self, Probe};
use crate::protocol::{errors, Client, LoginOutcome, StatusResp};
use crate::text::{fmt_bytes, fmt_secs, out};
use std::thread;
use std::time::Duration;

pub fn login(s: &Session) -> Result<()> {
    if s.p.flag("all") {
        return login_all(s);
    }
    let test = s.p.flag("test").then(|| match s.p.value("probe") {
        Some(v) => Probe::parse(v),
        None => Ok(Probe::Server),
    });
    let test = test.transpose()?;
    let fallback = !s.explicit_user();
    let mut last: Option<Error> = None;
    for (i, mut user) in s.login_candidates()?.into_iter().enumerate() {
        // With fallbacks, a broken entry (bad ifname, unbindable ip, no
        // password on a non-tty) must not stop the others from being tried.
        let prepared = s
            .prepare(&user)
            .and_then(|pc| s.ensure_password(&mut user).map(|_| pc));
        let (ip, client) = match prepared {
            Ok(v) => v,
            Err(e) if fallback => {
                crate::log_warn!("user={}: {e}", user.username);
                last = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        };
        if i == 0 {
            if let Some(pr) = &test {
                match probe::check(&client, pr) {
                    Ok(o) if o.is_online() => {
                        crate::log_info!("already online, skipping login");
                        return Ok(());
                    }
                    Ok(_) => {}
                    Err(e) => crate::log_debug!("probe failed: {e}"),
                }
            }
        }
        let acid = s.acid(&client);
        match login_one(s, &client, &user, &ip, acid) {
            Ok(()) => return Ok(()),
            // Someone else holds this address: no candidate can do better.
            Err(e) if !fallback || e.code() == Some("ip_already_online_error") => return Err(e),
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
    let mut failures = 0;
    for mut user in users.iter().cloned() {
        let attempt = s.prepare(&user).and_then(|(ip, client)| {
            s.ensure_password(&mut user)?;
            let acid = s.acid(&client);
            login_one(s, &client, &user, &ip, acid)
        });
        if let Err(e) = attempt {
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

/// Wait before retrying a transient rejection: 10x the network retry delay,
/// doubling each time (10s, 20s, 40s by default).
fn transient_delay(s: &Session, attempt: u32) -> Duration {
    let base = s.cfg.retry_delay_ms.max(100).saturating_mul(10);
    Duration::from_millis(base.saturating_mul(1u64 << attempt.min(6)))
}

/// One user, with retries on network errors and transient rejections.
/// "Already online" counts as success when it is this user; otherwise it is
/// reported with a hint.
fn login_one(s: &Session, client: &Client, user: &User, ip: &str, acid: i64) -> Result<()> {
    let req = s.login_request(user, ip, acid);
    let attempts = s.cfg.retry.max(1);
    for i in 1..=attempts {
        match client.login(&req) {
            Ok(LoginOutcome::LoggedIn { ip, .. }) => {
                crate::log_info!("login ok user={} ip={ip}", req.username);
                return Ok(());
            }
            Ok(LoginOutcome::AlreadyOnline { online_as }) => {
                if online_as.is_empty() || online_as == user.username {
                    crate::log_warn!("already online as {}, nothing to do", user.username);
                    return Ok(());
                }
                return Err(Error::rejected(
                    "ip_already_online_error",
                    format!(
                        "already online as {online_as}; run 'srun switch {}' to change accounts",
                        user.username
                    ),
                ));
            }
            Err(e @ Error::Network(_)) if i < attempts => {
                crate::log_warn!("attempt {i}/{attempts}: {e}");
                thread::sleep(Duration::from_millis(s.cfg.retry_delay_ms));
            }
            Err(e) if e.code().is_some_and(errors::is_transient) && i < attempts => {
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
    unreachable!("the last attempt always returns")
}

/// Who the portal thinks is online on this connection.
pub fn online_user(client: &Client) -> Result<Option<StatusResp>> {
    let st = client.status()?;
    Ok(if st.is_online() { Some(st) } else { None })
}

/// The address a session should be logged out on: the chosen one, else
/// the one the portal reports.
fn session_ip(ip: &str, st: &StatusResp) -> String {
    if ip.is_empty() {
        st.online_ip.clone()
    } else {
        ip.to_string()
    }
}

pub fn logout(s: &Session) -> Result<()> {
    if s.explicit_user() {
        for user in s.select_users()? {
            let (ip, client) = s.prepare(&user)?;
            let acid = s.acid(&client);
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
    let acid = s.acid(&client);
    logout_one(&client, &st.user_name, &session_ip(&ip, &st), acid)
}

pub fn logout_one(client: &Client, username: &str, ip: &str, acid: i64) -> Result<()> {
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
    let (ip, client) = s.prepare(&user)?;
    match online_user(&client)? {
        Some(st) if st.user_name == user.username => {
            crate::log_info!("already online as {}", user.username);
        }
        Some(st) => {
            s.ensure_password(&mut user)?;
            let acid = s.acid(&client);
            let old_ip = session_ip(&ip, &st);
            logout_one(&client, &st.user_name, &old_ip, acid)?;
            if let Err(e) = login_one(s, &client, &user, &ip, acid) {
                return Err(restore_previous(
                    s,
                    &client,
                    &st.user_name,
                    &old_ip,
                    acid,
                    e,
                ));
            }
        }
        None => {
            s.ensure_password(&mut user)?;
            let acid = s.acid(&client);
            login_one(s, &client, &user, &ip, acid)?;
        }
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

/// After a failed switch, put the previous account back so the machine does
/// not stay offline, and fold the result into the error to report.
fn restore_previous(
    s: &Session,
    client: &Client,
    previous: &str,
    ip: &str,
    acid: i64,
    e: Error,
) -> Error {
    let prev = s
        .cfg
        .users
        .iter()
        .find(|u| u.username == previous)
        .cloned()
        .and_then(|mut u| s.ensure_password(&mut u).ok().map(|_| u));
    let Some(prev) = prev else {
        crate::log_warn!(
            "login failed and {previous} has no usable password in the config; now offline"
        );
        return e;
    };
    crate::log_warn!("login failed ({e}); restoring {previous}");
    match login_one(s, client, &prev, ip, acid) {
        Ok(()) => Error::rejected(
            e.code().unwrap_or_default(),
            format!("{e}; still online as {previous}"),
        ),
        Err(e2) => Error::rejected(
            "",
            format!("{e}; restoring {previous} also failed: {e2}; now offline"),
        ),
    }
}

pub fn status(s: &Session) -> Result<()> {
    // The portal reports the session of the address the request comes from,
    // so -i / --ifname only make sense as a local bind address.
    let wants_bind =
        s.cfg.strict_bind || s.p.value("ip").is_some() || s.p.value("ifname").is_some();
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
        s.bound_client(&ip)?
    } else {
        s.client.clone()
    };
    print_status(&client.status()?);
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
