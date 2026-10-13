# Validation

Tests exercise policy through production pure functions, repository semantics through disposable Git and Jujutsu repositories, and submission through controlled process and HTTP boundaries.
Fixtures use deterministic identities, timestamps and private credential directories.
Neither production credentials nor production pull requests form part of acceptance.
A failed run retains its diagnostics; automatic retries do not constitute acceptance.
The [behavioural contract inventory](behavioural-contracts.md) defines independent expectations, permissible normalisation and bounded generated dimensions.
The [assurance guide](assurance.md) specifies generated campaigns, known-defect sensitivity, critical mutation analysis, semantic fuzzing and retained evidence.

## Platform coverage

Business policy is intended to be platform independent.
Selection, trailer interpretation, description rendering, configuration precedence and submission decisions have shared contracts, exercised through pure functions and in-memory boundaries.
The application also contains native adapters which invoke operating-system services and external executables.
Correctness of the shared policy does not, by itself, establish correctness of those adapters on every supported system.

| Area | Platform relevance | Validation |
| --- | --- | --- |
| Business policy and workflow decisions | The expected decisions are common to macOS and Linux. | The same unit assertions use values and in-memory command, HTTP, credential and clock boundaries. |
| Processes and captured streams | Process spawning, Unix sockets, signals, process groups and child waiting use the host kernel and platform implementations of Rust dependencies. | Native regressions execute deadlines, cancellation, bounded capture and descendant termination on both systems. |
| Credentials | macOS uses the `security` command; Linux uses Secret Service over D-Bus, selected by platform-specific compilation. | macOS tests verify the command contract through an isolated process boundary; Linux additionally exercises a private native credential store with `just credentials`. |
| Repository and executable integration | Git, git-branchless, Jujutsu, shells and filesystem operations execute in the host environment. | Disposable-repository acceptance and native compilation verify their integration with the application. |

For example, an in-memory command response can establish that submission stops after a capture error, but cannot establish that an actual descendant process terminates.
The native regression exercises that effect against the operating system.
Running it on both supported systems checks the shared Unix contract against both implementations; a passing macOS execution alone supplies no Linux execution evidence.

Local recipes, including `just check`, `just coverage`, `just parity` and `just backends`, execute on the host operating system.
They neither launch Docker nor automatically repeat themselves on another platform.
The [CI workflow](../.github/workflows/ci.yml) supplies the automated platform matrix: `just check` runs on macOS and Ubuntu runners, with `just credentials` additionally running on Ubuntu.
Separate backend, parity, coverage and benchmark jobs currently run on Ubuntu.
Thus, CI checks both supported platforms without requiring each developer to reproduce both environments locally.

Changes to shared native adapters require relevant regression evidence on macOS and Linux, obtained through CI or direct execution on those systems.
An optional Linux machine, virtual machine or container can provide earlier Linux evidence when developing on macOS.
Docker is one such environment choice, not a project dependency or a test requirement, and there is no repository script which provisions a local two-platform run.
Routine development may use native local checks and the CI matrix; duplicating every validation gate in Docker for every change is unnecessary.

## Behavioural evidence

Independent requirements and native Git observations take precedence when a legacy implementation shares a defect.
The generated property suite supplements example-based regressions with reproducible combinations of trailers, structured snapshots, literal refs, selections and managed descriptions.
Disposable-repository assertions independently check Git syntax, selected remotes, URL rewriting, uncertain publication and preservation of unrelated worktrees and index bytes.
Bounded transition and fault campaigns require stable PR identity, retained human text, no submission effects during preview or refused preflight, and observation before reconciliation after uncertainty.
`just assurance` executes these campaigns, demonstrates detection of six specifically reintroduced defects and replays the retained semantic fuzz corpus.
`just mutants [BASE]` and `just fuzz [SECONDS] [SEED]` provide the additional assertion-sensitivity and instrumented input campaigns described in the [assurance guide](assurance.md).

