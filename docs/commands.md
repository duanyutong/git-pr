# Common commands

Submission commands operate from the Git repository or Jujutsu workspace.
The examples below assume that `git-pr` is installed as described in the [README](../README.md#installation).

## Authentication

Managed pull-request submissions use GitHub authentication configured through the GitHub CLI:

```sh
gh auth login
```

Standalone branch pushes use Git's authentication and do not load GitHub API credentials.

## Repository initialisation

Plain Git requires no additional initialisation.
Managed commits without a `Remote-Ref` message trailer require a maintained local branch or Jujutsu bookmark.
Submission stores their publication associations in repository-local Git configuration and publishes their original objects with expected-object leases.
Existing trailers remain compatible and are preserved verbatim.
Git-branchless may manage the local stack, but is not required for submission.

## Submission

An ordinary invocation on an unmanaged standalone Git branch pushes the complete branch to the selected remote under the same name and sets its upstream.
This applies to branches containing one or several commits, including merge commits and commits authored by automation.
The branch push uses ordinary fast-forward protection and does not prepare publication associations, create pull requests, reconcile descriptions or link a native stack.
Repeated invocations retain this behaviour, including when the remote branch is already current.

Automatic branch publication requires a checked-out branch distinct from the configured trunk, with no publication association or `Remote-Ref` trailer and no other local branch anchored within its unpublished history.
Existing publication evidence, explicit revision selections and explicitly supplied pull-request options retain the managed workflow.
Configured stack preferences alone do not prevent standalone branch publication.
Use an explicit range or commit argument to request a managed pull request for an otherwise standalone branch.

Managed selections may comprise the current stack, a range or explicitly named commits:

```sh
git pr                             # Standalone branch push or existing managed stack.
git pr main..feature                # Range excluding its base.
git pr feature-a feature-c          # Individual commits in ancestry order.
```

Subsequent managed submissions update the existing PRs for the selected commits.

Changes to published base relationships require a verified transition before publication.
The transition procedure prevents GitHub from recognising an unintended indirect merge whilst branch references and PR relationships disagree.
Incomplete evidence stops submission; `--yes`, `--no-stack` and native stack mode do not bypass these checks.

Publication also stops if an open PR outside the verified selection targets a branch whose published object would change.
An unchanged selected prefix may be resubmitted whilst later PRs remain outside the selection, provided that no publication-base transition is required.
Unchanged objects are verified against the actual push destination and published with explicit expected-object leases; tracking refs and PR-head observations alone do not establish this exception.
Include the complete dependent stack where appropriate.
A head with only closed or merged PR history is rejected; retrying does not create a replacement PR automatically.
A recorded merge may target another stack branch rather than the repository trunk, so the diagnostic shall not prescribe an unconditional rebase onto the trunk.
Dry runs remain local previews and do not establish that these remote safety requirements are satisfied.

## Pull request options

New PRs are drafts by default.
With `--draft=false`, titles matching `draftPattern` still produce drafts; other titles produce ready-for-review PRs.
Existing PR draft and ready-for-review states are preserved.
Native GitHub stack management is optional; its resolved configuration authorises routine creation and extension without another prompt.
This applies equally to `git-pr.githubStack=true`, the enabling environment setting and `--github-stack`, including when standard input is redirected.
`--no-stack` disables native management, and an explicit `--github-stack=false` overrides the Git setting.
Dissolving and rebuilding an existing stack retains its separate confirmation; `--yes` accepts that confirmation automatically.
Native GitHub stacks do not support auto-merge.
When a native membership change requires an existing PR with auto-merge enabled, submission stops during preparation whenever that condition is observable, before local publication metadata or remote state is changed.
Disable auto-merge explicitly to use native stacks, or retain it and use `--no-stack` for optional native management.
Publication recovery may require restoration of existing native membership even with optional native management disabled; that recovery requirement remains authoritative.
The client preserves auto-merge settings and checks eligibility again before native mutation, including before dissolving a stack for rebuilding.
Concurrent changes can still cause a later server rejection; completed effects remain retained and unconfirmed mutations require remote verification.

```sh
git pr --draft=false                # Drafts only for titles matching draftPattern.
git pr --github-stack               # Native GitHub stack membership enabled.
```

## Configuration

Git manages both user defaults and repository overrides:

```sh
git config --global git-pr.defaultDraft true       # User default.
git config --local git-pr.defaultDraft false       # Repository override.
git config --local git-pr.addLabel 'review, needs-ci'
git config --local git-pr.addTipLabel ''  # Inherited tip labels cleared.
git config --global git-pr.hyperlinks auto
git config --local git-pr.hyperlinks never  # Complete visible PR URLs.
git config --global git-pr.previewLimit 20  # Commits visible in the selection preview.
git config --local git-pr.previewLimit 0    # Complete selection preview.
git config --show-origin --get-regexp '^git-pr\.'
```

The final command lists configured values and their source files.
The [configuration reference](configuration.md#git-configuration) defines the supported keys and precedence.

## Command reference and execution limits

The built-in help lists flags and output fields:

```sh
git pr --help
```

The [configuration reference](configuration.md) describes precedence and execution checkpoints.
A dry run previews publication associations and subsequent operations without changing local or remote state.
Submission preserves commit identities and stores associations in [repository-local Git configuration](publication-identity.md).

## Console presentation and GitHub waits

Interactive submission presents named stages for repository discovery, working-copy validation, commit preparation, branch publication, PR reconciliation, descriptions, labels and native stack linking where applicable.
One refreshing status line identifies the current stage and operation, with elapsed time and completed counts where the operation exposes a reliable total.
Repository checks, the selected stack, submission progress and PR results have distinct headings and spacing.
Completed stages retain a compact outcome and aligned duration; a failed stage retains a failure marker before the explanatory error.
Completion messages distinguish resources checked from confirmed changes, including explicit statements when no changes were required.
Failure summaries identify confirmed branch publications and PR changes, and name mutations whose outcomes remain unconfirmed.
These summaries use standard error, remain available without verbosity flags and do not constitute the successful PR-result listing.
Known native eligibility failures present the cause and recovery actions once, after timing summaries; debug output additionally includes the complete contextual error chain.
The commit-discovery stage completes before its selected-stack preview is displayed.
Routine push, creation and native-link announcements occupy the live line rather than producing a second permanent list.
Warnings, dry-run previews, checkpoint messages and recovery instructions remain durable.

The selected-stack preview abbreviates stacks exceeding eight commits, showing the first and last four commits and the omitted count.
The final default listing presents every selected, unskipped PR as an aligned number and complete title, with non-zero outcome counts and repository context above the list.
Every PR row begins with an outcome glyph before its number: green `+` for created, yellow `~` for updated and subdued `=` for unchanged.
The glyphs remain visible without colour, including when all outcomes match; the heading retains ordinary created, updated and unchanged counts without additional glyphs.
Outcome markers do not change stack order.
Hyperlink-capable output uses clickable PR numbers; otherwise, complete URLs appear beneath the titles.
Titles wrap at word boundaries and preserve Unicode grapheme clusters; exceptionally long words split only between grapheme clusters.
An updated status includes confirmed title, description and label changes in the current invocation.
The preview and final listing follow `git-pr.stackUp`, labelled `tip first` by default and `tip last` when inverted.
Use `--stack-up=false` to invert both displays; custom output templates retain their existing representation.
Titles wrap beneath their starting column, or beneath the PR identity when a narrow pane leaves insufficient title space.
Interactive title and visible-address truncation respects grapheme boundaries and terminal display-cell widths.
PR title rows remain single-line; summaries and operational status retain complete facts through deliberate additional rows where necessary.
Use `--output=url` for complete copyable addresses, or redirect output to retain complete titles and addresses without inserted wrapping.

Interactive rich results use OSC 8 hyperlinks by default, including in remote sessions and unfamiliar terminal emulators.
The following invocation retains visible URLs when the terminal does not provide clickable hyperlinks:

```sh
git pr --hyperlinks=never
```

Use `git -c git-pr.hyperlinks=never pr` for a Git-scoped invocation override, or configure `git-pr.hyperlinks=never` persistently through Git.
The [hyperlink contract](configuration.md#terminal-hyperlinks) specifies eligibility, precedence and the preservation of plain and custom output.

Rich lists require both output streams to be interactive.
Redirected standard output retains the plain listing and custom-template representation.
Colour is disabled for redirected standard error, CI, `TERM=dumb`, or a non-empty `NO_COLOR` environment variable.
`NO_COLOR` disables colour while retaining interactive progress and eligible hyperlinks.
The renderer uses one worker, sleeps when idle or awaiting confirmation, and performs no Git or API operations.

During ordinary activity, the status line shows the current operation and elapsed time.
During a scheduled wait, the line shows the reason and remaining duration.
When GitHub supplies primary-quota headers, the display includes the most recently observed REST or GraphQL balance and reset countdown.
These are observations from ordinary requests; the application does not poll the rate-limit endpoint.
Secondary throttling displays the retry delay, since GitHub does not expose its remaining secondary allowance.

Confirmation prompts and durable output clear the status line before printing.
Redirected standard error, CI and terminals declaring `TERM=dumb` use durable wait notices without animation.
The existing `-v` and `-vv` diagnostics remain available alongside progress.
Control-C retains the existing cancellation behaviour and exit status 130.

Requests are serialised, mutations are spaced by at least one second after the preceding mutation completes, and content writes use at most five PRs per request.
A conservative session-local budget also limits content operations to sixty per minute and 450 per hour, counting each batched PR separately.
These local estimates cannot account for other applications using the same GitHub identity or undisclosed service limits.

Rate-limit rejection is retried at most three times, respecting Retry-After and primary-reset headers.
Secondary throttling without a server deadline starts with a sixty-second delay and exponential backoff.
Transient read failures receive bounded shorter retries.
Automatic server-directed waiting is bounded to fifteen minutes per request; a longer required deadline produces an explanatory error rather than an early retry.
Local content-budget pacing may require a longer wait, which is displayed and remains interruptible.

Uncertain description writes are re-observed before recovery, including failures which occur after some or all updates have applied.
Recovery omits completed fields, preserves fresh unmanaged description text and rejects changed PR identities, lifecycle, bases or incompatible title edits.
Long waits also require fresh descriptions before publication resumes.
Recovery reduces the write-batch size and makes at most three further attempts; failure identifies the outstanding PRs in the affected batch.
PR creation and topology mutations are not blindly replayed after an uncertain transport failure.

## Submission timing

Default output explains submission outcomes, failures and recovery actions without diagnostic flags.
Use `git pr -v` or `git pr --verbose` for application decision explanations and detailed elapsed-time records.
Decision explanations include selected bases, required updates and unchanged PR content.
Place verbosity switches before positional revisions, for example `git pr -v base..tip`.
To retain diagnostics whilst preserving the ordinary PR list on standard output:

```sh
git pr -v 2>git-pr-timing.log
```

Each operation emits a start and an end record with a shared identifier, elapsed seconds since submission began, its duration and its outcome.
A start record without a corresponding end record identifies an operation whose completion was not recorded.
The summary is emitted on successful completion, checkpoint exits and ordinary submission errors.
Argument-parsing, process-environment and current-directory failures which precede submission retain their direct error messages.

The records distinguish configuration and authentication, commit discovery and preparation, publication, PR reconciliation, description and stack-section processing, and native stack linking.
Command timing includes Git, Jujutsu and credential subprocesses, including structured reads and bulk editing.
HTTP timing records each attempt separately; dispatch queueing, deliberate mutation spacing and explicit rate-limit retry waits have separate records.
Command durations include executable startup, Git hooks, backend processing and any network work performed by the child; they do not distinguish those internal components.

The final elapsed time measures submission wall time.
Accumulated category durations may overlap because phases contain operations and workers can wait concurrently; their sum is not total wall time.
A long command duration directs investigation towards that subprocess, a long HTTP duration towards the service or connection, and a long wait or queue duration towards the named scheduling constraint.
The instrumentation adds no discovery queries, subprocesses, service requests or scheduled sleeps.

Use `git pr --debug`, `git pr -vv` or `git pr -v -v` when detailed command arguments, command results and HTTP payloads are also required.
The debug level includes verbose output; aliases share the same level rather than enabling independent logging controls.
Timing records at level one omit arbitrary command arguments, command results, URL queries and request or response bodies.
Existing warnings and failure diagnostics remain visible.
Endpoint paths may identify private repositories or resources; level two additionally includes repository content and should be reviewed before sharing.
Existing URL-credential redaction remains applicable at level two.
The ordinary command and request payload logger begins after configuration discovery; timing records cover discovery itself at both levels.
The verbosity switches do not change submission effects; `--dry-run` remains the separate option for previewing without publication.
