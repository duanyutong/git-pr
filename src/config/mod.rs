mod batch;
pub(crate) mod checkpoint;
pub(crate) mod files;
pub(crate) mod paths;
pub(crate) mod settings;

use batch::Values;
use checkpoint::Checkpoint;
use paths::{command_path, expand_path};

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    cli::{Flags, hyperlinks, output},
    commands::Commands,
    credentials::{self, CredentialStore, Secret},
    diagnostics::errors::Fatal,
    git::resolve_ref,
    model::{Repository, comma_list},
    stack::selection::{DraftPatterns, Revisions, Selection},
};

// These independent switches are part of the command's public configuration contract.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub repo: Repository,
    pub token: Secret,
    pub timeout: i64,
    pub tags: Vec<String>,
    pub labels: Vec<String>,
    pub tip_labels: Vec<String>,
    pub include_other_authors: bool,
    pub dry_run: bool,
    pub stop_after: Option<Checkpoint>,
    pub output: String,
    pub hyperlinks: hyperlinks::Mode,
    pub use_icon_font: bool,
    pub auto_accept: bool,
    pub github_stack: bool,
    pub no_stack: bool,
    pub skip_draft: bool,
    pub include_draft: bool,
    pub draft_patterns: DraftPatterns,
    pub stack_up: bool,
    pub preview_limit: usize,
    pub draft: bool,
    pub emoji: bool,
    pub compact_stack: bool,
    pub selection: Selection,
    pub transition_journal: String,
}

pub(crate) enum Loaded {
    Ready(Box<Config>),
    TagsSaved,
}

fn git_value(commands: &Commands, args: &[&str]) -> Result<Option<String>> {
    let args = args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let output = commands
        .runner
        .execute("git", &args, &commands.cwd, &commands.env)
        .context("failed to execute Git configuration query")?;
    // Git reserves status 1 with no diagnostic for an absent configuration key.
    if output.status == 1 && output.text.is_empty() {
        return Ok(None);
    }
    output
        .success()
        .map(Some)
        .with_context(|| format!("failed to read Git configuration: git {}", args.join(" ")))
}