The legacy compatibility specification is commit `132dc66ea004d03606585c9bdfe95289bd0b9dcb` of `duanyutong/git-pr-go`.
Its nine test suites contain 74 test functions and 301 executed leaf cases; the original suite reports 29.0% statement coverage.
`tests/fixtures/reference-cases.json` records source digests, assertion locations, Rust test mappings and captured-call indices.
`tests/fixtures/reference.jsonl` contains 312 calls to unchanged reference production functions.
`scripts/reference_inventory.py` verifies the ledger against fresh reference results and existing Rust tests.
Nine reference functions concerning standalone configuration files are expressly superseded by the native Git configuration contract; their ledger entries identify the replacement tests rather than claiming unchanged file semantics.
Supplementary Rust tests exercise submission and external boundaries absent from the reference suite.
The defect regressions intentionally supersede reference behaviour which truncated dotted repository names, restored skipped drafts or removed labels still required by another policy source.
The separate Jujutsu workspace comparison also requires the excluded, unnumbered base commit in full-stack description context, using its original object identity and the exact fixture title and author.
This exception changes only the expected managed footer; repository snapshots, human description text, publication destinations and all other PR fields retain exact comparison.
Failed worktree discovery leaves local refs unchanged; failed history reads prevent description patches; failed PR lookup cannot authorise PR creation.
These corrected contracts are verified separately and are not normalised away by the differential comparison.

`just parity` compares 14 parser cases and 24 repository scenarios comprising 36 submissions.
Each implementation receives an independently seeded repository.
Comparisons cover exit status, stdout, stderr, commit hashes, trees, parents, complete messages, refs, checkout state, Jujutsu working-copy identity, PR content and external-operation arguments.
Counts are checked per invocation and phase.
Unattended native-link scenarios grant the legacy reference `--yes` solely to express the authorisation already supplied by the candidate's resolved native-stack option.
The candidate receives no acceptance switch; output, PR state and publication effects remain subject to exact comparison.
Separate native Git configuration tests verify global enablement, disabling overrides and retained confirmation for dissolution and rebuilding.
The candidate adds Git reads for `output`, `addLabel`, `addTipLabel`, `compactStack`, `readTimeout`, `commandTimeout` and `stackUp`.
Grouped discovery includes these logical settings before revision resolution, including when a syntactically valid revision subsequently fails to resolve.
An explicit `--stack-up` or deprecated `--reverse` value omits both orientation keys from the existing Boolean batch.
Invalid range syntax is rejected before discovery.
Its differential scenario requires unchanged repository state, identical failure output and no candidate external operations.
The comparison expands each declared batch into its logical setting reads, preserving the configuration contract whilst permitting consolidation into three ordinary subprocesses.
The four documented help additions are removed solely for comparison with the older parser help text.
The revised `-v` help is compared with its legacy wording.
The canonical `stack-up` option and deprecated `reverse` help are compared with the legacy `reverse` block; reference invocations receive the equivalent legacy flag.
The candidate preview follows the configured orientation; comparison reverses only its selected-commit rows when necessary to match the reference preview, preserving exact row contents and all subsequent output.
Separate regressions verify both preview orientations, final listing order, unchanged submission order, Git Boolean conversion, alias migration and conflicting or overridden settings.
The corrected `branch-from-title` compatibility description is compared with its legacy wording, which incorrectly promises a branch-generation effect.
The comparison also accounts for Git Boolean conversion for `skipDraft` and `branchFromTitle`.
Additional regressions verify refreshed human descriptions, process-group ownership after direct-child exit, cached schema capability, batched descendant observations and literal branch validation against Git.
Native configuration tests include repeated typed values, malformed superseded values and explicit CLI overrides.
The reference reads for `draft`, `github-stack`, `stack-emoji` and `branch-from-title` are mapped explicitly to `defaultDraft`, `githubStack`, `stackEmoji` and `branchFromTitle`; candidate reads must use the current names.
For each invocation containing the explicit `--github-stack` switch, the comparison permits omission of at most one redundant reference read of `githubStack` and requires zero candidate reads of that setting.
Required reads and duplicate reference reads remain subject to exact validation.
Publication comparisons expand bounded multi-ref pushes into their logical per-ref effects and require the same object identities and destinations.
Operation budgets require exclusive Git publication ownership, no production `gh` subprocesses, at most 128 refspecs and 64 KiB of refspec arguments per push, and at most 25 PR aliases per GraphQL request.
Content mutations additionally respect a five-alias maximum and a 1 MiB encoded request budget.
Request-boundary regressions require exactly 25 renders for 25 large descriptions, including renewal of an expired observation whose content remains unchanged.
A changed observation must instead participate in a fresh render, retaining newly observed human text whilst respecting the same request budget.
Unchanged submissions shall perform no description, label or native-topology mutation and no unconditional wait.
Completion barriers remain explicit between publication, creation, base changes, descriptions, labels and native linking.

