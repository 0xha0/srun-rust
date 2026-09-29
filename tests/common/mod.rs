//! A fake srun portal on 127.0.0.1 used by the integration tests. It checks
//! the login signature the same way the real server does, so wiring bugs in
//! chksum/info/password show up here before touching a real network.

#![allow(dead_code)]

use srun::http::url::percent_decode;
use srun::protocol::{hash, info};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

pub const TOKEN: &str = "cfe8aa21bef38ed80e2c1a8e06774a709fd65947f75a0513a35648a4ff6b4f09";
pub const CLIENT_IP: &str = "10.9.8.7";
pub const PASSWORD: &str = "s3cret pass";

pub const ONLINE_JSON: &str = r#"{"ServerFlag":4294967040,"add_time":1790604688,"all_bytes":12207641799,"bytes_in":11916888389,"bytes_out":3062795247,"checkout_date":0,"domain":"after-auth","error":"ok","group_id":"7","keepalive_time":1790663295,"online_ip":"10.9.8.7","products_name":"学生-10元含300GB不限速流量-202311","real_name":"","remain_bytes":84199644785,"remain_seconds":0,"sum_bytes":225715260616,"sum_seconds":6824184,"sysver":"1.01.20220802","user_balance":10,"user_charge":0,"user_mac":"00:11:22:33:44:55","user_name":"1120240001","wallet_balance":0}"#;

#[derive(Default)]
pub struct State {
    pub online: bool,
    pub acid: i64,
    pub accept_empty_password: bool,
    pub chunked: bool,
    pub last_login: Option<HashMap<String, String>>,
    pub last_logout: Option<HashMap<String, String>>,
    pub logins: u32,
    pub online_user: String,
    /// Reject the next login with this ecode, once.
    pub reject_once: Option<String>,
}

pub struct MockServer {
    pub port: u16,
    pub state: Arc<Mutex<State>>,
}

impl MockServer {
    pub fn start() -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State {
            acid: 8,
            ..Default::default()
        }));
        let st = state.clone();
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { break };
                let st = st.clone();
                thread::spawn(move || handle(conn, st, port));
            }
        });
        MockServer { port, state }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn client(&self) -> srun::protocol::Client {
        srun::protocol::Client::new(srun::http::Url::parse(&self.url()).unwrap())
    }
}

fn parse_query(path: &str) -> (String, HashMap<String, String>) {
    let (p, q) = path.split_once('?').unwrap_or((path, ""));
    let mut map = HashMap::new();
    for pair in q.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(percent_decode(k), percent_decode(v));
    }
    (p.to_string(), map)
}

fn respond(
    conn: &mut TcpStream,
    status: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    chunked: bool,
) {
    let mut out = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    if chunked {
        out.push_str("Transfer-Encoding: chunked\r\n\r\n");
        let _ = conn.write_all(out.as_bytes());
        for chunk in body.chunks(7) {
            let _ = conn.write_all(format!("{:x}\r\n", chunk.len()).as_bytes());
            let _ = conn.write_all(chunk);
            let _ = conn.write_all(b"\r\n");
        }
        let _ = conn.write_all(b"0\r\n\r\n");
    } else {
        out.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        let _ = conn.write_all(out.as_bytes());
        let _ = conn.write_all(body);
    }
    let _ = conn.flush();
}

fn jsonp(conn: &mut TcpStream, q: &HashMap<String, String>, json: &str, chunked: bool) {
    let cb = q.get("callback").cloned().unwrap_or_default();
    let body = format!("{cb}({json})");
    respond(
        conn,
        "200 OK",
        &[("Content-Type", "text/javascript")],
        body.as_bytes(),
        chunked,
    );
}