pub(crate) fn load(
    flags: &Flags,
    env: &BTreeMap<String, String>,
    commands: &mut Commands,
    store: &dyn CredentialStore,
    files: &dyn files::Files,
    defer_git_authentication: bool,
) -> Result<Loaded> {
    let stop_after = Checkpoint::parse(settings::fallback(
        flags.string("stop-after"),
        env,
        "GIT_PR_STOP_AFTER",
    ))?;
    let revisions = Revisions::parse(&flags.args)?;
    let dry_run = settings::enabled(flags, env, "dry-run", "GIT_PR_DRY_RUN");
    if dry_run {
        commands.env.insert("GIT_OPTIONAL_LOCKS".into(), "0".into());
    }
    let root = repository_root(commands)?;
    configure_deadlines(flags, commands)?;
    if save_tags(flags, commands, dry_run)? {
        return Ok(Loaded::TagsSaved);
    }
    let strings = string_settings(flags, commands)?;
    let hyperlinks = hyperlink_mode(flags, &strings)?;
    let preview_limit = settings::preview_limit(strings.get("git-pr.previewLimit"))?;
    let booleans = boolean_settings(flags, env, commands)?;
    let no_stack = settings::enabled(flags, env, "no-stack", "GIT_PR_NO_STACK");
    let github_stack = settings::github_stack(
        no_stack,
        flags
            .supplied
            .contains("github-stack")
            .then(|| flags.boolean("github-stack")),
        env.get("GIT_PR_GITHUB_STACK").map(String::as_str),
        booleans.boolean("git-pr.githubStack"),
    );
    let defaults = comma_list(&strings.string("git-pr.tags"));
    let tags = if flags.string("t").is_empty() {
        defaults
    } else {
        comma_list(flags.string("t"))
    };
    let mut skip_draft = settings::enabled(flags, env, "skip-draft", "GIT_PR_SKIP_DRAFT");
    if !skip_draft {
        skip_draft = booleans.boolean("git-pr.skipDraft").unwrap_or(false);
    }
    let stack_up = stack_up(flags, &booleans, commands)?;
    let draft = boolean_setting(flags, &booleans, "draft", "git-pr.defaultDraft", true);
    let emoji = booleans.boolean("git-pr.stackEmoji").unwrap_or(false);
    let output = output_format(flags, env, &strings)?;
    let draft_patterns = DraftPatterns::new(&comma_list(&draft_patterns(flags, &strings)))?;
    let repo = discover(commands, root, files, &strings)?;
    let token = if (!defer_git_authentication || repo.jj)
        && !dry_run
        && !matches!(
            stop_after,
            Some(Checkpoint::Validate | Checkpoint::GetCommits)
        ) {
        load_token(flags, env, commands, store, files, &repo)?
    } else {
        Secret::default()
    };
    if repo.jj && !dry_run {
        commands.jj(&["util", "snapshot"])?;
    }
    let selection = resolve_selection(revisions, repo.jj, commands)?;
    commands.verbose = flags.verbosity() > 1;
    let config = Box::new(Config {
        repo,
        token,
        timeout: flags.timeout(),
        tags,
        labels: settings::labels(&flags.labels, &strings.string("git-pr.addLabel")),
        tip_labels: settings::labels(&flags.tip_labels, &strings.string("git-pr.addTipLabel")),
        include_other_authors: flags.boolean("include-other-authors"),
        dry_run,
        stop_after,
        output,
        hyperlinks,
        use_icon_font: boolean_setting(
            flags,
            &booleans,
            "use-icon-font",
            "git-pr.useIconFont",
            true,
        ),
        auto_accept: settings::enabled(flags, env, "yes", "GIT_PR_YES"),
        github_stack,
        no_stack,
        skip_draft,
        include_draft: settings::enabled(flags, env, "include-draft", "GIT_PR_INCLUDE_DRAFT"),
        draft_patterns,
        stack_up,
        preview_limit,
        draft,
        emoji,
        compact_stack: compact_stack(flags, &booleans),
        selection,
        transition_journal: strings.string("git-pr.transitionJournal"),
    });
    Ok(Loaded::Ready(config))
}

impl Config {
    pub(crate) fn authenticate(
        &mut self,
        flags: &Flags,
        env: &BTreeMap<String, String>,
        commands: &Commands,
        store: &dyn CredentialStore,
        files: &dyn files::Files,
    ) -> Result<()> {
        if self.dry_run
            || !self.token.0.is_empty()
            || matches!(
                self.stop_after,
                Some(Checkpoint::Validate | Checkpoint::GetCommits)
            )
        {
            return Ok(());
        }
        self.token = load_token(flags, env, commands, store, files, &self.repo)?;
        Ok(())
    }
}

fn load_token(
    flags: &Flags,
    env: &BTreeMap<String, String>,
    commands: &Commands,
    store: &dyn CredentialStore,
    files: &dyn files::Files,
    repo: &Repository,
) -> Result<Secret> {
    let home = env.get("HOME").map_or("", String::as_str);
    let hosts = expand_path(flags.string("gh-hosts"), home);
    commands.diagnostics.phase("authentication", || {
        credentials::load(&hosts, &repo.host, store, commands, files)
    })
}

fn output_format(
    flags: &Flags,
    env: &BTreeMap<String, String>,
    strings: &Values,
) -> Result<String> {
    let configured = strings.string("git-pr.output");
    let value = settings::fallback(flags.string("output"), env, "GIT_PR_OUTPUT");
    output::resolve_format(if value.is_empty() { &configured } else { value })
}

fn hyperlink_mode(flags: &Flags, strings: &Values) -> Result<hyperlinks::Mode> {
    flags.hyperlinks.map_or_else(
        || {
            hyperlinks::Mode::parse(strings.get("git-pr.hyperlinks").unwrap_or("auto"))
                .map_err(anyhow::Error::msg)
                .context("invalid git-pr.hyperlinks configuration")
        },
        Ok,
    )
}

fn draft_patterns(flags: &Flags, strings: &Values) -> String {
    let mut patterns = flags.string("draft-pattern").to_owned();
    if patterns.is_empty() {
        patterns = strings.string("git-pr.draftPattern");
    }
    if patterns.is_empty() {
        patterns = "wip:*,draft:*,*[wip]*,*[draft]*".into();
    }
    patterns
}

