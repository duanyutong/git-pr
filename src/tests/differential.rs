use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use anyhow::Result;
use serde_json::Value;

use crate::{
    cli::Flags,
    commands::{Clock, Commands, Console, ProcessRunner},
    credentials::SystemCredentials,
    github::http::{HttpTransport, Request, Transport},
    submission::Application,
};

#[derive(Debug)]
struct LoopbackTransport {
    endpoint: String,
    inner: HttpTransport,
}
impl Transport for LoopbackTransport {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        let url = reqwest::Url::parse(request.url)?;
        let path = format!(
            "{}{}",
            url.path(),
            url.query().map_or_else(String::new, |q| format!("?{q}"))
        );
        self.inner
            .request(Request {
                url: &format!("{}{path}", self.endpoint),
                ..request
            })
            .map_err(
                |error| match error.downcast::<crate::github::http::HttpStatusError>() {
                    Ok(mut status) => {
                        status.url = request.url.into();
                        status.into()
                    }
                    Err(error) => error,
                },
            )
    }
}

#[derive(Debug)]
struct RecordingClock;
impl Clock for RecordingClock {
    fn sleep(&self, duration: std::time::Duration) {
        use std::io::Write as _;
        let start = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        std::thread::sleep(duration);
        let end = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::var("GIT_PR_WAIT_FILE").unwrap())
            .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({"start":start,"end":end,"scheduled":duration.as_secs_f64()})
        )
        .unwrap();
    }
}

#[test]
fn differential_entrypoint() {
    let Ok(path) = std::env::var("GIT_PR_TEST_INPUT") else {
        return;
    };
    let input: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let args: Vec<String> = serde_json::from_value(input["args"].clone()).unwrap();
    let flags = Flags::parse(args).unwrap();
    let env: BTreeMap<String, String> = serde_json::from_value(input["env"].clone()).unwrap();
    let commands = Commands {
        runner: Arc::new(ProcessRunner::default()),
        terminal: Arc::new(Console::default()),
        cwd: PathBuf::from(input["cwd"].as_str().unwrap()),
        env: env.iter().map(|(k, v)| (k.into(), v.into())).collect(),
        verbose: false,
        diagnostics: crate::diagnostics::Diagnostics::default(),
    };
    let app = Application {
        commands,
        transport: Arc::new(LoopbackTransport {
            endpoint: input["endpoint"].as_str().unwrap().into(),
            inner: HttpTransport::default(),
        }),
        clock: Arc::new(RecordingClock),
        credentials: Arc::new(SystemCredentials),
        files: Arc::new(crate::config::files::NativeFiles),
    };
    let result = app.execute(&flags, &env);
    let status = result
        .as_ref()
        .map_or_else(crate::diagnostics::errors::status, |()| 0);
    if let Err(error) = result {
        eprintln!("\x1b[31m{error:#}\x1b[0m");
    }
    std::fs::write(input["status"].as_str().unwrap(), status.to_string()).unwrap();
}
