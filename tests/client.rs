mod common;

use common::{MockServer, CLIENT_IP, PASSWORD};
use srun::protocol::{LoginOutcome, LoginRequest, PasswordMode};
use srun::Error;

fn req(ip: &str, mode: PasswordMode) -> LoginRequest {
    LoginRequest {
        username: "1120240001".into(),
        password: PASSWORD.into(),
        ip: ip.into(),
        acid: 8,
        password_mode: mode,
    }
}

#[test]
fn detects_acid_through_redirect_chain() {
    let m = MockServer::start();
    assert_eq!(m.client().detect_acid().unwrap(), 8);
    m.state.lock().unwrap().acid = 21;
    assert_eq!(m.client().detect_acid().unwrap(), 21);
}

#[test]
fn login_with_detected_ip_and_official_params() {
    let m = MockServer::start();
    let LoginOutcome::LoggedIn { ip, resp } =
        m.client().login(&req("", PasswordMode::Real)).unwrap()
    else {
        panic!("expected a fresh login")
    };
    assert_eq!(ip, CLIENT_IP);
    assert_eq!(resp.suc_msg, "login_ok");
    let st = m.state.lock().unwrap();
    let p = st.last_login.as_ref().unwrap();
    assert_eq!(p["n"], "200");
    assert_eq!(p["type"], "1");
    assert_eq!(p["os"], "Windows 10");
    assert_eq!(p["name"], "Windows");
    assert_eq!(p["double_stack"], "0");
    assert_eq!(p["ac_id"], "8");
    assert!(p["info"].starts_with("{SRBX1}"));
    assert!(p["callback"].starts_with("jsonp"));
}

#[test]
fn login_with_explicit_ip_and_double_stack() {
    let m = MockServer::start();
    let mut c = m.client();
    c.double_stack = true;
    let LoginOutcome::LoggedIn { ip, .. } = c.login(&req("10.1.2.3", PasswordMode::Real)).unwrap()
    else {
        panic!("expected a fresh login")
    };
    assert_eq!(ip, "10.1.2.3");
    let st = m.state.lock().unwrap();
    let p = st.last_login.as_ref().unwrap();
    assert_eq!(p["ip"], "10.1.2.3");
    assert_eq!(p["double_stack"], "1");
}

#[test]
fn wrong_password_is_rejected_with_code() {
    let m = MockServer::start();
    let mut r = req("", PasswordMode::Real);
    r.password = "nope".into();
    let err = m.client().login(&r).unwrap_err();
    match &err {
        Error::Rejected { code, message } => {
            assert_eq!(code, "E2553");
            assert!(message.contains("wrong password"), "{message}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(err.exit_code(), 3);
}

#[test]
fn empty_password_mode_only_works_when_server_allows() {
    let m = MockServer::start();
    assert!(m.client().login(&req("", PasswordMode::Empty)).is_err());
    m.state.lock().unwrap().accept_empty_password = true;
    m.client().login(&req("", PasswordMode::Empty)).unwrap();
}

#[test]
fn second_login_reports_already_online() {
    let m = MockServer::start();
    m.client().login(&req("", PasswordMode::Real)).unwrap();
    match m.client().login(&req("", PasswordMode::Real)).unwrap() {
        LoginOutcome::AlreadyOnline { online_as } => assert_eq!(online_as, "1120240001"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn status_offline_online_and_chunked() {
    let m = MockServer::start();
    let c = m.client();
    assert!(!c.status().unwrap().is_online());
    c.login(&req("", PasswordMode::Real)).unwrap();
    m.state.lock().unwrap().chunked = true;
    let s = c.status().unwrap();
    assert!(s.is_online());
    assert_eq!(s.server_flag, 4294967040);
    assert_eq!(s.sum_bytes, 225715260616);
    assert_eq!(s.user_name, "1120240001");
    assert!(!s.products_name.is_ascii());
}

#[test]
fn logout_then_logout_again() {
    let m = MockServer::start();
    let c = m.client();
    c.login(&req("", PasswordMode::Real)).unwrap();
    c.logout("1120240001", CLIENT_IP, 8).unwrap();
    let err = c.logout("1120240001", CLIENT_IP, 8).unwrap_err();
    match err {
        Error::Rejected { code, .. } => assert_eq!(code, "not_online_error"),
        other => panic!("unexpected {other:?}"),
    }
    let st = m.state.lock().unwrap();
    let p = st.last_logout.as_ref().unwrap();
    assert_eq!(p["action"], "logout");
    assert_eq!(p["ac_id"], "8");
}

#[test]
fn challenge_failure_is_rejected() {
    let m = MockServer::start();
    let err = m.client().challenge("", "").unwrap_err();
    assert!(matches!(err, Error::Rejected { .. }));
}

#[test]
fn unreachable_server_is_network_error() {
    let mut c = srun::protocol::Client::new(srun::http::Url::parse("http://127.0.0.1:1").unwrap());
    c.opts.connect_timeout = std::time::Duration::from_millis(300);
    let err = c.status().unwrap_err();
    assert!(matches!(err, Error::Network(_)));
    assert_eq!(err.exit_code(), 4);
}
