# Configuration contract

Options accept one or two leading dashes.
Parsing stops at the first positional argument; `--` explicitly ends option parsing.
Boolean options may use `=true` or `=false`, including the accepted Boolean spellings `1`, `0`, `t`, `f`, `T`, `F`, `True`, `False`, `TRUE` and `FALSE`.
A following positional `false` is a revision, not an option value.
Repeated scalar options retain the final value; label options accumulate.
The verbosity switches are cumulative up to level two; an explicit false value resets verbosity.
The aliases `-v` and `--verbose` select verbose output, whilst `-vv` and `--debug` select debug output directly.
Repeating `-v` or `--verbose` selects debug output, including mixed spellings.
Unsupported symmetric-difference ranges, incomplete ranges and ranges combined with other commit arguments are rejected before repository discovery or working-copy changes.
Valid revision expressions continue to resolve after the initial Jujutsu snapshot.

| Options | Behaviour and default |
| --- | --- |
| `--draft` | Overrides `git-pr.defaultDraft`; defaults to true. Title patterns remain applicable. |
| `--stack-up` | Displays the base at the bottom and tip at the top; overrides `git-pr.stackUp`; true. Use `--stack-up=false` to invert the layout. |
| `--reverse` | Deprecated alias for `--stack-up`, with identical Boolean values. |
| `--github-stack` | Authorises routine native stack creation and extension without a confirmation prompt. Explicit CLI values precede the environment and Git configuration. |
| `--no-stack` | Disables native stack management regardless of other enabling sources. |
| `--yes`, `-y` | Accept confirmation prompts, including native stack dissolution and rebuilding, automatically; otherwise non-interactive input declines. Routine native linking requires no prompt. |
| `--dry-run` | Reports proposed association, publication and PR operations without rewriting commits, updating refs, snapshotting Jujutsu or changing configuration. |
| `--include-other-authors` | Includes commits whose author email differs from `user.email`; disabled by default. |
| `--skip-draft`, `--include-draft` | Excludes matching titles or overrides that exclusion. Filtering does not alter commit identities. |
| `--draft-pattern` | Comma-separated, case-insensitive wildcard patterns; CLI precedes `git-pr.draftPattern`. Defaults to `wip:*,draft:*,*[wip]*,*[draft]*`. |
| `--default-tags` | Saves comma-separated `git-pr.tags` and exits when supplied a non-empty value. |
| `-t` | Overrides default tags for the invocation when non-empty; per-commit `Tags` trailers remain applicable. |
| `--add-label`, `--add-tip-label` | Repeatable label lists for all submitted PRs or the submission tip. CLI values replace the corresponding Git-configured list. |
| `--output` | Preset or custom output template; defaults to `url-title`. |
| `--hyperlinks` | `auto`, `always` or `never` for links in rich PR results; overrides `git-pr.hyperlinks`; `auto`. Use `never` to retain visible URLs. |
| `--gh-hosts` | GitHub host configuration; defaults to `~/.config/gh/hosts.yml`. A leading `~` expands against `HOME`. |
| `--compact-stack` | Uses at most three adjacent-PR links instead of a complete stack list; overrides `git-pr.compactStack`; false. |
| `--read-timeout` | Positive structured-command deadline in seconds; overrides `git-pr.readTimeout`; 120. |
| `--command-timeout` | Positive backend/publication command deadline in seconds; overrides `git-pr.commandTimeout`; 600. |
| `--timeout` | HTTP timeout in seconds; defaults to 20. Non-positive values expire immediately. |
| `--stop-after` | Stops at a named checkpoint, as specified below. |
| `--branch-from-title` | Accepted configuration switch; it does not currently alter branch selection. |
| `-v`, `--verbose` | Application decision explanations and phase, command, HTTP and wait timings on standard error, followed by an elapsed-time summary. |
| `-vv`, `-v -v`, `--debug` | Verbose output plus detailed subprocess and HTTP diagnostics. |
| `--version`, `--help`, `-h` | Display version or usage and exit. |

Environment switches `GIT_PR_DRY_RUN`, `GIT_PR_YES`, `GIT_PR_NO_STACK`, `GIT_PR_SKIP_DRAFT` and `GIT_PR_INCLUDE_DRAFT` enable their corresponding behaviour only when equal to `1`; an explicit false CLI value does not cancel such an enabling environment switch.
When present, `GIT_PR_GITHUB_STACK` enables native management for `1` or case-insensitive `true`; other values disable it.
A supplied `--github-stack` takes precedence.
The Git `githubStack` setting is queried only when no CLI value, environment value or disabling override determines the result.
Invalid Boolean values in an overridden setting therefore cannot prevent the selected mode from running.
`GIT_PR_OUTPUT` and `GIT_PR_STOP_AFTER` supply fallbacks when their CLI values are empty.
Command-line arguments and consumed `HOME` and `GIT_PR_*` environment values must contain valid UTF-8; invalid values produce an explicit error without a panic.
Unrelated non-Unicode environment entries are ignored by application configuration and remain available to child processes.

