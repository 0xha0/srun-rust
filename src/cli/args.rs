//! Hand-written argument parser: `-x`, `-x VALUE`, `-vv`, `--long`,
//! `--long VALUE`, `--long=VALUE`, `--` terminator. No external crates.

use crate::error::{Error, Result};

#[derive(Clone, Copy)]
pub struct OptSpec {
    pub short: Option<char>,
    pub long: &'static str,
    pub takes_value: bool,
    pub meta: &'static str,
    pub help: &'static str,
}

impl OptSpec {
    pub const fn flag(short: Option<char>, long: &'static str, help: &'static str) -> Self {
        OptSpec {
            short,
            long,
            takes_value: false,
            meta: "",
            help,
        }
    }

    pub const fn value(
        short: Option<char>,
        long: &'static str,
        meta: &'static str,
        help: &'static str,
    ) -> Self {
        OptSpec {
            short,
            long,
            takes_value: true,
            meta,
            help,
        }
    }
}

#[derive(Default, Debug)]
pub struct Parsed {
    pub positionals: Vec<String>,
    /// (long name, value). Flags are stored with `None`; repeated flags repeat.
    pub opts: Vec<(String, Option<String>)>,
}

impl Parsed {
    pub fn flag(&self, long: &str) -> bool {
        self.opts.iter().any(|(k, _)| k == long)
    }

    pub fn count(&self, long: &str) -> usize {
        self.opts.iter().filter(|(k, _)| k == long).count()
    }

    pub fn value(&self, long: &str) -> Option<&str> {
        self.opts
            .iter()
            .rev()
            .find(|(k, v)| k == long && v.is_some())
            .and_then(|(_, v)| v.as_deref())
    }

    pub fn values(&self, long: &str) -> Vec<&str> {
        self.opts
            .iter()
            .filter(|(k, v)| k == long && v.is_some())
            .filter_map(|(_, v)| v.as_deref())
            .collect()
    }
}

fn find_long<'a>(specs: &'a [OptSpec], name: &str) -> Option<&'a OptSpec> {
    specs.iter().find(|s| s.long == name)
}

fn find_short(specs: &[OptSpec], c: char) -> Option<&OptSpec> {
    specs.iter().find(|s| s.short == Some(c))
}

pub fn parse(args: &[String], specs: &[OptSpec]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let mut i = 0;
    let mut only_positional = false;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if only_positional {
            out.positionals.push(a.clone());
            continue;
        }
        if a == "--" {
            only_positional = true;
            continue;
        }
        if let Some(rest) = a.strip_prefix("--") {
            let (name, inline) = match rest.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (rest, None),
            };
            let spec = find_long(specs, name)
                .ok_or_else(|| Error::usage(format!("unknown option --{name}")))?;
            if spec.takes_value {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        if i >= args.len() {
                            return Err(Error::usage(format!("--{name} needs a value")));
                        }
                        i += 1;
                        args[i - 1].clone()
                    }
                };
                out.opts.push((spec.long.to_string(), Some(v)));
            } else {
                if inline.is_some() {
                    return Err(Error::usage(format!("--{name} does not take a value")));
                }
                out.opts.push((spec.long.to_string(), None));
            }
            continue;
        }
        if a.len() > 1 && a.starts_with('-') && !a[1..].starts_with(|c: char| c.is_ascii_digit()) {
            let chars: Vec<char> = a[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                let c = chars[j];
                j += 1;
                let spec = find_short(specs, c)
                    .ok_or_else(|| Error::usage(format!("unknown option -{c}")))?;
                if spec.takes_value {
                    let v: String = if j < chars.len() {
                        let s: String = chars[j..].iter().collect();
                        j = chars.len();
                        s
                    } else {
                        if i >= args.len() {
                            return Err(Error::usage(format!("-{c} needs a value")));
                        }
                        i += 1;
                        args[i - 1].clone()
                    };
                    out.opts.push((spec.long.to_string(), Some(v)));
                } else {
                    out.opts.push((spec.long.to_string(), None));
                }
            }
            continue;
        }
        out.positionals.push(a.clone());
    }
    Ok(out)
}

/// Render an options block for `--help`.
pub fn usage_block(specs: &[OptSpec]) -> String {
    let mut lines = Vec::new();
    let mut width = 0;
    let mut rendered = Vec::new();
    for s in specs {
        let mut left = String::new();
        match s.short {
            Some(c) => left.push_str(&format!("-{c}, ")),
            None => left.push_str("    "),
        }
        left.push_str("--");
        left.push_str(s.long);
        if s.takes_value {
            left.push(' ');
            left.push_str(s.meta);
        }
        width = width.max(left.len());
        rendered.push((left, s.help));
    }
    for (left, help) in rendered {
        lines.push(format!("  {left:<width$}  {help}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPECS: &[OptSpec] = &[
        OptSpec::flag(Some('v'), "verbose", "more output"),
        OptSpec::value(Some('c'), "config", "PATH", "config file"),
        OptSpec::value(None, "acid", "N", "ac id"),
        OptSpec::flag(None, "strict-bind", "bind"),
    ];

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_mixed() {
        let p = parse(
            &args(&[
                "login",
                "-vv",
                "-c",
                "a.json",
                "--acid=8",
                "--strict-bind",
                "x",
            ]),
            SPECS,
        )
        .unwrap();
        assert_eq!(p.positionals, vec!["login", "x"]);
        assert_eq!(p.count("verbose"), 2);
        assert_eq!(p.value("config"), Some("a.json"));
        assert_eq!(p.value("acid"), Some("8"));
        assert!(p.flag("strict-bind"));
    }

    #[test]
    fn short_with_attached_value_and_terminator() {
        let p = parse(&args(&["-ca.json", "--", "-v"]), SPECS).unwrap();
        assert_eq!(p.value("config"), Some("a.json"));
        assert_eq!(p.positionals, vec!["-v"]);
    }

    #[test]
    fn errors() {
        assert!(parse(&args(&["--nope"]), SPECS).is_err());
        assert!(parse(&args(&["-c"]), SPECS).is_err());
        assert!(parse(&args(&["--verbose=1"]), SPECS).is_err());
        assert!(parse(&args(&["-z"]), SPECS).is_err());
    }

    #[test]
    fn negative_number_is_positional() {
        let p = parse(&args(&["-5"]), SPECS).unwrap();
        assert_eq!(p.positionals, vec!["-5"]);
    }
}
