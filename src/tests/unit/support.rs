//! In-memory boundaries; unexpected operations fail without invoking a native adapter.

use crate::{
    cli::Flags,
    commands::{Clock, CommandOutput, CommandRunner, Commands, Terminal},
    config::{self, files::Files},
    credentials::CredentialStore,
    github::http::{Request, Transport},
    model::Commit,
    submission::Application,
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

thread_local! {
    pub(crate) static MESSAGE_SERIALISATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static PR_REFRESH_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static LABEL_FRESHNESS_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static LITERAL_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static WORKER_STARTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static CONTENT_ENCODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static DRAFT_PREPARATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static DRAFT_EVALUATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static BODY_ANALYSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static BASE_PLANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static DESCRIPTION_BYTES_WRITTEN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn count_description_bytes(bytes: usize) {
    DESCRIPTION_BYTES_WRITTEN.update(|count| count + bytes);
}

fn config_key(key: &str) -> String {
    let mut key = key.to_owned();
    if let Some(first) = key.find('.') {
        key[..first].make_ascii_lowercase();
    }
    if let Some(last) = key.rfind('.') {
        key[last + 1..].make_ascii_lowercase();
    }
    key
}

#[derive(Clone, Debug)]
pub(crate) enum Response {
    Output(i32, String),
    Failure(String),
}

#[derive(Debug, Default)]
pub(crate) struct Memory {
    pub clock_start: Option<std::time::Instant>,
    pub clock_elapsed: Duration,
    pub changes_on_wait: BTreeMap<u64, String>,
    pub responses: BTreeMap<String, VecDeque<Response>>,
    pub http_responses: BTreeMap<String, VecDeque<Response>>,
    pub config: BTreeMap<String, String>,
    pub files: BTreeMap<PathBuf, String>,
    pub file_errors: BTreeMap<PathBuf, std::io::ErrorKind>,
    pub attempt_error: Option<std::io::ErrorKind>,
    pub commits: Vec<Commit>,
    pub prs: BTreeMap<u64, Value>,
    pub changes_on_push: BTreeMap<u64, Option<Value>>,
    pub remote_refs: BTreeMap<String, String>,
    pub trace: Vec<Value>,
    pub rich_output: bool,
    pub stdout: String,
    pub stderr: String,
    pub answers: VecDeque<bool>,
    pub jj: bool,
    pub branchless: bool,
    pub token: String,
    pub branch: String,
}

#[derive(Debug)]
pub(crate) struct World(pub Mutex<Memory>);

pub(crate) fn commit(number: u64) -> Commit {
    Commit {
        hash: format!("{number:040x}"),
        change_id: format!("change{number}"),
        title: format!("Feature {number}"),
        message: format!("Body {number}"),
        author_name: "Unit Author".into(),
        author_email: "unit@example.com".into(),
        attrs: vec![("remote-ref".into(), format!("topic/{number}"))],
        ..Commit::default()
    }
}

pub(crate) fn log(commits: &[Commit]) -> String {
    commits
        .iter()
        .enumerate()
        .rev()
        .map(|(i, c)| {
            format!(
                "{}\0{}\0{}\02024-01-01T00:00:00Z\0tree{}\0{}\0{}\0\n",
                c.hash,
                c.author_name,
                c.author_email,
                i + 1,
                if i == 0 {
                    format!("{:040x}", 0)
                } else {
                    commits[i - 1].hash.clone()
                },
                c.full_message()
            )
        })
        .fold(String::new(), |mut text, row| {
            text.push_str(&row);
            text
        })
}

impl World {
    pub(crate) fn new(count: u64) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Memory {
            config: BTreeMap::from([
                ("user.name".into(), "Unit Author".into()),
                ("user.email".into(), "unit@example.com".into()),
            ]),
            files: BTreeMap::from([(
                PathBuf::from("/unit/home/.config/gh/hosts.yml"),
                "github.com:\n  oauth_token: unit-token\n".into(),
            )]),
            commits: (1..=count).map(commit).collect(),
            token: "unit-token".into(),
            branch: format!("topic/{count}"),
            ..Memory::default()
        })))
    }
    pub(crate) fn commands(self: &Arc<Self>) -> Commands {
        Commands {
            runner: self.clone(),
            terminal: self.clone(),
            cwd: "/unit/repo".into(),
            env: BTreeMap::new(),
            verbose: false,
            explain: false,
            outcomes: crate::model::outcome::Outcomes::default(),
            diagnostics: crate::diagnostics::Diagnostics::default(),
        }
    }
    pub(crate) fn app(self: &Arc<Self>) -> Application {
        Application {
            commands: self.commands(),
            clock: self.clone(),
            transport: self.clone(),
            credentials: self.clone(),
            files: self.clone(),
        }
    }
    pub(crate) fn run(self: &Arc<Self>, arguments: &[&str]) -> Result<()> {
        let flags = Flags::parse(arguments.iter().map(|v| (*v).into()).collect()).unwrap();
        self.app().execute(
            &flags,
            &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
        )
    }
    pub(crate) fn config(self: &Arc<Self>) -> Box<config::Config> {
        let config::Loaded::Ready(config) = config::load(
            &Flags::default(),
            &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
            &mut self.commands(),
            self.as_ref(),
            self.as_ref(),
            false,
        )
        .unwrap() else {
            panic!("expected configuration");
        };
        config
    }
    pub(crate) fn reply(&self, key: &str, status: i32, value: &str) {
        self.0.lock().unwrap().responses.insert(
            key.into(),
            VecDeque::from([Response::Output(status, value.into())]),
        );
    }
    pub(crate) fn fail(&self, key: &str) {
        self.0.lock().unwrap().responses.insert(
            key.into(),
            VecDeque::from([Response::Failure("unavailable".into())]),
        );
    }
    pub(crate) fn http(&self, key: &str, responses: Vec<Response>) {
        self.0
            .lock()
            .unwrap()
            .http_responses
            .insert(key.into(), responses.into());
    }
    pub(crate) fn trace(&self) -> Vec<Value> {
        self.0.lock().unwrap().trace.clone()
    }
}

