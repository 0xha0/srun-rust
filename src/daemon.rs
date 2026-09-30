//! `srun daemon`: stay online. Foreground loop, no fork, no pidfile; the
//! platform's init system (procd, systemd, launchd, task scheduler) keeps
//! the process alive and restarts it.

use crate::cli::commands::{logout_one, online_user};
use crate::cli::session::Session;
use crate::config::User;
use crate::error::{Error, Result};
use crate::probe::{self, Online, Probe};
use crate::protocol::{errors, LoginOutcome};
use crate::term;
use std::time::{Duration, Instant};

pub const MAX_BACKOFF_SECS: u64 = 600;

/// What one user's turn in a round produced.
enum Turn {
    /// The address is online (probe or login succeeded); the round is done.
    Satisfied,
    /// The portal rejected this user for good; try the next one.
    Rejected,
    /// Network trouble or a transient rejection: end the round and back off.
    EndRound,
}

pub fn run(s: &Session) -> Result<()> {
    let interval: u64 = match s.p.value("interval") {
        Some(v) => v
            .parse()
            .map_err(|_| Error::usage(format!("interval must be seconds, got {v}")))?,
        None => s.cfg.daemon.interval,
    };
    let interval = interval.max(1);
    let probe = match s.p.value("probe") {
        Some(v) => Probe::parse(v)?,
        None => Probe::parse(&s.cfg.daemon.probe)?,
    };
    let logout_on_exit = s.p.flag("logout-on-exit") || s.cfg.daemon.logout_on_exit;
    // Without --all the list is [default, fallbacks...]: one session at a
    // time, so a round stops at the first satisfied user. With --all every
    // user has its own bound connection and each gets a turn.
    let one_at_a_time = !s.p.flag("all");
    let mut users = if one_at_a_time {
        s.login_candidates()?
    } else {
        s.select_users()?
    };
    for u in users.iter_mut() {
        s.ensure_password(u)?;
    }

    term::install_stop_handlers();
    crate::log_info!(
        "daemon started interval={interval}s probe={probe} users={}",
        users.len()
    );

    let mut failures: u32 = 0;
    let started = Instant::now();
    while !term::stop_requested() {
        let mut round_ok = !one_at_a_time;
        for user in &users {
            if term::stop_requested() {
                break;
            }
            match turn(s, user, &probe) {
                Turn::Satisfied if one_at_a_time => {
                    round_ok = true;
                    break;
                }
                Turn::Satisfied => {}
                Turn::Rejected => {
                    if !one_at_a_time {
                        round_ok = false;
                    }
                }
                Turn::EndRound => {
                    round_ok = false;
                    if one_at_a_time {
                        break;
                    }
                }
            }
        }
        let delay = if round_ok {
            failures = 0;
            interval
        } else {
            failures = failures.saturating_add(1);
            let backoff = interval.saturating_mul(1u64 << failures.min(10));
            let jitter = started.elapsed().as_secs() % (interval / 8).max(1);
            backoff.min(MAX_BACKOFF_SECS) + jitter
        };
        crate::log_debug!("next check in {delay}s");
        if !term::sleep_interruptible(Duration::from_secs(delay)) {
            break;
        }
    }

    if logout_on_exit {
        // Log out only what the portal says is online (with --all, on each
        // user's own bound connection).
        let targets = if one_at_a_time {
            &users[..1.min(users.len())]
        } else {
            &users[..]
        };
        for user in targets {
            let Ok((ip, client)) = s.prepare(user) else {
                continue;
            };
            match online_user(&client) {
                Ok(Some(st)) => {
                    let ip = if ip.is_empty() {
                        st.online_ip.clone()
                    } else {
                        ip
                    };
                    if let Err(e) = logout_one(&client, &st.user_name, &ip, s.acid(&client)) {
                        crate::log_warn!("logout user={}: {e}", st.user_name);
                    }
                }
                Ok(None) => crate::log_debug!("not online, nothing to log out"),
                Err(e) => crate::log_warn!("logout check failed: {e}"),
            }
        }
    }
    crate::log_info!("daemon stopped");
    Ok(())
}

fn turn(s: &Session, user: &User, probe: &Probe) -> Turn {
    let (ip, client) = match s.prepare(user) {
        Ok(v) => v,
        Err(e) => {
            crate::log_warn!("user={}: {e}", user.username);
            return Turn::Rejected;
        }
    };
    match probe::check(&client, probe) {
        Ok(Online::Portal(st)) => {
            crate::log_debug!("online as {} ip={}", st.user_name, st.online_ip);
            return Turn::Satisfied;
        }
        Ok(Online::Tcp) => {
            crate::log_debug!("online (probe ok)");
            return Turn::Satisfied;
        }
        Ok(Online::Offline) => {}
        Err(e) => {
            // The portal itself is unreachable: a login would only burn
            // more connect timeouts against the same host.
            crate::log_warn!("probe failed: {e}; ending this round");
            return Turn::EndRound;
        }
    }
    let acid = s.acid(&client);
    match client.login(&s.login_request(user, &ip, acid)) {
        Ok(LoginOutcome::LoggedIn { ip, .. }) => {
            crate::log_info!("login ok user={} ip={ip}", user.username);
            Turn::Satisfied
        }
        Ok(LoginOutcome::AlreadyOnline { online_as }) => {
            crate::log_info!("already online as {online_as}");
            Turn::Satisfied
        }
        Err(e) if e.code().is_some_and(errors::is_transient) => {
            crate::log_warn!("login user={}: {e}; backing off", user.username);
            Turn::EndRound
        }
        Err(e @ Error::Rejected { .. }) => {
            crate::log_warn!("login rejected user={}: {e}, trying next", user.username);
            Turn::Rejected
        }
        Err(e) => {
            // Network trouble: never switch accounts over it.
            crate::log_warn!(
                "login failed user={}: {e}; ending this round",
                user.username
            );
            Turn::EndRound
        }
    }
}
