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

Local recipes, including `just check`, `just coverage` and `just backends`, execute on the host operating system.
They neither launch Docker nor automatically repeat themselves on another platform.
The [CI workflow](../.github/workflows/ci.yml) supplies the automated platform matrix: `just check` runs on macOS and Ubuntu runners, with `just credentials` additionally running on Ubuntu.
Separate backend and coverage jobs run on Ubuntu.
Thus, CI checks both supported platforms without requiring each developer to reproduce both environments locally.

Changes to shared native adapters require relevant regression evidence on macOS and Linux, obtained through CI or direct execution on those systems.
An optional Linux machine, virtual machine or container can provide earlier Linux evidence when developing on macOS.
Docker is one such environment choice, not a project dependency or a test requirement, and there is no repository script which provisions a local two-platform run.
Routine development may use native local checks and the CI matrix; duplicating every validation gate in Docker for every change is unnecessary.

## Behavioural evidence

Independent requirements and native repository observations determine the expected behaviour.
The generated property suite supplements example-based regressions with reproducible combinations of trailers, structured snapshots, literal refs, selections and managed descriptions.
Disposable-repository assertions independently check Git syntax, selected remotes, URL rewriting, uncertain publication and preservation of unrelated worktrees and index bytes.
Bounded transition and fault campaigns require stable PR identity, retained human text, no submission effects during preview or refused preflight, and observation before reconciliation after uncertainty.
`just assurance` executes these campaigns, demonstrates detection of six specifically reintroduced defects and replays the retained semantic fuzz corpus.
`just mutants [BASE]` and `just fuzz [SECONDS] [SEED]` provide the additional assertion-sensitivity and instrumented input campaigns described in the [assurance guide](assurance.md).

The Rust application is exercised directly through its command-line entry point, production workflow and injected process, HTTP, credential and clock boundaries.
Expected results derive from the documented contracts and independently observed repository state.
Failed worktree discovery leaves local refs unchanged; failed history reads prevent description patches; failed PR lookup cannot authorise PR creation.
Configuration regressions cover global enablement, disabling overrides, repeated typed values, malformed superseded values, explicit command-line overrides, Git Boolean conversion and both stack orientations.
Invalid revision-range syntax is rejected before discovery.
Valid revision resolution and configuration discovery retain bounded command counts.

Operation budgets require exclusive Git publication ownership, no production `gh` subprocesses, at most 128 refspecs and 64 KiB of refspec arguments per push, and at most 25 PR aliases per GraphQL request.
Content mutations additionally respect a five-alias maximum and a 1 MiB encoded request budget.
Request-boundary regressions require exactly 25 renders for 25 large descriptions, including renewal of an expired observation whose content remains unchanged.
A changed observation instead participates in a fresh render, preserving newly observed human text within the same request budget.
An unchanged submission performs no push, description update, label mutation, native-topology mutation, local ref synchronisation or unconditional wait.
Observation freshness and cache timestamps use the same injected clock; unit fixtures advance that clock explicitly to exercise expiry independently of host speed.
Routine operation-budget cases cover batching boundaries; the optional 1,000-PR case is invoked with `cargo test --locked unchanged_submission_at_large_scale -- --ignored` and carries no elapsed-time acceptance threshold.
Completion barriers remain explicit between publication, creation, base changes, descriptions, labels and native linking.

Computation budgets require one analysis per unchanged observed body and one predecessor-base plan per active submission, with native management disabled and enabled.
Changed observations invalidate parsed body state before mutation; regression cases retain refreshed human text and preserve newline handling.
Legacy-footer regressions require preservation of human issue references, conservative treatment of ambiguous PR lists and idempotent migration with or without emoji and with either line-ending convention.
Further regressions require historical merge markers to remain authoritative, retention of known open PRs in explicit-selection context, expiring commit-association observations and bounded refreshes before base or native reconciliation.
Pagination regressions require two GraphQL requests for a 101-PR branch history, preservation of initial label pages, bounded follow-up aliases and exclusion of foreign source repositories on every page.
Invalid cursors, incomplete label responses and missing REST repository identities fail without caching incomplete results.
Label regressions require refreshed policy decisions, preservation of unrelated labels and retention of confirmed effects after partial failure.