fn response(responses: &mut BTreeMap<String, VecDeque<Response>>, key: &str) -> Option<Response> {
    responses.get_mut(key).and_then(|queue| {
        if queue.len() > 1 {
            queue.pop_front()
        } else {
            queue.front().cloned()
        }
    })
}

impl CommandRunner for World {
    fn edit_messages(
        &self,
        program: &str,
        args: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
        messages: &[(String, String)],
    ) -> Result<CommandOutput> {
        let mut m = self.0.lock().unwrap();
        m.trace
            .push(json!({"kind":"command","program":program,"args":args}));
        let mut changed = false;
        for (i, c) in m.commits.iter_mut().enumerate() {
            if let Some((_, message)) = messages.iter().find(|(oid, _)| *oid == c.hash) {
                let (title, body) = message.split_once('\n').unwrap();
                c.title = title.into();
                (c.message, c.attrs) =
                    crate::model::trailers::parse(&body.lines().collect::<Vec<_>>());
                changed = true;
            }
            if changed {
                c.hash = format!("b{:039x}", i + 1);
            }
        }
        drop(m);
        Ok(CommandOutput::default())
    }
    fn execute(
        &self,
        program: &str,
        args: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
    ) -> Result<CommandOutput> {
        let mut m = self.0.lock().unwrap();
        m.trace
            .push(json!({"kind":"command","program":program,"args":args}));
        let key = format!("{program} {}", args.join(" "));
        if let Some(reply) = response(&mut m.responses, &key) {
            return match reply {
                Response::Output(status, text) => Ok(CommandOutput { status, text }),
                Response::Failure(error) => bail!("{error}"),
            };
        }
        let strings = args.iter().map(String::as_str).collect::<Vec<_>>();
        let view = if strings.first() == Some(&"--ignore-working-copy") {
            &strings[1..]
        } else {
            strings.as_slice()
        };
        if program == "git" && view.get(1) == Some(&"--null") && view.contains(&"--get-regexp") {
            let pattern = regex::Regex::new(view.last().unwrap()).unwrap();
            let prefix = view
                .iter()
                .find(|arg| arg.starts_with("--type="))
                .map_or_else(
                    || "git config --get ".to_owned(),
                    |kind| format!("git config {kind} --get "),
                );
            let mut values: BTreeMap<_, _> = m
                .config
                .iter()
                .filter(|(key, _)| pattern.is_match(&config_key(key)))
                .map(|(key, value)| (config_key(key), value.clone()))
                .collect();
            let overrides: Vec<_> = m
                .responses
                .keys()
                .filter(|key| {
                    key.strip_prefix(&prefix)
                        .is_some_and(|name| pattern.is_match(&name.to_ascii_lowercase()))
                })
                .cloned()
                .collect();
            for key in overrides {
                match response(&mut m.responses, &key).unwrap() {
                    Response::Failure(error) => bail!("{error}"),
                    Response::Output(0, value) => {
                        values.insert(
                            key.strip_prefix(&prefix).unwrap().to_ascii_lowercase(),
                            value,
                        );
                    }
                    Response::Output(status, text) => return Ok(CommandOutput { status, text }),
                }
            }
            return Ok(CommandOutput {
                status: i32::from(values.is_empty()),
                text: values
                    .into_iter()
                    .fold(String::new(), |mut output, (key, value)| {
                        use std::fmt::Write as _;
                        if view.contains(&"--show-origin") {
                            output.push_str("local\0file:/unit/repo/.git/config\0");
                        }
                        write!(output, "{key}\n{value}\0").unwrap();
                        output
                    }),
            });
        }
        if program == "git" && view.first() == Some(&"push") {
            for lease in view
                .iter()
                .filter_map(|arg| arg.strip_prefix("--force-with-lease="))
            {
                let (reference, expected) = lease.split_once(':').unwrap();
                if m.remote_refs.get(reference).map_or("", String::as_str) != expected {
                    return Ok(CommandOutput {
                        status: 1,
                        text: "stale lease".into(),
                    });
                }
            }
            for spec in view.iter().filter(|arg| arg.contains(":refs/heads/")) {
                let (oid, reference) = spec.split_once(':').unwrap();
                if oid.is_empty() {
                    m.remote_refs.remove(reference);
                } else {
                    m.remote_refs.insert(reference.into(), oid.into());
                }
                for pr in m.prs.values_mut().filter(|pr| {
                    pr["head"]["ref"].as_str() == reference.strip_prefix("refs/heads/")
                        && pr["head"].get("sha").is_some()
                }) {
                    pr["head"]["sha"] = json!(oid);
                }
            }
            for (number, pr) in std::mem::take(&mut m.changes_on_push) {
                if let Some(pr) = pr {
                    m.prs.insert(number, pr);
                } else {
                    m.prs.remove(&number);
                }
            }
        }
        let text = match (program, view) {
            (
                "git",
                ["config", "--get", key] | ["config", "--type=bool" | "--type=int", "--get", key],
            ) => {
                let Some(value) = m.config.get(*key) else {
                    return Ok(CommandOutput {
                        status: 1,
                        text: String::new(),
                    });
                };
                value.clone()
            }
            (
                "git",
                ["config", "--replace-all", key, value]
                | ["config", "--local", "--replace-all" | "--add", key, value],
            ) => {
                m.config.insert((*key).into(), (*value).into());
                String::new()
            }
            (
                "git",
                ["config", "--local", "--fixed-value", "--unset-all", key, _]
                | ["config", "--local", "--unset-all", key],
            ) => {
                m.config
                    .retain(|candidate, _| config_key(candidate) != config_key(key));
                String::new()
            }
            ("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"]) => {
                "/unit/repo/.git\n".into()
            }
            ("git", ["rev-parse", "--show-toplevel"]) | ("jj", ["workspace", "root"]) => {
                "/unit/repo".into()
            }
            ("git", ["remote", "-v"]) => "origin https://github.com/org/repo.git (push)".into(),
            ("git", ["remote", "get-url", "--push", "--all", "--", _]) => {
                "https://github.com/org/repo.git".into()
            }
            ("git", ["ls-remote", "--refs", "--", _, refs @ ..]) => refs
                .iter()
                .filter_map(|reference| {
                    m.remote_refs
                        .get(*reference)
                        .map(|oid| format!("{oid}\t{reference}"))
                })
                .collect::<Vec<_>>()
                .join("\n"),
            ("git", ["symbolic-ref", reference]) => {
                format!("{}/main", reference.strip_suffix("/HEAD").unwrap())
            }
            ("git", ["branchless", "--version"]) => {
                if !m.branchless {
                    bail!("branchless unavailable");
                }
                "branchless".into()
            }
            ("git", ["branch", "--show-current"]) => m.branch.clone(),
            ("git", ["branch", _]) => m
                .commits
                .iter()
                .enumerate()
                .map(|(i, c)| format!("topic/{}|{}", i + 1, c.hash))
                .collect::<Vec<_>>()
                .join("\n"),
            (
                "git",
                [
                    "log",
                    format,
                    "--no-decorate",
                    "--no-color",
                    "--encoding=UTF-8",
                    "--no-show-signature",
                    range,
                ],
            ) if format.starts_with("--format=%H") => {
                let tip = range.split("..").nth(1).unwrap();
                let end = m
                    .commits
                    .iter()
                    .position(|c| c.hash == tip)
                    .map_or(m.commits.len(), |i| i + 1);
                m.commits[..end]
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(i, c)| {
                        format!(
                            "{}\0{}\0{}\02024-01-01T00:00:00Z\0tree{}\0{}\0{}\0\n",
                            c.hash,
                            c.author_name,
                            c.author_email,
                            i + 1,
                            if i == 0 {
                                format!("{:040x}", 0)
                            } else {
                                m.commits[i - 1].hash.clone()
                            },
                            c.full_message()
                        )
                    })
                    .fold(String::new(), |mut text, row| {
                        text.push_str(&row);
                        text
                    })
            }
            ("git", ["show", "--no-show-signature", "-s", "--format=%H %T", _]) => {
                format!("{:040x} tree0", 0)
            }
            (
                "jj",
                [
                    "--at-operation",
                    _,
                    "log",
                    "-r",
                    _,
                    "--no-graph",
                    "-T",
                    template,
                ],
            ) if template.contains("contained_in") => {
                let pattern =
                    regex::Regex::new(r#"contained_in\(("(?:[^"\\]|\\.)*")\), "(\d+),""#).unwrap();
                let revisions: Vec<_> = pattern
                    .captures_iter(template)
                    .map(|captures| {
                        (
                            serde_json::from_str::<String>(&captures[1]).unwrap(),
                            captures[2].parse::<usize>().unwrap(),
                        )
                    })
                    .collect();
                m.commits
                    .iter()
                    .filter_map(|commit| {
                        let members: Vec<_> = revisions
                            .iter()
                            .filter(|(revision, _)| {
                                revision == &commit.hash
                                    || revision == &commit.change_id
                                    || revision == commit.remote_ref()
                            })
                            .map(|(_, index)| format!("{index},"))
                            .collect();
                        (!members.is_empty())
                            .then(|| format!("{}\0{}\n", commit.hash, members.concat()))
                    })
                    .collect()
            }
            ("jj", ["--at-operation", _, "log", ..]) => m
                .commits
                .iter()
                .map(|c| format!("{} {} {}\n", c.hash, c.change_id, c.conflicted))
                .fold(String::new(), |mut text, row| {
                    text.push_str(&row);
                    text
                }),
            (
                "git",
                [
                    "for-each-ref",
                    "--format=%(refname:strip=2)|%(objectname)",
                    "refs/heads/",
                ],
            ) => m
                .commits
                .iter()
                .map(|c| format!("{}|{}\n", c.remote_ref(), c.hash))
                .fold(String::new(), |mut text, row| {
                    text.push_str(&row);
                    text
                }),
            ("git", ["merge-base", "--independent", tips @ ..]) => {
                tips.last().unwrap_or(&"").to_string()
            }
            ("git", ["diff-tree", ..]) => "file.txt".into(),
            ("git", ["worktree", "list", "--porcelain"]) => {
                "worktree /unit/repo\nbranch refs/heads/protected".into()
            }
            ("git", ["rev-list", "--count", range]) => {
                let from = range.split("..").next().unwrap();
                let index = m
                    .commits
                    .iter()
                    .position(|c| c.hash == from)
                    .map_or(0, |i| i + 1);
                (m.commits.len() - index).to_string()
            }
            ("git", ["rev-parse", "--verify", name]) => resolve_mock_revision(&m.commits, name),
            ("git", ["rev-parse", "--revs-only", "--end-of-options", names @ ..]) => names
                .iter()
                .map(|name| resolve_mock_revision(&m.commits, name))
                .collect::<Vec<_>>()
                .join("\n"),
            ("git", ["rev-parse", name]) => (*name).into(),
            ("git", ["reword", hash, "-m", message])
            | ("jj", ["describe", "-r", hash, "-m", message]) => {
                let index = m
                    .commits
                    .iter()
                    .position(|c| c.hash == *hash || c.change_id == *hash)
                    .unwrap();
                let c = &mut m.commits[index];
                let (title, body) = message.split_once('\n').unwrap();
                c.title = title.into();
                let (body, attrs) =
                    crate::model::trailers::parse(&body.lines().collect::<Vec<_>>());
                c.message = body;
                c.attrs = attrs;
                for (i, c) in m.commits.iter_mut().enumerate().skip(index) {
                    c.hash = format!("b{:039x}", i + 1);
                }
                String::new()
            }
            ("jj", ["version"]) => {
                if !m.jj {
                    bail!("jj unavailable");
                }
                "jj".into()
            }

            ("jj", ["op", "log", ..]) => "operation".into(),
            ("jj", ["bookmark", "list", ..]) => m
                .commits
                .iter()
                .map(|c| format!("{}\00\0{} {}\n", c.remote_ref(), c.hash, c.change_id))
                .fold(String::new(), |mut text, row| {
                    text.push_str(&row);
                    text
                }),
            ("jj", ["git", "root"]) => "/unit/repo/.jj/repo/store/git".into(),
            ("jj", ["log", "-r", revision, "--no-graph", "-T", template]) => {
                if *template == "commit_id" || *revision == "heads(@- | (@ ~ empty()))" {
                    m.commits.last().map_or(String::new(), |c| c.hash.clone())
                } else if *template == "change_id" {
                    m.commits
                        .iter()
                        .find(|c| c.hash == *revision)
                        .map_or_else(|| "change1".into(), |c| c.change_id.clone())
                } else if template.starts_with("conflict ++") {
                    "false\0EMPTY|NO-DESC\0Other Author\0other@example.com\0working|hash|".into()
                } else if template.starts_with("if(empty") {
                    if template.contains("\\0") {
                        "EMPTY|NO-DESC\0Other Author\0other@example.com\0working|hash|".into()
                    } else {
                        "EMPTY|NO-DESC".into()
                    }
                } else if template.starts_with("change_id ++") {
                    "working|hash|".into()
                } else if revision.starts_with("bookmarks(") {
                    String::new()
                } else {
                    m.commits
                        .iter()
                        .find(|c| {
                            c.change_id == *revision
                                || c.hash == *revision
                                || c.remote_ref() == *revision
                                || revision
                                    .strip_prefix("topic/")
                                    .is_some_and(|n| c.change_id == format!("change{n}"))
                        })
                        .map_or_else(|| (*revision).into(), |c| c.hash.clone())
                }
            }
            ("gh", ["pr", "create", ..]) => {
                let value = |name| {
                    args.iter()
                        .position(|v| v == name)
                        .map_or("", |i| args[i + 1].as_str())
                };
                let n = m.prs.keys().max().copied().unwrap_or(100) + 1;
                m.prs.insert(n,json!({"number":n,"body":value("--body"),"title":value("--title"),"head":{"ref":value("--head")},"base":{"ref":value("--base")},"state":"open","merged":false,"draft":args.iter().any(|v|v=="--draft")}));
                format!("https://github.com/org/repo/pull/{n}")
            }
            ("gh", ["pr", "edit", number, "--base", base]) => {
                m.prs.get_mut(&number.parse().unwrap()).unwrap()["base"]["ref"] = json!(base);
                String::new()
            }
            (
                "git",
                [
                    "status" | "diff" | "check-ref-format" | "checkout" | "update-ref"
                    | "merge-base" | "for-each-ref" | "push",
                    ..,
                ],
            )
            | ("jj", ["bookmark", ..] | ["git", "export" | "import"] | ["util", "snapshot"])
            | ("gh", ["pr", "edit" | "list", ..] | ["stack", ..]) => String::new(),
            _ => panic!("unexpected in-memory command: {key}"),
        };
        drop(m);
        Ok(CommandOutput { status: 0, text })
    }
}

