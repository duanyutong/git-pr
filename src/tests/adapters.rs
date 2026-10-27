use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use serde_json::json;

use super::support::{FakeCredentials, Fixture, MemoryTerminal};
use crate::{
    cli::Flags,
    commands::{CommandOutput, CommandRunner, Commands, parallel},
    config::{self, Loaded},
    credentials::{self, CredentialStore, SystemCredentials},
    git::Git,
    github::GitHub,
    github::http::{HttpStatusError, Request, Transport},
    jj,
    model::{Commit, Repository},
};

#[test]
fn metadata_existence_distinguishes_absence_from_filesystem_failure() {
    use crate::config::files::{Files, NativeFiles};
    let directory = tempfile::tempdir().unwrap();
    let metadata = directory.path().join(".jj");
    assert!(!NativeFiles.exists(&metadata).unwrap());
    std::fs::create_dir(&metadata).unwrap();
    assert!(NativeFiles.exists(&metadata).unwrap());
    std::fs::remove_dir(&metadata).unwrap();
    std::os::unix::fs::symlink(".jj", &metadata).unwrap();
    assert!(NativeFiles.exists(&metadata).is_err());
}

#[derive(Debug, Default)]
struct Script {
    responses: BTreeMap<String, (i32, String)>,
    calls: Mutex<Vec<String>>,
}

#[derive(Debug)]
struct RawPathCommands(Arc<dyn CommandRunner>);

impl CommandRunner for RawPathCommands {
    fn edit_messages(
        &self,
        _program: &str,
        _args: &[String],
        _cwd: &std::path::Path,
        _env: &std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString>,
        _messages: &[(String, String)],
    ) -> anyhow::Result<crate::commands::CommandOutput> {
        anyhow::bail!("command adapter does not support bulk message editing")
    }

    fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
    ) -> Result<CommandOutput> {
        self.0.execute(program, args, cwd, env)
    }

    fn execute_data(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
        input: &[u8],
    ) -> Result<crate::commands::StructuredOutput> {
        let mut isolated = vec!["-i".to_owned()];
        isolated.extend(
            env.iter().map(|(key, value)| {
                format!("{}={}", key.to_str().unwrap(), value.to_str().unwrap())
            }),
        );
        isolated.push(program.into());
        isolated.extend_from_slice(args);
        crate::commands::ProcessRunner::default().execute_data(
            "/usr/bin/env",
            &isolated,
            cwd,
            &BTreeMap::new(),
            input,
        )
    }
}

#[derive(Debug)]
struct Reply {
    body: &'static str,
    status: u16,
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires the disposable D-Bus session supplied by just credentials"]
fn linux_secret_service_login_collection_uses_github_attributes() {
    assert_eq!(std::env::var("GIT_PR_TEST_SECRET_SERVICE").unwrap(), "1");
    let service =
        dbus_secret_service::SecretService::connect(dbus_secret_service::EncryptionType::Plain)
            .unwrap();
    let collection = service.get_collection_by_alias("login").unwrap();
    collection.unlock().unwrap();
    let item = collection
        .create_item(
            "git-pr fixture",
            std::collections::HashMap::from([("username", ""), ("service", "gh:fixture.invalid")]),
            b"disposable-fixture-token",
            true,
            "text/plain",
        )
        .unwrap();
    assert_eq!(
        SystemCredentials
            .get("gh:fixture.invalid", &scripted(&[]))
            .unwrap(),
        "disposable-fixture-token"
    );
    item.delete().unwrap();
    assert!(
        SystemCredentials
            .get("gh:fixture.invalid", &scripted(&[]))
            .is_err()
    );
    let executable = std::env::current_exe().unwrap();
    let fixture = "tests::adapters::linux_credential_failure_fixture";
    let absent = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(&executable)
        .args(["--exact", fixture, "--nocapture"])
        .env("GIT_PR_TEST_SECRET_SERVICE_FAILURE", "connect")
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/absent.sock", absent.path().display()),
        )
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"));
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/secret_service.py"
        ))
        .arg("--deny-search")
        .arg(executable)
        .args(["--exact", fixture, "--nocapture"])
        .env("GIT_PR_TEST_SECRET_SERVICE_FAILURE", "search")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_credential_failure_fixture() {
    let Ok(mode) = std::env::var("GIT_PR_TEST_SECRET_SERVICE_FAILURE") else {
        return;
    };
    if mode == "search" {
        let service =
            dbus_secret_service::SecretService::connect(dbus_secret_service::EncryptionType::Plain)
                .unwrap();
        service
            .get_collection_by_alias("login")
            .unwrap()
            .unlock()
            .unwrap();
    } else {
        assert_eq!(mode, "connect");
    }
    let error = SystemCredentials
        .get("gh:fixture.invalid", &scripted(&[]))
        .unwrap_err();
    let Some(dbus_secret_service::Error::Dbus(error)) =
        error.downcast_ref::<dbus_secret_service::Error>()
    else {
        panic!("expected a D-Bus boundary failure: {error:#}");
    };
    if mode == "search" {
        assert_eq!(
            error.name(),
            Some("org.freedesktop.DBus.Error.AccessDenied")
        );
        assert!(error.to_string().contains("SearchItems"));
    } else {
        assert!(error.to_string().contains("absent.sock"));
    }
}

impl Transport for Reply {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        if self.status != 200 {
            return Err(HttpStatusError {
                quota: crate::github::limits::Quota::default(),
                url: request.url.into(),
                status: reqwest::StatusCode::from_u16(self.status).unwrap(),
                body: self.body.into(),
            }
            .into());
        }
        Ok(self.body.as_bytes().to_vec())
    }
}

#[derive(Debug)]
struct HeadQuery;

impl Transport for HeadQuery {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        let url = reqwest::Url::parse(request.url).unwrap();
        if url.path() == "/graphql" {
            let body: serde_json::Value = serde_json::from_slice(request.body.unwrap()).unwrap();
            assert_eq!(body["variables"]["owner"], "calendly");
            assert_eq!(body["variables"]["name"], "switchboard");
            assert_eq!(body["variables"]["h0"], "topic/a b&c");
            return Ok(serde_json::to_vec(
                &json!({"data":{"repository":{"p0":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}}),
            )?);
        }
        panic!("unexpected query: {}", url.path());
    }
}

