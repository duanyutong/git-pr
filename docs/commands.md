# Common commands

Submission commands operate from the Git repository or Jujutsu workspace.
The examples below assume that `git-pr` is installed as described in the [README](../README.md#installation).

## Authentication

GitHub authentication is configured through the GitHub CLI:

```sh
gh auth login
```

## Repository initialisation

The following initialisation applies only to Git repositories using git-branchless for trailer insertion:

```sh
git branchless init
```

Commits without a `Remote-Ref` message trailer require a local branch or bookmark.
The tool inserts the trailer through git-branchless or Jujutsu and force-pushes the recorded destination.
Git can operate without either rewrite backend when all required trailers already exist.
Jujutsu does not require git-branchless initialisation.

## Submission

The selection may comprise the current stack, a range or explicitly named commits:

```sh
git pr                             # Current stack.
git pr main..feature                # Range excluding its base.
git pr feature-a feature-c          # Individual commits in ancestry order.
```

Subsequent invocations update the existing PRs for the selected commits.

Existing PRs must retain their published base relationships.
Reordering published commits, inserting a commit below an existing PR, or selecting commits in a manner which would change an existing PR's base is currently rejected before trailer rewriting or publication.
This restriction prevents GitHub from recognising an unintended indirect merge whilst branch references and PR relationships disagree.
Restore the published order or arrange a separately verified transition before retrying; `--yes`, `--no-stack` and native stack mode do not bypass this requirement.

Publication also stops if an open PR outside the verified selection targets a branch which would be pushed.
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
A dry run previews missing trailers and subsequent operations without changing local or remote state.
Draft filtering applies before rewriting and is reapplied when the rewritten stack is read.

## Console presentation and GitHub waits

Interactive submission presents named stages for repository discovery, working-copy validation, commit preparation, branch publication, PR reconciliation, descriptions, labels and native stack linking where applicable.
One refreshing status line identifies the current stage and operation, with elapsed time and completed counts where the operation exposes a reliable total.
Repository checks, the selected stack, submission progress and PR results have distinct headings and spacing.
Completed stages retain a compact outcome and aligned duration; a failed stage retains a failure marker before the explanatory error.
The commit-discovery stage completes before its selected-stack preview is displayed.
Routine push, creation and native-link announcements occupy the live line rather than producing a second permanent list.
Warnings, dry-run previews, checkpoint messages and recovery instructions remain durable.

The selected-stack preview abbreviates stacks exceeding eight commits, showing the first and last four commits and the omitted count.
The final default listing separates each selected, unskipped PR into a block containing its number, created, updated or unchanged status, complete title and full URL, together with aggregate counts above the list.
Titles wrap at word boundaries and preserve Unicode grapheme clusters; exceptionally long words split only between grapheme clusters.
An updated status includes confirmed title, description and label changes in the current invocation.
The preview and final listing follow `git-pr.stackUp`, with the base at the bottom and tip at the top by default.
Use `--stack-up=false` to invert both displays; custom output templates retain their existing representation.
Preview truncation and section rules respect grapheme boundaries and terminal character widths.

Rich lists require both output streams to be interactive.
Redirected standard output retains the plain listing and custom-template representation.
Colour is disabled for redirected standard error, CI, `TERM=dumb`, or a non-empty `NO_COLOR` environment variable.
`NO_COLOR` disables colour while retaining interactive progress.
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

Use `git pr -v` to record progress and elapsed time for a normal submission.
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

Use `git pr -vv` or `git pr -v -v` when detailed command arguments, command results and HTTP payloads are also required.
Timing records at level one omit arbitrary command arguments, command results, URL queries and request or response bodies.
Existing warnings and failure diagnostics remain visible.
Endpoint paths may identify private repositories or resources; level two additionally includes repository content and should be reviewed before sharing.
Existing URL-credential redaction remains applicable at level two.
The ordinary command and request payload logger begins after configuration discovery; timing records cover discovery itself at both levels.
The verbosity switches do not change submission effects; `--dry-run` remains the separate option for previewing without publication.
