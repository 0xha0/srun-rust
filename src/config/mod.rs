//! JSON config file: global portal parameters plus a list of users.

pub mod paths;

use crate::error::{Error, Result};
use crate::protocol::PasswordMode;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_SERVER: &str = "http://10.0.0.55";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Acid {
    Fixed(i64),
    Auto(String),
}

impl Default for Acid {
    fn default() -> Self {
        Acid::Auto("auto".to_string())
    }
}

impl Acid {
    pub fn parse(s: &str) -> Result<Acid> {
        if s == "auto" {
            return Ok(Acid::default());
        }
        s.parse::<i64>()
            .map(Acid::Fixed)
            .map_err(|_| Error::usage(format!("acid must be a number or 'auto', got {s}")))
    }

    pub fn fixed(&self) -> Option<i64> {
        match self {
            Acid::Fixed(n) => Some(*n),
            Acid::Auto(_) => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    pub interval: u64,
    /// `server`, `none`, or `HOST:PORT`.
    pub probe: String,
    pub logout_on_exit: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        DaemonConfig {
            interval: 60,
            probe: "server".to_string(),
            logout_on_exit: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct User {
    pub name: String,
    pub username: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ifname: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: String,
    pub acid: Acid,
    pub password_mode: PasswordMode,
    pub retry: u32,
    pub retry_delay_ms: u64,
    pub strict_bind: bool,
    pub double_stack: bool,
    pub os: String,
    pub name: String,
    pub n: i64,
    #[serde(rename = "type")]
    pub utype: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_user: Option<String>,
    pub daemon: DaemonConfig,
    pub users: Vec<User>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server: DEFAULT_SERVER.to_string(),
            acid: Acid::default(),
            password_mode: PasswordMode::Real,
            retry: 3,
            retry_delay_ms: 1000,
            strict_bind: false,
            double_stack: false,
            os: "Windows 10".to_string(),
            name: "Windows".to_string(),
            n: 200,
            utype: 1,
            default_user: None,
            daemon: DaemonConfig::default(),
            users: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| Error::config(format!("read {}: {e}", path.display())))?;
        serde_json::from_str(&raw)
            .map_err(|e| Error::config(format!("parse {}: {e}", path.display())))
    }

    /// Load the file if it exists, otherwise defaults.
    pub fn load_or_default(path: &Path) -> Result<Config> {
        if path.exists() {
            Config::load(path)
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() && !dir.exists() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| Error::config(format!("create {}: {e}", dir.display())))?;
            }
        }
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::internal(format!("serialize config: {e}")))?;
        text.push('\n');
        // Write next to the target and rename, so a crash or a full disk
        // never leaves a truncated config behind.
        let tmp = path.with_extension("json.tmp");
        write_private(&tmp, text.as_bytes())
            .map_err(|e| Error::config(format!("write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            Error::config(format!("replace {}: {e}", path.display()))
        })?;
        Ok(())
    }

    /// Match by alias first, then by portal username.
    pub fn find_user(&self, name: &str) -> Option<&User> {
        self.users
            .iter()
            .find(|u| u.name == name)
            .or_else(|| self.users.iter().find(|u| u.username == name))
    }

    pub fn find_user_mut(&mut self, name: &str) -> Option<&mut User> {
        let idx = self
            .users
            .iter()
            .position(|u| u.name == name)
            .or_else(|| self.users.iter().position(|u| u.username == name))?;
        self.users.get_mut(idx)
    }

    /// Default user first, then the others in file order.
    pub fn users_default_first(&self) -> Vec<User> {
        let mut v = Vec::with_capacity(self.users.len());
        if let Some(d) = self.default_user() {
            v.push(d.clone());
        }
        for u in &self.users {
            if !v.iter().any(|x| x.name == u.name) {
                v.push(u.clone());
            }
        }
        v
    }

    /// The user chosen by `default_user`, else the only user, else none.
    pub fn default_user(&self) -> Option<&User> {
        if let Some(n) = &self.default_user {
            return self.find_user(n);
        }
        if self.users.len() == 1 {
            return self.users.first();
        }
        None
    }
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(())
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(not(unix))]
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

pub fn path_for_read(explicit: Option<&str>) -> PathBuf {
    paths::resolve(explicit)
}

pub fn path_for_write(explicit: Option<&str>) -> PathBuf {
    paths::resolve_for_write(explicit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let raw = r#"{"server":"http://1.2.3.4","acid":"auto","users":[{"name":"a","username":"u","password":"p","ifname":"eth0"}],"default_user":"a"}"#;
        let c: Config = serde_json::from_str(raw).unwrap();
        assert_eq!(c.server, "http://1.2.3.4");
        assert_eq!(c.acid, Acid::default());
        assert_eq!(c.retry, 3);
        assert_eq!(c.password_mode, PasswordMode::Real);
        assert_eq!(c.default_user().unwrap().ifname.as_deref(), Some("eth0"));
        let text = serde_json::to_string_pretty(&c).unwrap();
        assert!(text.contains("\"acid\": \"auto\""));
        assert!(!text.contains("\"ip\""), "None fields are skipped");
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.users, c.users);
    }

    #[test]
    fn acid_variants() {
        let c: Config = serde_json::from_str(r#"{"acid":8}"#).unwrap();
        assert_eq!(c.acid.fixed(), Some(8));
        assert_eq!(Acid::parse("auto").unwrap().fixed(), None);
        assert_eq!(Acid::parse("12").unwrap().fixed(), Some(12));
        assert!(Acid::parse("x").is_err());
    }

    #[test]
    fn save_and_load_tmp() {
        let dir = std::env::temp_dir().join(format!("srun-test-{}", std::process::id()));
        let path = dir.join("sub").join("config.json");
        let mut c = Config::default();
        c.users.push(User {
            name: "x".into(),
            username: "u".into(),
            password: "p".into(),
            ip: Some("1.1.1.1".into()),
            ifname: None,
        });
        c.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert_eq!(back.users[0].ip.as_deref(), Some("1.1.1.1"));
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