impl crate::config::files::RepositoryLock for () {}
impl Files for World {
    fn lock(&self, path: &Path) -> std::io::Result<Box<dyn crate::config::files::RepositoryLock>> {
        if let Some(kind) = self.0.lock().unwrap().file_errors.get(path) {
            return Err(std::io::Error::from(*kind));
        }
        Ok(Box::new(()))
    }
    fn attempt_id(&self) -> std::io::Result<String> {
        let failure = self.0.lock().unwrap().attempt_error;
        if let Some(kind) = failure {
            return Err(kind.into());
        }
        Ok("00000000-0000-4000-8000-000000000001".into())
    }

    fn read(&self, path: &Path) -> std::io::Result<String> {
        let mut m = self.0.lock().unwrap();
        m.trace.push(json!({"kind":"file","path":path}));
        if let Some(kind) = m.file_errors.get(path) {
            return Err(std::io::Error::from(*kind));
        }
        m.files
            .get(path)
            .cloned()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }
    fn exists(&self, path: &Path) -> std::io::Result<bool> {
        let memory = self.0.lock().unwrap();
        if let Some(kind) = memory.file_errors.get(path) {
            return Err(std::io::Error::from(*kind));
        }
        Ok(path.file_name().is_some_and(|name| name == ".jj") && memory.jj)
    }
}
impl CredentialStore for World {
    fn get(&self, service: &str, _: &Commands) -> Result<String> {
        let mut m = self.0.lock().unwrap();
        m.trace.push(json!({"kind":"credential","service":service}));
        Ok(m.token.clone())
    }
}
impl Clock for World {
    fn sleep(&self, duration: Duration) {
        let mut memory = self.0.lock().unwrap();
        memory.clock_elapsed += duration;
        if duration >= Duration::from_secs(2) {
            for (number, body) in std::mem::take(&mut memory.changes_on_wait) {
                memory.prs.get_mut(&number).unwrap()["body"] = json!(body);
            }
        }
        memory
            .trace
            .push(json!({"kind":"wait","millis":duration.as_millis()}));
        drop(memory);
    }
    fn now(&self) -> std::time::Instant {
        let mut memory = self.0.lock().unwrap();
        let start = *memory
            .clock_start
            .get_or_insert_with(std::time::Instant::now);
        start + memory.clock_elapsed
    }
}
impl Terminal for World {
    fn selection(&self, _: &[&Commit], _: bool, _: usize) -> bool {
        self.0.lock().unwrap().rich_output
    }
    fn results(
        &self,
        _: &[&Commit],
        _: &crate::model::Repository,
        _: bool,
        _: crate::cli::hyperlinks::Mode,
        _: &[crate::model::pull_request::PullRequestSummary],
        _: bool,
    ) -> bool {
        self.0.lock().unwrap().rich_output
    }