The reference fixture substitutes loopback HTTP transport and instruments its existing sleeps without changing their durations.
The native extension fixture records the otherwise hidden second publication and models stack membership and base changes.
The Rust fixture invokes the production application with its real command runner, credential adapter, filesystem adapter and terminal, plus a clock which records and performs requested sleeps.
The application executable is also exercised directly for CLI behaviour.
Timing distinguishes elapsed wait intervals, their overlapping union, scheduled sleep totals, HTTP work and child publication.

Normalisation removes terminal colour, disposable paths and reference runtime backtraces; it preserves substantive errors, statuses and completed effects except for explicitly enumerated corrected contracts.
The exact Rust test-runner notice identifying the differential entrypoint as running for over 60 seconds is excluded from output comparison; raw evidence retains it.
These exceptions comprise the removed waiting announcement, accurate unchanged-PR and unchanged-stack reporting, the managed-footer separator, direct native-API diagnostics, and success when the reference's redundant native push alone would have failed.
The unavailable-native-service fixture additionally requires candidate failure and reference success explicitly: a native HTTP 404 shall no longer be converted into successful submission.
Unit regressions cover failure propagation at each native request boundary, including recreation after successful unstacking.
Computation budgets require one analysis per unchanged observed body and one predecessor-base plan per active submission, with native management disabled and enabled.
Changed observations invalidate parsed body state before mutation; regression cases retain refreshed human text and preserve newline handling.
Legacy-footer regressions require preservation of human issue references, conservative treatment of ambiguous PR lists, and idempotent migration with or without emoji and with either line-ending convention.
The captured `TestGeneratePRBodyWithCommitMessage` fixture retains the reference output; its candidate assertion explicitly requires preservation of the existing unmanaged description instead of replacement by the commit message.
Dedicated regressions require authoritative historical merge markers, bounded refreshes before native reconciliation, and preservation of unmanaged descriptions during adoption and repetition.
Further regressions require retention of known open PRs in explicit-selection context, expiring commit-association caches, and bounded refreshes before base reconciliation.
Pagination regressions require two GraphQL requests for a 101-PR branch history, preservation of initial label pages, bounded follow-up aliases and exclusion of foreign source repositories on every page.
Invalid cursors, incomplete label responses and missing REST repository identities must fail without caching incomplete results.
Label regressions require refreshed policy decisions, preservation of unrelated labels and retention of confirmed effects after partial failure.
The former rewriting dry-run scenario separately requires an unchanged candidate repository snapshot, zero service requests and zero rewrite or publication commands, whilst confirming that the reference did rewrite.
Native backend regressions additionally require unchanged Jujutsu operation identities, including with unsnapshotted files present.
Bulk backend transactions may produce different unpublished recovery objects and backend-owned recovery refs.
Comparisons therefore retain every initial object and every object ever reachable from a user ref, checkout or Jujutsu working copy, including its complete parent closure, whilst allowing backend-internal intermediate objects to differ.
Raw snapshots retain those internal differences for inspection.
Published commit hashes, trees, parents, complete messages, public refs and working-copy identities remain exact comparisons.
Rust reports fatal failures as an error chain with status 2, without a Go runtime stack trace.
Version output identifies this package's own release.

Native regressions require rejection of merge-resolution changes and changed commits without messages before submission effects, and acceptance of Jujutsu descriptions containing pipe characters.
Selection-recovery regressions cover snapshot reuse, tip-relative depth offsets, empty range boundaries, missing identities and bounded fallback queries with ordered results.
Additional regressions require preservation of explicit selections after rewriting against an advanced trunk, correct attribution, filtering and explicit selection of Jujutsu working copies, and failure when a working-copy observation is unavailable.
Publication preflight rejects the configured trunk and duplicate destinations introduced by inferred trailers before rewriting or publication.
Signature-display configuration shall neither contaminate structured Git metadata nor invoke signature verifiers during discovery.
Named-branch regressions require correct recovery from another checkout, preservation of the excluded base, and correct publication when a tag shares the branch name.
Unnamed-tip fallback verifies the selected first-parent position before rewriting, and an unavailable anchor or vanished range prevents successful submission.
Explicit Git arguments are deduplicated before resolution and resolved in batches of at most 128 revisions and 64 KiB of arguments.
Verified linear ancestry supplies selection depths even when the remote trunk has advanced independently.
Descendant discovery uses bounded candidate reduction followed by verification against every input batch; 129 branch tips require four ancestry subprocesses, whilst divergent histories remain explicitly ambiguous.

