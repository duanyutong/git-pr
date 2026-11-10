//! Construction of production boundaries and process exit handling.

use std::{collections::BTreeMap, sync::Arc};

use crate::{
    cli::{self, Flags, ParseError, output},
    commands::{Commands, Console, ProcessRunner, SystemClock, Terminal},
    config::settings,
    credentials::SystemCredentials,
    diagnostics::{errors, logs},
    github::http::HttpTransport,
};

use super::Application;

/// Execute a submission using the process environment and standard terminal streams.
///
/// The result is a process exit status. Repository mutations follow the configured
/// submission checkpoints; dry runs report proposed operations without applying repository changes.
#[must_use]
pub fn run(args: Vec<String>) -> u8 {
    run_parsed(Flags::parse(args))
}

/// Execute an invocation whilst retaining diagnostics for invalid process arguments.
#[must_use]
pub fn run_os(args: Vec<std::ffi::OsString>) -> u8 {
    let flags = args
        .into_iter()
        .map(std::ffi::OsString::into_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ParseError::NonUnicode)
        .and_then(Flags::parse);
    run_parsed(flags)
}

fn run_parsed(flags: Result<Flags, ParseError>) -> u8 {
    if let Ok(flags) = &flags
        && flags.supplied.contains("logs")
    {
        return list_logs(flags.string("logs") == "all");
    }
    let (log, log_error) =
        match logs::directory().and_then(|directory| logs::InvocationLog::create(&directory)) {
            Ok(log) => (Some(Arc::new(log)), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
    let terminal: Arc<dyn Terminal> = Arc::new(crate::cli::progress::ProgressTerminal::with_log(
        Arc::new(Console::default()),
        log.clone(),
        flags.as_ref().is_ok_and(|flags| flags.verbosity() > 0),
    ));
    let mut status = run_with_terminal(flags, &terminal);
    if let Some(error) = terminal.failure() {
        terminal.err(&format!("ERROR: {error}. Completed operations are retained; inspect repository and PR state if submission was underway.\n"));
        status = status.max(1);
    }
    let log_error = log
        .as_ref()
        .and_then(|log| log.finish(status))
        .or(log_error);
    if let Some(error) = log_error {
        terminal.err(&format!(
            "WARNING: invocation diagnostics could not be saved: {}\n",
            errors::redact_url_credentials(&error)
        ));
    }
    status
}

fn list_logs(all: bool) -> u8 {
    let terminal = Console::default();
    match logs::directory().and_then(|directory| logs::list(&directory, all)) {
        Ok(list) => terminal.out(&list),
        Err(error) => {
            terminal.err(&format!("ERROR: cannot list invocation logs: {error:#}\n"));
            return 1;
        }
    }
    u8::from(terminal.failure().is_some())
}

fn run_with_terminal(flags: Result<Flags, ParseError>, terminal: &Arc<dyn Terminal>) -> u8 {
    let flags = match flags {
        Ok(flags) => flags,
        Err(ParseError::NonUnicode) => {
            terminal.err("ERROR: command-line arguments must be valid UTF-8\n");
            return 2;
        }
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
        explain: false,
        outcomes: crate::model::outcome::Outcomes::default(),
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
        Err(error) => errors::status(&error),
    }
}