#[test]
fn github_owner_query_and_all_draft_creation_cases() {
    let f = Fixture::new(0, true);
    let Loaded::Ready(mut config) = config::load(
        &Flags::default(),
        &BTreeMap::from([("HOME".into(), f.home.to_string_lossy().into_owned())]),
        &mut f.app.commands.clone(),
        &FakeCredentials,
        &crate::config::files::NativeFiles,
    )
    .unwrap() else {
        panic!("configuration must load")
    };
    config.repo.name = "calendly/switchboard".into();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &f.app.commands,
        transport: &HeadQuery,
        template: "",
    };
    assert_eq!(github.find_head("topic/a b&c", "").unwrap(), 0);
    config.repo.name = "org/repo".into();
    for (flag, title, expected) in [
        (false, "Feature", false),
        (true, "Feature", true),
        (false, "WIP: pending", true),
        (true, "WIP: pending", true),
    ] {
        config.draft = flag;
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &f.app.commands,
            transport: f.service.as_ref(),
            template: "",
        };
        let mut commit = Commit {
            title: title.into(),
            ..cm()
        };
        github.create(&mut commit, "main").unwrap();
        assert_eq!(
            f.service.prs.lock().unwrap()[&commit.pr_number]["draft"],
            expected
        );
    }
}

#[test]
fn github_lookup_fallback_errors_creation_and_status_diagnostics() {
    let f = Fixture::new(0, true);
    let Loaded::Ready(mut config) = config::load(
        &Flags::default(),
        &BTreeMap::from([("HOME".into(), f.home.to_string_lossy().into_owned())]),
        &mut f.app.commands.clone(),
        &FakeCredentials,
        &crate::config::files::NativeFiles,
    )
    .unwrap() else {
        panic!("configuration must load")
    };
    config.draft = false;
    assert!(config.stack_up);
    assert!(!config.emoji);
    assert!(!config.github_stack);
    let mut commit = cm();
    commit.attrs.clear();
    for (reply, output, expected) in [
        (
            Reply {
                body: r#"[{"number":17,"state":"open","head":{"ref":"topic","repo":{"full_name":"org/repo"}}}]"#,
                status: 200,
            },
            "PR #17\tFeature\tOPEN",
            17,
        ),
        (
            Reply {
                body: "No commit found",
                status: 404,
            },
            "42 Feature",
            0,
        ),
        (
            Reply {
                body: "[]",
                status: 200,
            },
            "no matches",
            0,
        ),
    ] {
        let commands = scripted(&[("gh pr list --limit=1 --search in:title Feature", 0, output)]);
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &commands,
            transport: &reply,
            template: "Template",
        };
        assert_eq!(github.find(&commit).unwrap(), expected);
        assert_eq!(github.find_head("", "").unwrap(), 0);
        assert_eq!(github.lookup(&commit).unwrap(), 0);
    }
    for reply in [
        Reply {
            body: "API unavailable",
            status: 500,
        },
        Reply {
            body: "not json",
            status: 200,
        },
    ] {
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &f.app.commands,
            transport: &reply,
            template: "",
        };
        assert!(github.find(&commit).is_err());
        let error = format!("{:#}", github.get(1).unwrap_err());
        assert!(if reply.status == 500 {
            error.contains("500 Internal Server Error") && error.contains("API unavailable")
        } else {
            error.contains("invalid GitHub response for PR #1")
        });
        assert!(github.find_head("topic", "").is_err());
        assert!(github.lookup(&cm()).is_err());
    }
    assert!(!f.output().contains("500 Internal Server Error"));
    assert!(!f.output().contains("API unavailable"));
    let commands = scripted(&[(
        "gh pr list --limit=1 --search in:title Feature",
        0,
        "18446744073709551616",
    )]);
    let reply = Reply {
        body: "[]",
        status: 200,
    };
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: &reply,
        template: "",
    };
    assert_eq!(github.find(&commit).unwrap(), 0);
    commit.pr_number = 9;
    assert_eq!(github.lookup(&commit).unwrap(), 9);
    assert_eq!(github.find(&commit).unwrap(), 9);

    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &f.app.commands,
        transport: f.service.as_ref(),
        template: "Cached template",
    };
    let mut commit = cm();
    assert_eq!(github.number(&mut commit, "main").unwrap(), 101);
    assert!(commit.newly_created);
    assert_eq!(
        f.service.prs.lock().unwrap()[&101]["body"],
        "Cached template"
    );
    commit.pr_number = 0;
    commit.pr_number = github.find(&commit).unwrap();
    github.verify_base(&commit, "main").unwrap();
    assert_eq!(commit.pr_number, 101);
    let mut unknown = cm();
    unknown.base_blocked = true;
    github.warn_blocked(&[unknown]);
    assert!(f.terminal.stderr.lock().unwrap().contains("(unknown)"));

    let commands = scripted(&[(
        "gh pr create --title Feature --body Template --head topic --base main",
        0,
        "invalid URL",
    )]);
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: &reply,
        template: "Template",
    };
    assert!(
        github
            .create(&mut cm(), "main")
            .unwrap_err()
            .to_string()
            .contains("invalid PR creation response")
    );
    for expected in [123, 456, 99999, 789] {
        let body = json!({"number":expected,"node_id":format!("PR_{expected}"),"title":"Feature","body":"Template","labels":[],"state":"open","merged":false,"head":{"ref":"topic","repo":{"full_name":"org/repo"}},"base":{"ref":"main","repo":{"full_name":"org/repo"}}}).to_string();
        let reply = Reply {
            body: Box::leak(body.into_boxed_str()),
            status: 200,
        };
        let github = GitHub {
            session: crate::github::Session::default(),
            transport: &reply,
            ..github
        };
        let mut commit = cm();
        github.create(&mut commit, "main").unwrap();
        assert_eq!(commit.pr_number, expected);
        assert!(commit.newly_created);
    }
}

