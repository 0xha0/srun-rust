//! `Client`: the four portal calls plus ac_id auto-detection.

use crate::error::{Error, Result};
use crate::http::{self, build_query, ConnOpts, Url};
use crate::protocol::{errors, hash, info};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

pub const PATH_CHALLENGE: &str = "/cgi-bin/get_challenge";
pub const PATH_PORTAL: &str = "/cgi-bin/srun_portal";
pub const PATH_USER_INFO: &str = "/cgi-bin/rad_user_info";
pub const DEFAULT_ACID: i64 = 12;
pub const DEFAULT_N: i64 = 200;
pub const DEFAULT_TYPE: i64 = 1;
pub const DEFAULT_OS: &str = "Windows 10";
pub const DEFAULT_NAME: &str = "Windows";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PasswordMode {
    #[default]
    Real,
    Empty,
}

impl std::str::FromStr for PasswordMode {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "real" => Ok(PasswordMode::Real),
            "empty" => Ok(PasswordMode::Empty),
            _ => Err(Error::usage(format!(
                "password mode must be real or empty, got {s}"
            ))),
        }
    }
}

/// The portal sends `ecode` as a number (0) or a string ("E2553").
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Ecode {
    Num(i64),
    Str(String),
}

impl Default for Ecode {
    fn default() -> Self {
        Ecode::Num(0)
    }
}

