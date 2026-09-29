//! Where the config file lives. Priority:
//! 1. `-c PATH`
//! 2. `$SRUN_CONFIG`
//! 3. `config.json` next to the executable (portable mode; only if it exists)
//! 4. the path baked in at build time with `SRUN_DEFAULT_CONFIG` (if any)
//! 5. platform defaults: `$XDG_CONFIG_HOME/srun/config.json` or
//!    `~/.config/srun/config.json`, then `/etc/srun/config.json`
//!    (`%APPDATA%\srun\config.json` on Windows)

use std::path::PathBuf;

pub const ENV_CONFIG: &str = "SRUN_CONFIG";
pub const FILE_NAME: &str = "config.json";
pub const BUILT_IN_DEFAULT: Option<&str> = option_env!("SRUN_DEFAULT_CONFIG");

#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub source: &'static str,
}

fn exe_sibling() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    Some(dir.join(FILE_NAME))
}

/// Candidate locations in priority order, whether or not they exist.
pub fn candidates() -> Vec<Candidate> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var(ENV_CONFIG) {
        if !p.is_empty() {
            v.push(Candidate {
                path: PathBuf::from(p),
                source: "env SRUN_CONFIG",
            });
        }
    }
    if let Some(p) = exe_sibling() {
        v.push(Candidate {
            path: p,
            source: "next to the executable",
        });
    }
    if let Some(p) = BUILT_IN_DEFAULT {
        v.push(Candidate {
            path: PathBuf::from(p),
            source: "built-in default",
        });
    }
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            v.push(Candidate {
                path: PathBuf::from(appdata).join("srun").join(FILE_NAME),
                source: "user config dir",
            });
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
            if !x.is_empty() {
                v.push(Candidate {
                    path: PathBuf::from(x).join("srun").join(FILE_NAME),
                    source: "user config dir",
                });
            }
        } else if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                v.push(Candidate {
                    path: PathBuf::from(home)
                        .join(".config")
                        .join("srun")
                        .join(FILE_NAME),
                    source: "user config dir",
                });
            }
        }
        v.push(Candidate {
            path: PathBuf::from("/etc/srun").join(FILE_NAME),
            source: "system config dir",
        });
    }
    v
}

fn fallback_for_write(c: &[Candidate]) -> PathBuf {
    // Never create files next to the executable or under /etc by default.
    c.iter()
        .find(|k| !matches!(k.source, "next to the executable" | "system config dir"))
        .or_else(|| c.first())
        .map(|k| k.path.clone())
        .unwrap_or_else(|| PathBuf::from(FILE_NAME))
}

/// The file to read: explicit path, else the first existing candidate, else
/// where a new one would be written (so `config path` is meaningful).
pub fn resolve(explicit: Option<&str>) -> PathBuf {
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }
    let c = candidates();
    c.iter()
        .find(|k| k.path.exists())
        .map(|k| k.path.clone())
        .unwrap_or_else(|| fallback_for_write(&c))
}

/// Where a new file should be written when none exists.
pub fn resolve_for_write(explicit: Option<&str>) -> PathBuf {
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }
    let c = candidates();
    c.iter()
        .find(|k| k.path.exists())
        .map(|k| k.path.clone())
        .unwrap_or_else(|| fallback_for_write(&c))
}