Workflow coverage includes first, changed and unchanged submissions; range and non-contiguous selection; descendant descriptions; all checkpoints; dry-run effects; author and draft filtering; labels, templates and history; native stack linking, rejection, rebuilding and partial failure.
Managed-footer regressions require explicit rejection of unmatched, reversed, nested and duplicate delimiters, preservation of observed human text and zero content mutations on repeated failed submissions.
Description-initialisation regressions reject reserved delimiters in commit messages and PR templates before the affected creation or content update, retain existing descriptions and permit safe repair and idempotent repetition in both footer styles and native modes.
Malformed remote regressions include raw slashes, whitespace and line boundaries in synthetic credentials, covering parser errors, verbose output, warnings, both command-failure protocols and percent-encoded equivalents.
Mixed-separator cases include nested scheme text and multiple `@` delimiters; diagnostic ambiguity may suppress additional text whilst successful observations retain their complete original bytes.
Explicit Jujutsu ranges and lists require one batched identity observation and no scalar tip-identity lookup, with and without rewrite eligibility.
An empty tip excluded from presentation retains its already observed recovery identity; an unavailable tip observation preserves fallible scalar recovery.
Safe repair and repeated successful submission are exercised with native management disabled and enabled, in both full-stack and compact presentation.
Full-stack regressions retain multiple unnumbered commits without additional PR creation or branch publication; compact navigation continues to contain numbered PRs only.
Synthetic credential regressions reject raw query and fragment separators whilst retaining diagnostic confidentiality and accepting their percent-encoded representation.
`just backends` requires actual git-branchless and Jujutsu executables at the declared versions.
Colocated and separate Jujutsu workspaces, described working copies, stable change-ID recovery and guarded ref reconciliation are exercised.
Missing backend executables constitute failure.
`just credentials` runs Linux authentication against a private D-Bus session and temporary Secret Service collection; macOS credential tests verify the security-command contract and supported encoded token representations through an isolated process boundary.

Native Git configuration tests use private system, user and repository settings, includes, conditional includes, worktree configuration and command-scope overrides.
They verify case-insensitive section and variable names, Git Boolean conversion, empty values, repeated scalar keys, CLI/environment overrides, invalid configuration errors and repository-local settings in separate Jujutsu workspaces.
Native Git path fixtures preserve trailing spaces, tabs and newlines whilst resolving the exact template and workspace metadata locations.
Isolated Jujutsu command fixtures verify the corresponding workspace and backing-repository path framing.
Standalone application JSON/YAML files are explicitly verified to have no effect.

Verbosity regressions require identical external operations, standard output and PR state with timing disabled, enabled and combined with detailed diagnostics, in both native-stack modes.
The timing adapter covers ordinary commands, structured reads and bulk editing exactly once per boundary invocation.
Additional cases verify checkpoints, discovery failure, bounded verbosity, concurrent event identifiers, disabled-formatting avoidance and explicit retry and mutation-spacing diagnostics.
Native CLI fixtures verify failure summaries in disposable non-repository directories.

## Coverage