fn handle(mut conn: TcpStream, st: Arc<Mutex<State>>, port: u16) {
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let _method = parts.next();
    let target = parts.next().unwrap_or("/").to_string();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
            break;
        }
    }
    let (path, q) = parse_query(&target);
    let mut s = st.lock().unwrap();
    match path.as_str() {
        "/" => respond(
            &mut conn,
            "302 Found",
            &[("Location", "/index_1.html")],
            b"",
            false,
        ),
        "/index_1.html" => {
            let loc = format!(
                "http://127.0.0.1:{port}/srun_portal_pc?ac_id={}&theme=bit",
                s.acid
            );
            respond(&mut conn, "302 Found", &[("Location", &loc)], b"", false)
        }
        "/srun_portal_pc" => respond(&mut conn, "200 OK", &[], b"<html>portal</html>", false),
        "/cgi-bin/get_challenge" => {
            let username = q.get("username").cloned().unwrap_or_default();
            if username.is_empty() {
                jsonp(
                    &mut conn,
                    &q,
                    r#"{"challenge":"","client_ip":"","ecode":"E5991","error":"invalid_username","error_msg":"","res":"failed","st":1}"#,
                    false,
                );
            } else {
                let json = format!(
                    r#"{{"challenge":"{TOKEN}","client_ip":"{CLIENT_IP}","ecode":0,"error":"ok","error_msg":"","expire":"180","online_ip":"{CLIENT_IP}","res":"ok","srun_ver":"MockSrun V1","st":1790663295}}"#
                );
                jsonp(&mut conn, &q, &json, false);
            }
        }
        "/cgi-bin/srun_portal" => match q.get("action").map(|s| s.as_str()) {
            Some("login") => {
                s.logins += 1;
                s.last_login = Some(q.clone());
                let g = |k: &str| q.get(k).cloned().unwrap_or_default();
                let (username, ip, acid) = (g("username"), g("ip"), g("ac_id"));
                let real = hash::hmd5(PASSWORD, TOKEN);
                let empty = hash::hmd5("", TOKEN);
                let sent = g("password");
                let hmd5 = sent.strip_prefix("{MD5}").unwrap_or("").to_string();
                let pw_ok = hmd5 == real || (s.accept_empty_password && hmd5 == empty);
                let expected_info = info(&username, PASSWORD, &ip, &acid, TOKEN);
                let expected_sum = hash::chksum(
                    TOKEN,
                    &username,
                    &hmd5,
                    &acid,
                    &ip,
                    &g("n"),
                    &g("type"),
                    &g("info"),
                );
                let json = if let Some(code) = s.reject_once.take() {
                    format!(
                        r#"{{"ecode":"{code}","error":"login_error","error_msg":"","res":"login_error","st":1}}"#
                    )
                } else if username == "nobody" {
                    r#"{"ecode":"E2531","error":"login_error","error_msg":"","res":"login_error","st":1}"#.to_string()
                } else if ip.is_empty() || acid.is_empty() {
                    r#"{"ecode":"E5991","error":"login_error","error_msg":"missing ip or ac_id","res":"login_error"}"#.to_string()
                } else if g("chksum") != expected_sum {
                    r#"{"ecode":"E5991","error":"login_error","error_msg":"bad checksum","res":"login_error"}"#.to_string()
                } else if !pw_ok {
                    r#"{"ecode":"E2553","error":"login_error","error_msg":"CHALLENGE failed, BAS respond timeout.","res":"login_error","st":1}"#.to_string()
                } else if g("info") != expected_info {
                    r#"{"ecode":"E5991","error":"login_error","error_msg":"bad info","res":"login_error"}"#.to_string()
                } else if s.online {
                    format!(
                        r#"{{"ServerFlag":0,"access_token":"{TOKEN}","client_ip":"{CLIENT_IP}","ecode":0,"error":"ok","error_msg":"","online_ip":"{CLIENT_IP}","res":"ok","suc_msg":"ip_already_online_error","st":1}}"#
                    )
                } else {
                    s.online = true;
                    s.online_user = username.clone();
                    format!(
                        r#"{{"ServerFlag":0,"access_token":"{TOKEN}","checkout_date":0,"client_ip":"{CLIENT_IP}","ecode":0,"error":"ok","error_msg":"","online_ip":"{CLIENT_IP}","real_name":"","remain_flux":0,"remain_times":0,"res":"ok","srun_ver":"MockSrun V1","suc_msg":"login_ok","sysver":"1.0","username":"{username}","wallet_balance":0,"st":1}}"#
                    )
                };
                jsonp(&mut conn, &q, &json, false);
            }
            Some("logout") => {
                s.last_logout = Some(q.clone());
                let json = if s.online {
                    s.online = false;
                    r#"{"client_ip":"10.9.8.7","ecode":0,"error":"ok","error_msg":"","online_ip":"10.9.8.7","res":"ok","srun_ver":"MockSrun V1","st":1}"#
                } else {
                    r#"{"client_ip":"10.9.8.7","ecode":0,"error":"not_online_error","error_msg":"","online_ip":"10.9.8.7","res":"not_online_error","srun_ver":"MockSrun V1","st":1}"#
                };
                jsonp(&mut conn, &q, json, false);
            }
            _ => respond(&mut conn, "400 Bad Request", &[], b"", false),
        },
        "/cgi-bin/rad_user_info" => {
            let chunked = s.chunked;
            if s.online {
                let json = ONLINE_JSON.replace(
                    "\"user_name\":\"1120240001\"",
                    &format!("\"user_name\":\"{}\"", s.online_user),
                );
                jsonp(&mut conn, &q, &json, chunked);
            } else {
                jsonp(
                    &mut conn,
                    &q,
                    r#"{"client_ip":"10.9.8.7","ecode":0,"error":"not_online_error","error_msg":"","online_ip":"10.9.8.7","res":"not_online_error","srun_ver":"MockSrun V1","st":1}"#,
                    chunked,
                );
            }
        }
        _ => respond(&mut conn, "404 Not Found", &[], b"", false),
    }
    drop(s);
    let _ = conn.read(&mut [0u8; 1]);
}