impl Ecode {
    pub fn as_code(&self) -> String {
        match self {
            Ecode::Num(n) => n.to_string(),
            Ecode::Str(s) => s.clone(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ChallengeResp {
    pub challenge: Option<String>,
    pub client_ip: String,
    pub online_ip: String,
    pub ecode: Ecode,
    pub error: String,
    pub error_msg: String,
    pub res: String,
    pub srun_ver: String,
    pub st: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PortalResp {
    pub access_token: String,
    pub client_ip: String,
    pub online_ip: String,
    pub ecode: Ecode,
    pub error: String,
    pub error_msg: String,
    pub res: String,
    pub suc_msg: String,
    pub ploy_msg: String,
    pub srun_ver: String,
    pub username: String,
    pub real_name: String,
    pub remain_flux: i64,
    pub remain_times: i64,
    pub wallet_balance: f64,
    pub st: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct StatusResp {
    pub error: String,
    pub online_ip: String,
    pub user_name: String,
    pub real_name: String,
    pub products_name: String,
    pub user_mac: String,
    pub domain: String,
    pub group_id: String,
    pub wallet_balance: f64,
    pub user_balance: f64,
    pub user_charge: f64,
    pub sum_bytes: u64,
    pub sum_seconds: u64,
    pub remain_bytes: u64,
    pub remain_seconds: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub all_bytes: u64,
    pub add_time: u64,
    pub keepalive_time: u64,
    pub checkout_date: u64,
    pub sysver: String,
    #[serde(rename = "ServerFlag")]
    pub server_flag: u64,
}

impl StatusResp {
    pub fn is_online(&self) -> bool {
        self.error == "ok"
    }
}

#[derive(Clone, Debug)]
pub struct Client {
    pub server: Url,
    pub opts: ConnOpts,
    pub n: i64,
    pub utype: i64,
    pub os: String,
    pub name: String,
    pub double_stack: bool,
}

#[derive(Clone, Debug)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    /// Empty means "use the address the server sees".
    pub ip: String,
    pub acid: i64,
    pub password_mode: PasswordMode,
}

#[derive(Debug)]
pub enum LoginOutcome {
    LoggedIn {
        ip: String,
        resp: Box<PortalResp>,
    },
    /// The portal refused a second session on this address. `online_as` is
    /// the account it reports online (empty if that lookup failed).
    AlreadyOnline {
        online_as: String,
    },
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Strip the `callback(` ... `)` wrapper, falling back to the raw body.
pub fn strip_jsonp(body: &str) -> &str {
    let trimmed = body.trim();
    match (trimmed.find('('), trimmed.rfind(')')) {
        (Some(a), Some(b)) if b > a => &trimmed[a + 1..b],
        _ => trimmed,
    }
}

/// A portal refusal as an `Error::Rejected`. `ecode` 0 means the code is in
/// `error` (e.g. `not_online_error`).
fn reject(ecode: &Ecode, error: &str, error_msg: &str) -> Error {
    let ecode = ecode.as_code();
    let code = if ecode == "0" || ecode.is_empty() {
        error
    } else {
        ecode.as_str()
    };
    Error::rejected(code, errors::explain(&ecode, error, error_msg))
}

impl Client {
    pub fn new(server: Url) -> Self {
        Client {
            server,
            opts: ConnOpts::default(),
            n: DEFAULT_N,
            utype: DEFAULT_TYPE,
            os: DEFAULT_OS.to_string(),
            name: DEFAULT_NAME.to_string(),
            double_stack: false,
        }
    }

    fn call<T: DeserializeOwned>(&self, path: &str, params: &[(&str, &str)]) -> Result<T> {
        let ts = now_secs();
        let callback = format!("jsonp{ts}");
        let millis = format!("{}", ts * 1000);
        let mut all: Vec<(&str, &str)> = vec![("callback", callback.as_str())];
        all.extend_from_slice(params);
        all.push(("_", millis.as_str()));
        let url = self
            .server
            .with_path(&format!("{path}?{}", build_query(&all)));
        if crate::log::enabled(crate::log::Level::Trace) {
            let shown: Vec<(&str, &str)> = all
                .iter()
                .map(|(k, v)| match *k {
                    "password" | "info" | "chksum" => (*k, "<redacted>"),
                    _ => (*k, *v),
                })
                .collect();
            crate::log_trace!("GET {}{path}?{}", self.server.origin(), build_query(&shown));
        }
        let resp = http::get(&url, &self.opts)?;
        if resp.status != 200 {
            return Err(Error::network(format!(
                "{path}: http status {}",
                resp.status
            )));
        }
        let text = resp.text();
        let json = strip_jsonp(&text);
        crate::log_trace!("{path} -> {json}");
        serde_json::from_str(json).map_err(|e| {
            Error::network(format!(
                "{path}: bad json ({e}): {}",
                crate::text::ascii(json)
            ))
        })
    }

    /// Follow the portal's redirect chain from `/` and read `ac_id` from the final URL.
    pub fn detect_acid(&self) -> Result<i64> {
        let mut url = self.server.with_path("/");
        for _ in 0..4 {
            if let Some(v) = url.query_param("ac_id") {
                return v
                    .parse::<i64>()
                    .map_err(|_| Error::network(format!("ac_id is not a number: {v}")));
            }
            let resp = http::get(&url, &self.opts)?;
            match (resp.status, resp.header("location")) {
                (301..=303 | 307 | 308, Some(loc)) => {
                    crate::log_debug!("redirect -> {loc}");
                    url = url.join(loc)?;
                }
                (status, _) => {
                    return Err(Error::network(format!(
                        "ac_id detection stopped at {url} (http {status})"
                    )))
                }
            }
        }
        Err(Error::network("ac_id detection: too many redirects"))
    }

    pub fn challenge(&self, username: &str, ip: &str) -> Result<ChallengeResp> {
        let c: ChallengeResp = self.call(PATH_CHALLENGE, &[("username", username), ("ip", ip)])?;
        if c.error != "ok" || c.challenge.is_none() {
            return Err(reject(&c.ecode, &c.error, &c.error_msg));
        }
        Ok(c)
    }

    pub fn login(&self, req: &LoginRequest) -> Result<LoginOutcome> {
        let ch = self.challenge(&req.username, &req.ip)?;
        let token = ch.challenge.clone().unwrap_or_default();
        let ip = if req.ip.is_empty() {
            ch.client_ip.clone()
        } else {
            req.ip.clone()
        };
        if ip.is_empty() {
            return Err(Error::network("server did not report a client ip"));
        }
        crate::log_debug!("challenge ok ip={ip} srun_ver={}", ch.srun_ver);
        let hmd5_input = match req.password_mode {
            PasswordMode::Real => req.password.as_str(),
            PasswordMode::Empty => "",
        };
        let hmd5 = hash::hmd5(hmd5_input, &token);
        let acid = req.acid.to_string();
        let info = info(&req.username, &req.password, &ip, &acid, &token);
        let n = self.n.to_string();
        let utype = self.utype.to_string();
        let chksum = hash::chksum(&token, &req.username, &hmd5, &acid, &ip, &n, &utype, &info);
        let password = format!("{{MD5}}{hmd5}");
        let double_stack = if self.double_stack { "1" } else { "0" };
        let resp: PortalResp = self.call(
            PATH_PORTAL,
            &[
                ("action", "login"),
                ("username", &req.username),
                ("password", &password),
                ("ac_id", &acid),
                ("ip", &ip),
                ("chksum", &chksum),
                ("info", &info),
                ("n", &n),
                ("type", &utype),
                ("os", &self.os),
                ("name", &self.name),
                ("double_stack", double_stack),
            ],
        )?;
        let already_online =
            resp.suc_msg == "ip_already_online_error" || resp.ecode.as_code() == "E2620";
        if already_online {
            // Not a failure: this address has a session. Report whose.
            let online_as = self.status().map(|st| st.user_name).unwrap_or_default();
            return Ok(LoginOutcome::AlreadyOnline { online_as });
        }
        if resp.res == "ok" && resp.error == "ok" {
            return Ok(LoginOutcome::LoggedIn {
                ip,
                resp: Box::new(resp),
            });
        }
        Err(reject(&resp.ecode, &resp.error, &resp.error_msg))
    }

    pub fn logout(&self, username: &str, ip: &str, acid: i64) -> Result<PortalResp> {
        let acid = acid.to_string();
        let resp: PortalResp = self.call(
            PATH_PORTAL,
            &[
                ("action", "logout"),
                ("username", username),
                ("ac_id", &acid),
                ("ip", ip),
            ],
        )?;
        if resp.error == "ok" || resp.error == "logout_ok" {
            return Ok(resp);
        }
        Err(reject(&resp.ecode, &resp.error, &resp.error_msg))
    }

    pub fn status(&self) -> Result<StatusResp> {
        self.call(PATH_USER_INFO, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONLINE: &str = r#"jsonp1({"ServerFlag":4294967040,"add_time":1790604688,"all_bytes":12207641799,"bytes_in":11916888389,"bytes_out":3062795247,"checkout_date":0,"domain":"after-auth","error":"ok","group_id":"7","keepalive_time":1790663295,"online_ip":"10.52.188.121","products_name":"学生-10元含300GB不限速流量-202311","real_name":"","remain_bytes":84199644785,"remain_seconds":0,"sum_bytes":225715260616,"sum_seconds":6824184,"sysver":"1.01.20220802","user_balance":10,"user_charge":0,"user_mac":"00:11:22:33:44:55","user_name":"1120240001","wallet_balance":0})"#;

    #[test]
    fn parses_real_status_sample() {
        let s: StatusResp = serde_json::from_str(strip_jsonp(ONLINE)).unwrap();
        assert!(s.is_online());
        assert_eq!(s.server_flag, 4294967040);
        assert_eq!(s.sum_bytes, 225715260616);
        assert_eq!(s.user_balance, 10.0);
        assert!(s.products_name.contains("300GB"));
    }

    #[test]
    fn ecode_untagged() {
        let a: ChallengeResp =
            serde_json::from_str(r#"{"ecode":0,"error":"ok","challenge":"t"}"#).unwrap();
        assert_eq!(a.ecode.as_code(), "0");
        let b: PortalResp =
            serde_json::from_str(r#"{"ecode":"E2553","error":"login_error"}"#).unwrap();
        assert_eq!(b.ecode.as_code(), "E2553");
    }

    #[test]
    fn jsonp_stripping() {
        assert_eq!(strip_jsonp("jsonp123({\"a\":1})"), "{\"a\":1}");
        assert_eq!(strip_jsonp("{\"a\":1}"), "{\"a\":1}");
        assert_eq!(strip_jsonp("  sdu({\"x\":\"(y)\"})\n"), "{\"x\":\"(y)\"}");
    }
}