Native fixtures execute the production command runner, filesystem access, terminal and credential adapter with isolated credentials and local service endpoints.
Dry-run and preparation checkpoints require unchanged repository snapshots and zero service mutations or publication commands.
Native backend regressions additionally require unchanged Jujutsu operation identities, including when unsnapshotted files are present.
Published objects, trees, parents, complete messages, public refs, checkout state and Jujutsu working-copy identities are independently asserted.
Internal unpublished recovery objects are inspected separately from public effects.

Native regressions require rejection of merge-resolution changes and changed commits without messages before submission effects, and acceptance of Jujutsu descriptions containing pipe characters.
Selection-recovery regressions cover snapshot reuse, tip-relative depth offsets, empty range boundaries, missing identities and bounded fallback queries with ordered results.
Additional regressions require preservation of explicit selections against an advanced trunk, correct attribution, filtering and explicit selection of Jujutsu working copies, and failure when a working-copy observation is unavailable.
Publication preflight rejects the configured trunk and duplicate resolved destinations before persistence or publication.
Signature-display configuration shall neither contaminate structured Git metadata nor invoke signature verifiers during discovery.
Named-branch regressions require correct recovery from another checkout, preservation of the excluded base, and correct publication when a tag shares the branch name.
Trailer-free selected commits require maintained anchors; an unavailable or ambiguous anchor prevents publication.
Explicit Git arguments are deduplicated before resolution and resolved in batches of at most 128 revisions and 64 KiB of arguments.
Verified linear ancestry supplies selection depths even when the remote trunk has advanced independently.
Descendant discovery uses bounded candidate reduction followed by verification against every input batch; 129 branch tips require four ancestry subprocesses, whilst divergent histories remain explicitly ambiguous.

Workflow coverage includes first, changed and unchanged submissions; range and non-contiguous selection; descendant descriptions; all checkpoints; dry-run effects; author and draft filtering; labels, templates and history; native stack linking, rejection, rebuilding and partial failure.
Native eligibility regressions distinguish disabled, enabled and unconfirmed auto-merge observations, reject incompatible creation or rebuilding before effects, and retain server evidence for later validation failures.
Output regressions distinguish checked resources, confirmed changes and unconfirmed mutations, and require equivalent submission effects for verbose and debug aliases.
Managed-footer regressions require explicit rejection of unmatched, reversed, nested and duplicate delimiters, preservation of observed human text and zero content mutations on repeated failed submissions.
Description-initialisation regressions reject reserved delimiters in commit messages and PR templates before the affected creation or content update, retain existing descriptions and permit safe repair and idempotent repetition in both footer styles and native modes.
Malformed remote regressions include raw slashes, whitespace and line boundaries in synthetic credentials, covering parser errors, verbose output, warnings, both command-failure protocols and percent-encoded equivalents.
Mixed-separator cases include nested scheme text and multiple `@` delimiters; diagnostic ambiguity may suppress additional text whilst successful observations retain their complete original bytes.
Explicit Jujutsu ranges and lists require one batched identity observation and no scalar tip-identity lookup, with and without existing trailers.
An empty tip excluded from presentation does not require a publication association.
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
Independent profile directories prevent integration execution from contributing to the unit percentage.

`just coverage` runs that strict core Rust unit gate and the separate Python versioning sanity guard.
Executable lines, regions and functions shall each be 100% covered in every declared unit module; integer counts, including zero uncovered entries, determine acceptance.
The exhaustive inventory also classifies native boundaries, mixed modules, declarations and generated implementations without assigning an aggregate application percentage target.
Test implementations and test-only helpers are excluded from the production denominator, whilst production generic functions instantiated with in-memory doubles remain valid production evidence.
CLI runtime, command, credential, HTTP and VCS adapters retain their behavioural assertions through ordinary and backend acceptance; Linux additionally requires native `just credentials` execution.

The collector cleans independent instrumentation storage, builds and executes only the declared Rust unit selection, and rejects missing, duplicate, foreign, empty or incomplete metric entries.
It clears the summary before validation or collection, preventing an earlier success from representing a failed attempt.
JSON, HTML and text reports under `coverage/unit/` identify the source scope, stable compiler and test selection.
Versioning reports remain separate under `coverage/`; their counts are neither averaged with Rust counts nor presented as complete repository coverage.
Stable doctests remain a separate required gate.
Ordinary validation and coverage acceptance require no nightly compiler.
Line, region and function percentages do not measure the strength of assertions or establish complete combinations of input and external state.
The [contract inventory](behavioural-contracts.md) and [assurance guide](assurance.md) report generated dimensions, mutation outcomes, native observations and exploration budgets separately.

## Reproducibility and cost

