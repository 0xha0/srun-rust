//! `srun daemon`: stay online. Foreground loop, no fork, no pidfile; the
//! platform's init system (procd, systemd, launchd, task scheduler) keeps
//! the process alive and restarts it.

use crate::cli::session::Session;
use crate::error::{Error, Result};
use crate::probe::{self, Probe};
use crate::protocol::LoginRequest;
use crate::term;
use std::time::{Duration, Instant};

pub const MAX_BACKOFF_SECS: u64 = 600;

pub fn run(s: &Session) -> Result<()> {
    let interval: u64 = match s.p.value("interval") {
        Some(v) => v
            .parse()
            .map_err(|_| Error::usage(format!("interval must be seconds, got {v}")))?,
        None => s.cfg.daemon.interval,
    };
    let interval = interval.max(5);
    let probe = match s.p.value("probe") {
        Some(v) => Probe::parse(v)?,
        None => Probe::parse(&s.cfg.daemon.probe)?,
    };
    let logout_on_exit = s.p.flag("logout-on-exit") || s.cfg.daemon.logout_on_exit;
    let mut users = if s.p.flag("all") {
        s.select_users()?
    } else {
        s.login_candidates()?
    };
    for u in users.iter_mut() {
        s.ensure_password(u)?;
    }
    // Without --all the list is [default, fallbacks...]: one session at a time.
    let one_at_a_time = !s.p.flag("all");

    term::install_stop_handlers();
    crate::log_info!(
        "daemon started interval={interval}s probe={} users={}",
        match &probe {
            Probe::Server => "server".to_string(),
            Probe::None => "none".to_string(),
            Probe::Tcp(t) => t.clone(),
        },
        users.len()
    );

    let mut acid_cache: Option<i64> = None;
    let mut failures: u32 = 0;
    let started = Instant::now();
    while !term::stop_requested() {
        let mut all_ok = true;
        let mut satisfied = false;
        for user in &users {
            if term::stop_requested() || (one_at_a_time && satisfied) {
                break;
            }
            let ip = match s.resolve_ip(user) {
                Ok(ip) => ip,
                Err(e) => {
                    crate::log_warn!("user={}: {e}", user.username);
                    all_ok = false;
                    continue;
                }
            };
            let client = match s.client_for(&ip) {
                Ok(c) => c,
                Err(e) => {
                    crate::log_warn!("user={}: {e}", user.username);
                    all_ok = false;
                    continue;
                }
            };
            // For the server probe, report who the portal says is online.
            let online = if probe == Probe::Server {
                match client.status() {
                    Ok(st) if st.is_online() => {
                        crate::log_debug!("online as {} ip={}", st.user_name, st.online_ip);
                        true
                    }
                    Ok(_) => false,
                    Err(e) => {
                        crate::log_warn!("probe failed: {e}");
                        false
                    }
                }
            } else {
                match probe::is_online(&client, &probe) {
                    Ok(b) => {
                        if b {
                            crate::log_debug!("online (probe ok)");
                        }
                        b
                    }
                    Err(e) => {
                        crate::log_warn!("probe failed: {e}");
                        false
                    }
                }
            };
            if online {
                satisfied = true;
                continue;
            }
            let acid = match acid_cache {
                Some(a) => a,
                None => {
                    // Only a detected/fixed value is kept; a failed detection
                    // (WAN still down) is retried next round.
                    let (a, reliable) = s.resolve_acid(&client);
                    if reliable {
                        acid_cache = Some(a);
                    }
                    a
                }
            };
            let req = LoginRequest {
                username: user.username.clone(),
                password: user.password.clone(),
                ip: ip.clone(),
                acid,
                password_mode: s.password_mode,
            };
            match client.login(&req) {
                Ok(o) => {
                    crate::log_info!("login ok user={} ip={}", user.username, o.ip);
                    satisfied = true;
                }
                Err(Error::Rejected { code, .. })
                    if code == "E2620" || code == "ip_already_online_error" =>
                {
                    crate::log_info!("already online user={}", user.username);
                    satisfied = true;
                }
                Err(e) if crate::cli::commands::is_transient(&e) => {
                    crate::log_warn!("login user={}: {e}; backing off", user.username);
                    all_ok = false;
                    if one_at_a_time {
                        break;
                    }
                }
                Err(e @ Error::Rejected { .. }) if one_at_a_time => {
                    crate::log_warn!("login rejected user={}: {e}, trying next", user.username);
                }
                Err(e) => {
                    // Network trouble: never switch accounts over it. End
                    // the round and let the backoff retry the same user.
                    crate::log_warn!(
                        "login failed user={}: {e}; ending this round",
                        user.username
                    );
                    all_ok = false;
                    if one_at_a_time {
                        break;
                    }
                }
            }
        }
        if one_at_a_time && !satisfied {
            all_ok = false;
        }
        let delay = if all_ok {
            failures = 0;
            interval
        } else {
            failures = failures.saturating_add(1);
            let backoff = interval.saturating_mul(1u64 << failures.min(10));
            let jitter = started.elapsed().as_secs() % 7;
            backoff.min(MAX_BACKOFF_SECS) + jitter
        };
        crate::log_debug!("next check in {delay}s");
        if !term::sleep_interruptible(Duration::from_secs(delay)) {
            break;
        }
    }

    if logout_on_exit {
        // Log out only what the portal says is online on this connection
        // (with --all, each user's own bound connection).
        let targets: Vec<&crate::config::User> = if one_at_a_time {
            users.iter().take(1).collect()
        } else {
            users.iter().collect()
        };
        for user in targets {
            let ip = s.resolve_ip(user).unwrap_or_default();
            let Ok(client) = s.client_for(&ip) else {
                continue;
            };
            match client.status() {
                Ok(st) if st.is_online() => {
                    let acid = acid_cache.unwrap_or(crate::protocol::client::DEFAULT_ACID);
                    let ip = if ip.is_empty() {
                        st.online_ip.clone()
                    } else {
                        ip
                    };
                    match client.logout(&st.user_name, &ip, acid) {
                        Ok(_) => crate::log_info!("logout ok user={}", st.user_name),
                        Err(e) => crate::log_warn!("logout user={}: {e}", st.user_name),
                    }
                }
                Ok(_) => crate::log_debug!("not online, nothing to log out"),
                Err(e) => crate::log_warn!("logout check failed: {e}"),
            }
        }
    }
    crate::log_info!("daemon stopped");
    Ok(())
}
