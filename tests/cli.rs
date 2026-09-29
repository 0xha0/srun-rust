//! End-to-end: the real binary against the mock portal, driven by a temp config.

mod common;

use common::{MockServer, CLIENT_IP, PASSWORD};
use std::path::PathBuf;
use std::process::Command;

fn tmp_config(m: &MockServer, extra: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("srun-cli-{}-{}", std::process::id(), m.port));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    let json = format!(
        r#"{{"server":"{}","users":[{{"name":"me","username":"1120240001","password":"{}"}}]{}}}"#,
        m.url(),
        PASSWORD,
        extra
    );
    std::fs::write(&path, json).unwrap();
    path
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_srun"))
        .args(args)
        .env_remove("SRUN_CONFIG")
        .env("HOME", std::env::temp_dir())
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
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
    let out = Command::new(env!("CARGO_BIN_EXE_srun"))
        .args(["-c", c, "status"])
        .env("SRUN_ASCII", "1")
        .env("HOME", std::env::temp_dir())
        .output()
        .unwrap();
    assert_ascii(&String::from_utf8_lossy(&out.stdout));

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

    let out = Command::new(env!("CARGO_BIN_EXE_srun"))
        .args(["-s", &url, "login", "-u", "1120240001", "--acid", "8", "-q"])
        .env("SRUN_PASSWORD", PASSWORD)
        .env("HOME", std::env::temp_dir())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stderr.is_empty(),
        "quiet mode prints nothing on success"
    );
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
    let dir = std::env::temp_dir().join(format!("srun-mgmt-{}-{}", std::process::id(), m.port));
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
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};
    let m = MockServer::start();
    let cfg = tmp_config(&m, r#","daemon":{"interval":5}"#);
    let mut child = Command::new(env!("CARGO_BIN_EXE_srun"))
        .args([
            "-c",
            cfg.to_str().unwrap(),
            "daemon",
            "--interval",
            "5",
            "--acid",
            "8",
            "-v",
        ])
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
    let wait_for = |needle: &str| -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(20);
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
    };
    wait_for("login ok user=1120240001");
    assert_eq!(m.state.lock().unwrap().logins, 1);
    // simulate a kick: server now says offline
    m.state.lock().unwrap().online = false;
    wait_for("login ok user=1120240001");
    assert_eq!(m.state.lock().unwrap().logins, 2);
    // SIGTERM -> clean exit 0 within a slice
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let start = Instant::now();
    let status = child.wait().unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "stop took {:?}",
        start.elapsed()
    );
    assert_eq!(status.code(), Some(0));
    wait_for("daemon stopped");
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
    let dir = std::env::temp_dir().join(format!("srun-fb-{}-{}", std::process::id(), m.port));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.json");
    std::fs::write(
        &cfg,
        format!(
            r#"{{"server":"{}","acid":8,"default_user":"bad","users":[{{"name":"ok","username":"1120240001","password":"{}"}},{{"name":"bad","username":"nobody","password":"x"}}]}}"#,
            m.url(),
            PASSWORD
        ),
    )
    .unwrap();
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
    let dir = std::env::temp_dir().join(format!("srun-sw-{}-{}", std::process::id(), m.port));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.json");
    std::fs::write(
        &cfg,
        format!(
            r#"{{"server":"{}","acid":8,"default_user":"a","users":[{{"name":"a","username":"1120240001","password":"{}"}},{{"name":"b","username":"other","password":"{}"}}]}}"#,
            m.url(),
            PASSWORD,
            PASSWORD
        ),
    )
    .unwrap();
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
    let dir = std::env::temp_dir().join(format!("srun-np-{}-{}", std::process::id(), m.port));
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
    let out = Command::new(env!("CARGO_BIN_EXE_srun"))
        .args(["-c", c, "-s", &m.url(), "login", "--acid", "8"])
        .env("SRUN_PASSWORD", PASSWORD)
        .env("HOME", std::env::temp_dir())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn passwords_are_obfuscated_at_rest() {
    let m = MockServer::start();
    let dir = std::env::temp_dir().join(format!("srun-obf-{}-{}", std::process::id(), m.port));
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
    let out = Command::new(env!("CARGO_BIN_EXE_srun"))
        .args(["config", "path"])
        .env("SRUN_CONFIG", &cfg)
        .env("HOME", std::env::temp_dir())
        .output()
        .unwrap();
    let o = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        o.starts_with(&format!("{} (exists)\nsearch order:\n", cfg.display())),
        "{o}"
    );
    assert!(o.contains("(env SRUN_CONFIG, exists)"), "{o}");
    assert!(o.contains("(next to the executable, missing)"), "{o}");
    // an explicit -c wins over the env
    let out = Command::new(env!("CARGO_BIN_EXE_srun"))
        .args(["-c", "/nonexistent/x.json", "config", "path"])
        .env("SRUN_CONFIG", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "/nonexistent/x.json (missing)\n"
    );
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
    let dir = std::env::temp_dir().join(format!("srun-swr-{}-{}", std::process::id(), m.port));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.json");
    std::fs::write(
        &cfg,
        format!(
            r#"{{"server":"{}","acid":8,"retry_delay_ms":50,"default_user":"a","users":[{{"name":"a","username":"1120240001","password":"{}"}},{{"name":"b","username":"other","password":"{}"}}]}}"#,
            m.url(),
            PASSWORD,
            PASSWORD
        ),
    )
    .unwrap();
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
