pub(crate) mod output;
pub(crate) mod progress;

use std::collections::{BTreeMap, HashSet};

pub(crate) const USAGE: &str = include_str!("usage.txt");
pub(crate) const DEFAULTS: &str = include_str!("flags.txt");

const BOOLEAN: [&str; 18] = [
    "compact-stack",
    "version",
    "v",
    "vv",
    "include-other-authors",
    "dry-run",
    "skip-draft",
    "include-draft",
    "yes",
    "y",
    "github-stack",
    "no-stack",
    "stack-up",
    "reverse",
    "branch-from-title",
    "draft",
    "h",
    "help",
];
const STRING: [&str; 9] = [
    "command-timeout",
    "read-timeout",
    "stop-after",
    "gh-hosts",
    "default-tags",
    "t",
    "output",
    "draft-pattern",
    "timeout",
];

#[derive(Clone, Debug, Default)]
pub(crate) struct Flags {
    booleans: BTreeMap<String, bool>,
    strings: BTreeMap<String, String>,
    timeout: Option<i64>,
    verbosity: u8,
    pub supplied: HashSet<String>,
    pub labels: Vec<String>,
    pub tip_labels: Vec<String>,
    pub args: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParseError {
    Help,
    Invalid(String),
}

impl Flags {
    pub(crate) const fn verbosity(&self) -> u8 {
        self.verbosity
    }

    pub(crate) fn timeout(&self) -> i64 {
        self.timeout.unwrap_or(20)
    }

    pub(crate) fn boolean(&self, key: &str) -> bool {
        self.booleans
            .get(key)
            .copied()
            .unwrap_or(matches!(key, "stack-up" | "reverse" | "draft"))
    }

    pub(crate) fn string(&self, key: &str) -> &str {
        self.strings.get(key).map_or(
            match key {
                "gh-hosts" => "~/.config/gh/hosts.yml",
                "timeout" => "20",
                _ => "",
            },
            String::as_str,
        )
    }

    pub(crate) fn parse(args: Vec<String>) -> Result<Self, ParseError> {
        let mut flags = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "--" {
                flags.args.extend(args);
                break;
            }
            if !arg.starts_with('-') || arg == "-" {
                flags.args.push(arg);
                flags.args.extend(args);
                break;
            }
            let name = arg.strip_prefix("--").unwrap_or_else(|| &arg[1..]);
            if name.starts_with(['-', '=']) {
                return Err(ParseError::Invalid(format!("bad flag syntax: {arg}")));
            }
            let (key, inline) = name
                .split_once('=')
                .map_or((name, None), |(k, v)| (k, Some(v)));
            if matches!(key, "h" | "help") {
                return Err(ParseError::Help);
            }
            if BOOLEAN.contains(&key) {
                let value = inline.unwrap_or("true");
                let boolean = parse_bool(value).ok_or_else(|| {
                    ParseError::Invalid(format!(
                        "invalid boolean value {value:?} for -{key}: parse error"
                    ))
                })?;
                if matches!(key, "v" | "vv") {
                    flags.verbosity = if !boolean {
                        0
                    } else if key == "vv" {
                        2
                    } else {
                        flags.verbosity.saturating_add(1).min(2)
                    };
                }
                flags
                    .booleans
                    .insert(if key == "y" { "yes" } else { key }.into(), boolean);
            } else if STRING.contains(&key) || matches!(key, "add-label" | "add-tip-label") {
                let value = inline
                    .map(str::to_owned)
                    .or_else(|| args.next())
                    .ok_or_else(|| {
                        ParseError::Invalid(format!("flag needs an argument: -{key}"))
                    })?;
                if key == "timeout" {
                    flags.timeout = Some(parse_integer(&value).map_err(|reason| {
                        ParseError::Invalid(format!(
                            "invalid value {value:?} for flag -{key}: {reason}"
                        ))
                    })?);
                }
                match key {
                    "add-label" => flags.labels.push(value),
                    "add-tip-label" => flags.tip_labels.push(value),
                    _ => {
                        flags.strings.insert(key.into(), value);
                    }
                }
            } else {
                return Err(ParseError::Invalid(format!(
                    "flag provided but not defined: -{key}"
                )));
            }
            flags.supplied.insert(key.into());
        }
        if flags.supplied.contains("stack-up")
            && flags.supplied.contains("reverse")
            && flags.boolean("stack-up") != flags.boolean("reverse")
        {
            return Err(ParseError::Invalid(
                "conflicting --stack-up and deprecated --reverse values; use --stack-up alone"
                    .into(),
            ));
        }
        Ok(flags)
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Some(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Some(false),
        _ => None,
    }
}

pub(crate) fn parse_integer(value: &str) -> Result<i64, &'static str> {
    let (sign, digits) = value.strip_prefix('-').map_or_else(
        || (1_i128, value.strip_prefix('+').unwrap_or(value)),
        |s| (-1, s),
    );
    let (radix, prefix, digits) = if digits.starts_with("0x") || digits.starts_with("0X") {
        (16, true, &digits[2..])
    } else if digits.starts_with("0b") || digits.starts_with("0B") {
        (2, true, &digits[2..])
    } else if digits.starts_with("0o") || digits.starts_with("0O") {
        (8, true, &digits[2..])
    } else if digits.starts_with('0') && digits.len() > 1 {
        (8, true, &digits[1..])
    } else {
        (10, false, digits)
    };
    let mut previous_digit = prefix;
    for character in digits.chars() {
        if character == '_' && previous_digit {
            previous_digit = false;
        } else if character.is_ascii() && character.is_digit(radix) {
            previous_digit = true;
        } else {
            return Err("parse error");
        }
    }
    if digits.is_empty() || !previous_digit {
        return Err("parse error");
    }
    let magnitude =
        i128::from_str_radix(&digits.replace('_', ""), radix).map_err(|_| "value out of range")?;
    i64::try_from(sign * magnitude).map_err(|_| "value out of range")
}