`just coverage-unit` requires complete line, region and function coverage from unit execution alone within the production modules named in [coverage-policy.toml](../coverage-policy.toml).
The [unit coverage specification](unit-coverage.md) defines attribution, the enforced application boundary and metric limitations.
Its application tests use the `::unit::` filter, selecting shared and owner-local unit namespaces.
The [source organisation](development.md#source-organisation) retains shared application fixtures under `src/tests/`, with Python versioning tests in `scripts/test_version.py`.
Coverage and assurance inventories use the complete hierarchical source paths; a source move shall preserve each module's classification and independently measured obligations.
Independent profile directories prevent integration or differential execution from contributing to the unit percentage.

`just coverage` requires that unit gate and then enforces complete application line coverage, including CLI, command, credential, HTTP and VCS adapters.
The package report also verifies that the declared unit modules remain completely exercised within aggregate acceptance.
The inventory includes every application source file; `lib.rs` contains declarations and re-exports without executable logic.
Test modules and isolated reference sources are excluded from the report, not application adapters.

The collector obtains the environment from `cargo llvm-cov show-env`, builds and executes instrumented Rust tests, runs native Linux credential acceptance where applicable, and collects profiles from external comparison processes.
Collision-safe LLVM profile filenames prevent child-process overwrites.
JSON, HTML and text reports are retained under `coverage/`.
Stable doctests remain a separate required gate.
`just coverage-branches` requires complete, nonempty branch outcomes from isolated unit execution under the pinned nightly compiler, retaining JSON and HTML.
This command runs as a separate PR and scheduled CI gate; its compiler prerequisites, evidence paths and instrumentation limits are specified in the [assurance guide](assurance.md#unit-branch-coverage).
Line, region and function percentages do not measure the strength of assertions or establish complete combinations of input and external state.
The [contract inventory](behavioural-contracts.md) and [assurance guide](assurance.md) report generated dimensions, mutation outcomes, native observations and exploration budgets separately.

## Reproducibility and cost

`just check` runs the same hygiene, semantic, test, doctest and release recipes used by CI.
Run `just check`, `just coverage`, `just parity` and `just backends` explicitly before committing changes for acceptance.
Pre-commit hooks provide formatting and hygiene checks; pre-push verifies version history and performs lightweight, read-only built-in file checks.
Command-runner pins reside in `scripts/dev-tools.env`; hook dependencies and their runtime versions are declared in `prek.toml`.
The Python project requirement and environment resolution are recorded in `pyproject.toml` and `uv.lock`.
The compiler and dependency graph are fixed by `rust-toolchain.toml` and `Cargo.lock`.
The fuzz workspace has its own locked dependency graph, and instrumented fuzzing and branch coverage use a separately pinned nightly compiler without replacing the ordinary stable toolchain.
The [platform coverage contract](#platform-coverage) distinguishes shared policy tests, native integration obligations and the CI jobs which execute them.
Neither an installed application nor a user's Git aliases are replaced by acceptance.

Ordinary tests use six threads.
Comparison and timing fixtures default to two concurrent, independent scenarios, each with bounded process and HTTP deadlines.
Existing scenario directories are archived under `target/validation/history` before a new run.
Git branchless fixtures preserve rewrite timestamps; Jujutsu fixtures declare their randomness seeds and operation clocks.
Only declared executable paths, fixture configuration and instrumentation variables are inherited; captured environments exclude unrelated user settings.
Fixture servers close their listeners and join their own threads.
Failed logs and traces remain available for diagnosis.

`just bench` measures first, amended and unchanged submissions for stacks of 1, 5 and 10 commits across git-branchless, colocated Jujutsu and separate Jujutsu workspaces, with native stacking disabled and enabled.
Three independent samples per workload produce 54 paired scenarios and 324 application invocations.
Both candidates use ordinary optimised builds.
Raw timings, medians, maximum observed latency, phase spans, subprocess counts, request counts and deliberate waits are retained in `target/validation/bench.json` and the scenario evidence.
Phase spans may overlap; their sum is not total latency.
The reference retains its one-second concurrent push waits, three-second synchronisation wait and one-millisecond rewrite delays.
The candidate records actual service pacing and retries; it has no unconditional visibility or rewrite waits.
Reports distinguish the concurrent waiting interval from the sum of scheduled waits.
Loopback measurements are not claims about live GitHub latency.

Measured validation phases append elapsed time and status to `target/validation/timings.jsonl`.
Compilation, warm tests, instrumentation and external comparisons shall be reported separately from submission latency.
CI permits 45 minutes for platform validation and 120 minutes for each boundary job, including acquisition of uncached tools.
Evidence artefacts are uploaded even after failure.
The [acceptance record](validation.md) identifies observed results and their limits.

The differential harness accepts `--latency-ms` values of 0, 20, 100 and 300 to delay each observed service request, excluding local command telemetry.
This models per-request service latency; it does not model packet loss, Git transport latency or unobserved internal requests made by the reference GitHub CLI.
Use `--jobs 1` for isolated timing comparisons and retain at least three alternating-order samples when making repeatability claims.

Non-zero latency collections use separate `bench-latency-<milliseconds>` or `parity-latency-<milliseconds>` paths and reports, preserving the ordinary matrix and its executable cache.

## Version automation

The identical `scripts/test_version.py` suites exercise Conventional Commit policy, generated increments, formatting preservation, staged and unstaged changes, initial commits, amendments, push revisions and annotated tags in disposable repositories.
Tag-publication tests use private local bare remotes, including failed publication, repeated invocation, published baselines, unrelated tags and conflicts.
The tests use standard-library `unittest` and form part of `just test`.
The `just coverage-versioning` gate uses coverage.py to require at least 95% line coverage of `scripts/version.py`, including its command subprocesses.
These gates form part of `just coverage`.

### Rust-to-Rust attribution

The comparison harness accepts `--bench --rust-reference REVISION --sizes 1 10 50 --jobs 1` to build a pinned previous Rust revision in an isolated archive.
The developer checkout remains unchanged.
Reports include the baseline commit, tree, executable digest and release profile, alongside the candidate source and executable digests.
Both executables receive independently seeded repositories and the same loopback service model; published objects and completed remote effects remain compared.
The baseline application and clock remain unmodified, so its elapsed durations include waits even when separate wait instrumentation is unavailable.
Serial execution avoids competition between benchmark scenarios; alternating pair order reduces systematic execution-order bias.
Larger sizes and repetition counts are explicit inputs, and each submission has a size-adjusted fixture deadline.
Failed comparisons cancel queued scenarios; already running scenarios retain their ordinary command deadlines.
These measurements establish fixture-specific attribution and shall not be presented as live GitHub forecasts.

## Error-handling acceptance

The [error-handling contract](error-handling.md) specifies diagnostic content, recovery guidance, partial-effect reporting and unavoidable process-termination limits.
Regressions distinguish failed observations from absence, dirty state and unrelated ancestry; verify credential-parser redaction and bounded server diagnostics; and require publication-verification failures to preserve both causes.
Native executable tests cover unavailable commands, deleted working directories, broken output streams and unexpected-panic diagnostics without exposing panic payloads.
Interruption acceptance additionally requires exit with status 130 when its diagnostic pipe is full.
HTTP failure diagnostics are emitted through the error chain rather than duplicated on stdout.
Differential exceptions for those corrected diagnostics retain exit statuses, request effects and repository snapshots.

## Published-stack safety regressions

Reordered submissions shall fail before trailer rewriting, branch publication or PR mutation, with native stack management both enabled and disabled.
The same requirement applies to inferred branch names, ambiguous open identities, exclusively closed or merged branch history, and open dependants outside the verified selection.
A disposable Git repository supplies actual ancestry to a host-lifecycle fixture which recognises indirect merges, optionally deletes merged heads and retargets dependent PRs.
The regression verifies that the guarded application cannot reach that transition; an explicit unsafe push inside the disposable fixture verifies that the oracle detects the original failure mechanism.
This fixture is not a claim of complete equivalence with GitHub's asynchronous processing.

Merged-downstack recovery regressions exercise ordinary, squash and rewritten merge histories against actual disposable Git repositories, with the obsolete base both present and deleted.
They require preservation of the open PR identity, verified retargeting before publication and an explicit expected-object lease for each published ref.
A concurrent remote update must survive a rejected lease.
In-memory cases exercise Git, git-branchless and Jujutsu preparation, missing and reused identities, unintegrated merge results, native membership, unselected dependants, changing metadata, lost acknowledgements and retry after a repaired base survives publication failure.
Preview and rewrite checkpoints shall not mutate remote bases, and failed initial evidence shall prevent rewriting and publication.
Normal submissions retain their existing request budgets.

Mutation-barrier tests change lifecycle state, PR identity, node identity and base relationships during a push, within the ordinary cache lifetime.
They require one publication followed by fresh observation and zero subsequent PR mutations or replacement creation.
Incoming-dependency tests include pagination, unselected identities on later pages, malformed responses and observation failures.
Unchanged-submission budgets retain one base plan, one analysis per observed description, bounded pushes, zero metadata mutations and zero scheduled waits.
The additional safety observations are specified in the [efficiency contract](submission-efficiency.md).

Differential comparisons explicitly distinguish unsafe legacy base changes from the corrected refusal contract.
Previously successful base-changing partial selections and native rebuilds with stale PR bases must now leave the candidate's repository and PR state unchanged.
Permitted native rebuild tests retain correct PR base relationships and vary the native membership metadata independently.
Completion listings remain byte-for-byte comparable after accounting for their later position; recognised failures after publication must omit the candidate listing.

Native Jujutsu fixtures construct a single-parent conflict through rebasing and independently verify the backend's refusal to push it.
Application submission shall reject that conflict before local rewriting or publication, including when the conflicted ancestor is skipped and its selected descendant resolves the conflict.
Conflicted descendants outside the selected publication ancestry shall not prevent submission of an earlier clean commit.
In-memory protocol cases require explicit Boolean conflict observations and preserve conflict ancestry across empty commits omitted from presentation.
A disposable Git repository with `push.followTags=true` verifies that branch publication does not also publish an unselected annotated tag.
Replacement-ref fixtures cover loose and packed refs, a custom replacement namespace and pre-existing replacement-disabling environment values.
Their observed metadata and published refs shall identify the original stored objects, whilst replacement and graft views that conceal a merge shall not bypass merge rejection.
Both the repository graft file and an explicitly inherited custom graft file are covered.
A valid named remote beginning with a dash shall remain a literal publication destination and shall not be interpreted as command options.

## Request resilience and live progress

Deterministic scheduler tests cover distinct REST and GraphQL primary quotas, server-clock-relative reset headers, Retry-After values and HTTP dates, shared secondary cooldowns and conservative rolling content budgets.
Dispatch tests distinguish HTTP 200 GraphQL throttling, explicit HTTP rate limits, permission errors and transient read failures.
Retries shall preserve the original error chain, respect server deadlines and stop at their attempt and waiting budgets.

The content fixture applies each possible prefix of a bounded mutation before returning HTTP 502, HTTP 504, partial GraphQL data, malformed data, throttling or a lost response.
Recovery shall preserve completed changes without duplicate writes, retain fresh human text, and reject changed identities, lifecycle, bases and incompatible title edits.
Acknowledged mutation aliases shall leave recovery before further observation or replay, including partial throttling responses and completed earlier recovery chunks.
Title reversions and unrelated human title edits following acknowledgement shall remain untouched; malformed or mismatched acknowledgements shall leave intended PRs recoverable.
Unrelated observation errors shall fail without disabling subsequent stack-schema checks.
A separate wait-boundary regression edits human text whilst the clock advances and verifies that content is freshly rendered before the delayed write.
These tests shall perform no repeated Git publication or PR creation.
The executable API-failure comparison requires four bounded content attempts with intervening observations, whilst retaining exact repository, PR and publication-effect comparisons against the reference.

Terminal tests cover a single animation worker, named stage outcomes, bounded selection previews, separated result blocks, complete wrapped titles, aligned timings, narrow displays, Unicode grapheme widths, elapsed and remaining timers, confirmation suspension and durable redirected wait notices.
Within-stage counters cover PR creation, base checks, descriptions and label reconciliation; current PR identities precede ancillary request and quota details.
Description counters advance only after unchanged content is verified, a REST update succeeds or an entire dispatched batch is acknowledged or successfully recovered.
Recovery assertions require monotonic counters without duplicate completion, whilst rejection preserves an incomplete count.
Native pseudo-terminal fixtures at 40 and 80 columns withhold each operation until its intermediate count and current PR identity have been observed, including a changing timer before the first operation completes.
A native renderer regression measures actual repaint intervals during a burst of progress notifications and requires coalescing to at most ten animation frames per second, whilst the final count remains synchronously available in the completion summary.
Durable stage summaries retain final counts and partial counts on failure together with the corresponding operation label, and subsequent stages clear prior progress.
Pseudo-terminal capability cases distinguish interactive streams, redirected standard output, redirected standard error, CI, `TERM=dumb` and `NO_COLOR`.
Rich and plain workflow fixtures shall produce identical command and HTTP effects.
Executable CLI comparisons remove legacy colour styling from the reference, require control-free redirected candidate diagnostics, and retain exact diagnostic text and exit-status comparisons.
Status regressions require description, title and label changes to appear as updated during the successful invocation, including recovered uncertain writes, without persisting that status into a subsequent unchanged invocation.
Pseudo-terminal acceptance additionally verifies prompt input, cleared interruption diagnostics and exit status 130.
The status renderer remains an execution boundary; quota scheduling and retry decisions belong to independently tested policy.