    fn out(&self, text: &str) {
        let mut m = self.0.lock().unwrap();
        m.stdout.push_str(text);
        m.stdout.push('\n');
    }
    fn err(&self, text: &str) {
        self.0.lock().unwrap().stderr.push_str(text);
    }
    fn confirm(&self, prompt: &str) -> bool {
        let mut m = self.0.lock().unwrap();
        m.trace.push(json!({"kind":"confirm","prompt":prompt}));
        m.answers.pop_front().unwrap_or(false)
    }
}
impl Transport for World {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        let mut m = self.0.lock().unwrap();
        let url = reqwest::Url::parse(request.url)?;
        let body: Option<serde_json::Value> =
            request.body.map(serde_json::from_slice).transpose()?;
        let path = url
            .path()
            .strip_prefix("/repos/org/repo/")
            .unwrap_or_else(|| url.path().trim_start_matches('/'));
        assert_eq!(request.token, "unit-token");
        m.trace.push(json!({"kind":"http","method":request.method,"path":path,"body":body,"query":url.query()}));
        if let Some(response) =
            response(&mut m.http_responses, &format!("{} {path}", request.method))
        {
            match response {
                Response::Failure(error) => bail!("{error}"),
                Response::Output(status, body) => {
                    if status == 200 {
                        return Ok(body.into_bytes());
                    }
                    return Err(crate::github::http::HttpStatusError {
                        quota: crate::github::limits::Quota::default(),
                        url: request.url.into(),
                        status: reqwest::StatusCode::from_u16(u16::try_from(status).unwrap())
                            .unwrap(),
                        body,
                    }
                    .into());
                }
            }
        }
        let value = crate::tests::http_service::respond(
            &mut m.prs,
            request.method,
            path,
            url.query(),
            body.as_ref(),
        )?;
        drop(m);
        if value.is_null() {
            return Ok(Vec::new());
        }
        Ok(serde_json::to_vec(&value).unwrap())
    }
}

fn resolve_mock_revision(commits: &[Commit], name: &str) -> String {
    let name = name.trim_end_matches("^{commit}");
    let name = name.strip_prefix("refs/heads/").unwrap_or(name);
    if name == "refs/remotes/origin/main" {
        format!("{:040x}", 0)
    } else if let Some(depth) = name.strip_prefix("HEAD~") {
        commits
            .len()
            .checked_sub(depth.parse::<usize>().unwrap() + 1)
            .and_then(|i| commits.get(i))
            .map_or_else(|| format!("{:040x}", 0), |c| c.hash.clone())
    } else if let Some(parent) = name.strip_suffix('^') {
        commits
            .iter()
            .position(|c| c.hash == parent)
            .and_then(|i| i.checked_sub(1))
            .map_or_else(|| format!("{:040x}", 0), |i| commits[i].hash.clone())
    } else if let Some(index) = name.strip_prefix("topic/") {
        commits[index.parse::<usize>().unwrap() - 1].hash.clone()
    } else {
        name.into()
    }
}