fn repository_root(commands: &mut Commands) -> Result<PathBuf> {
    let git_error = match commands.git_data(&["rev-parse", "--show-toplevel"], &[]) {
        Ok(root) => return Ok(command_path(&root)),
        Err(error) => error,
    };
    let git_dir = commands.data("jj", &["git", "root"], &[]).map_err(|jj_error| {
        git_error.context(format!("ERROR: could not discover a Git repository or Jujutsu workspace.\nYou need to run it in a git repository with remote configured.\nCheck the directory, Git installation and repository access. For usage, see `git-pr --help`.\nJujutsu workspace discovery also failed: {jj_error:#}"))
    })?;
    let root = {
        let root = commands
            .data("jj", &["workspace", "root"], &[])
            .context("ERROR: detected jj workspace but failed to resolve workspace root")?;
        commands
            .env
            .insert("GIT_DIR".into(), command_path(&git_dir).into_os_string());
        root
    };
    Ok(command_path(&root))
}

fn discover(
    commands: &Commands,
    root: PathBuf,
    files: &dyn files::Files,
    strings: &Values,
) -> Result<Repository> {
    let remote_output = commands.git(&["remote", "-v"])?;
    let branch = if strings.needs_publication_branch() {
        commands
            .git(&["branch", "--show-current"])
            .context("failed to identify the current Git branch for publication remote selection")?
    } else {
        String::new()
    };
    let configured = strings
        .get(&format!("branch.{branch}.pushRemote"))
        .or_else(|| strings.get("remote.pushDefault"))
        .or_else(|| strings.get(&format!("branch.{branch}.remote")));
    let remote = crate::git::policy::select_remote(&remote_output, configured)?;
    let mut repo = crate::git::policy::parse_remote_patterns(
        remote,
        &crate::git::policy::REMOTE_PATTERNS,
    )
    .with_context(|| {
        format!(
            "failed to parse remote url for the publication remote; Git remote listing: {:?}",
            crate::diagnostics::errors::redact_url_credentials(&remote_output)
        )
    })?;
    repo.root = root;
    let remote_trunk = commands
        .git(&[
            "symbolic-ref",
            &format!("refs/remotes/{}/HEAD", repo.remote),
        ])
        .with_context(|| format!(
            "ERROR: failed to detect remote trunk branch: refs/remotes/{}/HEAD is unavailable.\nFetch the remote if necessary, then run `git remote set-head {} --auto` to record its default branch locally.\nAlternatively, use `git remote set-head {} <default-branch>` after verifying the remote's default branch",
            repo.remote, repo.remote, repo.remote
        ))?;
    repo.trunk = remote_trunk
        .strip_prefix(&format!("refs/remotes/{}/", repo.remote))
        .filter(|branch| !branch.is_empty())
        .with_context(|| format!(
            "ERROR: failed to detect remote trunk branch: expected a nonempty branch under refs/remotes/{}/",
            repo.remote
        ))?
        .into();
    repo.user = strings.string("user.name");
    if repo.user.is_empty() {
        bail!(
            "ERROR: user.name not found in git config; set it with `git config user.name 'Your Name'`"
        );
    }
    repo.email = strings.string("user.email");
    if repo.email.is_empty() {
        bail!(
            "ERROR: user.email not found in git config; set it with `git config user.email 'you@example.com'`"
        );
    }
    let jj_metadata = repo.root.join(".jj");
    repo.jj = files.exists(&jj_metadata).with_context(|| {
        format!(
            "could not inspect Jujutsu workspace metadata at {}; check file access before submitting",
            jj_metadata.display()
        )
    })?;
    if repo.jj {
        commands.jj(&["version"]).context("Jujutsu workspace detected, but `jj version` failed; install or repair jj before submitting this workspace")?;
    }
    repo.branchless = !repo.jj && commands.git(&["branchless", "--version"]).is_ok();
    Ok(repo)
}