#[test]
fn git_stack_empty_titles_empty_commits_and_backend_diagnostics() {
    let hash = "a".repeat(40);
    for (body, files, expected, accepted) in [
        (
            "\n    Body only",
            "",
            "empty title and no file changes",
            true,
        ),
        (
            "\n    Body only",
            "changed.txt",
            "empty title but contains file changes",
            false,
        ),
        ("Empty feature", "", "has no file changes", true),
    ] {
        let log = format!("{hash}\0A\0a@example.com\02025-01-01T00:00:00Z\0tree\0\0{body}\0");
        let terminal = Arc::new(MemoryTerminal::default());
        let mut commands = scripted(&[
            (
                "git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature main..HEAD",
                0,
                &log,
            ),
            (
                &format!("git diff-tree --no-commit-id --name-only -r {hash}"),
                0,
                files,
            ),
            (
                "git show --no-show-signature -s --format=%H %T main",
                0,
                "base tree",
            ),
        ]);
        commands.terminal = terminal.clone();
        let repo = Repository::default();
        let result = Git {
            commands: &commands,
            repo: &repo,
        }
        .stack("main", "HEAD", false);
        assert_eq!(result.is_ok(), accepted);
        assert!(terminal.stdout.lock().unwrap().contains(expected));
    }
    let repo = Repository {
        jj: true,
        ..Repository::default()
    };
    let log = format!("{hash}\0A\0a@example.com\02025-01-01T00:00:00Z\0tree\0\0Feature\0");
    let mut commands = scripted(&[
        (
            "jj --ignore-working-copy op log --limit 1 --no-graph -T id",
            0,
            "op",
        ),
        (
            &format!(
                "jj --at-operation op log -r {hash} --no-graph -T commit_id ++ \" \" ++ change_id ++ \" \" ++ conflict ++ \"\\n\""
            ),
            0,
            &format!("{hash} change false"),
        ),
        (
            "git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature main..HEAD",
            0,
            &log,
        ),
        (
            &format!("git diff-tree --no-commit-id --name-only -r {hash}"),
            0,
            "changed.txt",
        ),
        (
            "git show --no-show-signature -s --format=%H %T main",
            0,
            "base tree",
        ),
        (
            &format!("jj --ignore-working-copy log -r {hash} --no-graph -T change_id"),
            1,
            "unavailable",
        ),
        (
            "jj --ignore-working-copy log -r @ --no-graph -T conflict ++ \"\\0\" ++ if(empty, \"EMPTY\", \"NONEMPTY\") ++ \"|\" ++ if(description, \"HAS-DESC\", \"NO-DESC\") ++ \"\\0\" ++ author.name() ++ \"\\0\" ++ author.email() ++ \"\\0\" ++ change_id ++ \"|\" ++ commit_id ++ \"|\" ++ description",
            1,
            "unavailable",
        ),
        ("git rev-list --count main..HEAD", 0, "invalid count"),
        (
            "git for-each-ref --contains missing --format=%(objectname) refs/heads/",
            1,
            "missing",
        ),
        (
            "git for-each-ref --contains main --format=%(objectname) refs/heads/",
            1,
            "unavailable",
        ),
    ]);
    commands.verbose = true;
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    assert!(git.stack("main", "HEAD", true).is_err());
    assert!(
        git.depth("main", "HEAD")
            .unwrap_err()
            .to_string()
            .contains("failed to parse commit count")
    );
    assert!(git.stack_tip("missing").is_err());
    assert!(git.stack_tip("main").is_err());
}
impl CommandRunner for Script {
    fn edit_messages(
        &self,
        _program: &str,
        _args: &[String],
        _cwd: &std::path::Path,
        _env: &std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString>,
        _messages: &[(String, String)],
    ) -> anyhow::Result<crate::commands::CommandOutput> {
        anyhow::bail!("command adapter does not support bulk message editing")
    }

    fn execute(
        &self,
        program: &str,
        args: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
    ) -> Result<CommandOutput> {
        let key = format!("{program} {}", args.join(" "));
        self.calls.lock().unwrap().push(key.clone());
        let fallback = (0, String::new());
        let (status, text) = self.responses.get(&key).unwrap_or_else(|| {
            if key.starts_with("git show --no-show-signature -s --format=%H %T ") {
                return &fallback;
            }
            panic!("Unscripted command: {key}")
        });
        Ok(CommandOutput {
            status: *status,
            text: text.clone(),
        })
    }
}
fn scripted(entries: &[(&str, i32, &str)]) -> Commands {
    Commands {
        runner: Arc::new(Script {
            responses: entries
                .iter()
                .map(|(k, s, v)| ((*k).into(), (*s, (*v).into())))
                .collect(),
            ..Script::default()
        }),
        terminal: Arc::new(MemoryTerminal::default()),
        cwd: std::env::temp_dir(),
        env: BTreeMap::new(),
        verbose: false,
        diagnostics: crate::diagnostics::Diagnostics::default(),
    }
}
fn cm() -> Commit {
    Commit {
        hash: "2222222222222222222222222222222222222222".into(),
        change_id: "changeid".into(),
        title: "Feature".into(),
        attrs: vec![("remote-ref".into(), "topic".into())],
        ..Commit::default()
    }
}

#[test]
fn command_errors_parallel_barriers_and_verbose_diagnostics() {
    assert_eq!(
        CommandOutput {
            status: 3,
            text: String::new()
        }
        .success()
        .unwrap_err()
        .to_string(),
        "exit code 3"
    );
    assert_eq!(
        CommandOutput {
            status: -1,
            text: " failed \n".into()
        }
        .success()
        .unwrap_err()
        .to_string(),
        "exit code -1\nfailed"
    );
    let mut commands = scripted(&[
        ("git test with spaces", 0, "one\ntwo\nthree"),
        ("gh test", 1, "broken"),
    ]);
    commands.verbose = true;
    assert_eq!(
        commands.git(&["test", "with spaces"]).unwrap(),
        "one\ntwo\nthree"
    );
    assert!(commands.run("gh", &["test"]).is_err());
    commands.debug("");
    commands.debug("one");
    commands.debug("\n");
    let completed = Mutex::new(Vec::new());
    assert!(
        parallel(vec![1, 2, 3], |i| {
            completed.lock().unwrap().push(i);
            if i == 2 {
                anyhow::bail!("failed")
            }
            Ok(i)
        })
        .is_err()
    );
    assert_eq!(completed.lock().unwrap().len(), 3);
    assert!(parallel(vec![1], |_| -> Result<()> { panic!("worker") }).is_err());
}