`just check` runs the same hygiene, semantic, test, doctest and release recipes used by CI.
Run `just check`, `just coverage` and `just backends` explicitly before committing changes for acceptance.
Pre-commit hooks provide formatting and hygiene checks; pre-push verifies version history and performs lightweight, read-only built-in file checks.
Command-runner pins reside in `scripts/dev-tools.env`; hook dependencies and their runtime versions are declared in `prek.toml`.
The Python project requirement and environment resolution are recorded in `pyproject.toml` and `uv.lock`.
The compiler and dependency graph are fixed by `rust-toolchain.toml` and `Cargo.lock`.
The fuzz workspace has its own locked dependency graph; only its separate instrumented campaign requires the pinned nightly compiler, without replacing the ordinary stable toolchain.
The [platform coverage contract](#platform-coverage) distinguishes shared policy tests, native integration obligations and the CI jobs which execute them.
Neither an installed application nor a user's Git aliases are replaced by acceptance.

Ordinary tests use six threads.
Native fixtures enforce bounded process and HTTP deadlines.
Git branchless fixtures preserve rewrite timestamps; Jujutsu fixtures declare their randomness seeds and operation clocks.
Only declared executable paths, fixture configuration and instrumentation variables are inherited; captured environments exclude unrelated user settings.
Fixture servers close their listeners and join their own threads.
Failed logs and traces remain available for diagnosis.

Measured validation phases append elapsed time and status to `target/validation/timings.jsonl`.
Compilation, warm tests and instrumentation shall be reported separately from submission latency.
CI permits 45 minutes for platform validation and 120 minutes for each boundary job, including acquisition of uncached tools.
Evidence artefacts are uploaded even after failure.
The [validation evidence guide](validation.md) specifies the provenance and limits required when reporting results.
The [efficiency contract](submission-efficiency.md#verification-and-interpretation) defines repeat-submission operation budgets independently of elapsed timing.
Local service fixtures establish correctness and operation counts; their timings do not establish live GitHub latency.

## Version automation

The `scripts/test_version.py` suite exercises Conventional Commit policy, generated increments, formatting preservation, staged and unstaged changes, initial commits, amendments, push revisions and annotated tags in disposable repositories.
Tag-publication tests use private local bare remotes, including failed publication, repeated invocation, published baselines, unrelated tags and conflicts.
The tests use standard-library `unittest` and form part of `just test`.
The `just coverage-versioning` gate uses coverage.py to retain a 95% statement-coverage sanity guard for `scripts/version.py`, including its command subprocesses.
A passing run reports the guard as passed and requires no additional work towards 100%; a failed guard emits detailed percentage and missing-statement diagnostics and fails acceptance.
JSON, HTML and diagnostic text remain available in `coverage/` for either outcome.
This guard forms part of `just coverage`, separately from the principal Rust unit result.

## Error-handling acceptance

The [error-handling contract](error-handling.md) specifies diagnostic content, recovery guidance, partial-effect reporting and unavoidable process-termination limits.
Regressions distinguish failed observations from absence, dirty state and unrelated ancestry; verify credential-parser redaction and bounded server diagnostics; and require publication-verification failures to preserve both causes.
Native executable tests cover unavailable commands, deleted working directories, broken output streams and unexpected-panic diagnostics without exposing panic payloads.
Interruption acceptance additionally requires exit with status 130 when its diagnostic pipe is full.
HTTP failure diagnostics are emitted through the error chain rather than duplicated on stdout.

## Published-stack safety regressions

Complete, verifiable reordered submissions shall preserve open PR identities through a safe publication transition, with native stack management both enabled and disabled.
Unverifiable transitions shall stop before remote mutation, including inferred branch names without sufficient evidence, ambiguous open identities, exclusively closed or merged branch history, and open dependants outside the verified selection of a topology transition or a changing publication destination.
An unchanged five-commit prefix of a larger published stack shall remain submittable with legacy trailers and local publication associations, with native stack management both enabled and disabled, whilst preserving unselected dependent PRs, remote refs and stored commit objects.
Destination-object regressions shall distinguish unchanged, missing and changed remote refs independently of PR-head observations, retain dependency-observation expiry for changing destinations, and reject concurrent updates through explicit expected-object leases.
A disposable Git repository supplies actual ancestry to a host-lifecycle fixture which recognises indirect merges, optionally deletes merged heads and retargets dependent PRs.
The regression verifies that planned reordering preserves open PRs; an explicit unsafe push in an independent disposable fixture verifies that the oracle detects the original failure mechanism.
Synthetic insertion cases exercise every stack position with the inserted branch both present and absent on the remote.
In-memory permutation cases cover Git, git-branchless and Jujutsu, complete native membership and ordinary PRs.
Interruption cases stop execution after each completed mutation and require a subsequent invocation to finish from fresh observations without replacing existing PR identities.
This fixture is not a claim of complete equivalence with GitHub's asynchronous processing.

Merged-downstack recovery regressions exercise ordinary, squash and rewritten merge histories against actual disposable Git repositories, with the obsolete base both present and deleted.
They require preservation of the open PR identity, verified retargeting before publication and an explicit expected-object lease for each published ref.
A concurrent remote update must survive a rejected lease.
In-memory cases exercise Git, git-branchless and Jujutsu preparation, missing and reused identities, unintegrated merge results, native membership, unselected dependants, changing metadata, lost acknowledgements and retry after a repaired base survives publication failure.
Preview and preparation checkpoints shall not mutate remote bases, and failed initial evidence shall prevent metadata writes and publication.
Normal submissions retain their existing request budgets.

Mutation-barrier tests change lifecycle state, PR identity, node identity and base relationships during a push, within the ordinary cache lifetime.
They require one publication followed by fresh observation and zero subsequent PR mutations or replacement creation.
Incoming-dependency tests include pagination, unselected identities on later pages, malformed responses and observation failures.
Unchanged-submission budgets retain one base plan, one analysis per observed description, zero pushes, zero metadata mutations and zero scheduled waits.
The additional safety observations are specified in the [efficiency contract](submission-efficiency.md).

Refused base-changing partial selections and native rebuilds with stale PR bases preserve repository and PR state.
Permitted native rebuild tests retain correct PR base relationships whilst varying native membership metadata independently.
Recognised failures after publication omit the successful completion listing.

Native Jujutsu fixtures construct a single-parent conflict through rebasing and independently verify the backend's refusal to push it.
Application submission shall reject that conflict before local metadata writes or publication, including when the conflicted ancestor is skipped and its selected descendant resolves the conflict.
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

Terminal tests cover a single animation worker, named stage outcomes, bounded selection previews, separated result blocks, complete wrapped titles, aligned timings, narrow displays, Unicode grapheme widths, elapsed and remaining timers, confirmation suspension and durable redirected wait notices.
Within-stage counters cover PR creation, base checks, descriptions and label reconciliation; current PR identities precede ancillary request and quota details.
Description counters advance only after unchanged content is verified, a REST update succeeds or an entire dispatched batch is acknowledged or successfully recovered.
Recovery assertions require monotonic counters without duplicate completion, whilst rejection preserves an incomplete count.
Native pseudo-terminal fixtures at 40 and 80 columns withhold each operation until its intermediate count and current PR identity have been observed, including a changing timer before the first operation completes.
A native renderer regression measures actual repaint intervals during a burst of progress notifications and requires coalescing to at most ten animation frames per second, whilst the final count remains synchronously available in the completion summary.
Durable stage summaries retain final counts and partial counts on failure together with the corresponding operation label, and subsequent stages clear prior progress.
Pseudo-terminal capability cases distinguish interactive streams, redirected standard output, redirected standard error, CI, `TERM=dumb` and `NO_COLOR`.
Rich and plain workflow fixtures shall produce identical command and HTTP effects.
Native executable regressions require control-free redirected diagnostics and assert their text and exit statuses.
Status regressions require confirmed published-head, description, title and label changes to appear as updated during the successful invocation, including recovered uncertain writes, without persisting that status into a subsequent unchanged invocation.
Pseudo-terminal acceptance additionally verifies prompt input, cleared interruption diagnostics and exit status 130.
The status renderer remains an execution boundary; quota scheduling and retry decisions belong to independently tested policy.

## Terminal width and information retention

Compact PR rows remain single-line and truncate only descriptive titles, with independent change and lifecycle indicators.
Regression cases require retained selected and historical rows, all outcome counts, complete repository context, Unicode grapheme boundaries, safe hyperlinks and complete redirected text.
Status cases assert complete progress, quota, retry and reset information independently of line-width assertions.
Native pseudo-terminal tests exercise independently sized standard-output and standard-error streams, resizing between one-row and two-row frames, and complete clearing before durable output.
Unclassified operational messages and substantive diagnostics remain complete; exceptionally narrow panes fall back to ordinary messages rather than clipped facts.