fn resolve_arg(commands: &Commands, reference: &str, jj: bool) -> Result<String> {
    if jj && let Ok(hash) = crate::jj::resolve_revision(commands, reference) {
        return Ok(hash);
    }
    resolve_ref(commands, reference)
}

fn resolve_selection(revisions: Revisions<'_>, jj: bool, commands: &Commands) -> Result<Selection> {
    let mut selection = Selection::default();
    match revisions {
        Revisions::Range { base, tip } => {
            selection.explicit = true;
            selection.base = resolve_arg(commands, base, jj)
                .with_context(|| format!("ERROR: failed to resolve base ref {base:?}"))?;
            selection.tip = resolve_arg(commands, tip, jj)
                .with_context(|| format!("ERROR: failed to resolve tip ref {tip:?}"))?;
        }
        Revisions::Commits([arg]) => {
            selection.explicit = true;
            selection.tip = resolve_arg(commands, arg, jj)
                .with_context(|| format!("ERROR: failed to resolve commit {arg:?}"))?;
            selection.base = resolve_ref(commands, &format!("{}^", selection.tip))
                .with_context(|| format!("ERROR: failed to resolve parent of {arg:?}"))?;
        }
        Revisions::Commits(args) => {
            selection.explicit = !args.is_empty();
            selection.selected = resolve_arguments(args, jj, commands)?;
        }
    }
    Ok(selection)
}

fn resolve_arguments(args: &[String], jj: bool, commands: &Commands) -> Result<Vec<String>> {
    let mut seen = std::collections::HashSet::new();
    let mut unique = Vec::new();
    for arg in args {
        if seen.insert(arg.as_str()) {
            unique.push(arg.clone());
        }
    }
    // Preserve per-argument diagnostics if a batch cannot resolve every revision.
    let resolved = if jj {
        crate::jj::resolve_revisions(commands, &unique).ok()
    } else {
        crate::git::resolve_refs(commands, &unique).ok()
    };
    let hashes = match resolved {
        Some(hashes) => hashes,
        None => unique
            .iter()
            .map(|arg| {
                resolve_arg(commands, arg, jj)
                    .with_context(|| format!("ERROR: failed to resolve commit {arg:?}"))
            })
            .collect::<Result<Vec<_>>>()?,
    };
    let mut seen = std::collections::HashSet::new();
    Ok(hashes
        .into_iter()
        .filter(|hash| seen.insert(hash.clone()))
        .collect())
}

pub(crate) fn load_template(cwd: &Path, files: &dyn files::Files) -> Result<String> {
    for path in [
        ".github/PULL_REQUEST_TEMPLATE.md",
        ".github/pull_request_template.md",
        "docs/PULL_REQUEST_TEMPLATE.md",
        "docs/pull_request_template.md",
        "PULL_REQUEST_TEMPLATE.md",
        "pull_request_template.md",
    ] {
        let path = cwd.join(path);
        match files.read(&path) {
            Ok(text) => return Ok(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "could not read PR template {}; check file access and UTF-8 encoding",
                        path.display()
                    )
                });
            }
        }
    }
    Ok(crate::stack::description::BODY_TEMPLATE.into())
}

fn configure_deadlines(flags: &Flags, commands: &Commands) -> Result<()> {
    let mut values = [120, 600];
    let settings = [
        ("read-timeout", "git-pr.readTimeout"),
        ("command-timeout", "git-pr.commandTimeout"),
    ];
    let keys: Vec<_> = settings
        .iter()
        .filter(|(flag, _)| !flags.supplied.contains(*flag))
        .map(|(_, key)| *key)
        .collect();
    let configured = Values::read(commands, &keys, Some("int"))?;
    for (index, (flag, key)) in settings.iter().enumerate() {
        let value = if flags.supplied.contains(*flag) {
            flags.string(flag).to_owned()
        } else {
            configured.string(key)
        };
        if flags.supplied.contains(*flag) || !value.is_empty() {
            values[index] = value
                .parse::<u64>()
                .ok()
                .filter(|v| *v > 0)
                .with_context(|| format!("{flag} must be a positive number of seconds"))?;
        }
    }
    commands.runner.configure_timeouts(
        std::time::Duration::from_secs(values[0]),
        std::time::Duration::from_secs(values[1]),
    );
    Ok(())
}

