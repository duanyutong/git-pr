//! Per-invocation diagnostic files and their filesystem-backed catalogue.

use std::{
    fmt::Write as _,
    fs::{self, File},
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Metadata {
    timestamp: String,
    repository: String,
    commits: Option<usize>,
    outcome: String,
    seconds: Option<f64>,
}

#[derive(Debug)]
pub(crate) struct InvocationLog {
    path: PathBuf,
    file: Mutex<File>,
    metadata: Mutex<Metadata>,
    failure: Mutex<Option<String>>,
    started: Instant,
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn directory() -> Result<PathBuf> {
    if let Some(state) = std::env::var_os("XDG_STATE_HOME").filter(|v| Path::new(v).is_absolute()) {
        return Ok(PathBuf::from(state).join("git-pr/logs"));
    }
    let home = std::env::var_os("HOME").context("HOME is unavailable for invocation logs")?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        bail!("HOME must be an absolute path for invocation logs");
    }
    Ok(home.join(if cfg!(target_os = "macos") {
        "Library/Logs/git-pr"
    } else {
        ".local/state/git-pr/logs"
    }))
}

impl InvocationLog {
    pub(crate) fn create(directory: &Path) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt as _;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        let now = SystemTime::now();
        let prefix = format!(
            "invocation-{:039}-",
            now.duration_since(UNIX_EPOCH)?.as_nanos()
        );
        let (file, path) = tempfile::Builder::new()
            .prefix(&prefix)
            .suffix(".log")
            .tempfile_in(directory)?
            .keep()?;
        let log = Self {
            path,
            file: Mutex::new(file),
            metadata: Mutex::new(Metadata {
                timestamp: chrono::DateTime::<chrono::Utc>::from(now)
                    .format("%Y-%m-%d %H:%M:%S%.3f UTC")
                    .to_string(),
                repository: "unknown".into(),
                commits: None,
                outcome: "incomplete".into(),
                seconds: None,
            }),
            failure: Mutex::default(),
            started: Instant::now(),
        };
        log.save_metadata(&lock(&log.metadata))?;
        log.record(&format!(
            "git-pr {} invocation\n",
            env!("CARGO_PKG_VERSION")
        ));
        if let Ok(directory) = std::env::current_dir() {
            log.record(&format!("Working directory: {}\n", directory.display()));
        }
        Ok(log)
    }

    fn save_metadata(&self, metadata: &Metadata) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("invocation log has no parent directory")?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(file.as_file_mut(), metadata)?;
        file.persist(self.path.with_extension("json"))?;
        Ok(())
    }

    fn remember(&self, result: Result<()>) {
        if let Err(error) = result {
            lock(&self.failure).get_or_insert_with(|| format!("{error:#}"));
        }
    }

    pub(crate) fn record(&self, text: &str) {
        let text = super::errors::redact_url_credentials(text);
        let text = crate::cli::progress::clean(&text, false);
        let mut file = lock(&self.file);
        let result = file.write_all(text.as_bytes()).and_then(|()| {
            if text.ends_with('\n') {
                Ok(())
            } else {
                file.write_all(b"\n")
            }
        });
        drop(file);
        self.remember(result.map_err(Into::into));
    }

    pub(crate) fn context(&self, repository: &str, commits: Option<usize>) {
        let mut metadata = lock(&self.metadata);
        if metadata.repository == repository && metadata.commits == commits {
            return;
        }
        repository.clone_into(&mut metadata.repository);
        metadata.commits = commits;
        let result = self.save_metadata(&metadata);
        drop(metadata);
        self.remember(result);
    }

    pub(crate) fn finish(&self, status: u8) -> Option<String> {
        let mut metadata = lock(&self.metadata);
        metadata.outcome = if status == 0 {
            "ok".into()
        } else {
            format!("exit {status}")
        };
        metadata.seconds = Some(self.started.elapsed().as_secs_f64());
        self.record(&format!(
            "Invocation finished: {} ({:.3}s)\n",
            metadata.outcome,
            metadata.seconds.unwrap_or_default()
        ));
        let result = self.save_metadata(&metadata);
        drop(metadata);
        self.remember(result);
        lock(&self.failure).clone()
    }
}

pub(crate) fn list(directory: &Path, all: bool) -> Result<String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok("No invocation logs recorded.\n".into());
        }
        Err(error) => return Err(error.into()),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file()
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("invocation-"))
            && path
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
    }
    paths.sort_unstable_by(|left, right| right.cmp(left));
    if paths.is_empty() {
        return Ok("No invocation logs recorded.\n".into());
    }
    let mut rows = Vec::new();
    for path in paths.iter().take(if all { usize::MAX } else { 20 }) {
        let metadata: Metadata = serde_json::from_reader(File::open(path)?)
            .with_context(|| format!("cannot read invocation metadata {}", path.display()))?;
        rows.push((path, metadata));
    }
    let repository_width = rows
        .iter()
        .map(|(_, metadata)| {
            crate::cli::progress::single_line(&metadata.repository)
                .chars()
                .count()
        })
        .max()
        .unwrap_or_default()
        .max("REPOSITORY".len());
    let mut output = format!(
        "{:<27}  {:<repository_width$}  {:>7}  {:<10}  {:>9}  LOG\n",
        "TIMESTAMP (UTC)", "REPOSITORY", "COMMITS", "OUTCOME", "DURATION"
    );
    for (path, metadata) in rows {
        writeln!(
            output,
            "{:<27}  {:<repository_width$}  {:>7}  {:<10}  {:>9}  {}",
            metadata.timestamp,
            crate::cli::progress::single_line(&metadata.repository),
            metadata
                .commits
                .map_or_else(|| "unknown".into(), |count| count.to_string()),
            crate::cli::progress::single_line(&metadata.outcome),
            metadata
                .seconds
                .map_or_else(|| "—".into(), |seconds| format!("{seconds:.3}s")),
            crate::cli::progress::single_line(&path.with_extension("log").display().to_string())
        )?;
    }
    if !all && paths.len() > 20 {
        output.push_str(
            "Showing the newest 20 invocations; use --logs=all for every retained log.\n",
        );
    }
    Ok(output)
}