## Git configuration

Policy preferences reside in the `[git-pr]` section of Git configuration.
Publication associations reside under `[branch]`, as specified in the [identity and recovery contract](publication-identity.md).
Configuration keys use camelCase, whilst command-line flags use kebab-case.
Git treats section and variable names as case-insensitive; punctuation remains significant.
The application queries `git config`; Git parses the files, processes `include` and `includeIf`, resolves configuration scopes and converts Boolean values.
No JSON, YAML or YML application configuration files are read.
GitHub CLI authentication files remain part of the separate credential lookup described below.

The [Git configuration rules](https://git-scm.com/docs/git-config) determine precedence: command configuration overrides worktree, repository, user and system configuration, in that order.
Worktree configuration applies when Git's `extensions.worktreeConfig` facility is enabled.
Within the resolved configuration, the last value for each supported key wins.
All policy preference keys are single-valued, including comma-separated label lists; repeated label keys do not accumulate across scopes.
An empty label value clears inherited labels, and an empty output value selects the default format.
Malformed Git configuration and invalid Boolean values are reported as errors.

| Key within `[git-pr]` | Meaning and default |
| --- | --- |
| `defaultDraft` | Creates all new PRs as drafts when true; when false, matching `draftPattern` titles still create drafts; `true`. |
| `stackUp` | Displays the base at the bottom and tip at the top; `true`. A false value inverts the layout. |
| `previewLimit` | Maximum number of commits displayed in the rich terminal selection preview; `20`. Set `0` to display every commit. |
| `githubStack` | Native GitHub stack management; `false`. |
| `compactStack` | Adjacent-PR navigation instead of complete stack history; `false`. |
| `readTimeout` | Positive structured-command deadline in seconds; `120`. |
| `commandTimeout` | Positive backend/publication command deadline in seconds; `600`. |
| `stackEmoji` | Description markers; `false`. |
| `skipDraft` | Draft-title filtering; `false`. |
| `draftPattern` | Comma-separated title patterns that create drafts even when `defaultDraft` is false; also used by `skipDraft`; `wip:*,draft:*,*[wip]*,*[draft]*`. |
| `tags` | Comma-separated default labels; empty. |
| `addLabel` | Comma-separated labels for all submitted PRs; empty. |
| `addTipLabel` | Comma-separated labels for the tip; empty. |
| `output` | Output preset or template; `url-title`. |
| `hyperlinks` | Links in rich PR results: `auto`, `always` or `never`; `auto`. |
| `branchFromTitle` | Accepted Boolean setting; currently inactive. |

Git performs Boolean conversion for `defaultDraft`, `stackUp`, the deprecated `reverse` alias, `githubStack`, `stackEmoji`, `compactStack`, `skipDraft` and `branchFromTitle`.
The accepted values therefore include Git's case-insensitive `yes`/`no`, `on`/`off`, `true`/`false` and integer forms.
A Boolean key without a value means true; an explicitly empty value means false.

Explicit CLI label options replace the corresponding configured label list.
Label reconciliation treats ASCII case variants as the same identity, preserves the service's spelling for deletions and does not remove a tip label that remains required by a common or commit-specific label setting.
Non-ASCII characters are preserved without additional case or accent normalisation.
Output precedence is a non-empty CLI value, then a non-empty `GIT_PR_OUTPUT` value, then the Git setting.
Other CLI and environment overrides retain the setting-specific rules described above.
In a separate Jujutsu workspace, the backing Git repository is resolved before application settings are queried.
The `--default-tags` option writes the repository's `git-pr.tags` value through Git.
Examples of configuration commands appear in [common commands](commands.md#configuration).

Output presets are `url-title`, `url` and `markdown`.
Templates support `{url}`, `{number}`, `{title}`, `{branch}`, `{hash}`, `{shorthash}`, `{host}`, `{repo}`, `{author}` and `{email}`.
The default template, `"{url} {title}"`, displays each PR's URL and title in plain output; its interactive presentation may use linked PR numbers.
Replacement is a single pass: placeholder-like text inside a commit title remains literal.
Invalid placeholder names are rejected.

### Terminal hyperlinks

`--hyperlinks=auto|always|never` and `--hyperlinks auto|always|never` select the same three modes as `git-pr.hyperlinks`.
The final explicit CLI value takes precedence over the effective Git setting, including when the CLI value is `auto`; the default is `auto` when neither is present.
Values are lowercase enum names, not Booleans, and empty or valueless settings are invalid.
Invalid CLI values fail before repository discovery; invalid effective Git values fail before authentication or publication.
Overridden invalid values do not prevent execution, although malformed Git configuration files remain errors.
Git resolves includes and scope precedence, including settings in a separate Jujutsu workspace's backing Git repository.

Automatic mode recognises direct VS Code and Zed terminals through `TERM_PROGRAM=vscode` or `TERM_PROGRAM=zed`.
An unknown terminal, invalid terminal identity or the presence of `TMUX`, `STY`, `SSH_CONNECTION` or `SSH_TTY` selects complete visible URLs.
These checks are conservative capability estimates; no terminal query is sent.
`always` bypasses that estimate within eligible rich results, and `never` displays complete URLs beneath each PR.
Neither mode forces rich formatting into redirected output, CI, dumb terminals or non-default output templates.
An explicitly supplied template equivalent to `{url} {title}` remains eligible for the default rich presentation.
No hyperlink environment override is supported; `FORCE_HYPERLINK` does not supersede the CLI or Git setting.

Hyperlinks are independent of colour, so `NO_COLOR` suppresses styling without disabling links.
Each linked PR number opens its repository's PR page; copying displayed text may copy only the number.
Use `never` or a plain URL output preset when complete addresses are required for copying or logs.
The setting affects console presentation only and is independent of `compactStack`, which controls PR-description navigation.

### Selected-commit preview

The rich terminal selection preview displays up to `git-pr.previewLimit` commits, with a default of 20.
For limits of at least two, larger selections retain commits at both ends, dividing the limit equally and assigning any additional slot to the first end in the configured display order.
A single intervening row reports the number of omitted commits.
The value must be a non-negative integer; `0` disables collapse.
Plain selection output and final PR results do not collapse commit lists.
Git resolves configuration scopes and includes; `git -c git-pr.previewLimit=0 pr` supplies an invocation-specific override.

### PR body templates

PR body templates are resolved relative to the discovered repository root, including when submission is invoked from a subdirectory.
The existing template precedence therefore applies consistently throughout the working tree.

### Stack orientation and migration

`git-pr.stackUp` and `--stack-up` govern the selected-commit preview, final PR listing and complete stack section in PR descriptions.
The default is true: the stack base appears at the bottom, and each dependent PR appears above its predecessor, with the tip at the top.
A false value inverts that display.
Orientation does not alter commit selection, publication order, dependency relationships or compact navigation.

The former `git-pr.reverse` key and `--reverse` flag remain deprecated aliases and emit a migration warning when used.
Their Boolean values retain exactly the same meaning: replace the name without negating the value.
Git resolves scope precedence and converts values separately for each key.
If both effective Git values are present, they must agree; conflicting values produce an error identifying both keys and the required migration.
Remove the deprecated key from its defining configuration scope after retaining the intended value under `git-pr.stackUp`.
An explicit value through either CLI name overrides both Git keys, including malformed overridden values.
Supplying both CLI names is permitted only when their final values agree.

### Draft selection

The `defaultDraft` setting supplies the default draft state for newly created PRs.
An explicit `--draft` value overrides that default for the invocation.
A newly created PR is a draft when this resolved default is true or its title matches a `draftPattern` pattern.

| Default after CLI overrides | Title matches `draftPattern` | New PR state |
| --- | --- | --- |
| `true` | Either | Draft |
| `false` | Yes | Draft |
| `false` | No | Ready for review |

For example, with `defaultDraft = false` and `draftPattern = "wip:*"`, `wip: add caching` creates a draft PR and `Add caching` creates a ready-for-review PR.
The same rule applies when `--draft=false` supplies the false value.
Existing PRs retain their draft or ready-for-review state on subsequent submissions.

The separate `skipDraft` setting requests exclusion of titles matching `draftPattern`, regardless of `defaultDraft` or an existing PR's state.
`--include-draft` overrides draft-title exclusion without restoring commits excluded by other selection rules.

### Execution checkpoints

| Checkpoint | Completed effects |
| --- | --- |
| `validate` | Configuration, repository discovery and working-tree validation; credentials are unnecessary. |
| `get-commits` | Selection, commit listing and initial title-pattern filtering. |
| `prepare` (`rewrite` alias) | Local publication associations resolved without writes or remote requests. |
| `push` | Pushes, local ref reconciliation, PR creation and base updates. |
| `pr-create` | Publication, ref reconciliation, PR creation, base updates and PR-number recovery; no propagation sleep. |

## Authentication and hosting

Authentication reads the selected host's `oauth_token`.
When absent, it consults the operating-system credential store under `gh:<host>`: the macOS Keychain or the Linux Secret Service login collection.
Run `gh auth login` to configure credentials.
HTTPS remote user information is excluded from the host used for credential lookup and API requests.
Embedded Git transport credentials remain available to Git; they do not replace the separate GitHub CLI credential lookup.
Rejected-remote diagnostics, verbose command traces and command-failure diagnostics redact URL user information whilst retaining the host and resource path.
Raw `?` and `#` characters in HTTPS user information require percent-encoding as `%3F` and `%23`, respectively; remote query strings and fragments are unsupported.
Malformed authority-like user information remains redacted when discovery rejects the remote.

GitHub.com is the supported hosting target; GitLab and Bitbucket are not supported.
Other hostnames are accepted in repository URLs and credential lookup, but all direct REST requests use `https://api.<host>/repos/<owner>/<repository>/…`.
GitHub Enterprise Server instead requires [`https://<host>/api/v3/…`](https://docs.github.com/en/enterprise-server@latest/rest/using-the-rest-api/getting-started-with-the-rest-api), which the current implementation does not construct.
GitHub Enterprise Cloud on GHE.com uses the compatible [`https://api.SUBDOMAIN.ghe.com/…`](https://docs.github.com/en/enterprise-cloud@latest/rest/pages/pages) form, but that deployment has not been validated.
Acceptance of a hostname does not establish complete support for its hosting service.

## Operational limitations

Publication selects the current branch's `pushRemote`, then `remote.pushDefault`, then the current branch's `remote`; without these settings, it selects `origin` or the sole unambiguous remote.
Current-branch discovery is omitted when `remote.pushDefault` determines the destination and no branch-specific `pushRemote` can override it.
Remote aliases and repository names are preserved in full, including hyphens, dots and a leading dot; only a terminal `.git` URL suffix is removed.
Stack discovery reads the complete selected history; structured command and HTTP observations exceeding 64 MiB fail explicitly.
Publication uses full object identities and batches at most 128 refspecs within a 64 KiB argument budget.
Git hooks run normally for each publication batch.
Pull-request observations and content mutations use at most 25 GraphQL aliases per request, with complete REST pagination where required.
Native linking uses the GitHub stack API directly and does not invoke the `gh-stack` extension.
When the selected PRs begin with the existing native stack's suffix, linking appends only the new PRs and preserves all preceding members, including merged history.
An unchanged submission still verifies publication through Git and obtains fresh PR observations, but performs no metadata mutation or fixed visibility wait.
Plain Git submissions require no rewrite backend.
Each trailer-free change must have a maintained local branch or bookmark; publication identity is stored separately from its message.
Unrelated, conflicted and protected checked-out refs are preserved.
If worktree discovery fails, local ref reconciliation is omitted and a warning reports that the completed push remains effective.
PR lookup errors stop reconciliation rather than indicating that no PR exists.
Description history reads must all succeed before any description is patched.
Managed stack delimiters shall either be absent or comprise exactly one `<!-- git-pr-stack-start -->` followed by one `<!-- git-pr-stack-end -->`.
A malformed or ambiguous delimiter structure produces an error identifying the PR and prevents updating that description.
Repair the delimiter pair, or remove all delimiters whilst preserving human text, before retrying.
Common, default and per-commit labels remain applicable when they also occur in the tip-label list.
A failed submission can leave completed pushes or PR updates in place; the diagnostics identify the failed phase.

## Preview and compact presentation

A dry run uses recorded repository history and does not snapshot uncommitted Jujutsu working-copy changes.
Publication associations are resolved or proposed in memory, whilst displayed commit identities remain unchanged.
Remote state is neither queried nor modified, and PR reconciliation is consequently reported as proposed work rather than a prediction of individual remote mutations.
Combining `--dry-run` with `--default-tags` previews the setting without writing it.
Checkpoints continue to limit the preview to their respective phases.

Compact presentation preserves content outside the managed footer and includes the preceding, current and following PR, where present.
It omits the complete historical list, emoji markers and configurable display orientation; navigation follows dependency order.
Switching back to a full footer cannot recover historical entries which no longer occur in any observed description or local history.
The complete footer remains the default.

Git performs integer conversion for configured command deadlines, including its supported numeric suffixes; CLI values are positive decimal seconds.
CLI deadline values override their corresponding Git settings.
Initial repository discovery and reads of the deadline settings use the built-in command defaults.
Each deadline applies to one subprocess; HTTP `--timeout` remains separate and neither setting imposes a total invocation deadline.
Captured output is limited to 64 MiB per stream, or 64 MiB for combined legacy diagnostics.
Oversized output terminates the command without accepting a truncated observation.
After the direct child exits, output draining has a one-second grace period; a descendant cannot prolong capture by keeping a stream open or continually writing to it.
