//! End-to-end: the real binary against the mock portal, driven by a temp config.

mod common;

use common::{MockServer, CLIENT_IP, PASSWORD};
use std::path::PathBuf;
use std::process::Command;

/// A fresh temp dir per test and mock server.
fn tmp_dir(m: &MockServer, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("srun-{tag}-{}-{}", std::process::id(), m.port));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a config with the given users JSON (array body) and extra top-level fields.
fn write_config(m: &MockServer, tag: &str, users: &str, extra: &str) -> PathBuf {
    let path = tmp_dir(m, tag).join("config.json");
    std::fs::write(
        &path,
        format!(r#"{{"server":"{}","users":[{users}]{extra}}}"#, m.url()),
    )
    .unwrap();
    path
}

fn tmp_config(m: &MockServer, extra: &str) -> PathBuf {
    write_config(
        m,
        "cli",
        &format!(r#"{{"name":"me","username":"1120240001","password":"{PASSWORD}"}}"#),
        extra,
    )
}

fn run_env(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_srun"));
    cmd.args(args)
        .env_remove("SRUN_CONFIG")
        .env("HOME", std::env::temp_dir());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn run(args: &[&str]) -> (i32, String, String) {
    run_env(args, &[])
}

fn assert_ascii(s: &str) {
    for b in s.bytes() {
        assert!(
            b == b'\n' || (0x20..=0x7E).contains(&b),
            "non-ascii byte {b:#x} in {s:?}"
        );
    }
}

#[test]
fn version_and_help() {
    let (code, o, _) = run(&["version"]);
    assert_eq!(code, 0);
    assert!(o.starts_with("srun 0."));
    let (code, o, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(o.contains("Commands:"));
    let (code, o, _) = run(&["login", "-h"]);
    assert_eq!(code, 0);
    assert!(o.contains("--strict-bind"));
    let (code, _, e) = run(&["frobnicate"]);
    assert_eq!(code, 2);
    assert!(e.contains("unknown command"));
}

#[test]
fn full_flow_with_config() {
    let m = MockServer::start();
    let cfg = tmp_config(&m, "");
    let c = cfg.to_str().unwrap();

    let (code, o, e) = run(&["-c", c, "status"]);
    assert_eq!(code, 0, "{e}");
    assert_eq!(o, "online: no\nip: 10.9.8.7\n");

    // default command is login; acid comes from the redirect chain
    let (code, _, e) = run(&["-c", c]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("acid detected: 8"), "{e}");
    assert!(
        e.contains(&format!("login ok user=1120240001 ip={CLIENT_IP}")),
        "{e}"
    );
    assert!(!e.contains(PASSWORD), "password must never be printed");

    let (code, o, e) = run(&["-c", c, "info"]);
    assert_eq!(code, 0, "{e}");
    assert!(
        o.starts_with(
            "online: yes\nip: 10.9.8.7\nusername: 1120240001\nproduct: \u{5b66}\u{751f}-10"
        ),
        "{o}"
    );
    assert!(o.contains("used: 210.21 GB\n"));
    assert!(o.contains("session: 16:16:47\n"), "{o}");
    assert!(o.contains("total time: 1895:36:24\n"), "{o}");

    // --ascii / SRUN_ASCII=1 escape server text for terminals without UTF-8
    let (code, o, _) = run(&["-c", c, "status", "--ascii"]);
    assert_eq!(code, 0);
    assert_ascii(&o);
    assert!(o.contains("product: \\u5B66\\u751F-10"), "{o}");
    let (_, o, _) = run_env(&["-c", c, "status"], &[("SRUN_ASCII", "1")]);
    assert_ascii(&o);

    let (code, _, e) = run(&["-c", c, "login", "--test"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("already online"));

    let (code, _, e) = run(&["-c", c, "logout", "--acid", "8"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("logout ok"));
    let (code, _, e) = run(&["-c", c, "logout", "--acid", "8"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("not online, nothing to do"), "{e}");
}

#[test]
fn adhoc_user_and_rejections() {
    let m = MockServer::start();
    let url = m.url();
    let (code, _, e) = run(&[
        "-s",
        &url,
        "login",
        "-u",
        "1120240001",
        "-p",
        "wrong",
        "--acid",
        "8",
    ]);
    assert_eq!(code, 3, "{e}");
    assert!(e.contains("rejected: wrong password (E2553)"), "{e}");

    let (code, _, e) = run(&["-s", &url, "login", "-u", "1120240001", "--acid", "8"]);
    assert_eq!(code, 2, "{e}");
    assert!(e.contains("no password"));

    let (code, _, e) = run_env(
        &["-s", &url, "login", "-u", "1120240001", "--acid", "8", "-q"],
        &[("SRUN_PASSWORD", PASSWORD)],
    );
    assert_eq!(code, 0, "{e}");
    assert!(e.is_empty(), "quiet mode prints nothing on success");
}

#[test]
fn config_errors() {
    let (code, _, e) = run(&["-c", "/nonexistent/srun.json", "status"]);
    assert_eq!(code, 5);
    assert!(e.contains("config: read"));
    let m = MockServer::start();
    let (code, _, e) = run(&["-s", &m.url(), "login"]);
    assert_eq!(code, 5, "{e}");
    assert!(e.contains("no user configured"));
}

#[test]
fn network_error_exit_code() {
    let (code, _, e) = run(&["-s", "http://127.0.0.1:1", "status"]);
    assert_eq!(code, 4, "{e}");
}

#[test]
fn user_management_round_trip() {
    let m = MockServer::start();
    let dir = tmp_dir(&m, "mgmt");
    let cfg = dir.join("cfg.json");
    let c = cfg.to_str().unwrap();

    let (code, o, _) = run(&["-c", c, "config", "path"]);
    assert_eq!(code, 0);
    assert!(o.contains("(missing)"));

    // add via --password-stdin; the positional is the portal username, --name an alias
    let out = Command::new(env!("CARGO_BIN_EXE_srun"))
        .args([
            "-c",
            c,
            "user",
            "add",
            "1120240001",
            "--name",
            "me",
            "--password-stdin",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut ch| {
            use std::io::Write;
            ch.stdin
                .take()
                .unwrap()
                .write_all(format!("{PASSWORD}\n").as_bytes())
                .unwrap();
            ch.wait_with_output()
        })
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(!text.contains(PASSWORD), "plaintext leaked: {text}");
    assert!(text.contains("\"password\": \"obf1:"), "{text}");
    assert!(text.contains("\"default_user\": \"me\""));

    let (code, _, e) = run(&[
        "-c", c, "user", "add", "u2", "--name", "second", "-p", "pw2", "--ifname", "lo",
    ]);
    assert_eq!(code, 0, "{e}");
    // adding again without --force is refused; with --force it updates
    let (code, _, e) = run(&["-c", c, "user", "add", "u2", "-p", "pw3"]);
    assert_eq!(code, 5, "{e}");
    assert!(e.contains("already exists"), "{e}");
    let (code, _, e) = run(&[
        "-c", c, "user", "add", "u2", "--name", "second", "-p", "pw3", "--ifname", "lo", "--force",
    ]);
    assert_eq!(code, 0, "{e}");
    let (code, o, _) = run(&["-c", c, "user", "list"]);
    assert_eq!(code, 0);
    assert_eq!(
        o,
        "me username=1120240001 (default)\nsecond username=u2 ifname=lo\n"
    );

    let (code, o, _) = run(&["-c", c, "config", "show"]);
    assert_eq!(code, 0);
    assert!(o.contains("\"password\": \"***\""));
    assert!(!o.contains(PASSWORD));

    // the stored default user logs in against the mock, then logs out
    let (code, _, e) = run(&["-c", c, "-s", &m.url(), "login", "--acid", "8"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("login ok user=1120240001"), "{e}");
    let (code, _, e) = run(&["-c", c, "-s", &m.url(), "logout", "--acid", "8"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("logout ok user=1120240001"), "{e}");

    // --user / default accept the alias or the username
    let (code, _, e) = run(&["-c", c, "user", "default", "u2"]);
    assert_eq!(code, 0, "{e}");
    let (code, _, e) = run(&["-c", c, "user", "remove", "1120240001"]);
    assert_eq!(code, 0, "{e}");
    let (code, o, _) = run(&["-c", c, "user", "list"]);
    assert_eq!(code, 0);
    assert_eq!(o, "second username=u2 ifname=lo (default)\n");
    let (code, _, e) = run(&["-c", c, "user", "remove", "ghost"]);
    assert_eq!(code, 5);
    assert!(e.contains("not found"));
    let (code, _, e) = run(&["-c", c, "user", "remove", "ghost", "--force"]);
    assert_eq!(code, 0, "{e}");

    let (code, _, e) = run(&["-c", c, "config", "init"]);
    assert_eq!(code, 5, "{e}");
    assert!(e.contains("already exists"));
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn daemon_relogs_after_kick_and_stops_on_sigterm() {
    let m = MockServer::start();
    let cfg = tmp_config(&m, r#","daemon":{"interval":1}"#);
    let (child, rx) = spawn_daemon(&cfg, &["--acid", "8"]);
    wait_for_line(&rx, "login ok user=1120240001");
    assert_eq!(m.state.lock().unwrap().logins, 1);
    // simulate a kick: server now says offline
    m.state.lock().unwrap().online = false;
    wait_for_line(&rx, "login ok user=1120240001");
    assert_eq!(m.state.lock().unwrap().logins, 2);
    let start = std::time::Instant::now();
    stop_daemon(child);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(3),
        "stop took {:?}",
        start.elapsed()
    );
    wait_for_line(&rx, "daemon stopped");
}

#[test]
fn login_when_already_online_is_a_noop_success() {
    let m = MockServer::start();
    let cfg = tmp_config(&m, "");
    let c = cfg.to_str().unwrap();
    let (code, _, e) = run(&["-c", c, "login", "--acid", "8"]);
    assert_eq!(code, 0, "{e}");
    let (code, _, e) = run(&["-c", c, "login", "--acid", "8"]);
    assert_eq!(code, 0, "{e}");
    assert!(
        e.contains("already online as 1120240001, nothing to do"),
        "{e}"
    );
    assert_eq!(m.state.lock().unwrap().logins, 2);
}

#[test]
fn logout_needs_no_config_and_uses_whoever_is_online() {
    let m = MockServer::start();
    let url = m.url();
    let (code, _, e) = run(&["-s", &url, "logout", "--acid", "8"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("not online, nothing to do"), "{e}");
    let (code, _, _) = run(&[
        "-s",
        &url,
        "login",
        "-u",
        "1120240001",
        "-p",
        PASSWORD,
        "--acid",
        "8",
    ]);
    assert_eq!(code, 0);
    let (code, _, e) = run(&["-s", &url, "logout", "--acid", "8"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("logout ok user=1120240001"), "{e}");
    let p = m.state.lock().unwrap().last_logout.clone().unwrap();
    assert_eq!(p["username"], "1120240001");
    assert!(!m.state.lock().unwrap().online);
}

#[test]
fn default_user_falls_back_to_others_on_rejection() {
    let m = MockServer::start();
    let cfg = write_config(
        &m,
        "fb",
        &format!(
            r#"{{"name":"ok","username":"1120240001","password":"{PASSWORD}"}},{{"name":"bad","username":"nobody","password":"x"}}"#
        ),
        r#","acid":8,"default_user":"bad""#,
    );
    let dir = cfg.parent().unwrap().to_path_buf();
    let c = cfg.to_str().unwrap();
    let (code, _, e) = run(&["-c", c, "login"]);
    assert_eq!(code, 0, "{e}");
    assert!(
        e.contains("warn: user=nobody: rejected: user does not exist (E2531)"),
        "{e}"
    );
    assert!(e.contains("login ok user=1120240001"), "{e}");
    // an explicit user never falls back
    let (code, _, _) = run(&["-s", &m.url(), "logout", "--acid", "8"]);
    assert_eq!(code, 0);
    let (code, _, e) = run(&["-c", c, "login", "--user", "bad"]);
    assert_eq!(code, 3, "{e}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn switch_is_idempotent_and_sets_default() {
    let m = MockServer::start();
    let cfg = write_config(
        &m,
        "sw",
        &format!(
            r#"{{"name":"a","username":"1120240001","password":"{PASSWORD}"}},{{"name":"b","username":"other","password":"{PASSWORD}"}}"#
        ),
        r#","acid":8,"default_user":"a""#,
    );
    let dir = cfg.parent().unwrap().to_path_buf();
    let c = cfg.to_str().unwrap();
    let (code, _, e) = run(&["-c", c, "login"]);
    assert_eq!(code, 0, "{e}");
    // login as the other user while online -> hint to switch
    let (code, _, e) = run(&["-c", c, "login", "--user", "b"]);
    assert_eq!(code, 3, "{e}");
    assert!(
        e.contains("already online as 1120240001; run 'srun switch other'"),
        "{e}"
    );
    let (code, _, e) = run(&["-c", c, "switch", "b"]);
    assert_eq!(code, 0, "{e}");
    assert!(
        e.contains("logout ok user=1120240001") && e.contains("login ok user=other"),
        "{e}"
    );
    assert!(e.contains("default user is now 'b'"), "{e}");
    assert_eq!(m.state.lock().unwrap().online_user, "other");
    let (code, _, e) = run(&["-c", c, "switch", "b"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("already online as other"), "{e}");
    assert!(!e.contains("default user is now"), "{e}");
    // 1: login a; 2: refused attempt for b while a is online; 3: switch
    assert_eq!(m.state.lock().unwrap().logins, 3);
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("\"default_user\": \"b\""));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stored_user_without_password_uses_env_or_fails_clearly() {
    let m = MockServer::start();
    let dir = tmp_dir(&m, "np");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.json");
    let c = cfg.to_str().unwrap();
    let (code, _, e) = run(&["-c", c, "user", "add", "1120240001", "--no-password"]);
    assert_eq!(code, 0, "{e}");
    let (code, o, _) = run(&["-c", c, "user", "list"]);
    assert_eq!(code, 0);
    assert!(o.contains("(no stored password)"));
    // stdin is not a tty in tests, so no prompt: clear error
    let (code, _, e) = run(&["-c", c, "-s", &m.url(), "login", "--acid", "8"]);
    assert_eq!(code, 2, "{e}");
    assert!(e.contains("no password for 1120240001"), "{e}");
    let (code, _, e) = run_env(
        &["-c", c, "-s", &m.url(), "login", "--acid", "8"],
        &[("SRUN_PASSWORD", PASSWORD)],
    );
    assert_eq!(code, 0, "{e}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn passwords_are_obfuscated_at_rest() {
    let m = MockServer::start();
    let dir = tmp_dir(&m, "obf");
    let cfg = dir.join("config.json");
    let c = cfg.to_str().unwrap();

    let (code, _, e) = run(&["-c", c, "user", "add", "1120240001", "-p", PASSWORD]);
    assert_eq!(code, 0, "{e}");
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(!text.contains(PASSWORD), "plaintext leaked: {text}");
    assert!(text.contains("\"password\": \"obf1:"), "{text}");
    let (code, _, e) = run(&["-c", c, "-s", &m.url(), "login", "--acid", "8"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("login ok user=1120240001"), "{e}");

    // --plain keeps it readable and `user list` says so
    let (code, _, _) = run(&["-c", c, "user", "add", "other", "-p", PASSWORD, "--plain"]);
    assert_eq!(code, 0);
    let (code, o, _) = run(&["-c", c, "user", "list"]);
    assert_eq!(code, 0);
    assert!(
        o.contains("1120240001 username=1120240001 (default)\n"),
        "{o}"
    );
    assert!(
        o.contains("other username=other (plaintext password)\n"),
        "{o}"
    );

    // config obfuscate converts the remaining plaintext entries
    let (code, _, e) = run(&["-c", c, "config", "obfuscate"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("obfuscated 1 password(s)"), "{e}");
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(!text.contains(PASSWORD));
    let (code, _, e) = run(&["-c", c, "config", "obfuscate"]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("nothing to do"), "{e}");

    // switch rewrites the file and must not leak decoded passwords
    let (code, _, e) = run(&["-c", c, "-s", &m.url(), "switch", "other", "--acid", "8"]);
    assert_eq!(code, 0, "{e}");
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(!text.contains(PASSWORD), "switch leaked plaintext: {text}");
    assert!(text.contains("\"default_user\": \"other\""));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn config_path_honours_env_and_lists_search_order() {
    let dir = std::env::temp_dir().join(format!("srun-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("my.json");
    std::fs::write(&cfg, "{}").unwrap();
    let (code, o, _) = run_env(
        &["config", "path"],
        &[("SRUN_CONFIG", cfg.to_str().unwrap())],
    );
    assert_eq!(code, 0);
    assert!(
        o.starts_with(&format!("{} (exists)\nsearch order:\n", cfg.display())),
        "{o}"
    );
    assert!(o.contains("(env SRUN_CONFIG, exists)"), "{o}");
    assert!(o.contains("(next to the executable, missing)"), "{o}");
    // an explicit -c wins over the env
    let (_, o, _) = run_env(
        &["-c", "/nonexistent/x.json", "config", "path"],
        &[("SRUN_CONFIG", cfg.to_str().unwrap())],
    );
    assert_eq!(o, "/nonexistent/x.json (missing)\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn transient_rejection_is_retried_with_backoff() {
    let m = MockServer::start();
    m.state.lock().unwrap().reject_once = Some("E2532".into());
    let url = m.url();
    let (code, _, e) = run(&[
        "-s",
        &url,
        "login",
        "-u",
        "1120240001",
        "-p",
        PASSWORD,
        "--acid",
        "8",
        "--retry-delay",
        "50",
    ]);
    assert_eq!(code, 0, "{e}");
    assert!(e.contains("E2532") && e.contains("retrying in 1s"), "{e}");
    assert!(e.contains("login ok"), "{e}");
    assert_eq!(m.state.lock().unwrap().logins, 2);
    // a permanent rejection is not retried
    m.state.lock().unwrap().online = false;
    m.state.lock().unwrap().reject_once = Some("E2553".into());
    let (code, _, e) = run(&[
        "-s",
        &url,
        "login",
        "-u",
        "1120240001",
        "-p",
        PASSWORD,
        "--acid",
        "8",
    ]);
    assert_eq!(code, 3, "{e}");
    assert_eq!(m.state.lock().unwrap().logins, 3);
}

#[test]
fn switch_restores_previous_account_when_new_login_fails() {
    let m = MockServer::start();
    let cfg = write_config(
        &m,
        "swr",
        &format!(
            r#"{{"name":"a","username":"1120240001","password":"{PASSWORD}"}},{{"name":"b","username":"other","password":"{PASSWORD}"}}"#
        ),
        r#","acid":8,"retry_delay_ms":50,"default_user":"a""#,
    );
    let dir = cfg.parent().unwrap().to_path_buf();
    let c = cfg.to_str().unwrap();
    let (code, _, e) = run(&["-c", c, "login"]);
    assert_eq!(code, 0, "{e}");
    m.state.lock().unwrap().reject_once = Some("E2531".into());
    let (code, _, e) = run(&["-c", c, "switch", "b"]);
    assert_eq!(code, 3, "{e}");
    assert!(e.contains("restoring 1120240001"), "{e}");
    assert!(e.contains("still online as 1120240001"), "{e}");
    let st = m.state.lock().unwrap();
    assert!(st.online);
    assert_eq!(st.online_user, "1120240001");
    drop(st);
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        text.contains("\"default_user\":\"a\""),
        "default must not change on failure"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn status_ip_binds_locally() {
    let m = MockServer::start();
    let url = m.url();
    let (code, o, e) = run(&["-s", &url, "status", "-i", "127.0.0.1"]);
    assert_eq!(code, 0, "{e}");
    assert!(o.starts_with("online: no"), "{o}");
    let (code, _, e) = run(&["-s", &url, "status", "-i", "203.0.113.9"]);
    assert_eq!(code, 4, "{e}");
}

#[cfg(unix)]
fn spawn_daemon(
    cfg: &std::path::Path,
    extra: &[&str],
) -> (std::process::Child, std::sync::mpsc::Receiver<String>) {
    use std::io::{BufRead, BufReader};
    let mut args = vec![
        "-c",
        cfg.to_str().unwrap(),
        "daemon",
        "--interval",
        "5",
        "-v",
    ];
    args.extend_from_slice(extra);
    let mut child = Command::new(env!("CARGO_BIN_EXE_srun"))
        .args(&args)
        .env("HOME", std::env::temp_dir())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
            let _ = tx.send(line);
        }
    });
    (child, rx)
}

#[cfg(unix)]
fn wait_for_line(rx: &std::sync::mpsc::Receiver<String>, needle: &str) -> Vec<String> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        if let Ok(l) = rx.recv_timeout(Duration::from_millis(200)) {
            seen.push(l.clone());
            if l.contains(needle) {
                return seen;
            }
        }
    }
    panic!("timeout waiting for {needle:?}, saw {seen:?}");
}

#[cfg(unix)]
fn stop_daemon(mut child: std::process::Child) {
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

#[cfg(unix)]
fn two_user_config(m: &MockServer, tag: &str) -> PathBuf {
    write_config(
        m,
        tag,
        &format!(
            r#"{{"name":"a","username":"1120240001","password":"{PASSWORD}"}},{{"name":"b","username":"other","password":"{PASSWORD}"}}"#
        ),
        r#","acid":"auto","retry_delay_ms":50,"default_user":"a""#,
    )
}

#[cfg(unix)]
#[test]
fn daemon_redetects_acid_after_failure() {
    let m = MockServer::start();
    m.state.lock().unwrap().acid_fail = true;
    let cfg = two_user_config(&m, "acid");
    let (child, rx) = spawn_daemon(&cfg, &[]);
    let seen = wait_for_line(&rx, "login ok user=1120240001");
    assert!(
        seen.iter().any(|l| l.contains("acid detection failed")),
        "{seen:?}"
    );
    assert_eq!(
        m.state.lock().unwrap().last_login.as_ref().unwrap()["ac_id"],
        "12"
    );
    // WAN "comes up": detection works again and must be used, not a cached 12
    {
        let mut st = m.state.lock().unwrap();
        st.acid_fail = false;
        st.online = false;
    }
    let seen = wait_for_line(&rx, "acid detected: 8");
    assert!(!seen.iter().any(|l| l.contains("user=other")), "{seen:?}");
    wait_for_line(&rx, "login ok user=1120240001");
    assert_eq!(
        m.state.lock().unwrap().last_login.as_ref().unwrap()["ac_id"],
        "8"
    );
    stop_daemon(child);
}

#[cfg(unix)]
#[test]
fn daemon_does_not_switch_account_on_network_error() {
    let m = MockServer::start();
    m.state.lock().unwrap().drop_next_login = true;
    let cfg = two_user_config(&m, "neterr");
    let (child, rx) = spawn_daemon(&cfg, &[]);
    let seen = wait_for_line(&rx, "ending this round");
    assert!(
        seen.iter()
            .any(|l| l.contains("login failed user=1120240001")),
        "{seen:?}"
    );
    assert!(!seen.iter().any(|l| l.contains("user=other")), "{seen:?}");
    {
        let st = m.state.lock().unwrap();
        assert_eq!(st.logins, 1);
        assert_eq!(st.last_login.as_ref().unwrap()["username"], "1120240001");
        assert!(!st.online);
    }
    // next round (after a 2s backoff): the same default user logs in
    let seen = wait_for_line(&rx, "login ok user=1120240001");
    assert!(!seen.iter().any(|l| l.contains("user=other")), "{seen:?}");
    assert_eq!(m.state.lock().unwrap().online_user, "1120240001");
    stop_daemon(child);
}
