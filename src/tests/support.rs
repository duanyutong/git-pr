use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use serde_json::{Value, json};

use crate::{
    cli::Flags,
    commands::{Clock, CommandOutput, CommandRunner, Commands, ProcessRunner, Terminal},
    credentials::CredentialStore,
    github::http::{Request, Transport},
    submission::Application,
};

#[derive(Debug, Default)]
pub(super) struct MemoryTerminal {
    pub stdout: Mutex<String>,
    pub stderr: Mutex<String>,
    pub answers: Mutex<Vec<bool>>,
}
impl Terminal for MemoryTerminal {
    fn out(&self, text: &str) {
        let mut s = self.stdout.lock().unwrap();
        s.push_str(text);
        if !text.ends_with('\n') {
            s.push('\n');
        }
    }
    fn err(&self, text: &str) {
        self.stderr.lock().unwrap().push_str(text);
    }
    fn confirm(&self, prompt: &str) -> bool {
        self.err(prompt);
        self.answers.lock().unwrap().pop().unwrap_or(false)
    }
}

#[derive(Debug, Default)]
pub(super) struct FastClock {
    pub waits: Mutex<Vec<Duration>>,
}
impl Clock for FastClock {
    fn sleep(&self, duration: Duration) {
        self.waits.lock().unwrap().push(duration);
    }
}

#[derive(Debug, Default)]
pub(super) struct FakeCredentials;
impl CredentialStore for FakeCredentials {
    fn get(&self, _: &str, _: &Commands) -> Result<String> {
        Ok("fixture-token".into())
    }
}

