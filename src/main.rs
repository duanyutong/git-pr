use std::{io::Write as _, process::ExitCode};

fn main() -> ExitCode {
    install_panic_hook();
    let arguments = std::env::args_os()
        .skip(1)
        .map(std::ffi::OsString::into_string)
        .collect::<Result<Vec<_>, _>>();
    arguments.map_or_else(
        |_| {
            let _ = writeln!(
                std::io::stderr().lock(),
                "ERROR: command-line arguments must be valid UTF-8"
            );
            ExitCode::from(2)
        },
        |arguments| ExitCode::from(git_pr::run(arguments)),
    )
}

fn install_panic_hook() {
    // A hook also runs in release builds, which abort rather than unwind.
    // Do not print the panic payload: dependency diagnostics may contain secrets.
    std::panic::set_hook(Box::new(|information| {
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(
            stderr,
            "ERROR: git-pr {} encountered an unexpected internal failure and must stop.",
            env!("CARGO_PKG_VERSION")
        );
        if let Some(location) = information.location() {
            let _ = writeln!(stderr, "Location: {location}");
        }
        let _ = writeln!(
            stderr,
            "Completed local and remote changes may remain. Inspect repository and PR state before retrying. Report this version and location at https://github.com/duanyutong/git-pr/issues; omit credentials and private content."
        );
    }));
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "The executable boundary is exercised in a disposable child process"
)]
mod tests {
    #[test]
    fn panic_diagnostic_child() {
        if std::env::var_os("GIT_PR_PANIC_FIXTURE").is_some() {
            super::install_panic_hook();
            panic!("secret-token-fixture");
        }
    }

    #[test]
    fn unexpected_failures_explain_the_failure_without_disclosing_payloads() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::panic_diagnostic_child", "--nocapture"])
            .env("GIT_PR_PANIC_FIXTURE", "1")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(env!("CARGO_PKG_VERSION")) && error.contains("Location:"));
        assert!(error.contains("internal failure") && error.contains("changes may remain"));
        assert!(!error.contains("secret-token-fixture"));
    }
}
