// Installation fixtures and subprocess failures shall fail the test immediately.
#![allow(clippy::unwrap_used)]

use std::{os::unix::fs::PermissionsExt as _, path::Path, process::Command};

fn isolated_command(temp: &tempfile::TempDir, program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(temp.path())
        .env_clear()
        .envs(std::env::vars_os().filter(|(key, _)| {
            matches!(
                key.to_str(),
                Some(
                    "PATH"
                        | "TMPDIR"
                        | "LLVM_PROFILE_FILE"
                        | "DYLD_LIBRARY_PATH"
                        | "DYLD_FALLBACK_LIBRARY_PATH"
                        | "LD_LIBRARY_PATH"
                )
            )
        }))
        .env("HOME", temp.path())
        .env("XDG_STATE_HOME", temp.path().join("state"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

fn installer(temp: &tempfile::TempDir) -> Command {
    let tools = temp.path().join("tools");
    std::fs::create_dir(&tools).unwrap();
    let cargo = tools.join("cargo");
    // Compilation is covered by Cargo; this boundary verifies the install contract.
    std::fs::write(
        &cargo,
        r#"#!/bin/sh
set -eu
touch "$INSTALL_INVOKED"
[ "$#" = 6 ]
[ "$1" = install ]
[ "$2" = --path ]
[ -f "$3/Cargo.toml" ]
[ "$4" = --locked ]
[ "$5" = --root ]
if [ "$INSTALL_FAIL" = 1 ]; then
    printf 'fixture: cargo installation failed\n' >&2
    exit 17
fi
mkdir -p "$6/bin"
cp "$INSTALL_BINARY" "$6/bin/git-pr"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = isolated_command(temp, concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"));
    let mut path = vec![tools];
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    command
        .env("PATH", std::env::join_paths(path).unwrap())
        .env("INSTALL_FAIL", "0")
        .env("INSTALL_INVOKED", temp.path().join("cargo-invoked"))
        .env("INSTALL_BINARY", env!("CARGO_BIN_EXE_git-pr"));
    command
}

fn plain_manual(bytes: Vec<u8>) -> String {
    // BSD man may retain overstrikes when its output is captured.
    let mut plain = String::new();
    for character in String::from_utf8(bytes).unwrap().chars() {
        if character == '\u{8}' {
            plain.pop();
        } else {
            plain.push(character);
        }
    }
    plain
}

fn assert_git_help(temp: &tempfile::TempDir, prefix: &Path) {
    let mut path = vec![prefix.join("bin")];
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    for width in [80, 120] {
        for args in [["pr", "--help"], ["help", "pr"]] {
            let output = isolated_command(temp, "git")
                .args(args)
                .env("PATH", std::env::join_paths(&path).unwrap())
                .env("MANPAGER", "cat")
                .env("PAGER", "cat")
                .env("MANWIDTH", width.to_string())
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            let help = plain_manual(output.stdout);
            for expected in ["git pr", "COMMITS", "-draft", "--logs", "-output"] {
                assert!(help.contains(expected), "missing {expected}: {help}");
            }
            for line in help.lines() {
                assert!(line.chars().count() <= width, "width {width}: {line}");
            }
            let words = help.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(words.contains("List saved invocation logs, newest first"));
            assert!(words.contains("Decision explanations and operation timings"));
            assert!(words.contains("Assume \"yes\" to prompts"));
        }
    }
    assert!(!temp.path().join(".gitconfig").exists());
}

#[test]
fn installation_uses_default_or_explicit_prefix_with_conflicting_cargo_settings() {
    for explicit in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join(if explicit {
            "prefix with spaces"
        } else {
            ".cargo"
        });
        let cargo_home = temp.path().join("cargo-home");
        let configured_root = temp.path().join("configured-root");
        let environment_root = temp.path().join("environment-root");
        std::fs::create_dir(&cargo_home).unwrap();
        std::fs::write(
            cargo_home.join("config.toml"),
            format!(
                "[install]\nroot = {}\n",
                serde_json::to_string(&configured_root).unwrap()
            ),
        )
        .unwrap();
        let mut install = installer(&temp);
        if explicit {
            install.arg(&prefix);
        }
        let output = install
            .env("CARGO_HOME", &cargo_home)
            .env("CARGO_INSTALL_ROOT", &environment_root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(!configured_root.exists() && !environment_root.exists());
        assert!(prefix.join("share/man/man1/git-pr.1").is_file());
        if explicit {
            assert!(!temp.path().join(".cargo").exists());
        }
        assert_git_help(&temp, &prefix);
    }
}

#[test]
fn empty_or_multiple_prefixes_are_rejected_before_cargo_runs() {
    for args in [vec![""], vec!["one", "two"]] {
        let temp = tempfile::tempdir().unwrap();
        let output = installer(&temp)
            .args(args)
            .env("CARGO_INSTALL_ROOT", temp.path().join("unexpected"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("installation prefix")
        );
        assert!(!temp.path().join("cargo-invoked").exists());
        assert!(!temp.path().join(".cargo").exists());
        assert!(!temp.path().join("unexpected").exists());
    }
}

#[test]
fn failed_binary_installation_does_not_install_a_manual() {
    let temp = tempfile::tempdir().unwrap();
    let prefix = temp.path().join("prefix");
    let output = installer(&temp)
        .arg(&prefix)
        .env("INSTALL_FAIL", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(17));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("cargo installation failed")
    );
    assert!(!prefix.exists());
}

#[test]
fn failed_manual_installation_reports_the_completed_binary_update() {
    let temp = tempfile::tempdir().unwrap();
    let prefix = temp.path().join("prefix");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    std::fs::write(prefix.join("bin/git-pr"), "previous binary").unwrap();
    std::fs::set_permissions(
        prefix.join("bin/git-pr"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(prefix.join("share"), "blocked manual directory").unwrap();
    let output = installer(&temp).arg(&prefix).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("the binary was installed")
            && error.contains("manual could not be installed"),
        "{error}"
    );
    assert!(error.contains(prefix.to_str().unwrap()));
    assert_eq!(
        std::fs::read(prefix.join("bin/git-pr")).unwrap(),
        std::fs::read(env!("CARGO_BIN_EXE_git-pr")).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(prefix.join("share")).unwrap(),
        "blocked manual directory"
    );
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("Installed git-pr and its manual")
    );
}

#[test]
fn standalone_generation_produces_a_renderable_manual_without_installation() {
    let temp = tempfile::tempdir().unwrap();
    let generated = isolated_command(
        &temp,
        concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/man.sh"),
    )
    .arg(env!("CARGO_BIN_EXE_git-pr"))
    .output()
    .unwrap();
    assert!(generated.status.success(), "{generated:?}");
    let manual = temp.path().join("git-pr.1");
    std::fs::write(&manual, generated.stdout).unwrap();
    let rendered = isolated_command(&temp, "man")
        .arg(&manual)
        .env("MANPAGER", "cat")
        .env("PAGER", "cat")
        .env("MANWIDTH", "80")
        .output()
        .unwrap();
    assert!(rendered.status.success(), "{rendered:?}");
    let text = plain_manual(rendered.stdout);
    assert!(text.contains("git pr") && text.contains("--logs"));
    assert!(!temp.path().join(".cargo").exists());
}

#[test]
fn failed_help_generation_is_propagated_without_partial_manual_output() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("broken-git-pr");
    std::fs::write(&binary, "#!/bin/sh\nexit 23\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = isolated_command(
        &temp,
        concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/man.sh"),
    )
    .arg(&binary)
    .output()
    .unwrap();
    assert_eq!(output.status.code(), Some(23));
    assert_eq!(output.stdout, b"");

    let prefix = temp.path().join("prefix");
    let installed = installer(&temp)
        .arg(&prefix)
        .env("INSTALL_BINARY", &binary)
        .output()
        .unwrap();
    assert_eq!(installed.status.code(), Some(1));
    let error = String::from_utf8(installed.stderr).unwrap();
    assert!(error.contains("the binary was installed"));
    assert!(error.contains("manual could not be installed"));
    assert!(prefix.join("bin/git-pr").exists());
    assert!(!prefix.join("share/man/man1/git-pr.1").exists());
}
