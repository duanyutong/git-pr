//! Construction of production boundaries and process exit handling.

use std::{collections::BTreeMap, sync::Arc};

use crate::{
    cli::{self, Flags, ParseError, output},
    commands::{Commands, Console, ProcessRunner, SystemClock, Terminal},
    config::settings,
    credentials::SystemCredentials,
    diagnostics::errors,
    github::http::HttpTransport,
};

use super::Application;

/// Execute a submission using the process environment and standard terminal streams.
///
/// The result is a process exit status. Repository mutations follow the configured
/// submission checkpoints; dry runs report proposed operations without applying repository changes.
#[must_use]
pub fn run(args: Vec<String>) -> u8 {
    let terminal: Arc<dyn Terminal> = Arc::new(crate::cli::progress::ProgressTerminal::new(
        Arc::new(Console::default()),
    ));
    let status = run_with_terminal(args, &terminal);
    if let Some(error) = terminal.failure() {
        terminal.err(&format!("ERROR: {error}. Completed operations are retained; inspect repository and PR state if submission was underway.\n"));
        return status.max(1);
    }
    status
}

fn run_with_terminal(args: Vec<String>, terminal: &Arc<dyn Terminal>) -> u8 {
    let flags = match Flags::parse(args) {
        Ok(flags) => flags,
        Err(error) => {
            if let ParseError::Invalid(message) = &error {
                terminal.err(&format!("{message}\n"));
            }
            terminal.out(cli::USAGE);
            terminal.err(cli::DEFAULTS);
            return if error == ParseError::Help { 0 } else { 2 };
        }
    };
    if flags.boolean("version") {
        terminal.out(&output::version(
            option_env!("VERGEN_GIT_SHA"),
            option_env!("VERGEN_GIT_DIRTY"),
        ));
        return 0;
    }
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(e) => {
            terminal.err(&format!("ERROR: cannot access the current working directory: {e}\nChange to an existing, accessible repository directory and retry.\n"));
            return 1;
        }
    };
    let commands = Commands {
        runner: Arc::new(ProcessRunner::default()),
        terminal: Arc::clone(terminal),
        cwd,
        env: BTreeMap::new(),
        verbose: false,
        diagnostics: crate::diagnostics::Diagnostics::default(),
    };
    let app = Application {
        commands,
        clock: Arc::new(SystemClock),
        transport: Arc::new(HttpTransport::default()),
        credentials: Arc::new(SystemCredentials),
        files: Arc::new(crate::config::files::NativeFiles),
    };
    let environment = match settings::unicode_environment(std::env::vars_os()) {
        Ok(environment) => environment,
        Err(error) => {
            terminal.err(&format!("{error:#}\n"));
            return 1;
        }
    };
    match app.execute(&flags, &environment) {
        Ok(()) => 0,
        Err(error) => {
            let diagnostic = format!("{error:#}");
            terminal.err(&format!(
                "\x1b[31m{}\x1b[0m\n",
                errors::redact_url_credentials(&diagnostic)
            ));
            errors::status(&error)
        }
    }
}