fn save_tags(flags: &Flags, commands: &Commands, dry_run: bool) -> Result<bool> {
    if flags.string("default-tags").is_empty() {
        return Ok(false);
    }
    let tags = comma_list(flags.string("default-tags"));
    if dry_run {
        commands.out(&format!(
            "[DRY-RUN] Would set default tags: {}",
            tags.join(", ")
        ));
    } else {
        commands
            .git(&["config", "--replace-all", "git-pr.tags", &tags.join(",")])
            .context(Fatal)?;
        commands.out(&format!("Set default tags: {}", tags.join(", ")));
    }
    Ok(true)
}

fn compact_stack(flags: &Flags, booleans: &Values) -> bool {
    boolean_setting(
        flags,
        booleans,
        "compact-stack",
        "git-pr.compactStack",
        false,
    )
}

fn boolean_setting(flags: &Flags, booleans: &Values, flag: &str, key: &str, default: bool) -> bool {
    if flags.supplied.contains(flag) {
        return flags.boolean(flag);
    }
    booleans.boolean(key).unwrap_or(default)
}

fn stack_up(flags: &Flags, booleans: &Values, commands: &Commands) -> Result<bool> {
    if flags.supplied.contains("reverse") {
        commands.warn("--reverse is deprecated; use --stack-up with the same Boolean value");
    }
    for flag in ["stack-up", "reverse"] {
        if flags.supplied.contains(flag) {
            return Ok(flags.boolean(flag));
        }
    }
    let current = booleans.boolean("git-pr.stackUp");
    let legacy = booleans.boolean("git-pr.reverse");
    if let (Some(current), Some(legacy)) = (current, legacy)
        && current != legacy
    {
        bail!(
            "conflicting git-pr.stackUp and deprecated git-pr.reverse values; retain the intended value in git-pr.stackUp and remove git-pr.reverse from Git configuration"
        );
    }
    if legacy.is_some() {
        commands.warn(
            "git-pr.reverse is deprecated; rename it to git-pr.stackUp without changing its value",
        );
    }
    Ok(current.or(legacy).unwrap_or(true))
}

fn boolean_settings(
    flags: &Flags,
    env: &BTreeMap<String, String>,
    commands: &Commands,
) -> Result<Values> {
    let mut keys = vec!["git-pr.stackEmoji"];
    if !flags.supplied.contains("github-stack")
        && !env.contains_key("GIT_PR_GITHUB_STACK")
        && !settings::enabled(flags, env, "no-stack", "GIT_PR_NO_STACK")
    {
        keys.push("git-pr.githubStack");
    }
    if !flags.supplied.contains("stack-up") && !flags.supplied.contains("reverse") {
        keys.extend(["git-pr.stackUp", "git-pr.reverse"]);
    }
    for (flag, key) in [
        ("draft", "git-pr.defaultDraft"),
        ("compact-stack", "git-pr.compactStack"),
        ("use-icon-font", "git-pr.useIconFont"),
    ] {
        if !flags.supplied.contains(flag) {
            keys.push(key);
        }
    }
    if !settings::enabled(flags, env, "skip-draft", "GIT_PR_SKIP_DRAFT") {
        keys.push("git-pr.skipDraft");
    }
    if !flags.boolean("branch-from-title") {
        keys.push("git-pr.branchFromTitle");
    }
    Values::read(commands, &keys, Some("bool"))
}

fn string_settings(flags: &Flags, commands: &Commands) -> Result<Values> {
    let mut keys = vec![
        "git-pr.tags",
        "git-pr.output",
        "git-pr.previewLimit",
        "git-pr.transitionJournal",
        "git-pr.addLabel",
        "git-pr.addTipLabel",
        "user.name",
        "user.email",
        "remote.pushDefault",
    ];
    if flags.string("draft-pattern").is_empty() {
        keys.push("git-pr.draftPattern");
    }
    if flags.hyperlinks.is_none() {
        keys.push("git-pr.hyperlinks");
    }
    Values::read_patterns(commands, &keys, None, &[r"branch\..*\.(pushremote|remote)"])
}