#[derive(Debug, Default)]
pub(super) struct Service {
    pub prs: Mutex<BTreeMap<u64, Value>>,
    pub trace: Mutex<Vec<Value>>,
    pub stack_errors: Mutex<Vec<String>>,
    pub http_errors: Mutex<BTreeMap<String, String>>,
    pub command_errors: Mutex<BTreeMap<String, (i32, String)>>,
    pub template_change: Mutex<Option<(PathBuf, String)>>,
    pub branchless: bool,
    pub indirect_merges: Option<bool>,
    pub stack_base: Mutex<Option<String>>,
    pub resolved_destinations: Mutex<BTreeMap<String, String>>,
    pub git_context: Mutex<Option<(PathBuf, BTreeMap<OsString, OsString>)>>,
}
impl Service {
    /// Model host lifecycle effects using the disposable remote's actual Git graph.
    fn recognise_indirect_merges(
        &self,
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
        delete: bool,
    ) -> Result<()> {
        let remote = cwd.join("../remote.git");
        let repository = format!("--git-dir={}", remote.display());
        let observed = self.prs.lock().unwrap().clone();
        for (number, pr) in observed.iter().filter(|(_, pr)| pr["state"] == "open") {
            let head = format!("refs/heads/{}", pr["head"]["ref"].as_str().unwrap());
            let base = format!("refs/heads/{}", pr["base"]["ref"].as_str().unwrap());
            let ancestry = self.execute(
                "git",
                &[
                    repository.clone(),
                    "merge-base".into(),
                    "--is-ancestor".into(),
                    head.clone(),
                    base,
                ],
                cwd,
                env,
            )?;
            if ancestry.status != 0 {
                continue;
            }
            {
                let mut prs = self.prs.lock().unwrap();
                let merged = prs.get_mut(number).unwrap();
                merged["state"] = json!("closed");
                merged["merged"] = json!(true);
                if delete {
                    for dependent in prs
                        .values_mut()
                        .filter(|p| p["state"] == "open" && p["base"]["ref"] == pr["head"]["ref"])
                    {
                        dependent["base"]["ref"] = pr["base"]["ref"].clone();
                    }
                }
            }
            if delete {
                let result = self.execute(
                    "git",
                    &[repository.clone(), "update-ref".into(), "-d".into(), head],
                    cwd,
                    env,
                )?;
                assert_eq!(result.status, 0);
            }
        }
        Ok(())
    }
    pub(super) fn record(&self, value: Value) {
        self.trace.lock().unwrap().push(value);
    }
    pub(super) fn gh(&self, args: &[String]) -> CommandOutput {
        let flag = |key: &str| {
            args.iter()
                .position(|a| a == key)
                .and_then(|i| args.get(i + 1))
                .map_or("", String::as_str)
        };
        let mut prs = self.prs.lock().unwrap();
        let text = match args.get(1).map(String::as_str) {
            Some("create") => {
                let change = self.template_change.lock().unwrap().take();
                if let Some((path, text)) = change {
                    std::fs::write(path, text).unwrap();
                }
                let number = prs.keys().max().copied().unwrap_or(100) + 1;
                let draft = args.iter().any(|a| a == "--draft");
                prs.insert(number,json!({"number":number,"title":flag("--title"),"body":flag("--body"),"head":{"ref":flag("--head")},"base":{"ref":flag("--base")},"draft":draft,"state":"open","merged":false,"labels":flag("--label").split(',').filter(|s|!s.is_empty()).collect::<Vec<_>>() }));
                format!("https://github.com/org/repo/pull/{number}")
            }
            Some("edit") => {
                let number = args[2].parse::<u64>().unwrap();
                let pr = prs.get_mut(&number).unwrap();
                if !flag("--base").is_empty() {
                    if pr["stack"].is_object() {
                        return CommandOutput{status:1,text:"GraphQL: Cannot change the base branch because the pull request is part of a stack. (updatePullRequest)".into()};
                    }
                    pr["base"]["ref"] = json!(flag("--base"));
                }
                if !flag("--add-label").is_empty() {
                    let labels = pr["labels"].as_array_mut().unwrap();
                    for label in flag("--add-label").split(',') {
                        let label = json!(label);
                        if !labels.contains(&label) {
                            labels.push(label);
                        }
                    }
                }
                if !flag("--remove-label").is_empty() {
                    pr["labels"]
                        .as_array_mut()
                        .unwrap()
                        .retain(|v| !flag("--remove-label").split(',').any(|s| v == s));
                }
                String::new()
            }
            Some("list") => String::new(),
            Some("link" | "unstack") => {
                let mut errors = self.stack_errors.lock().unwrap();
                if !errors.is_empty() {
                    let text = errors.remove(0);
                    drop(errors);
                    return CommandOutput {
                        status: i32::from(!text.is_empty() && !text.contains("remain stacked")),
                        text,
                    };
                }
                drop(errors);
                if args[1] == "unstack" {
                    for pr in prs.values_mut() {
                        pr.as_object_mut().unwrap().remove("stack");
                    }
                }
                "linked".into()
            }
            _ => panic!("Unexpected gh command: {args:?}"),
        };
        CommandOutput { status: 0, text }
    }
}
impl CommandRunner for Service {
    fn edit_messages(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
        messages: &[(String, String)],
    ) -> Result<CommandOutput> {
        self.record(json!({"kind":"command","program":program,"args":args}));
        ProcessRunner::default().edit_messages(program, args, cwd, env, messages)
    }
    fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
    ) -> Result<CommandOutput> {
        self.record(json!({"kind":"command","program":program,"args":args}));
        let key = format!("{program} {}", args.join(" "));
        if program == "git"
            && self.indirect_merges.is_some()
            && !args
                .first()
                .is_some_and(|arg| arg.starts_with("--git-dir="))
        {
            *self.git_context.lock().unwrap() = Some((cwd.to_owned(), env.clone()));
        }
        if let Some((status, text)) = self.command_errors.lock().unwrap().get(&key) {
            return Ok(CommandOutput {
                status: *status,
                text: text.clone(),
            });
        }
        if program == "gh" {
            return Ok(self.gh(args));
        }
        if program == "git" && args == ["remote", "-v"] {
            return Ok(CommandOutput{status:0,text:"origin\thttps://github.com/org/repo.git (fetch)\norigin\thttps://github.com/org/repo.git (push)".into()});
        }
        if program == "git" && args == ["branchless", "--version"] && !self.branchless {
            return Ok(CommandOutput {
                status: 199,
                text: String::new(),
            });
        }
        let mut isolated: BTreeMap<_, _> = std::env::vars_os()
            .filter(|(key, _)| {
                matches!(
                    key.to_str(),
                    Some(
                        "PATH"
                            | "TMPDIR"
                            | "TMP"
                            | "TEMP"
                            | "LLVM_PROFILE_FILE"
                            | "DYLD_LIBRARY_PATH"
                            | "DYLD_FALLBACK_LIBRARY_PATH"
                            | "LD_LIBRARY_PATH"
                    )
                )
            })
            .collect();
        isolated.extend(env.clone());
        let mut isolated_args = vec!["-i".to_owned()];
        isolated_args.extend(
            isolated.into_iter().map(|(key, value)| {
                format!("{}={}", key.to_string_lossy(), value.to_string_lossy())
            }),
        );
        isolated_args.push(program.into());
        if program == "git"
            && args.starts_with(&["ls-remote".into(), "--refs".into(), "--".into()])
            && args.len() > 3
        {
            let destinations = self.resolved_destinations.lock().unwrap();
            if let Some(destination) = destinations.get(&args[3]) {
                isolated_args.extend_from_slice(&args[..3]);
                isolated_args.push(destination.clone());
                isolated_args.extend_from_slice(&args[4..]);
            } else {
                isolated_args.extend_from_slice(args);
            }
        } else {
            isolated_args.extend_from_slice(args);
        }
        let result = ProcessRunner::default().execute(
            "/usr/bin/env",
            &isolated_args,
            cwd,
            &BTreeMap::new(),
        )?;
        if result.status == 0
            && program == "git"
            && args.starts_with(&[
                "remote".into(),
                "get-url".into(),
                "--push".into(),
                "--all".into(),
                "--".into(),
            ])
        {
            let mut destinations = self.resolved_destinations.lock().unwrap();
            let mut urls = Vec::new();
            for (index, target) in result.text.lines().enumerate() {
                let url = if index == 0 {
                    "https://github.com/org/repo.git".to_owned()
                } else {
                    format!("https://github.com/org/repo-{index}.git")
                };
                destinations.insert(url.clone(), target.to_owned());
                urls.push(url);
            }
            drop(destinations);
            return Ok(CommandOutput {
                status: 0,
                text: urls.join("\n"),
            });
        }
        if result.status == 0 && program == "git" && args.first().is_some_and(|arg| arg == "push") {
            for spec in args.iter().filter(|arg| arg.contains(":refs/heads/")) {
                let (oid, reference) = spec.split_once(':').unwrap();
                for pr in self.prs.lock().unwrap().values_mut().filter(|pr| {
                    pr["head"]["ref"].as_str() == reference.strip_prefix("refs/heads/")
                        && pr["head"].get("sha").is_some()
                }) {
                    pr["head"]["sha"] = json!(oid);
                }
            }
            if let Some(delete) = self.indirect_merges {
                self.recognise_indirect_merges(cwd, env, delete)?;
            }
        }
        Ok(result)
    }
}
impl Transport for Service {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        assert_eq!(request.token, "fixture-token");
        assert_eq!(request.timeout, 20);
        let url = reqwest::Url::parse(request.url).unwrap();
        let body: Option<serde_json::Value> =
            request.body.map(serde_json::from_slice).transpose()?;
        let path = url
            .path()
            .strip_prefix("/repos/org/repo/")
            .unwrap_or_else(|| url.path().trim_start_matches('/'));
        self.record(json!({"kind":"http","method":request.method,"path":path,"query":url.query(),"body":body}));
        if let Some(error) = self
            .http_errors
            .lock()
            .unwrap()
            .get(&format!("{} {path}", request.method))
        {
            anyhow::bail!("{error}");
        }
        let mut prs = self.prs.lock().unwrap();
        let mut value = crate::tests::http_service::respond(
            &mut prs,
            request.method,
            path,
            url.query(),
            body.as_ref(),
        )?;
        drop(prs);
        if request.method == "PATCH"
            && body.as_ref().is_some_and(|body| body.get("base").is_some())
            && let Some(delete) = self.indirect_merges
        {
            let (cwd, env) = self.git_context.lock().unwrap().clone().unwrap();
            self.recognise_indirect_merges(&cwd, &env, delete)?;
        }
        if request.method == "GET" && path.starts_with("stacks/") {
            if let Some(base) = self.stack_base.lock().unwrap().as_ref() {
                value["base"]["ref"] = json!(base);
            }
        } else if request.method == "POST" && path == "stacks" {
            self.stack_base.lock().unwrap().take();
        }
        if value.is_null() {
            return Ok(Vec::new());
        }
        Ok(serde_json::to_vec(&value).unwrap())
    }
}