#[test]
fn native_credential_encoding_and_redacted_debug() {
    for (input, want) in [
        (" token \n", "token"),
        ("go-keyring-base64:dG9rZW4=", "token"),
        ("go-keyring-encoded:746f6b656e", "token"),
    ] {
        assert_eq!(credentials::keychain::decode_keychain(input).unwrap(), want);
    }
    for bad in [
        "go-keyring-base64:!",
        "go-keyring-encoded:f",
        "go-keyring-encoded:gg",
        "go-keyring-encoded:ff",
    ] {
        assert!(credentials::keychain::decode_keychain(bad).is_err());
    }
    assert_eq!(
        format!("{:?}", credentials::Secret("hidden".into())),
        "[redacted]"
    );
    #[cfg(target_os = "macos")]
    {
        let commands = scripted(&[(
            "/usr/bin/security find-generic-password -s gh:github.com -wa ",
            0,
            "go-keyring-base64:dG9rZW4=",
        )]);
        assert_eq!(
            SystemCredentials.get("gh:github.com", &commands).unwrap(),
            "token"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_credential_command_failure_is_propagated_without_decoding_its_output() {
    let mut commands = scripted(&[(
        "/usr/bin/security find-generic-password -s gh:fixture.invalid -wa ",
        1,
        "fixture keychain is locked",
    )]);
    commands.verbose = true;
    let error = SystemCredentials
        .get("gh:fixture.invalid", &commands)
        .unwrap_err();
    assert!(format!("{error:#}").contains("fixture keychain is locked"));
}

#[test]
fn credential_file_missing_host_malformed_and_empty_token() {
    #[derive(Debug)]
    struct Empty;
    impl CredentialStore for Empty {
        fn get(&self, _: &str, _: &Commands) -> Result<String> {
            anyhow::bail!("not found")
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hosts.yml");
    let commands = scripted(&[]);
    assert!(
        credentials::load(
            &path,
            "github.com",
            &Empty,
            &commands,
            &crate::config::files::NativeFiles
        )
        .unwrap_err()
        .to_string()
        .contains("failed to load")
    );
    for body in ["{}", "github.com: null"] {
        std::fs::write(&path, body).unwrap();
        assert!(
            credentials::load(
                &path,
                "github.com",
                &Empty,
                &commands,
                &crate::config::files::NativeFiles
            )
            .unwrap_err()
            .to_string()
            .contains("no GitHub config")
        );
    }
    std::fs::write(&path, "github.com: [").unwrap();
    assert!(
        credentials::load(
            &path,
            "github.com",
            &Empty,
            &commands,
            &crate::config::files::NativeFiles
        )
        .is_err()
    );
    std::fs::write(&path, "github.com:\n  user: fixture\n").unwrap();
    assert!(
        credentials::load(
            &path,
            "github.com",
            &Empty,
            &commands,
            &crate::config::files::NativeFiles
        )
        .unwrap_err()
        .to_string()
        .contains("failed to read the credential store")
    );
    assert_eq!(
        credentials::load(
            &path,
            "github.com",
            &FakeCredentials,
            &commands,
            &crate::config::files::NativeFiles
        )
        .unwrap()
        .0,
        "fixture-token"
    );
}

fn loaded_config(fixture: &Fixture) -> Box<config::Config> {
    match config::load(
        &Flags::default(),
        &BTreeMap::from([("HOME".into(), fixture.home.to_string_lossy().into_owned())]),
        &mut fixture.app.commands.clone(),
        &FakeCredentials,
        &crate::config::files::NativeFiles,
    )
    .unwrap()
    {
        Loaded::Ready(config) => config,
        Loaded::TagsSaved => panic!("unexpected early exit"),
    }
}

#[test]
fn repository_roots_preserve_significant_trailing_path_whitespace() {
    for suffix in [" ", "\n", " \t\n"] {
        let mut fixture = Fixture::new(0, true);
        let root = fixture.temp.path().join(format!("repository{suffix}"));
        std::fs::rename(&fixture.cwd, &root).unwrap();
        fixture.cwd = root.clone();
        fixture.app.commands.cwd = root.clone();
        fixture.app.commands.runner = Arc::new(RawPathCommands(fixture.service.clone()));
        fixture.write(
            ".github/PULL_REQUEST_TEMPLATE.md",
            "Exact repository template\n",
        );

        let config = loaded_config(&fixture);

        assert_eq!(config.repo.root, root.canonicalize().unwrap());
        assert_eq!(
            config::load_template(&config.repo.root, &crate::config::files::NativeFiles).unwrap(),
            "Exact repository template\n"
        );
        std::os::unix::fs::symlink(".jj", root.join(".jj")).unwrap();
        let error = config::load(
            &Flags::default(),
            &BTreeMap::new(),
            &mut fixture.app.commands.clone(),
            &FakeCredentials,
            &crate::config::files::NativeFiles,
        )
        .err()
        .unwrap();
        assert!(
            error
                .to_string()
                .contains("could not inspect Jujutsu workspace")
        );
    }
}

#[test]
fn git_configuration_scopes_includes_and_empty_overrides() {
    let mut f = Fixture::new(0, true);
    let system = f.home.join("system.gitconfig");
    let global = f.home.join("user.gitconfig");
    std::fs::write(
        &system,
        "[GIT-PR]\nOUTPUT = url\nADDLABEL = system\nHYPERLINKS = never\nPREVIEWLIMIT = 6\n",
    )
    .unwrap();
    std::fs::write(&global, "").unwrap();
    f.app
        .commands
        .env
        .remove(&OsString::from("GIT_CONFIG_NOSYSTEM"));
    f.app
        .commands
        .env
        .insert("GIT_CONFIG_SYSTEM".into(), system.into_os_string());
    f.app
        .commands
        .env
        .insert("GIT_CONFIG_GLOBAL".into(), global.clone().into_os_string());
    assert_eq!(loaded_config(&f).labels, ["system"]);
    assert_eq!(loaded_config(&f).output, "{url}");
    assert_eq!(loaded_config(&f).preview_limit, 6);
    assert_eq!(
        loaded_config(&f).hyperlinks,
        crate::cli::hyperlinks::Mode::Never
    );

    std::fs::write(f.home.join("included config"), "[git-pr]\noutput = markdown\naddLabel = global, review\nstackUp = OFF\ndefaultDraft = YES\nstackEmoji = on\nskipDraft = off\nbranchFromTitle = 1\nhyperlinks = always\n").unwrap();
    std::fs::write(
        f.home.join("conditional"),
        "[git-pr]\naddTipLabel = tip\nhyperlinks = auto\npreviewLimit = 12\n",
    )
    .unwrap();
    std::fs::write(&global, format!("[include]\npath = \"included config\"\n[includeIf \"gitdir:{}/\"]\npath = conditional\n", f.cwd.canonicalize().unwrap().display())).unwrap();
    let c = loaded_config(&f);
    assert_eq!(c.labels, ["global", "review"]);
    assert_eq!(c.tip_labels, ["tip"]);
    assert_eq!(c.hyperlinks, crate::cli::hyperlinks::Mode::Auto);
    assert_eq!(c.preview_limit, 12);
    assert_eq!(c.output, "[#{number}]({url}) {title}");
    assert!(!c.stack_up && c.draft && c.emoji && !c.skip_draft);

    f.git(&["config", "git-pr.hyperlinks", "never"]);
    f.git(&["config", "git-pr.previewLimit", "30"]);
    assert_eq!(loaded_config(&f).preview_limit, 30);
    assert_eq!(
        loaded_config(&f).hyperlinks,
        crate::cli::hyperlinks::Mode::Never
    );
    f.git(&["config", "git-pr.addLabel", ""]);
    f.git(&["config", "git-pr.addTipLabel", ""]);
    f.git(&["config", "git-pr.output", ""]);
    let c = loaded_config(&f);
    assert!(c.labels.is_empty() && c.tip_labels.is_empty());
    assert_eq!(
        c.output,
        crate::cli::output::resolve_format("url-title").unwrap()
    );
    f.git(&["config", "--add", "git-pr.addLabel", "earlier"]);
    f.git(&["config", "--add", "Git-Pr.AddLabel", "local, needs review"]);
    f.git(&["config", "git-pr.output", "url"]);
    f.git(&["config", "git-pr.stackUp", "ON"]);
    f.git(&["config", "git-pr.skipDraft", "YES"]);
    let c = loaded_config(&f);
    assert_eq!(c.labels, ["local", "needs review"]);
    assert_eq!(c.output, "{url}");
    assert!(c.stack_up && c.skip_draft);

    f.git(&["config", "extensions.worktreeConfig", "true"]);
    f.git(&["config", "--worktree", "git-pr.output", "markdown"]);
    f.git(&["config", "--worktree", "git-pr.hyperlinks", "always"]);
    f.git(&["config", "--worktree", "git-pr.previewLimit", "40"]);
    assert_eq!(loaded_config(&f).preview_limit, 40);
    assert_eq!(
        loaded_config(&f).hyperlinks,
        crate::cli::hyperlinks::Mode::Always
    );
    assert_eq!(loaded_config(&f).output, "[#{number}]({url}) {title}");
    let nested = f.cwd.join("nested/deep");
    std::fs::create_dir_all(&nested).unwrap();
    f.app.commands.cwd = nested;
    assert_eq!(loaded_config(&f).output, "[#{number}]({url}) {title}");
    f.app.commands.env.extend([
        ("GIT_CONFIG_COUNT".into(), "3".into()),
        ("GIT_CONFIG_KEY_1".into(), "git-pr.hyperlinks".into()),
        ("GIT_CONFIG_VALUE_1".into(), "auto".into()),
        ("GIT_CONFIG_KEY_2".into(), "git-pr.previewLimit".into()),
        ("GIT_CONFIG_VALUE_2".into(), "0".into()),
        ("GIT_CONFIG_KEY_0".into(), "git-pr.output".into()),
        ("GIT_CONFIG_VALUE_0".into(), "url".into()),
    ]);
    assert_eq!(loaded_config(&f).output, "{url}");
    assert_eq!(loaded_config(&f).preview_limit, 0);
    assert_eq!(
        loaded_config(&f).hyperlinks,
        crate::cli::hyperlinks::Mode::Auto
    );
}

#[test]
fn git_configuration_rejects_invalid_values_and_command_failures() {
    let f = Fixture::new(0, true);
    for key in [
        "git-pr.githubStack",
        "git-pr.stackUp",
        "git-pr.defaultDraft",
        "git-pr.stackEmoji",
        "git-pr.skipDraft",
        "git-pr.branchFromTitle",
    ] {
        f.git(&["config", key, "invalid"]);
        let error = format!("{:#}", f.run(&["--stop-after=validate"]).unwrap_err());
        assert!(
            error.contains(key) && error.contains("bad boolean"),
            "{error}"
        );
        f.git(&["config", "--unset", key]);
    }
    f.git(&["config", "git-pr.output", "invalid"]);
    assert!(format!("{:#}", f.run(&[]).unwrap_err()).contains("invalid --output"));
    f.git(&["config", "--unset", "git-pr.output"]);
    let bulk_args = f
        .trace()
        .into_iter()
        .find(|e| {
            e["program"] == "git" && e["args"][1] == "--null" && e["args"][2] == "--get-regexp"
        })
        .unwrap()["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    for status in [1, 3, 128, 199] {
        f.service.command_errors.lock().unwrap().insert(
            format!("git {bulk_args}"),
            (status, "configuration unavailable".into()),
        );
        let error = format!("{:#}", f.run(&["--stop-after=validate"]).unwrap_err());
        assert!(error.contains("configuration unavailable"), "{error}");
    }
    f.service.command_errors.lock().unwrap().clear();
    std::fs::write(f.cwd.join(".git/config"), "[invalid\n").unwrap();
    assert!(f.run(&["--stop-after=validate"]).is_err());
}

#[test]
fn standalone_configuration_files_are_not_application_settings() {
    let f = Fixture::new(0, true);
    for (path, body) in [
        (
            ".config/git-pr.json",
            r#"{"add_label":["ignored"],"output":"bad"}"#,
        ),
        (".config/git-pr.yaml", "add_label: [ignored]\noutput: bad\n"),
        (".config/git-pr.local.yml", "add_tip_label: [ignored]\n"),
    ] {
        f.write(path, body);
    }
    std::fs::create_dir_all(f.home.join(".config/git-pr")).unwrap();
    std::fs::write(f.home.join(".config/git-pr/config.yaml"), "output: bad\n").unwrap();
    let c = loaded_config(&f);
    assert!(c.labels.is_empty() && c.tip_labels.is_empty());
    assert_eq!(
        c.output,
        crate::cli::output::resolve_format("url-title").unwrap()
    );
    assert_eq!(
        config::paths::expand_path("~/hosts", "/home/me"),
        Path::new("/home/me/hosts")
    );
    assert_eq!(config::paths::expand_path("", "home"), Path::new(""));
}

#[test]
fn templates_precedence_and_submission_cache_boundary() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        config::load_template(temp.path(), &crate::config::files::NativeFiles).unwrap(),
        crate::stack::description::BODY_TEMPLATE
    );
    let paths = [
        ".github/PULL_REQUEST_TEMPLATE.md",
        ".github/pull_request_template.md",
        "docs/PULL_REQUEST_TEMPLATE.md",
        "docs/pull_request_template.md",
        "PULL_REQUEST_TEMPLATE.md",
        "pull_request_template.md",
    ];
    for path in paths.iter().rev() {
        let file = temp.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, path).unwrap();
        assert_eq!(
            config::load_template(temp.path(), &crate::config::files::NativeFiles).unwrap(),
            *path
        );
    }
    // The application resolves the template once and lends that snapshot to every PR operation.
    let cached = config::load_template(temp.path(), &crate::config::files::NativeFiles).unwrap();
    std::fs::write(temp.path().join(paths[0]), "changed").unwrap();
    assert_eq!(cached, paths[0]);
}

#[test]
fn config_flags_git_values_environment_and_identity_failures() {
    let f = Fixture::new(1, true);
    for (key, value) in [
        ("git-pr.githubStack", "true"),
        ("git-pr.stackUp", "false"),
        ("git-pr.defaultDraft", "false"),
        ("git-pr.stackEmoji", "true"),
        ("git-pr.branchFromTitle", "true"),
        ("git-pr.skipDraft", "true"),
        ("git-pr.draftPattern", "topic:*"),
        ("git-pr.output", "markdown"),
        ("git-pr.addLabel", "configured"),
        ("git-pr.addTipLabel", "configured-tip"),
    ] {
        // Git owns case-insensitive matching for both section and variable names.
        f.git(&["config", &key.to_ascii_uppercase(), value]);
    }
    let load = |args: Vec<&str>, extra: Vec<(&str, &str)>| {
        let flags = Flags::parse(args.into_iter().map(str::to_owned).collect()).unwrap();
        let mut env = BTreeMap::from([("HOME".into(), f.home.to_string_lossy().into_owned())]);
        env.extend(extra.into_iter().map(|(k, v)| (k.into(), v.into())));
        match config::load(
            &flags,
            &env,
            &mut f.app.commands.clone(),
            &FakeCredentials,
            &crate::config::files::NativeFiles,
        )
        .unwrap()
        {
            Loaded::Ready(c) => c,
            Loaded::TagsSaved => panic!("unexpected early exit"),
        }
    };
    let c = load(vec![], vec![]);
    assert!(c.github_stack && c.emoji && c.skip_draft);
    assert!(!c.stack_up && !c.draft);
    assert!(c.draft_patterns.matches("TOPIC: value"));
    assert!(!c.draft_patterns.matches("WIP: value"));
    assert_eq!(c.output, "[#{number}]({url}) {title}");
    assert_eq!(c.labels, ["configured"]);
    assert_eq!(c.tip_labels, ["configured-tip"]);
    let c = load(
        vec![
            "--stack-up=true",
            "--draft=true",
            "--github-stack=false",
            "--include-draft",
            "--branch-from-title",
            "--output=url",
            "--add-label=cli",
            "--add-tip-label=cli-tip",
        ],
        vec![("GIT_PR_GITHUB_STACK", "1")],
    );
    assert!(c.stack_up && c.draft && c.include_draft);
    assert!(!c.github_stack);
    assert_eq!(c.output, "{url}");
    assert_eq!(c.labels, ["cli"]);
    assert_eq!(c.tip_labels, ["cli-tip"]);
    let c = load(
        vec!["--no-stack=false", "--dry-run=false"],
        vec![
            ("GIT_PR_NO_STACK", "1"),
            ("GIT_PR_DRY_RUN", "1"),
            ("GIT_PR_YES", "1"),
            ("GIT_PR_OUTPUT", "url"),
            ("GIT_PR_INCLUDE_DRAFT", "1"),
            ("GIT_PR_STOP_AFTER", "rewrite"),
        ],
    );
    assert!(c.no_stack && c.dry_run && c.auto_accept && c.include_draft);
    assert_eq!(c.output, "{url}");
    assert_eq!(c.stop_after, Some(config::checkpoint::Checkpoint::Rewrite));
    for (cmd, out, message) in [
        ("git config --get user.name", "", "user.name not found"),
        ("git config --get user.email", "", "user.email not found"),
        (
            "git remote -v",
            "origin /local/repo (push)",
            "failed to parse remote url",
        ),
        (
            "git symbolic-ref refs/remotes/origin/HEAD",
            "refs/remotes/origin/",
            "failed to detect remote trunk",
        ),
    ] {
        let original = cmd.strip_prefix("git config --get ").map(|key| {
            let value = f.git(&["config", "--get", key]);
            f.git(&["config", "--replace-all", key, out]);
            (key, value)
        });
        if original.is_none() {
            f.service
                .command_errors
                .lock()
                .unwrap()
                .insert(cmd.into(), (0, out.into()));
        }
        assert!(
            f.run(&["--stop-after=validate"])
                .unwrap_err()
                .to_string()
                .contains(message)
        );
        f.service.command_errors.lock().unwrap().remove(cmd);
        if let Some((key, value)) = original {
            f.git(&["config", "--replace-all", key, &value]);
        }
    }
}

#[test]
fn jj_revision_and_change_id_resolution_errors() {
    for (output, valid) in [("", false), ("hash", true), ("a\nb", false)] {
        let commands = scripted(&[(
            "jj --ignore-working-copy log -r rev --no-graph -T commit_id ++ \"\\n\"",
            0,
            output,
        )]);
        assert_eq!(jj::resolve_revision(&commands, "rev").is_ok(), valid);
    }
    let commands = scripted(&[(
        "jj --ignore-working-copy log -r hash --no-graph -T change_id",
        0,
        "status\nchange-id\n",
    )]);
    assert_eq!(jj::change_id(&commands, "hash").unwrap(), "change-id");
    let commands = scripted(&[(
        "jj --ignore-working-copy log -r hash --no-graph -T change_id",
        0,
        "",
    )]);
    assert!(jj::change_id(&commands, "hash").is_err());
    for (output, clean) in [
        ("EMPTY|NO-DESC", true),
        ("NONEMPTY|HAS-DESC", true),
        ("NONEMPTY|NO-DESC", false),
        ("bad", false),
        ("a|b|c", false),
    ] {
        assert_eq!(jj::policy::clean_status(output), clean);
    }
}

#[test]
fn git_ref_validation_rewrite_failures_and_missing_local_branch() {
    let commands = scripted(&[
        ("git check-ref-format --branch bad ref", 1, "invalid"),
        (
            "git branch --format=%(refname:strip=2)|%(objectname)",
            0,
            "(HEAD detached at 123)|123\nmain|123\nbranch|abcdef\nmalformed",
        ),
    ]);
    let repo = Repository::default();
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    for reference in [" bad", "bad ", "(HEAD detached at abc)", "bad ref"] {
        assert!(Git::validate_remote_ref(reference).is_err());
    }
    assert_eq!(
        git.local_branch(
            &cm(),
            &commands
                .git(&["branch", "--format=%(refname:strip=2)|%(objectname)"])
                .unwrap()
        ),
        ""
    );
    assert!(
        git.reword(&cm())
            .unwrap_err()
            .to_string()
            .contains("neither jj nor git-branchless")
    );
    let repo = Repository { jj: true, ..repo };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let commit = Commit {
        change_id: String::new(),
        ..cm()
    };
    assert!(
        git.reword(&commit)
            .unwrap_err()
            .to_string()
            .contains("has no change ID")
    );
}

#[test]
fn local_ref_reconciliation_guards_checked_out_and_unrelated_refs() {
    let old = "1111111111111111111111111111111111111111";
    let new = cm().hash;
    for (ancestor, checked, update) in [(1, false, 0), (0, true, 0), (0, false, 0), (0, false, 1)] {
        let commands = scripted(&[
            (
                "git worktree list --porcelain",
                0,
                if checked {
                    "branch refs/heads/topic"
                } else {
                    ""
                },
            ),
            (
                "git for-each-ref --format=%(refname:strip=2)|%(objectname) refs/heads/",
                0,
                &format!("topic|{old}"),
            ),
            (
                &format!("git merge-base --is-ancestor {old} {new}"),
                ancestor,
                "",
            ),
            (
                &format!("git update-ref refs/heads/topic {new} {old}"),
                update,
                "failed",
            ),
        ]);
        Git {
            commands: &commands,
            repo: &Repository::default(),
        }
        .sync_refs(&[cm()]);
    }
    let commands = scripted(&[
        ("git worktree list --porcelain", 1, "failed"),
        (
            "git for-each-ref --format=%(refname:strip=2)|%(objectname) refs/heads/",
            0,
            &format!("topic|{new}"),
        ),
        (&format!("git merge-base --is-ancestor {new} {new}"), 0, ""),
    ]);
    Git {
        commands: &commands,
        repo: &Repository::default(),
    }
    .sync_refs(&[cm(), Commit { skip: true, ..cm() }, Commit::default()]);
}

#[test]
fn jj_bookmark_reconciliation_covers_conflicts_stale_exports_and_failures() {
    let old = "1111111111111111111111111111111111111111";
    let new = cm().hash;
    for (targets, change, ancestor, set_status, export_status, export_output) in [
        (String::new(), "", 0, 0, 0, ""),
        (format!("{old}\n{new}"), "", 0, 0, 0, ""),
        (old.to_owned(), "different", 1, 0, 0, ""),
        (old.to_owned(), "changeid", 1, 1, 0, ""),
        (old.to_owned(), "changeid", 1, 0, 1, "failed"),
        (
            old.to_owned(),
            "changeid",
            1,
            0,
            0,
            "Failed to export bookmarks",
        ),
        (new.clone(), "", 0, 0, 0, ""),
    ] {
        let observation = if targets.is_empty() {
            String::new()
        } else {
            format!(
                "topic\0{}\0{}\n",
                u8::from(targets.contains('\n')),
                targets
                    .lines()
                    .map(|oid| format!("{oid} {change}"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let commands = scripted(&[
            ("jj git import", 0, ""),
            (
                "jj --ignore-working-copy bookmark list -T name ++ \"\\0\" ++ if(conflict, \"1\", \"0\") ++ \"\\0\" ++ added_targets.map(|c| c.commit_id() ++ \" \" ++ c.change_id()).join(\",\") ++ \"\\n\"",
                0,
                &observation,
            ),
            (
                "jj --ignore-working-copy log -r bookmarks(exact:\"topic\") --no-graph -T commit_id ++ \"\\n\"",
                0,
                &targets,
            ),
            (
                "git for-each-ref --format=%(refname:strip=2)|%(objectname) refs/heads/",
                0,
                &format!("topic|{old}"),
            ),
            (
                &format!("jj --ignore-working-copy log -r {old} --no-graph -T change_id"),
                0,
                change,
            ),
            (
                &format!("git merge-base --is-ancestor {old} {new}"),
                ancestor,
                "",
            ),
            (
                &format!("jj bookmark set topic -r {new} --allow-backwards"),
                set_status,
                "failed",
            ),
            ("jj git export", export_status, export_output),
        ]);
        Git {
            commands: &commands,
            repo: &Repository {
                jj: true,
                ..Repository::default()
            },
        }
        .sync_refs(&[cm()]);
    }
}

#[test]
fn http_transport_serialisation_authentication_status_and_timeouts() {
    use crate::github::http::{HttpTransport, Request, Transport};
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for status in ["200 OK", "204 No Content", "429 Too Many Requests"] {
            let (mut stream, _) = server.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut data = Vec::new();
            let mut byte = [0];
            while !data.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                data.push(byte[0]);
            }
            let header = String::from_utf8(data).unwrap();
            let length = header
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length: ")
                        .map(|s| s.parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            requests.push((header, body));
            let body = if status.starts_with("204") {
                ""
            } else {
                "{\"message\":\"fixture\"}"
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        requests
    });
    let transport = HttpTransport::default();
    let url = format!("http://{address}/pulls/1");
    let body = json!({"title":"Feature","body":"Text"});
    assert_eq!(
        transport
            .request(Request {
                method: "GET",
                url: &url,
                token: "secret",
                body: None,
                timeout: 2
            })
            .unwrap(),
        br#"{"message":"fixture"}"#
    );
    assert_eq!(
        transport
            .request(Request {
                method: "PATCH",
                url: &url,
                token: "secret",
                body: Some(&serde_json::to_vec(&body).unwrap()),
                timeout: 2
            })
            .unwrap(),
        Vec::<u8>::new()
    );
    let error = transport
        .request(Request {
            method: "GET",
            url: &url,
            token: "secret",
            body: None,
            timeout: 2,
        })
        .unwrap_err();
    assert!(error.to_string().contains("429 Too Many Requests"));
    assert!(
        error
            .downcast_ref::<crate::github::http::HttpStatusError>()
            .is_some()
    );
    let requests = worker.join().unwrap();
    assert!(
        requests[0]
            .0
            .to_lowercase()
            .contains("authorization: bearer secret")
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&requests[1].1).unwrap(),
        body
    );
    assert!(
        transport
            .request(Request {
                method: "GET",
                url: &url,
                token: "secret",
                body: None,
                timeout: 0
            })
            .unwrap_err()
            .to_string()
            .contains("deadline")
    );
    assert!(
        transport
            .request(Request {
                method: "GET",
                url: "invalid",
                token: "secret",
                body: None,
                timeout: 1
            })
            .is_err()
    );
    assert!(
        transport
            .request(Request {
                method: "bad method",
                url: &url,
                token: "secret",
                body: None,
                timeout: 1
            })
            .is_err()
    );
    assert!(
        transport
            .request(Request {
                method: "GET",
                url: &url,
                token: "secret",
                body: None,
                timeout: 1
            })
            .is_err()
    );
    assert!(
        !format!(
            "{:?}",
            Request {
                method: "GET",
                url: &url,
                token: "secret",
                body: None,
                timeout: 1
            }
        )
        .contains("secret")
    );
}

#[test]
fn batched_configuration_preserves_shadowed_invalid_values_and_cli_overrides() {
    let f = Fixture::new(0, true);
    for (key, value) in [("git-pr.stackUp", "YES"), ("git-pr.commandTimeout", "2k")] {
        f.git(&["config", "--add", key, "1"]);
        f.git(&["config", "--add", key, value]);
    }
    assert!(loaded_config(&f).stack_up);
    f.git(&["config", "--replace-all", "git-pr.stackUp", "invalid"]);
    f.git(&["config", "--add", "git-pr.stackUp", "true"]);
    // Git converts every matching value, including superseded values.
    assert!(
        f.app
            .commands
            .git(&["config", "--type=bool", "--get", "git-pr.stackUp"])
            .is_err()
    );
    assert!(f.run(&["--stop-after=validate"]).is_err());
    f.git(&["config", "--replace-all", "git-pr.stackUp", "invalid"]);
    f.git(&[
        "config",
        "--replace-all",
        "git-pr.commandTimeout",
        "invalid",
    ]);
    assert!(
        f.run(&[
            "--stack-up=false",
            "--command-timeout=2",
            "--stop-after=validate"
        ])
        .is_ok()
    );
    f.git(&["config", "git-pr.githubStack", "invalid"]);
    for flag in ["--github-stack", "--github-stack=false", "--no-stack"] {
        f.run(&[
            "--stack-up=false",
            "--command-timeout=2",
            "--stop-after=validate",
            flag,
        ])
        .unwrap();
    }
}

#[test]
fn literal_branch_validation_agrees_with_native_git() {
    let directory = tempfile::tempdir().unwrap();
    let mut names = vec![
        "@",
        "HEAD",
        "refs/heads/HEAD",
        "foo.lock/bar",
        "foo.LOCK",
        "a//b",
        "a/",
        "/a",
        "a..b",
        "a@{b",
        "a./b",
        ".a",
        "日本語/é",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    for byte in 1..=127_u8 {
        let c = char::from(byte);
        names.extend([
            format!("{c}topic"),
            format!("topic/{c}part"),
            format!("topic{c}"),
        ]);
    }
    for name in names {
        let output = std::process::Command::new("git")
            .args(["check-ref-format", "--branch", &name])
            .current_dir(directory.path())
            .env_clear()
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert_eq!(
            crate::git::policy::valid_branch_name(&name),
            output.status.success(),
            "{name:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn native_git_remote_selection_preserves_subsection_case_and_configuration_scope() {
    let f = Fixture::new(1, false);
    f.git(&["remote", "set-url", "origin", "https://github.com/org/repo"]);
    f.git(&["remote", "add", "a-backup", "https://github.com/org/backup"]);
    f.git(&[
        "remote",
        "add",
        "upstream",
        "https://github.com/org/upstream",
    ]);
    f.git(&["config", "remote.pushDefault", "origin"]);
    f.git(&["branch", "Feature", "HEAD"]);
    f.git(&["checkout", "Feature"]);
    f.git(&["config", "branch.Feature.remote", "upstream"]);
    f.git(&["config", "branch.feature.pushRemote", "a-backup"]);
    let global = f.home.join("remote-default");
    std::fs::write(&global, "[remote]\npushDefault = a-backup\n").unwrap();
    let mut commands = f.app.commands.clone();
    f.service.command_errors.lock().unwrap().insert(
        "git remote -v".into(),
        (0, "a-backup https://github.com/org/backup (push)\norigin https://github.com/org/repo (push)\nupstream https://github.com/org/upstream (push)".into()),
    );
    commands
        .env
        .insert("GIT_CONFIG_GLOBAL".into(), global.into_os_string());
    for (key, value, expected) in [
        ("remote.pushDefault", "origin", "origin"),
        ("branch.Feature.pushRemote", "a-backup", "a-backup"),
    ] {
        f.git(&["config", key, value]);
        f.git(&[
            "symbolic-ref",
            &format!("refs/remotes/{expected}/HEAD"),
            &format!("refs/remotes/{expected}/main"),
        ]);
        let flags = Flags::parse(vec!["--dry-run".into()]).unwrap();
        let Loaded::Ready(config) = config::load(
            &flags,
            &BTreeMap::new(),
            &mut commands,
            &FakeCredentials,
            &crate::config::files::NativeFiles,
        )
        .unwrap() else {
            panic!("expected configuration");
        };
        assert_eq!(config.repo.remote, expected);
        assert_eq!(
            config.repo.name,
            if expected == "origin" {
                "org/repo"
            } else {
                "org/backup"
            }
        );
    }
}

#[test]
fn legacy_stack_orientation_uses_git_conversion_and_can_be_overridden() {
    let f = Fixture::new(0, true);
    f.git(&["config", "git-pr.reverse", "OFF"]);
    assert!(!loaded_config(&f).stack_up);
    f.git(&["config", "git-pr.stackUp", "0"]);
    assert!(!loaded_config(&f).stack_up);
    f.git(&["config", "git-pr.stackUp", "YES"]);
    let error = format!("{:#}", f.run(&["--stop-after=validate"]).unwrap_err());
    assert!(error.contains("conflicting git-pr.stackUp"), "{error}");
    f.git(&["config", "git-pr.reverse", "invalid"]);
    f.git(&["config", "git-pr.stackUp", "invalid"]);
    for flag in ["--stack-up=false", "--reverse=false"] {
        f.run(&[flag, "--stop-after=validate"]).unwrap();
    }
}

#[test]
fn global_native_stack_setting_links_without_yes_or_prompt() {
    let mut f = Fixture::new(2, true);
    let global = f.home.join("global.gitconfig");
    std::fs::write(&global, "[git-pr]\ngithubStack = true\n").unwrap();
    f.app
        .commands
        .env
        .insert("GIT_CONFIG_GLOBAL".into(), global.into_os_string());
    f.run(&[]).unwrap();
    assert!(
        f.trace().iter().any(|e| e["path"] == "stacks"
            && e["body"]["pull_requests"] == serde_json::json!([101, 102]))
    );
    assert!(!f.output().contains("skipped GitHub Stack"));
}

#[test]
fn hyperlink_git_values_are_validated_unless_overridden() {
    let fixture = Fixture::new(0, true);
    for value in ["invalid", "", "true"] {
        fixture.git(&["config", "git-pr.hyperlinks", value]);
        let error = format!("{:#}", fixture.run(&["--stop-after=validate"]).unwrap_err());
        assert!(
            error.contains("git-pr.hyperlinks") && error.contains("expected auto, always or never")
        );
        fixture
            .run(&["--stop-after=validate", "--hyperlinks=auto"])
            .unwrap();
    }
    fixture.git(&["config", "git-pr.hyperlinks", "invalid"]);
    fixture.git(&["config", "--add", "git-pr.hyperlinks", "never"]);
    assert_eq!(
        loaded_config(&fixture).hyperlinks,
        crate::cli::hyperlinks::Mode::Never
    );
}