#[derive(Debug)]
pub(super) struct Fixture {
    pub temp: tempfile::TempDir,
    pub cwd: PathBuf,
    pub home: PathBuf,
    pub service: Arc<Service>,
    pub terminal: Arc<MemoryTerminal>,
    pub clock: Arc<FastClock>,
    pub app: Application,
    pub hashes: Vec<String>,
    pub trunk: String,
}
impl Fixture {
    pub(super) fn new(count: usize, trailers: bool) -> Self {
        Self::with_service(count, trailers, Service::default())
    }
    pub(super) fn with_service(count: usize, trailers: bool, service: Service) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("repo");
        let home = temp.path().join("home");
        let remote = temp.path().join("remote.git");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(home.join(".config/gh")).unwrap();
        std::fs::write(
            home.join(".config/gh/hosts.yml"),
            "github.com:\n  user: fixture\n  oauth_token: fixture-token\n",
        )
        .unwrap();
        let service = Arc::new(service);
        let terminal = Arc::new(MemoryTerminal::default());
        let clock = Arc::new(FastClock::default());
        let env = BTreeMap::from([
            ("HOME".into(), home.as_os_str().to_owned()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_AUTHOR_DATE".into(), "2025-01-01T00:00:00Z".into()),
            ("GIT_COMMITTER_DATE".into(), "2025-01-01T00:00:00Z".into()),
            ("LC_ALL".into(), "C".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ]);
        let commands = Commands {
            runner: service.clone(),
            terminal: terminal.clone(),
            cwd: cwd.clone(),
            env,
            verbose: false,
            diagnostics: crate::diagnostics::Diagnostics::default(),
        };
        commands.git(&["init", "--initial-branch=main"]).unwrap();
        commands
            .git(&["config", "user.name", "Fixture Author"])
            .unwrap();
        commands
            .git(&["config", "user.email", "fixture@example.com"])
            .unwrap();
        commands
            .git(&["config", "commit.gpgsign", "false"])
            .unwrap();
        commands.git(&["config", "log.date", "default"]).unwrap();
        commands.git(&["config", "color.ui", "false"]).unwrap();
        std::fs::write(cwd.join("base.txt"), "base\n").unwrap();
        commands.git(&["add", "base.txt"]).unwrap();
        commands.git(&["commit", "-m", "Base"]).unwrap();
        let trunk = commands.git(&["rev-parse", "HEAD"]).unwrap();
        commands
            .git(&["init", "--bare", remote.to_str().unwrap()])
            .unwrap();
        commands
            .git(&["remote", "add", "origin", remote.to_str().unwrap()])
            .unwrap();
        commands.git(&["push", "origin", "main"]).unwrap();
        commands
            .git(&[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ])
            .unwrap();
        if service.branchless {
            commands
                .git(&["branchless", "init", "--main-branch", "main"])
                .unwrap();
        }
        let app = Application {
            commands,
            clock: clock.clone(),
            transport: service.clone(),
            credentials: Arc::new(FakeCredentials),
            files: Arc::new(crate::config::files::NativeFiles),
        };
        let mut fixture = Self {
            temp,
            cwd,
            home,
            service,
            terminal,
            clock,
            app,
            hashes: Vec::new(),
            trunk,
        };
        for i in 1..=count {
            fixture.add_commit(i, trailers, "fixture@example.com");
        }
        fixture.service.trace.lock().unwrap().clear();
        fixture
    }
    pub(super) fn git(&self, args: &[&str]) -> String {
        self.app.commands.git(args).unwrap()
    }
    pub(super) fn add_commit(&mut self, i: usize, trailers: bool, email: &str) {
        let branch = format!("topic/{i}");
        self.git(&["checkout", "-b", &branch]);
        std::fs::write(
            self.cwd.join(format!("file-{i}.txt")),
            format!("content {i}\n"),
        )
        .unwrap();
        self.git(&["add", "."]);
        let message = format!(
            "Feature {i}\n\nBody {i}{}",
            if trailers {
                format!("\n\nRemote-Ref: {branch}")
            } else {
                String::new()
            }
        );
        self.git(&[
            "-c",
            &format!("user.email={email}"),
            "commit",
            "-m",
            &message,
        ]);
        self.hashes.push(self.git(&["rev-parse", "HEAD"]));
    }
    pub(super) fn run(&self, args: &[&str]) -> Result<()> {
        let flags = Flags::parse(args.iter().map(|s| (*s).into()).collect()).unwrap();
        self.app.clone().execute(
            &flags,
            &BTreeMap::from([("HOME".into(), self.home.to_string_lossy().into_owned())]),
        )
    }
    pub(super) fn output(&self) -> String {
        self.terminal.stdout.lock().unwrap().clone()
    }
    pub(super) fn trace(&self) -> Vec<Value> {
        self.service.trace.lock().unwrap().clone()
    }
    pub(super) fn write(&self, path: &str, content: &str) {
        let path = self.cwd.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}
