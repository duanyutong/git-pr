# Acceptance record

## Confirmed writes and literal repository state

A further bounded review confirms four defect classes in publication recovery and native repository observations.
Description recovery previously replayed acknowledged updates after partial GraphQL failures, permitting a subsequent human title edit to be overwritten or to obstruct unrelated pending updates.
Recovery now excludes each acknowledged alias whose returned PR number matches its intended target, including partial throttling responses and successful recovery chunks preceding a later failure.
Stored Git objects now govern metadata and ancestry despite local replacement refs or legacy graft files, preventing publication metadata from diverging from transferred objects and preventing virtual history from concealing merge parents.
Invocation-scoped settings preserve repository settings and the caller's environment.
Root discovery preserves significant trailing path whitespace and removes only the command's newline record terminator, retaining exact template and workspace metadata locations.
Publication and recovery terminate Git options before literal remote destinations, including valid names beginning with a dash.

Isolated counterexamples reproduce the acknowledged-write replay, replacement metadata mismatch, path truncation and leading-dash remote failure before their corrections.
Nine additional Rust tests cover these cases, concealed merge ancestry, acknowledged recovery chunks, malformed acknowledgements and unrelated GraphQL observation errors.
Native replacement fixtures cover loose and packed refs, a custom replacement namespace and existing replacement-disabling environment values.
Native merge fixtures cover replacement refs, repository graft files and explicitly inherited custom graft files.
The remote-name regression also verifies recovery after a lost push response.
Native Git path fixtures retain spaces, tabs and newlines; separate Jujutsu path framing is verified through isolated command boundaries.
Normal operation budgets remain unchanged, with no additional discovery or publication subprocesses.

Final macOS arm64 acceptance passes `just check`, `just coverage`, `just coverage-branches`, `just parity`, `just backends` and `just assurance`.
The application passes 464 library tests, two executable tests and six CLI tests; versioning passes 28 unit tests and 19 integration tests, giving 519 Rust tests in total.
Application coverage includes all 7,856 measured lines and 904 functions.
Independent stable unit coverage includes all 6,393 declared policy lines, 10,427 regions and 733 functions; pinned nightly instrumentation covers all 1,438 measured unit branch outcomes.
The coverage inventory, classifications, exclusions and completeness requirements remain unchanged.
Versioning package coverage remains 423 of 429 measured lines under its separate threshold.
Differential acceptance passes 24 workflow scenarios and 14 direct CLI comparisons; native acceptance covers git-branchless and both Jujutsu layouts.
Generated contracts pass with `PROPTEST_CASES=128` and `GIT_PR_ASSURANCE_CASES=8`; all six retained historical defects are detected by assertion failures, and fuzz replay passes 777 retained and generated cases.
These observations establish the reproduced contracts within the stated local platform and input bounds, without establishing atomicity with concurrent external actors or universal correctness.

Earlier validation attempts retain regression-test lint failures, a superseded completion expectation, an uncovered observation-error branch and a stale historical-defect anchor alongside their corrected passing runs.
The sensitivity anchor alone changed after the five primary gates; targeted harness tests, repository hygiene and Python type checks passed before assurance was repeated.
Source hashes verify that application code and Rust tests remained unchanged throughout that repetition.
Reproductions, gate logs, coverage summaries and source inventories reside under `target/validation/follow-up-audit/`.

## Publication safety and release recovery

A bounded correctness review confirms four defect classes in publication and release recovery.
Raw Git publication previously accepted unresolved Jujutsu conflict trees which native Jujutsu refused.
Conflict metadata now accompanies existing Jujutsu observations, and checks before rewriting and after recovery reject conflicts in the observed ancestry of selected publication heads.
Excluded ancestors and empty commits removed from presentation retain their publication barrier, whilst excluded conflicted descendants do not prevent submission of resolved ancestors.
Branch and release-tag pushes explicitly disable automatic annotation following, preventing `push.followTags=true` from publishing unrelated annotated tags.
Incoming PR-dependant observations expire independently for each group of at most 25 destinations after 30 seconds, measured from its first page request; unsuccessful refreshes do not renew them.
Automatic release discovery uses version-tag targets advertised by `origin`, so local tags retained after a failed push cannot cause a retry to omit their publication.

Isolated regressions reproduce the preceding conflict publication, unintended tag publication, stale-dependant acceptance and automatic-release retry failures before their corrections.
Seventeen additional tests cover these behaviours, strict metadata decoding, selection boundaries, slow acquisition, unsuccessful refresh and conflict observation after rewriting.
The final macOS arm64 run passes `just check`, `just coverage`, `just coverage-branches`, `just parity`, `just backends` and `just assurance`.
The application passes 455 library tests, two executable tests and six CLI tests; versioning passes 28 unit tests and 19 integration tests, giving 510 Rust tests in total.
Application coverage includes all 7,807 measured lines and 897 functions.
Independent stable unit coverage includes all 6,344 declared policy lines, 10,347 regions and 726 functions; pinned nightly instrumentation covers all 1,432 measured unit branch outcomes.
Versioning package coverage includes 423 of 429 measured lines, or 98.60%, under its separate existing threshold.
Coverage classifications, exclusions and completeness requirements remain unchanged.

Parity passes all 24 workflow scenarios and 14 direct CLI comparisons, including the existing operation budgets.
Conflict inspection adds no subprocess, and fresh dependant observations require no additional request.
All six deliberately reintroduced historical defects are detected, and semantic fuzz replay passes 777 retained and generated cases across three targets.
An additional review campaign passes 4,096 cases for each of the seventeen generated properties with seed `20261016`; the final assurance gate repeats the ordinary generated and bounded workflow campaigns after the corrections.
These observations establish the stated local evidence and bounded freshness policy, without establishing atomicity with external GitHub actors or universal correctness.

An initial Clippy assertion-style failure and a subsequent uncovered post-rewrite error region are retained alongside the corrected passing runs; the missing error-path assertion is now a regression.
Gate logs, coverage summaries, input hashes and reproduction evidence reside under `target/validation/correctness-performance-review/`.
Release reproduction results are retained in the agent tool transcript and indexed in `release-evidence.json`, with archived baseline sources under `target/validation/release-audit-baseline/`.

## Hierarchical Rust module organisation

Production modules with children consistently use `directory/mod.rs`, with descriptive filenames for leaf modules.
The application source root contains only `lib.rs` and `main.rs` beside the responsibility-based directories; the independent versioning helper follows the same convention.
CLI resources accompany argument parsing and presentation, GitHub owns its HTTP adapter, configuration owns settings precedence, the model owns commit trailers, and stack policy owns selection and managed descriptions.
The public entry point, visibility boundaries, command sequencing and trailer representation are preserved.

An inventory comparison retains all 493 test entries after normalising the intended module-namespace changes.
A supplementary comparison of 94 existing Rust source files finds no unexpected implementation changes after accounting for module declarations, imports, namespace and resource paths, whitespace and formatter trailing commas.
Coverage classifications and thresholds remain unchanged; the two new declaration-only module roots are explicitly inventoried.

Final macOS arm64 acceptance passes `just check`, `just coverage`, `just coverage-branches`, `just parity`, `just backends` and `just assurance`.
The application passes 444 library tests, two executable tests and six CLI tests; versioning passes 27 unit tests and 14 integration tests.
Application coverage includes all 7,737 measured lines and all 888 functions.
Independent stable unit coverage includes all 6,274 declared policy lines, 10,226 regions and 717 functions; pinned nightly instrumentation covers all 1,426 measured unit branch outcomes.
Differential acceptance passes 24 workflow scenarios and 14 direct CLI comparisons, whilst native acceptance covers git-branchless and both Jujutsu layouts.
All six retained historical defects are detected when deliberately reintroduced into disposable copies, and final fuzz replay passes 777 cases across the three semantic targets.
These local results retain the existing operating-system and input-space limits described in the testing and assurance specifications.

The initial sandboxed dependency-policy attempt failed to resolve GitHub; the network-enabled acceptance rerun passed, and both records are retained.
File mappings, test inventories, the implementation comparison, coverage summaries, frozen input hashes and complete gate logs reside under `target/validation/hierarchical-modules/`.

## Bounded progress repainting

Rapid condition-variable notifications previously bypassed the renderer's intended animation interval.
A retained native regression observed 101 terminal redraws within 127.462 ms during a burst of progress notifications.
The renderer now retains a monotonic deadline between animation frames, coalescing notifications whilst preserving immediate state updates and synchronous completion or failure summaries.
The regression measures actual repaint intervals, requires an initial observation before the update burst and verifies the final 100/100 summary immediately after completion returns.
All 19 progress tests pass, including native pseudo-terminal assertions at 40 and 80 columns.
This result establishes reduced terminal-write frequency; it does not establish a reduction in end-to-end submission latency.

Final macOS arm64 acceptance passes `just check`, `just coverage`, `just coverage-branches`, `just parity` and `just backends`.
The application passes 444 library tests, two executable tests and six CLI tests; versioning passes 27 unit tests and 14 integration tests.
Application coverage includes all 7,733 measured lines and all 888 functions, whilst the independent unit branch collector covers all 1,426 outcomes.
The coverage inventory, classifications, exclusions and completeness requirements remain unchanged.
The accompanying review established no further actionable defect within the examined GitHub scheduling and recovery, Git and Jujutsu state handling, description rendering or process boundaries.
Final logs, the pre-change reproduction, the source hash inventory and the measured coverage summary reside under `target/validation/post-progress-review/`.

## Live progress within submission stages

Interactive status prioritises completed and total item counts, current PR identities and a live timer ahead of stage, request and quota details.
PR creation identifies the current title before an identity exists, whilst base checks, description batches and label reconciliation identify their current PRs.
Description counts advance after unchanged content is verified, an individual REST update succeeds or a dispatched batch is acknowledged or successfully recovered.
Recovery counts each batch once, and failures retain a conservative incomplete count.
Durable summaries retain the operation label beside its count so that a subset of base checks cannot be mistaken for the entire reconciliation stage.
The wider summary column preserves the stage name at ordinary terminal widths; narrow rendering remains bounded by terminal cell width.

Native pseudo-terminal acceptance at 40 and 80 columns withholds work until every intermediate count and current PR identity has appeared, and observes an advancing timer before releasing the first operation.
Assertions also cover unchanged content, mixed transport paths, uncertain and rejected writes, creation, base and label failures, Unicode, progress reset and completed or partially completed summaries.
Existing batching, scheduling, retry, publication and worker semantics remain unchanged, and redirected output retains its previous contract.

Final macOS arm64 acceptance passes `just check`, `just coverage`, `just coverage-branches`, `just parity` and `just backends`.
The application passes 443 library tests, two executable tests and six CLI tests; versioning passes 27 unit tests and 14 integration tests.
Application coverage includes all 7,727 measured lines and all 888 functions.
Independent stable unit coverage includes all 6,270 declared policy lines, 10,226 regions and 717 functions, whilst pinned nightly instrumentation covers all 1,426 measured unit branch outcomes.
The coverage inventory, classifications, exclusions and completeness requirements remain unchanged.
Final acceptance logs reside under `target/validation/live-progress/`, with application reports under `coverage/` and branch evidence under `target/validation/assurance/branches/`.

## Complete application lines and unit branch outcomes

Application acceptance covers all 7,645 measured application lines and all 876 functions on macOS arm64.
Independent stable unit execution covers all 6,217 declared policy lines, 10,131 regions and 711 functions.
The pinned nightly unit collector covers all 1,426 measured branch outcomes, with zero uncovered outcomes.
The application inventory, unit classifications and source exclusions remain unchanged.

Additional assertions exercise interrupted and failed reads, unavailable command and progress workers, failed exit observation, invalid terminal input, bounded HTTP observations, diagnostic write failures and credential failures.
Borrowed I/O and fallible worker-construction boundaries permit deterministic failure assertions against the production error policy whilst retaining native process and terminal fixtures.
Simplified private interfaces encode existing caller invariants; publication identity checks, command sequencing and trailer representation remain protected by behavioural assertions.

Acceptance passes `just check`, `just coverage`, `just coverage-branches`, `just parity` and `just backends`.
The application passes 433 library tests, two executable tests and six CLI tests; versioning passes 27 unit tests and 14 integration tests.
Coverage-policy validation passes 15 Python tests, and assurance validation passes 14 Python tests.
The versioning package retains 382 of 390 measured lines, or 97.95%, under its separate existing package threshold.

Separate Linux aarch64 execution passes native credential acceptance, all five HTTP boundary tests and Clippy.
The Linux credential adapter covers all 14 measured lines, including unavailable D-Bus and denied Secret Service search failures in disposable sessions.
These Linux observations establish the stated boundary results rather than completion of the full Linux acceptance suite.

Application line coverage and unit branch outcomes now have strict 100% gates; ordinary and scheduled CI execute the branch requirement under the pinned nightly compiler.
Exact integer counters determine acceptance, including rejection when a displayed percentage rounds to 100% despite a missed outcome.
Complete measured coverage does not establish all input combinations, all execution paths or absence of defects.
Final acceptance logs reside in `target/validation/coverage-completion/`, with application reports under `coverage/`, branch evidence under `target/validation/assurance/branches/` and separate Linux evidence under `target/validation/linux-credentials-*`.

## Submission timing and verbosity levels

`-v` now enables monotonic phase, command, HTTP, dispatch-queue and scheduled-wait diagnostics, including repository discovery and authentication.
`-vv` and repeated `-v` additionally enable the existing detailed payload diagnostics.
No `--debug` option or additional configuration read is introduced.
Start and end records share identifiers; the final summary records elapsed time and bounded aggregates on success, checkpoints and ordinary submission failure.
Phase and concurrent-operation durations may overlap and are expressly not additive wall time.

The command decorator forwards deadlines, structured input, results and bulk message editing without introducing a second execution or counting an adapter's nested execution twice.
HTTP attempts retain their existing ordering and retry policy; mutation-spacing and rate-limit waits report their purpose and requested duration separately from measured time.
Disabled diagnostics neither construct labels nor allocate timing state.
Timing records omit command payloads and HTTP bodies, whilst detailed diagnostics remain an explicit level-two choice.

Both native-stack modes pass regressions comparing external operations, standard output and PR state with diagnostics disabled and at both enabled levels.
Repeated unchanged submissions retain a single push and no scheduled wait.
Additional cases cover bounded verbosity, explicit false resets, early failures, checkpoints, concurrent event pairing, structured commands, bulk editing and rate-limit attempts.
Native CLI fixtures verify discovery and failure summaries in disposable directories.

Acceptance passes `just check`, `just coverage`, `just parity` and `just backends` on macOS.
The application suites pass 300 library tests, two executable tests and six CLI tests; versioning passes 27 unit tests and 14 integration tests.
The declared application policy modules retain complete unit coverage: 5,380 lines, 8,837 regions and 613 functions.
Aggregate application line coverage is 6,179 of 6,228 lines, or 99.21%.
The timing boundary itself has 211 of 212 lines covered and every function exercised.
Parity passes 24 repository scenarios comprising 36 submissions per implementation, and 14 direct CLI comparisons.
Native backend acceptance passes with git-branchless 0.11.1 and the declared Jujutsu 0.46.0 installation.

Logs and retained coverage reports reside in `target/validation/verbose-timing/`.
No submission against the developer's working stack was executed.
The change supplies measurements for investigating the reported latency; it does not establish the cause or claim a reduction in that latency.

## Early rejection, publication discovery and label identity

Publication discovery omits current-branch lookup when `remote.pushDefault` determines the destination and no branch-specific `pushRemote` can override it.
This removes one Git subprocess in the affected configuration, whilst retaining branch-specific precedence and case-sensitive branch names.
The regression rejects an injected branch-discovery failure as irrelevant to the already determined destination.

Explicit native-stack CLI values, environment values and disabling overrides exclude the unused `githubStack` Boolean from the Git query.
Malformed overridden values cannot prevent the selected mode from running; malformed effective settings remain errors.
In-memory regressions cover enabling, disabling, empty environment values and competing overrides, and native Git fixtures verify invalid Boolean values under CLI overrides.

Revision arguments are classified once into borrowed range endpoints or commit arguments before repository discovery.
Unsupported symmetric-difference ranges, incomplete ranges and mixed range/list arguments fail with the existing diagnostic and zero external operations.
Valid expressions retain resolution after the initial Jujutsu snapshot.
Native Jujutsu acceptance verifies that rejected syntax leaves the operation identity unchanged despite unsnapshotted working-copy content.

Label reconciliation uses consistent ASCII case-insensitive identities for deduplication, observation matching, required-label protection and cache removal.
Deletion uses the observed service spelling, and one successful deletion retires all cached case variants of that identity.
The regression performs one required addition and one obsolete-label deletion, then requires zero mutations during repeated reconciliation and after a fresh observation with different capitalisation.
Unrelated labels and non-ASCII characters are preserved; additional Unicode case or accent equivalences are not inferred.
Credential-free public reads of the GitHub CLI repository's `bug` and `BUG` label endpoints returned the same label identity; the observations are retained with the test evidence.

The `branch-from-title` help text now describes its existing inactive compatibility behaviour accurately.
Differential comparison permits only this exact wording correction, the omission of a redundant overridden native-stack setting read, and the earlier rejection of invalid range syntax.
The native-stack comparison requires zero candidate setting reads and permits at most one reference read for an explicitly enabled invocation.
The invalid-range comparison preserves failure-output and repository-state comparisons whilst requiring zero candidate external operations.
Python regressions reject concealed duplicate reads, unexpected discovery, changed failure status and additional repository objects.

All four required acceptance gates pass on macOS arm64 with the configured six Rust test threads.
The application passes 295 library tests, two executable tests and five CLI tests; versioning passes 27 unit tests and 14 integration tests.
The 227 application unit tests cover all 5,291 declared policy lines, 8,695 regions and 590 functions across 31 modules.
Application line coverage is 5,878 of 5,926 lines, or 99.19%; versioning package line coverage is 382 of 390 lines, or 97.95%.
Parity passes all 24 repository scenarios, comprising 36 submissions per implementation, and 14 direct CLI comparisons.
The final harness-only correction additionally passes all 12 Python validation regressions, complete lint and the full parity gate.
Native backend acceptance passes with the declared git-branchless and Jujutsu installations.

The concluding review identifies no further confirmed actionable defect within the examined discovery, selection and reconciliation paths.
This is a bounded review result rather than proof of universal correctness or optimality.
Logs, reports, four pre-change failing regressions and the subsequent parity-contract corrections reside in `target/validation/review-after-0.2.23/`.

## Validation order, fallible metadata and retained cached state

Output settings and draft-pattern resource limits are validated before backend discovery, credential access or an initial Jujutsu snapshot.
Required credentials are obtained before that snapshot, whilst successful submissions retain snapshot-before-revision-resolution ordering.
The archived revision reproduces snapshots preceding rejection of invalid output settings and unavailable credentials.
A native Jujutsu regression verifies that rejected output settings leave the operation identity unchanged despite unsnapshotted working-copy content.

Commit-discovery checkpoints, rewrite checkpoints and previews omit PR-template filesystem reads because those paths do not render creation content.
An inaccessible template therefore cannot prevent those workflows from completing.
Publication paths retain template inspection before rewriting or publication, including recognition of an existing initial description matching the template.
Regressions verify both omission of unused reads and rejection of required, inaccessible templates before publication.

Jujutsu metadata inspection propagates filesystem errors with the affected path and an access-recovery instruction.
Permission failures and symbolic-link loops cannot silently authorise selection of another backend.
Native filesystem fixtures distinguish absent metadata, existing metadata and inspection failures.

Output templates prepare URL and number requirements once, derive only the required values and reserve the final output capacity before emission.
Formatting regressions preserve repeated fields, literal braces and multibyte text.
Cached PR-number assignment updates the number without replacing the commit or copying its owned descriptions and other metadata.
The archived 100-commit fixture replaces 99 unchanged descriptions and copies 5,940,000 description bytes.
Both positive and negative cache regressions preserve each description’s original storage address and all unrelated commit metadata, with zero worker starts and zero external operations during assignment.
Asynchronous operations that may create a PR retain their existing complete-snapshot semantics.

All four required acceptance gates pass sequentially on macOS arm64.
The application passes 291 library tests, two executable tests and five CLI tests; versioning passes 27 unit tests and 14 integration tests.
The 223 application unit tests cover all 5,264 declared policy lines, 8,648 regions and 585 functions across 31 modules.
Application line coverage is 5,851 of 5,899 lines, or 99.19%; versioning package line coverage is 382 of 390 lines, or 97.95%.
Coverage uses one Rust test thread; the final check and backend gates use the configured six concurrent Rust test threads.
Parity passes all 24 repository scenarios, comprising 36 submissions per implementation, and 14 direct CLI comparisons.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.

The concluding review covers configuration precedence and framing, revision selection and identity recovery, draft matching, rewriting, batched publication, partial-push verification, PR observations, descriptions, labels, cache freshness, native topology, process capture and diagnostics.
It identifies no further confirmed actionable defect within those examined paths; this finding does not constitute proof of universal correctness or optimality.
Final logs, reports and isolated pre-change reproduction evidence reside in `target/validation/iterative-review-after-0.2.22/`.
The diagnostic Clippy attempt remains separately identified; its subsequent correction changes only assertion diagnostics.

## REST observation validation and shared associations

Complete REST observations require explicit PR identity, reconciled content, labels, state, merge status and references before entering the cache.
An omitted description cannot establish empty human content; explicitly null descriptions remain valid empty content.
Required-field regressions preserve prior cached snapshots, unknown membership and service descriptions after decoding failure, and prevent rendering or mutation from incomplete observations.
Successful creation responses must identify the requested head and base in the selected repository, with open, unmerged state.
Mismatched creation responses leave the commit unassigned and caches unchanged, without a second creation request.
Their diagnostics identify the requested head and base and explain that the remote creation may already have succeeded, requiring verification before another submission.
These regressions use malformed synthetic responses; they do not establish that the live GitHub service emits such responses.

Commit-association pagination deduplicates overlapping identities whilst preserving first-observation order.
The overlap fixture retains 101 unique PRs from 102 observations and resolves its sole open PR without a false ambiguity diagnostic.
Repeated full pages, including reordered pages and pages containing exclusively foreign source repositories, fail after the second request with association and page context.
Incomplete pagination results cannot populate the cache.

Association decoding retains only PR numbers, state and head names after repository validation.
The pre-change 100-PR corpus retained 6,000,000 unused description bytes, plus unrelated titles and labels.
The new decoder skips those fields during deserialisation, including malformed unused metadata, whilst preserving required identities.
Cache insertion and retrieval share one immutable association list without copying its vector or owned strings.
The response bytes must still be received and scanned; these results establish allocation and retention changes rather than a live-service latency forecast.

All four required acceptance gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 285 library tests, two executable tests and five CLI tests; versioning passes 27 unit tests and 14 integration tests.
The 219 application unit tests cover all 5,236 declared policy lines, 8,623 regions and 582 functions across 31 modules.
Application line coverage is 5,823 of 5,871 lines, or 99.18%; versioning package line coverage is 382 of 390 lines, or 97.95%.
Parity passes all 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Final logs and retained JSON reports reside in `target/validation/perf-review-after-0.2.21/`; the isolated pre-change reproductions reside in `target/validation/review-after-0.2.21/`.
The final check log is `check-after-coverage.log`; earlier diagnostic attempts remain separately identifiable and do not constitute final acceptance.

## Shared history, borrowed context and prepared rewrites

History extraction accumulates unique identities directly into one shared vector and identity set, without retaining an individual parsed history list for each PR.
The isolated version 0.2.20 corpus retained 1,000,000 entries and 16,384,000 bytes of vector capacity for 1,000 complete PR footers.
The committed regression processes the same row cardinality whilst retaining only 1,000 unique entries in one vector, whose capacity is bounded at 2,048 entries.
Managed and legacy descriptions preserve source-order duplicate precedence, merged markers, prior observations and both presentation orders.
The required inspection of observed footer bytes remains proportional to the input size.

Description context borrows submitted commits and owns only newly discovered descendants.
The previous 100-commit fixture copied 6,000,000 commit-message bytes solely to construct that context; borrowed context requires no such message copies.
Both full-stack and compact renderers accept borrowed commits across independently owned submitted and descendant collections.
Native topology deserialises member numbers without retaining descriptions, labels or other unrelated member fields.
The previous 100-member fixture retained 6,000,000 unused description bytes during topology reconciliation.
Required topology fields and membership safeguards remain enforced; missing, invalid, zero and duplicate identities cannot authorise native changes.

Single-commit and delimiter-fallback rewrites reuse already serialised messages in the existing reverse command order.
Both backend regressions require one serialisation per commit for empty, single, bulk and literal-fallback inputs, with exact scalar command arguments.
The pre-change scalar fixture required two serialisations for one commit and six for three fallback commits, despite issuing only one and three rewrite commands respectively.
Trailer representation, managed descriptions and external-operation sequencing retain their existing acceptance contracts.

All four required acceptance gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 279 library tests, two executable tests and five CLI tests; versioning passes 27 unit tests and 14 integration tests.
The 213 application unit tests cover all 5,163 declared policy lines, 8,549 regions and 576 functions across 31 modules.
Application line coverage is 5,750 of 5,798 lines, or 99.17%; versioning package line coverage remains 382 of 390 lines, or 97.95%.
Parity passes all 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Final logs and retained JSON reports reside in `target/validation/perf-review-after-0.2.20/`; the isolated pre-change measurements reside in `target/validation/review-after-0.2.20/reproductions-final.log`.
Earlier diagnostic attempts remain distinguishable from the final passing gates.

## Publication destinations, batched revisions and footer fragments

Publication destination regressions cover branch push settings, the default push remote, branch tracking settings, origin fallback, detached checkouts and case-sensitive branch subsections.
Native Git fixtures verify scope precedence and subsection identity without contacting a remote.
An additional alphabetically earlier backup remote cannot override a configured origin destination.
Missing configured remotes, ambiguous destinations and multiple push URLs stop before publication or HTTP observation.
Destination settings remain part of the three existing configuration queries.

Initial Jujutsu resolution requires two processes for 100 ordinary arguments and nine for 1,000, including one immutable operation observation.
Native Jujutsu fixtures verify aliases, quoted Unicode bookmarks and independently rejected zero-match and multiple-match arguments.
Malformed records, unexpected argument indices, incomplete membership framing, invalid object identities and the 64 KiB command budget retain explicit failures.

GraphQL field normalisation retains the original description allocation before deserialisation, verified with a 120,000-byte UTF-8 description.
Malformed connection shapes and missing content produce errors without caching partial heads.
The full-stack renderer borrows non-current rows; a 1,000-PR unchanged-footer corpus requires zero description bytes written in both presentation orders.
Repeated 100-PR submissions with large human descriptions require exactly 100 body analyses, zero content encodings and zero description bytes written, with native stack management disabled and enabled.
Human descriptions remain byte-for-byte unchanged.

All four required acceptance gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 274 library tests, two executable tests and five CLI tests; versioning passes 27 unit tests and 14 integration tests.
The 208 application unit tests cover all 5,160 declared policy lines, 8,537 regions and 574 functions across 31 modules.
Application line coverage is 5,747 of 5,795 lines, or 99.17%; versioning package line coverage remains 382 of 390 lines, or 97.95%.
Parity passes all 24 repository scenarios, comprising 36 submissions per implementation, and the ten harness regressions retain strict configuration-query accounting.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Acceptance logs and JSON coverage reports reside in `target/validation/remaining-after-0.2.19/`.
Superseded failed runs remain separately identifiable from the final passing runs.


## Shared observations and bounded freshness checks

Initial discovery obtains the trunk identity from the existing base commit/tree observation, and descendant discovery consumes the immutable observed tip without resolving it again.
The five-PR regressions reject either redundant resolution command and require one base observation with native stack management disabled and enabled.
Malformed base observations identify the affected reference and stop before publication or remote observation.

Description-only descendant reads retain commit validation but omit rewrite-recovery depths, Jujutsu operation reads and change-ID queries.
The three-commit Jujutsu fixture with one descendant requires one initial operation read and one initial batched change-ID read in either footer style, compared with two operation reads and an additional descendant identity query before the change.
An invalid descendant reports the commit-validation failure and prevents description mutations.

Label refresh preflight checks the current target before considering its remaining bounded group.
The 100-PR unchanged fixture requires 100 target checks and zero suffix probes or requests, compared with 1,300 freshness probes before the change.
Expiry of 51 initial observations requires three grouped refresh requests and 75 suffix probes; a single expired initial observation requires one request.
Slow preceding mutations retain immediate target freshness checks, and confirmed label effects retain their original acquisition age.

Cached PR readers share immutable snapshots, and snapshot bodies and description analysis share immutable byte storage.
An exact unchanged render retains the observed body without allocating a complete replacement; required line-ending normalisation remains effective.
The isolated pre-change 100-PR fixture, with 60,000 human-description bytes per PR, copied 12,169,968 body bytes through 200 cached reads in ordinary mode and 18,254,952 bytes through 300 reads in native mode.
Both modes also allocated 6,084,984 unchanged render bytes.
The committed storage regression requires identical cached snapshot ownership, shared analysis storage, retained body ownership after an unchanged update, and preserved body storage after a title-only mutation.
Previously acquired snapshots remain unchanged, whilst an expired observation containing newer human text replaces the applicable analysis and retains that edit.
Deferred preparation, request-size limits, partial mutation responses and native membership protections retain their existing regressions.

All four required acceptance gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 261 library tests, two executable tests and five CLI tests; versioning passes 27 unit tests and 14 integration tests.
Its 197 application unit tests cover all 4,892 policy lines, 8,124 regions and 543 functions across 31 modules.
Application line coverage is 5,479 of 5,527 lines, or 99.13%; versioning package line coverage remains 382 of 390 lines, or 97.95%.
Parity passes all 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Tests use disposable repositories, isolated credentials and local service fixtures.
Evidence resides in `target/validation/perf-review-after-0.2.18/final-check.log`, `final-coverage.log`, `final-parity.log` and `final-backends.log` within the same directory, together with the retained coverage reports.
The isolated pre-change observations remain in `reproductions.log`.
Earlier failed checks remain available, including `coverage-initial.log`, `coverage-adapter-fixture.log` and `check-test-lock-lifetime.log`, which record the subsequently covered descendant error path, corrected adapter observation and shortened test lock lifetime.

## Cached reconciliation, descendant ranges and prepared content

Cached identity lookups and unchanged base comparisons execute directly, retaining the eight-operation completion barriers for unresolved work.
The 100-PR cached reconciliation fixtures require zero worker threads with native stack management disabled and enabled, compared with 200 in the isolated pre-change reproduction.
Base comparison borrows observed metadata and avoids copying unchanged descriptions or cloning their commits.
Regressions preserve result order, completion of started workers before failure, cache-expiry refresh and creation after an observed absent PR.
Repeated unchanged submissions preserve PR metadata and require no remote mutations.

Descendant discovery retains the immutable observed tip, including an empty tip excluded from presentation and a described Jujutsu working copy.
History reads include only commits beyond that tip; the three-commit prefix and one-commit descendant fixture therefore reads four records rather than seven.
The working-copy regression requires one history read, and the moved-checkout regression retains the original discovery anchor.

Changed content inputs retain their encoded bytes for request-size accounting and GraphQL envelope construction.
REST dispatch, verbose diagnostics and explicit rate-limit retries reuse prepared request bytes.
The deferred 25-PR fixture requires exactly 25 content encodings when refreshed observations remain equal; changed human metadata invalidates preparation and requires one additional encoding.
Escaped UTF-8 payloads, request-size boundaries, sparse aliases and partial mutation responses retain their existing acceptance coverage.

Draft patterns are prepared once per invocation, and each distinct original title is classified once across selection, rewrite recovery and PR creation.
The five-PR fixture prepares four patterns rather than 24 and requires five title classifications with rewriting disabled or enabled.
Literal patterns use anchored sections and ordered KMP searches, with linear preparation and matching for each pattern.
Patterns containing `?` retain byte-oriented, non-backtracking regex matching and the dependency's O(P × T) worst-case bound.
Aggregate raw pattern bytes and regex compilation remain bounded, with explicit configuration guidance on rejection.
The equivalence matrix covers 406 patterns and 15 titles, including Unicode case folding, literal punctuation, newline bytes and byte-oriented question marks.
The 32 KiB near-match regression requires at most twice the title length in counted literal-search steps and zero additional steps on repeated classification.

An isolated unoptimised-build benchmark records the median of seven measurements for a 32 KiB near-match title and a 513-byte literal section.
Legacy matching requires 47.73 ms; the prepared literal matcher requires 0.57 ms for initial classification and 0.026 ms for preparation.
These measurements concern the matcher alone and do not establish an end-to-end submission latency guarantee.
Deterministic operation budgets, rather than timing thresholds, govern the committed regressions.

All four required acceptance gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 255 library tests, two executable tests and five CLI tests; versioning passes 27 unit tests and 14 integration tests.
Its 191 application unit tests cover all 4,845 policy lines, 8,049 regions and 533 functions across 31 modules.
Application line coverage is 5,432 of 5,480 lines, or 99.12%; versioning package line coverage remains 382 of 390 lines, or 97.95%.
Parity passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Tests use disposable repositories, isolated credentials and local service fixtures.
Evidence resides in `target/validation/perf-review-after-0.2.17/final-check.log`, `final-coverage.log`, `final-parity.log` and `final-backends.log` within the same directory, together with the coverage reports.
The matcher measurements reside in `pattern-benchmark-kmp.log`; the isolated pre-change reproduction remains in `reproductions-final.log`.
Earlier failed checks remain available, including `kmp-coverage.log`, whose uncovered production fallback path is now exercised by a disposable-repository CLI regression.

## Malformed credential paths and Jujutsu tip observation

Diagnostic redaction treats an authority-like colon as potentially introducing a password whose raw slashes cannot establish a trustworthy URL boundary.
Such ambiguous text is redacted through the diagnostic's final `@`, covering mixed separators, multiple `@` delimiters and nested scheme text.
This conservative rule can suppress additional URLs or adjacent text; successful command observations retain their original bytes.
Remote-parser guidance requires percent-encoding of credential path separators, and the regressions verify parser errors, verbose output, warnings, both command-failure protocols and accepted encoded equivalents.
The diagnostic scan locates the final `@` once and terminates after conservative redaction, retaining linear work in input length.

Structured stack observations retain the Jujutsu tip identity obtained by the batched change-ID query before presentation filtering.
Explicit ranges and lists reuse that identity rather than launching a second scalar lookup, with and without rewrite eligibility.
An empty tip retains its observed recovery identity despite exclusion from the displayed stack.
An observation unable to associate an unresolved requested tip with a commit retains a fallible scalar fallback.
The regressions reject attempts to invoke the redundant query, verify unchanged selected identities and require preserved failure causes in the fallback.

All four required acceptance gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 245 library tests, two executable tests and four CLI tests; versioning passes 27 unit tests and 14 integration tests.
Its 181 application unit tests cover all 4,595 policy lines, 7,664 regions and 497 functions across 31 modules.
Application line coverage is 5,178 of 5,227 lines, or 99.06%; versioning package line coverage remains 382 of 390 lines, or 97.95%.
Parity passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Evidence resides in `target/validation/redaction-and-tip-reuse/check-final.log`, `coverage.log`, `parity.log` and `backends.log`, together with the coverage reports.
The initial failed check remains in `check.log`; the accepted implementation separates recovery-identity selection into a focused helper, and its dependency check obtains the current advisory database successfully.
The isolated pre-change reproductions reside in `target/validation/review-after-0.2.16/reproductions-final.log`; `native-remote.log` in the same directory confirms preservation of the malformed synthetic URL by native Git.

## Description initialisation, credential whitespace and deferred content

Description initialisation rejects reserved managed-footer delimiters in the selected commit message or PR template before the affected PR creation or content update.
The diagnostic identifies the source and specifies removal of the delimiters or substitution of `&lt;` for their opening `<` in literal examples, without quoting human text.
Existing human descriptions remain unaffected by unused initialisation inputs, and exact observed initialisation bytes reuse their existing delimiter analysis.
Regressions require preserved descriptions and absence of affected mutations on repeated rejection, followed by safe repair and idempotent repetition in both footer styles and native modes.

Malformed credential-bearing remote diagnostics redact whitespace before the authority's `@`, including spaces, tabs, line boundaries and Unicode whitespace.
Parser diagnostics, verbose output, warnings and both command-failure protocols preserve confidentiality whilst successful command bytes remain unchanged.
Percent-encoded equivalents remain supported.
Ambiguous adjacent text before an explicit path separator is redacted conservatively.

A content update deferred at the request-size limit retains its rendered body, encoded input and measured size.
The retained preparation is reused only if the complete refreshed PR observation remains equal; changed observations require rendering again and retain refreshed human text.
The 25-PR regression requires exactly 25 render invocations for unchanged observations, including expiry without content changes, and two bounded mutation requests.
REST content construction reuses one payload for size validation and dispatch, and GraphQL dispatch transfers the prepared variables without copying their contents.

All four required acceptance gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 241 library tests, two executable tests and four CLI tests.
Its 177 unit tests cover all 4,560 policy lines, 7,621 regions and 493 functions across 31 modules.
Application line coverage is 5,143 of 5,192 lines, or 99.06%; versioning package line coverage remains 382 of 390 lines, or 97.95%.
Parity passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Evidence resides in `target/validation/description-input-boundaries/check-accepted.log`, `coverage.log`, `parity.log` and `backends.log`, together with the coverage reports.
The initial failed lint attempts remain in `check.log` and `check-final.log`; the accepted implementation separates preparation from dispatch and transfers encoded variables directly.
The isolated pre-change reproductions remain in `target/validation/review-after-0.2.15/reproductions-final.log`, with a separate native remote-format probe in `native-remote.log` within the same directory.

## Managed-footer integrity, unnumbered context and malformed credentials

Managed-footer analysis requires absent delimiters or exactly one correctly ordered pair.
Unmatched, reversed, nested and duplicate delimiters produce an error naming the affected PR and specifying repair, without updating that description or replacing a valid cached observation with an invalid analysis.
Regressions require preservation of human text on repeated failed submissions, safe repair and zero metadata mutations on subsequent unchanged submissions, with native management disabled and enabled in both footer styles.
A malformed later description also prevents dispatch of an already prepared GraphQL content batch.

Full-stack context retains multiple unnumbered commits without creating additional PRs or publishing additional branch destinations.
Context links use complete commit identities and escaped title and author text; numbered PRs remain deduplicated, whilst compact navigation continues to contain numbered PRs only.
Where such context is present, the header reports entry positions.
The separate Jujutsu workspace parity case requires the precise added base-commit row whilst retaining exact comparison of human text, other PR fields, repository state and publication effects.
Replay probes additionally reject absent context, incorrect context titles or links, altered human text, altered PR titles and altered public refs.

HTTPS discovery rejects raw query and fragment delimiters and accepts percent-encoded equivalents within user information.
Rejected-remote diagnostics, verbose traces, warnings and both command-failure protocols redact malformed user information containing raw `?` or `#`; successful command bytes remain available for parsing and transport.

All four required acceptance gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 238 library tests, two executable tests and four CLI tests.
Its 174 unit tests cover all 4,515 policy lines, 7,550 regions and 489 functions across 31 modules.
Application line coverage is 5,098 of 5,147 lines, or 99.05%; versioning package line coverage remains 382 of 390 lines, or 97.95%.
Parity passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Evidence resides in `target/validation/footer-boundaries/check-accepted.log`, `coverage.log`, `parity-final.log`, `backends.log`, `lint-harness.log` and `parity-replay.log`, together with the coverage reports.
Earlier failed lint and fixture-trace assertions remain recorded, as does the initial parity comparison which required the explicit corrected-footer expectation.
The isolated pre-change reproductions remain in `target/validation/footer-boundaries-review/reproductions.log`.

## Remote confidentiality and description-history efficiency

HTTPS remote discovery separates user information from the host used for authentication lookup and API construction.
URL user information is redacted in rejected-remote diagnostics, verbose command traces, command failures, warnings and the final operational-error display.
Successful command observations retain their original bytes for parsing and Git transport.
Regressions verify credential-bearing HTTPS remotes, rejected remotes, combined and structured command failures, multiple URLs and Unicode user information using synthetic credentials.

Description-history extraction refreshes expired cached PR observations in groups of at most 25 immediately before consumption.
The 51-PR regressions require three requests for either expired PR metadata alone or expired PR and branch metadata, compared with 51 individual REST reads in the isolated reproduction.
Fresh repeated history extraction requires no requests.
Updated human text and merge state participate in extraction; failed refreshes stop planning, and a PR absent from its former branch retains an authoritative read by its original number.
Cached head selection now uses one borrowed pass with constant auxiliary space, preserving observation order and stopping at the first matching open PR without copying bodies or labels.

All four required acceptance gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 232 library tests, two executable tests and four CLI tests.
The 168 application unit tests cover all 4,473 executable policy lines, 7,482 regions and 488 functions across 31 modules.
Application line coverage is 5,056 of 5,105 lines, or 99.04%; versioning package line coverage remains 382 of 390 lines, or 97.95%.
Parity passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Evidence resides in `target/validation/remote-history/check-final.log`, `coverage.log`, `parity.log` and `backends.log` within the same directory, together with the coverage reports.
The initial failed lint attempt remains in `check.log`; its redundant test-struct update was removed, and the restricted-network advisory fetch succeeded during the subsequent authorised execution.
The isolated pre-change reproductions remain in `target/validation/review-current/reproductions.log`.

## Native topology validation and batched merged-PR preflight

Native topology responses must identify the requested stack, provide a non-empty base and member list, and contain unique non-zero PR identities.
Selected PRs which claim membership must appear in the observed member list; invalid observed stack identities and duplicate selected PR identities are rejected.
Validation precedes native confirmation, append, dissolution and recreation, whilst valid selected slices retain their surrounding members.
The regression for a response to stack #9 which claims stack #10 requires failure with no native mutation.

Merged-PR preflight refreshes each group of at most 25 eligible branch observations immediately before consumption and reuses the existing batched merge-status reader.
The 51-branch regressions require three GraphQL requests for either expired branch observations or expired PR observations, compared with 51 requests in the original branch-expiry reproduction.
Fresh complete observations require no requests; a changed head observation selects the refreshed open PR rather than retaining the former merged identity.
Failed branch or merge-status observations still stop submission before rewriting.

All four required acceptance gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 227 library tests, two executable tests and four CLI tests.
The 163 application unit tests cover all 4,434 executable policy lines, 7,422 regions and 488 functions across 31 modules.
Application line coverage is 5,013 of 5,062 lines, or 99.03%; versioning package line coverage remains 382 of 390 lines, or 97.95%.
Parity passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Evidence resides in `target/validation/native-preflight/check-final.log`, `coverage.log`, `parity.log` and `backends.log` within the same directory, together with the coverage reports.
The original isolated reproductions remain in `target/validation/review-next/reproductions.log`.

## Failure diagnostics and recovery

The [error-handling contract](error-handling.md) defines retained causes, practical recovery guidance, confidentiality requirements and the treatment of completed effects.
Configuration, credentials, template access, subprocess execution, state observations, HTTP responses and workflow phases now distinguish genuine absence from failed observation.
Missing remote default-branch metadata identifies the required reference and its verified recovery procedure.
Failed push or PR-creation verification retains both failures and the uncertainty concerning completed remote effects.
HTTP errors appear once through the error chain; bounded server diagnostics include status-specific guidance.
Unexpected-panic diagnostics omit payloads, and interruption exits with status 130 even when its stderr diagnostic pipe is full.
The contract expressly excludes guarantees against forced termination, memory exhaustion and unavailable diagnostic streams.

Successful submission retains its existing command and request budgets.
Commit validation filters in place, preserves retained order and avoids unnecessary swaps; failed fallback observations prevent submission rather than establish an assumed result.
The differential comparison asserts corrected diagnostics in enumerated failure scenarios whilst retaining publication effects, repository snapshots and operation-budget checks.

All four required gates pass sequentially on macOS arm64 with `RUST_TEST_THREADS=1`.
The application passes 224 library tests, two executable tests and four CLI tests.
The independent application unit boundary comprises 160 tests, covering all 4,392 executable policy lines, 7,338 regions and 483 functions across 31 modules.
Application line coverage is 4,971 of 5,020 lines, or 99.02%; versioning package line coverage remains 382 of 390 lines, or 97.95%.
Parity passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Tests use disposable repositories, isolated credentials and local service fixtures.
Evidence resides in `target/validation/error-handling/check-final-source-network.log`, `coverage-final.log`, `parity-accepted.log` and `backends-final.log` within the same directory, together with the coverage reports.
Earlier failed runs remain available, including a restricted-network advisory-fetch failure and the parity failure which required an exact assertion for the corrected preparation diagnostic.

## Shared description analysis and predecessor bases

Description reconciliation analyses each unchanged observed PR body once, sharing managed-section boundaries between history extraction and body replacement.
Changed observations invalidate that analysis before rendering; tests preserve refreshed human text, legacy-footer migration and both newline conventions.
History entries are calculated lazily, so compact footers do not require full-history analysis.
PR creation, number recovery and native linking now reuse one predecessor-base calculation after rewriting.
The counter regressions cover native management disabled and enabled, first and repeated submissions, changed footers, rewritten explicit selections and compact presentation.
Disposable git-branchless tests verify the same counts through native trailer rewriting and repeat submission.

All four required acceptance gates pass sequentially on macOS arm64.
The application passes 207 library tests; the 146 unit tests cover all 4,269 executable policy lines, 7,156 regions and 465 functions across 31 modules.
Application line coverage is 4,736 of 4,782 lines, or 99.04%.
Parity passes the complete CLI and repository matrix, and backend validation uses git-branchless 0.11.1 and Jujutsu 0.46.0.
The dependency-policy gate initially rejected the existing rustls 0.23.45 lock entry following four reported advisories; updating that entry to 0.23.46 restores compliance without changing other dependency versions.
Evidence resides in `target/validation/current-config/check-fixed.log`, `coverage.log`, `parity.log` and `backends.log` within the same directory.
The initial audit and failed dependency-policy run remain available alongside the successful validation records.
These measurements establish the stated computation and operation budgets; they do not establish a universal latency optimum.

## Iterative review and bounded discovery

The review corrected twelve issues affecting publication destinations, Jujutsu working-copy attribution and observation, explicit selection, structured metadata and discovery cost.
Publication rejects the configured trunk and duplicate destinations introduced by inferred trailers before rewriting or publication.
Working-copy author filtering uses the recorded commit identity; unavailable metadata prevents submission, and explicit lists may include the described working-copy commit.
Selections retain their excluded base after rewriting, including independently advanced trunks and empty range boundaries.
Named branches can be rewritten and submitted from another checkout without losing the selection.
Literal branch inventories and fully qualified recovery refs prevent equally named tags from altering publication destinations or recovered identities.
Signature-display configuration cannot alter structured Git metadata or launch verification processes during discovery.

Initial selection deduplicates arguments and resolves at most 128 revisions and 64 KiB of revision arguments per subprocess.
The 1,000-commit regression requires eight resolution commands and no per-commit depth commands on verified linear histories, including independently advanced trunks.
Descendant-tip discovery uses bounded reduction and complete candidate verification; 129 branch tips require four ancestry commands instead of 256.
These results establish operation-count bounds rather than live-service latency guarantees.

The independent application unit boundary comprises 143 tests, covering all 4,216 executable lines, 7,082 regions and 454 functions across 31 modules.
The application coverage run passes 203 library tests and covers 4,683 of 4,729 executable lines, or 99.03%.
Versioning-policy unit coverage remains complete; versioning package line coverage remains 382 of 390 lines, or 97.95%.
The required `just check`, `just coverage`, `just parity` and `just backends` gates pass natively on macOS arm64.
Parity passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Backend validation uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Evidence resides in `target/validation/review-round9/` and `coverage/`.
Earlier sections retain the evidence obtained before these corrections.

## History validation and selection recovery

Merge commits in the discovered history are rejected before PR observation, rewriting or publication.
Changed commits without messages now reach title validation rather than disappearing from the stack.
Disposable Git regressions preserve the original checkout and require zero submission effects for both a merge containing additional resolution work and a changed commit without a message.
Jujutsu cleanliness uses fixed status markers; the native described-working-copy regression includes a pipe character in its title.

Selection recovery reuses verified snapshot depths and Jujutsu change identities.
The 100-commit linear selection regression requires two Git identity-resolution processes, compared with 102 in the original reproduction; a diverged trunk requires three.
The bounded fallback preserves requested order and resolves at most 128 revisions per command.
Native Git validation requires revision-only output, whilst incomplete, malformed, missing and ambiguous identities prevent successful recovery.
Regressions include tip-relative offsets, repeated requested depths, duplicate Jujutsu change IDs and failed fallback commands.

The independent application unit boundary comprises 136 tests, covering all 4,118 executable lines, 6,921 regions and 443 functions across the declared 31 modules.
The full application coverage run passes 188 library tests and covers 4,585 of 4,631 executable lines, or 99.01%.
Versioning-policy unit coverage remains complete; versioning package line coverage remains 382 of 390 lines, or 97.95%.
The required `just check`, `just coverage`, `just parity` and `just backends` gates pass natively on macOS arm64.
Check and coverage execute sequentially with `RUST_TEST_THREADS=1`.
Parity passes all 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation; backend validation uses git-branchless 0.11.1 and Jujutsu 0.46.0.
Validation evidence resides in `target/validation/history-selection/`.

## Source identity, paginated observations and label reconciliation

Commit-association responses now require a source repository identity before branch matching.
Foreign and deleted source repositories are excluded; missing identities fail explicitly without populating the association cache.
GraphQL observations retain repository validation on every page and exclude duplicate PR identities before requesting additional labels.

Incomplete PR and label connections retain their initial results and advance independently in batches of at most 25 aliases.
The 101-PR history regression requires two GraphQL requests, compared with 104 requests in the reproduced REST fallback.
A further regression completes 26 overflowing label connections, one requiring an additional page, in four requests including the initial observation.
Missing, repeated and malformed cursors, inconsistent identities and failed pages cannot establish a complete cached observation.

Label decisions are calculated after description updates and refreshed before each PR's policy is applied.
The 51-PR expiry regression requires three observation requests; its fresh repetition requires none.
Confirmed additions and removals remain cached after partial failure, whilst unrelated labels remain intact.
The original slow-submission reproduction also passes against the corrected implementation: a required label removed during a 31-second description mutation is restored before successful completion.

The required `just check`, `just coverage`, `just parity` and `just backends` gates pass natively on macOS arm64.
Check and coverage run sequentially with `RUST_TEST_THREADS=1`.
The acceptance suite comprises 181 library tests, two CLI tests, 27 versioning unit tests, 14 versioning integration tests and eight coverage-collector regressions.
The independent application unit boundary comprises 131 tests and covers all 4,059 executable lines, 6,808 regions and 433 functions across 31 modules.
Application package line coverage is 4,526 of 4,572 lines, or 98.99%.
The versioning-policy modules retain complete unit coverage; versioning package line coverage remains 382 of 390 lines, or 97.95%.
The differential comparison passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Backend validation uses git-branchless 0.11.1 and Jujutsu 0.46.0.

All repository and service fixtures are disposable and use isolated credentials.
The acceptance logs reside in `target/validation/identity-pagination-labels/`.

## Legacy footer ownership and native failure propagation

Legacy migration and historical-identity extraction now share a conservative section recogniser.
Ordinary issue references, ambiguous plain PR lists and mixed human sections remain intact and do not introduce historical PR lookups.
Recognised legacy footers migrate idempotently with or without emoji; regressions cover historical markers, current-PR annotations, ordering headers and CRLF input.
The immutable reference fixtures remain unchanged.

Native HTTP failures now propagate rather than being converted into successful submission.
Regressions exercise PR reads, topology reads, appends, unstacking, base correction and recreation, including failure after a successful unstack.
Completed PR updates remain effective.
The differential unavailable-service fixture explicitly requires the corrected candidate failure status whilst retaining the reference's successful status as historical evidence.

The required `just check`, `just coverage`, `just parity` and `just backends` gates pass natively on macOS arm64.
Check and coverage run sequentially with `RUST_TEST_THREADS=1`.
The acceptance suite comprises 175 library tests, two CLI tests, 27 versioning unit tests, 14 versioning integration tests and eight coverage-collector regressions.
The independent application unit boundary comprises 125 tests and covers all 3,939 executable lines, 6,646 regions and 422 functions across 31 modules.
Application package line coverage is 4,406 of 4,452 lines, or 98.97%.
The versioning-policy modules retain complete unit coverage; versioning package line coverage remains 382 of 390 lines, or 97.95%.
The differential comparison passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Backend validation uses git-branchless 0.11.1 and Jujutsu 0.46.0.

All repository and service fixtures are disposable and use isolated credentials.
The acceptance logs reside in `target/validation/footer-native-fixes/`.

## Context identities and base reconciliation

Explicit-selection context now reuses an observed open PR for its remote branch before consulting commit associations.
The regression retains predecessor PR #101 in PR #102's footer even when the commit-association endpoint returns no match.
Historical fallback results, including absence, expire after 30 seconds; failed requests remain uncached, and association summaries do not replace complete PR observations.

Base reconciliation refreshes expired observations immediately before each group of at most 25 changes.
The 51-PR regression requires three refresh queries and no individual PR reads, compared with 51 individual reads before this correction.
A repetition with fresh observations requires no requests.
Further regressions verify original identity retention after branch renaming, propagation of base-update and blocked-base flags, and cessation after refresh or update failure.

The required `just check`, `just coverage`, `just parity` and `just backends` gates pass natively on macOS arm64.
The final check and coverage runs use `RUST_TEST_THREADS=1` and execute sequentially.
Initial overlapping runs exceeded the existing 1,000-PR budget fixture's cache-freshness window and produced additional refresh requests; the unchanged isolated assertion subsequently passed in 7.92 seconds.
The failed-run logs are retained alongside the final acceptance evidence.

The acceptance suite comprises 171 library tests, two CLI tests, 27 versioning unit tests, 14 versioning integration tests and eight coverage-collector regressions.
The independent application unit boundary comprises 121 tests and covers all 3,896 executable lines, 6,570 regions and 419 functions across 31 modules.
Application package line coverage is 4,363 of 4,409 lines, or 98.96%.
The versioning-policy modules retain complete unit coverage; versioning package line coverage remains 382 of 390 lines, or 97.95%.
The differential comparison passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Backend validation uses git-branchless 0.11.1 and Jujutsu 0.46.0.

All repository and service fixtures are disposable and use isolated credentials.
The acceptance logs reside in `target/validation/context-bases/`.

## Historical status, native refreshes and description adoption

Historical PRs outside the current selection now require an observed GitHub merge status before a merged marker is displayed.
Fresh cached observations are reused; remaining identities are deduplicated and queried in batches of at most 25.
Closed but unmerged PRs retain an unmerged marker, and failed or incomplete status observations prevent description mutation.
Compact navigation and presentation without merge markers require no additional historical-status requests.

Native reconciliation refreshes expired cached observations in bounded groups before reading stack membership.
The 51-PR expiry regression requires three refresh queries and zero individual PR reads; fresh observations require no refresh queries.
A separate historical-status regression resolves 51 distinct identities, including a repeated input identity, in three queries.
An unmanaged description survives adoption and repetition, and legacy-footer migration preserves surrounding human and bot text even when the commit supplies a message.
Commit messages and templates initialise empty or unchanged placeholder bodies only.
The descendant-tip algorithm specification now reflects the removal of fallback after a failed bulk reduction.

The required `just check`, `just coverage`, `just parity` and `just backends` gates pass natively on macOS arm64.
The acceptance suite comprises 167 library tests, two CLI tests, 27 versioning unit tests, 14 versioning integration tests and eight coverage-collector regressions.
The independent application unit boundary comprises 117 tests and covers all 3,869 executable lines, 6,501 regions and 417 functions across 31 modules.
The two versioning-policy modules retain complete coverage of 107 lines, 166 regions and 15 functions.
Application package line coverage is 4,336 of 4,382 lines, or 98.95%; versioning package coverage remains 382 of 390 lines, or 97.95%.
The differential comparison passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
The captured description-adoption reference fixture retains its original output and explicitly asserts the authorised candidate policy change.

All repository and service fixtures are disposable and use isolated credentials.
The acceptance logs reside in `target/validation/history-fixes/`.

## Descendant context, expired observations and template resolution

Descendant discovery now propagates failed reference resolution, enumeration, ancestry analysis and history reads before description mutation.
The large-tip selection algorithm retains linear comparison complexity whilst distinguishing an ordinary negative ancestry result from an execution failure.
PR body templates are resolved relative to the discovered repository or workspace root.

Expired cached PRs are refreshed through bounded head observations before each content mutation batch.
A regression with 51 expired PRs requires three refresh queries and three mutation requests, with no individual PR reads; an unchanged repetition requires no HTTP requests.
The original PR number remains authoritative when a branch is renamed or reused, and failed refreshes prevent the pending content mutation.
Individual reads remain necessary when the original identity is absent from the refreshed head observation.

The required `just check`, `just coverage`, `just parity` and `just backends` gates pass on macOS arm64.
The differential comparison passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
The macOS arm64 validation comprises 162 library tests, two CLI tests, 27 versioning unit tests, 14 versioning integration tests and eight coverage-collector regressions.
The independent application unit boundary comprises 112 tests and covers all 3,776 executable lines, 6,353 regions and 416 functions across 31 modules.
The two versioning-policy modules retain complete coverage of 107 lines, 166 regions and 15 functions.
Application package line coverage is 4,243 of 4,289 lines, or 98.93%; versioning package coverage remains 382 of 390 lines, or 97.95%.

Validation uses native local tools and disposable repositories.
The [platform coverage contract](testing.md#platform-coverage) assigns cross-platform checks to CI and explains the optional role of containers.
The acceptance logs reside in `target/validation/review-next/`.

## Additional submission correctness and efficiency corrections

The five further corrections preserve human description content acquired during refresh, retain process-group ownership through output collection, remember unsupported GraphQL stack-field capability, batch descendant observations, and remove avoidable configuration and reference-validation subprocesses.
The [submission specification](submission-efficiency.md) records the resulting ownership, observation and algorithmic contracts.

The required `just check`, `just coverage`, `just parity` and `just backends` gates pass on macOS arm64 and Linux aarch64 with Rust 1.99.0, git-branchless 0.11.1 and Jujutsu 0.46.0.
Linux additionally passes `just credentials` against an isolated native credential store.
The macOS acceptance suite comprises 157 library tests, two CLI tests, 27 versioning unit tests, 14 versioning integration tests and eight coverage-collector regressions, with separate doctests, semantic checks and release compilation.
Linux executes the corresponding suite and its separately selected native credential test.
Each platform's differential comparison passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.

The independent application unit boundary comprises 107 tests and covers all 3,737 executable lines, 6,282 regions and 413 functions across 31 modules on both platforms.
The two versioning-policy modules retain complete coverage of 107 lines, 166 regions and 15 functions.
Application package line coverage is 4,204 of 4,250 lines on macOS and 4,210 of 4,256 lines on Linux, rounding to 98.92% on each platform.
Versioning package coverage remains 382 of 390 lines, or 97.95%.

The following operation counts describe the corrected paths; unsupported-schema counts concern empty heads and therefore exclude native membership reads which may be required for existing PRs.

| Operation | Previous path | Corrected path |
| --- | --- | --- |
| Ordinary configuration discovery | 16 scalar Git queries | Three grouped Git queries |
| Literal reference validation | One Git invocation per distinct reference | Linear validation in memory; no subprocess |
| Observation of 100 empty heads without the GraphQL stack field | Four failed batch probes and 100 REST queries | One failed probe and four reduced batch queries |
| Observation of 26 descendant heads | 26 individual head queries | Two bounded head queries |

The configuration and reference budgets are asserted through 1,000-commit in-memory workflows.
The native Git naming comparison covers ASCII, Unicode, component boundaries and reserved spellings in disposable repositories.
Further regressions verify preservation of newly refreshed human text, omission of unchanged content mutations, native membership checks before base changes, and termination of descendants after their direct parent exits and capture fails.
Configuration tests retain Git's includes, precedence and typed-conversion semantics, including malformed superseded values and explicit CLI overrides.

The unchanged conventional differential fixture records 16 commands, one API request and no deliberate wait.
These are operation-count observations from isolated fixtures; this correction does not establish a new live-service latency estimate.
No submission was executed against the developer's working stack.
Complete acceptance logs, platform coverage summaries and operation evidence reside in `target/validation/review-fixes/`.
The following sections retain acceptance records from preceding revisions.

## Submission follow-up corrections

The six follow-up corrections cover native outcome reporting, bounded subprocess output and configurable deadlines, per-observation cache expiry, optional compact descriptions and request-byte budgets, non-mutating previews, and broader comparative validation.
Full-stack descriptions remain the default; `--compact-stack` explicitly selects adjacent-PR navigation with at most three links per description.
The [submission specification](submission-efficiency.md) and [configuration contract](configuration.md) state the resulting behaviours and limits.

The required `just check`, `just coverage`, `just parity` and `just backends` gates pass on both macOS arm64 and Linux aarch64 with Rust 1.99.0, git-branchless 0.11.1 and Jujutsu 0.46.0.
Linux acceptance additionally passes `just credentials` against an isolated native credential store.
The macOS workspace run comprises 144 library tests, two CLI tests, 27 versioning unit tests, 14 versioning integration tests, eight coverage-collector regressions, separate doctests, semantic checks and release compilation.
Linux runs the corresponding suite and executes the separately selected credential test through its dedicated gate.
Each platform's differential gate passes 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.

The independent application unit boundary covers all 3,588 executable lines, 6,078 regions and 399 functions across 30 modules on both platforms.
The two versioning-policy modules retain complete coverage of 107 lines, 166 regions and 15 functions.
Application package line coverage is 4,008 of 4,052 lines on macOS, or 98.91%, and 4,014 of 4,058 lines on Linux, or 98.92%.
The differing denominators reflect platform-specific native credential code.
Versioning package coverage remains 382 of 390 lines, or 97.95%.

Disposable backend regressions verify unchanged refs and history after dry runs in ordinary Git, git-branchless, colocated Jujutsu and separate Jujutsu repositories.
Jujutsu assertions also preserve operation identities when unsnapshotted files exist.
Native process regressions exercise output-limit termination, command deadlines, binary output, shared diagnostic ordering, cancellation of descendants and bounded draining when a descendant continues writing after direct-child completion.
Protocol assertions cover selective cache expiry, preservation of human description content, declined native rebuilds, encoded request limits, and compact navigation through 1,000 commits.
The complete acceptance logs and coverage summaries reside in `target/validation/followups/`.

### Serial Rust comparison

The reference is Rust revision `fd311895aeebf5c7f3a433a685232f64add36558`, preceding the submission redesign.
The candidate includes both that redesign and the six follow-up corrections; the comparison therefore does not isolate the marginal timing effect of the follow-up corrections.
Both applications use ordinary release compilation, independently seeded repositories and the same loopback service model.
The baseline application and clock are unmodified.
Its elapsed durations include deliberate waits, whilst separate baseline wait measurements are unavailable and are represented as null in the summary.

One scenario runs at a time, after compilation and acceptance have completed.
The 1-commit and 10-commit workloads each have three paired samples; the middle sample reverses implementation order.
Each 50-commit and 100-commit workload has one additional paired scale probe, with exact outcome comparison and operation budgets retained.
The matrix uses the default full-stack footer and covers conventional git-branchless stacks of 1, 10, 50 and 100 commits; conventional colocated and separate Jujutsu stacks of 10 and 50 commits; and native-enabled 10-commit stacks in all three backends.
Compact descriptions have separate algorithmic and protocol assertions and are not attributed the latency of this matrix.

All 25 paired scenarios, comprising 150 submission invocations across 11 workloads, pass the corrected output, object, PR-state and operation-budget comparisons.
Across the seven repeated workloads, median unchanged latency decreases by approximately 62–84%, and amended latency by 35–72%.
Unchanged candidate submissions perform no metadata mutation or deliberate wait.
The native separate-workspace first-submission median is 2.4% higher than the reference, despite improvements in its amended and unchanged phases.
Its candidate first-submission samples range from 18.792 to 34.716 seconds with identical command and request counts; the dispersion and sample count preclude a universal first-submission speedup claim.
First submissions retain mutation pacing, and the measurements do not establish live-service latency.

Durations below are medians where three pairs were collected and individual observations where one pair was collected.
All durations are in seconds; HTTP figures describe unchanged submissions and count directly observed requests.

| Backend | Commits | Native | Pairs | First, reference/candidate | Amended, reference/candidate | Unchanged, reference/candidate | Unchanged HTTP, reference/candidate |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| git-branchless | 1 | Disabled | 3 | 8.463 / 4.638 | 7.411 / 2.675 | 7.370 / 2.607 | 7 / 1 |
| git-branchless | 10 | Disabled | 3 | 20.872 / 15.421 | 11.498 / 4.466 | 11.644 / 4.359 | 70 / 1 |
| git-branchless | 10 | Enabled | 3 | 19.947 / 16.398 | 11.710 / 4.199 | 11.643 / 4.306 | 70 / 2 |
| git-branchless | 50 | Disabled | 1 | 87.365 / 66.203 | 33.793 / 15.139 | 36.036 / 19.070 | 350 / 2 |
| git-branchless | 100 | Disabled | 1 | 237.363 / 128.572 | 70.948 / 29.533 | 65.163 / 27.888 | 700 / 4 |
| Colocated Jujutsu | 10 | Disabled | 3 | 23.605 / 16.448 | 15.478 / 6.576 | 14.993 / 5.207 | 70 / 1 |
| Colocated Jujutsu | 10 | Enabled | 3 | 23.055 / 16.415 | 18.343 / 5.065 | 18.379 / 4.359 | 70 / 2 |
| Colocated Jujutsu | 50 | Disabled | 1 | 80.608 / 66.240 | 61.193 / 34.608 | 113.914 / 33.826 | 350 / 2 |
| Separate Jujutsu | 10 | Disabled | 3 | 32.497 / 24.382 | 27.770 / 12.323 | 22.625 / 8.598 | 70 / 1 |
| Separate Jujutsu | 10 | Enabled | 3 | 31.609 / 32.359 | 32.931 / 21.502 | 33.434 / 5.506 | 70 / 2 |
| Separate Jujutsu | 50 | Disabled | 1 | 84.900 / 67.713 | 51.068 / 16.129 | 49.023 / 9.204 | 350 / 2 |

The [machine-readable summary](benchmark-followups.json) retains source, harness and executable digests, platform details, raw duration samples, medians, maxima and operation counts.
The recorded candidate build precedes the mandatory commit-hook package-version increment.
Complete scenario evidence resides in `target/validation/bench-rust-fd311895/`, with its aggregate report at `target/validation/bench-rust-fd311895.json`.
The repeated samples provide descriptive observations rather than confidence intervals or live GitHub latency forecasts; the larger single-sample probes establish no latency distribution.
Reference requests hidden within the GitHub CLI are excluded from direct HTTP counts; native extension publication is modelled without timing an installed extension.

The larger probes exposed an output-framing difference: Rust's test runner inserts its 60-second notice at different positions in the two implementations' output.
Comparison excludes only the exact notice naming the differential entrypoint; original output, durations, repository snapshots and operation traces remain retained.
Collection and comparison harness digests are recorded separately, and the production source and executable remain unchanged throughout collection.
The retained evidence is re-evaluated under the corrected comparison without repeating or altering measured submissions.

An initial collection was interrupted after review and a reproducing regression identified continuous descendant output which could exceed the drain grace period.
Its superseded executable and partial observations are retained under `target/validation/history/` and excluded from the final timing conclusions.
The following sections retain acceptance records from preceding revisions.

## Submission efficiency implementation

The submission redesign gives Git exclusive publication ownership and uses one application API adapter for PR and native-stack reconciliation.
It removes unconditional visibility waits, unchanged metadata writes, repeated stack discovery without a rewrite, per-commit branch inventories and native extension publication.
Structured snapshots, bulk backend rewriting, bounded publication and API batches, indexed selection, linear graph reduction and shared description rendering implement the principal workflow and algorithm changes.
The [implementation specification](submission-efficiency.md) records ownership, limits, recovery, cancellation and retained presentation semantics.

The required `just check`, `just coverage`, `just parity` and `just backends` gates pass on macOS arm64 with Rust 1.99.0, git-branchless 0.11.1 and Jujutsu 0.46.0.
Acceptance includes 133 library tests, two CLI tests, 27 versioning unit tests, 14 versioning integration tests, eight coverage-collector regressions, separate doctests, semantic checks and release compilation.
The strict application unit boundary covers all 3,448 executable lines, 5,801 regions and 383 functions across 30 modules.
Application package line coverage is 3,784 of 3,821 lines, or 99.03%; versioning package coverage remains 382 of 390 lines, or 97.95%.
Scale assertions cover 1, 5, 10, 50, 100, 101 and 1,000 commits, and linear candidate-tip comparison counts through 1,000 tips.
Native process tests additionally verify binary protocols, deadlines and cancellation of owned commands and descendants.
Linux-specific execution remains an obligation of CI.

### Release benchmark matrix

The completed matrix contains 54 paired scenarios and 324 submission invocations across 18 workloads.
Each workload has three independent samples, with reference/candidate execution order reversed for the middle sample.
Four independent scenarios ran concurrently; compilation completed before timed submissions.
The reference remains the pinned Go executable specification, and the candidate uses the ordinary release profile.
These are loopback-service and instrumented-process measurements, rather than live GitHub latency estimates or a controlled single-process microbenchmark.

Median unchanged latency decreases by approximately 55–74% across the matrix, amended latency by 52–76%, and first-submission latency by 10–48%.
First submissions retain required mutation pacing and PR creation; the reduction in subprocesses and redundant work nevertheless lowers their observed medians.
For ten unchanged PRs, observed direct HTTP requests decrease from 70 to one in conventional mode and two in native mode.
The candidate performs no metadata mutation or deliberate wait in these unchanged cases.
A single-PR selection does not require native topology linking, so its enabled and disabled request budgets coincide.

All durations below are medians in seconds.
HTTP figures count directly observed requests; unobserved reference GitHub CLI requests are excluded.
Modelled native child publication counts establish fixture ownership expectations, without claiming to time an installed extension.
The [machine-readable report](benchmark-efficiency.json) retains source and executable digests, platform details, raw timing samples, maxima and operation counts.
Its recorded build precedes the mandatory commit-hook version increment.

| Backend | Commits | Native | First, reference/candidate (s) | Amended, reference/candidate (s) | Unchanged, reference/candidate (s) | Unchanged HTTP, reference/candidate |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| git-branchless | 1 | Disabled | 9.038 / 4.731 | 7.567 / 2.799 | 7.484 / 2.663 | 7 / 1 |
| git-branchless | 1 | Enabled | 9.086 / 5.064 | 7.737 / 2.819 | 7.944 / 2.892 | 7 / 1 |
| git-branchless | 5 | Disabled | 15.987 / 10.592 | 10.279 / 4.111 | 10.668 / 3.897 | 35 / 1 |
| git-branchless | 5 | Enabled | 16.087 / 11.701 | 10.740 / 4.226 | 10.743 / 4.065 | 35 / 2 |
| git-branchless | 10 | Disabled | 25.854 / 17.147 | 15.054 / 4.867 | 14.117 / 4.702 | 70 / 1 |
| git-branchless | 10 | Enabled | 21.912 / 16.798 | 12.873 / 4.356 | 11.433 / 4.347 | 70 / 2 |
| Colocated Jujutsu | 1 | Disabled | 8.051 / 4.783 | 7.898 / 3.327 | 7.488 / 3.282 | 7 / 1 |
| Colocated Jujutsu | 1 | Enabled | 8.217 / 4.962 | 7.739 / 3.072 | 7.535 / 3.379 | 7 / 1 |
| Colocated Jujutsu | 5 | Disabled | 11.984 / 9.713 | 11.356 / 4.480 | 11.927 / 3.611 | 35 / 1 |
| Colocated Jujutsu | 5 | Enabled | 13.149 / 10.878 | 11.287 / 4.672 | 11.542 / 3.348 | 35 / 2 |
| Colocated Jujutsu | 10 | Disabled | 19.418 / 16.514 | 17.167 / 4.054 | 15.396 / 4.526 | 70 / 1 |
| Colocated Jujutsu | 10 | Enabled | 19.324 / 16.703 | 15.714 / 4.296 | 14.491 / 3.997 | 70 / 2 |
| Separate Jujutsu | 1 | Disabled | 8.299 / 5.363 | 8.179 / 3.720 | 8.291 / 3.308 | 7 / 1 |
| Separate Jujutsu | 1 | Enabled | 8.275 / 5.265 | 8.294 / 3.886 | 8.084 / 3.345 | 7 / 1 |
| Separate Jujutsu | 5 | Disabled | 13.089 / 11.197 | 11.115 / 5.004 | 11.954 / 4.408 | 35 / 1 |
| Separate Jujutsu | 5 | Enabled | 14.456 / 11.745 | 12.174 / 5.892 | 12.181 / 4.415 | 35 / 2 |
| Separate Jujutsu | 10 | Disabled | 22.839 / 19.032 | 16.198 / 6.871 | 17.116 / 5.034 | 70 / 1 |
| Separate Jujutsu | 10 | Enabled | 20.505 / 18.507 | 16.254 / 6.367 | 15.778 / 4.093 | 70 / 2 |

An earlier benchmark collection was interrupted after review identified a child-process cancellation defect.
Its partial results are excluded from this final-source collection and retained separately with the interrupted-run diagnostic.
The final cancellation regression verifies terminal interruption, descendant termination and direct-child reaping before process exit.

### Serial service-latency probes

Four additional paired scenarios exercise a five-commit conventional git-branchless stack with injected delays of 0, 20, 100 and 300 milliseconds per observed service request.
Each collection runs alone with one scenario worker, reuses the final matrix executable, and alternates reference/candidate order across successive delay values.
All four comparisons pass and their application and executable digests equal those of the matrix.
Each delay has one paired sample; these observations constitute sensitivity checks rather than estimates of a latency distribution.
The first and amended submissions establish the state for the unchanged observation reported below.

| Injected delay per observed request (ms) | Reference unchanged (s) | Candidate unchanged (s) | Direct HTTP requests, reference/candidate |
| ---: | ---: | ---: | ---: |
| 0 | 8.867 | 3.041 | 35 / 1 |
| 20 | 9.060 | 2.964 | 35 / 1 |
| 100 | 10.071 | 3.157 | 35 / 1 |
| 300 | 12.152 | 3.397 | 35 / 1 |

The candidate retains one direct observation request, zero metadata mutations and zero deliberate waits in every unchanged probe.
The delay model excludes Git transport, packet loss and requests hidden inside the reference GitHub CLI.
Consequently, the serial observations shall not be interpreted as live-service forecasts or pooled with the four-worker matrix.
Raw reports reside in `target/validation/bench-latency-{0,20,100,300}.json` and `target/validation/submission-efficiency-latency.json`.

### Evidence retention

Final acceptance logs reside in `target/validation/submission-efficiency-{check,coverage,parity,backends}.log`.
Complete matrix evidence resides in `target/validation/bench/` and `target/validation/submission-efficiency-matrix.json`; coverage reports reside under `coverage/`.
The committed summary preserves the numerical evidence without retaining disposable repositories in Git.
The following sections retain acceptance results from preceding revisions.

## Managed validation environments: 13 October 2026

The development configuration now follows the sibling git-ls repository's prek-managed hook environments and uv-managed Python 3.15.0 project environment.
The bespoke executable installer, archive inventory and executable-path wrapper have been removed.
Current recipes, fixture scripts and CI artifact collection use `target/validation/` for disposable evidence; historical records have been retained at that location.
Shared hook definitions match git-ls, except for the additional backend and reference dependencies required by this application.

The complete `just check`, `just coverage`, `just parity` and `just backends` gates pass on macOS arm64 with Python 3.15.0 and Rust 1.99.0.
Acceptance includes 97 library tests, two CLI integration tests, 27 versioning unit tests, 14 versioning integration tests, eight coverage-collector regressions, separate doctests, semantic checks and release compilation.
The independent unit reports retain complete line, region and function coverage across 26 application modules and two versioning-policy modules.
Application package coverage is 2,798 of 2,803 lines, or 99.82%; versioning package coverage is 382 of 390 lines, or 97.95%.
The differential comparison passes all 14 CLI cases and 24 repository scenarios, comprising 36 submissions per implementation.
Native backend acceptance passes with git-branchless 0.11.1 and Jujutsu 0.46.0.

Validation ran one gate at a time with temporary limits of two Cargo workers, two Rust test threads, two Go execution workers and one concurrent prek hook and batch.
Initial tool provisioning was interrupted after parallel Rust and Go installations affected host responsiveness; subsequent preparation completed sequentially before the setup command was rerun successfully.
Persistent installation-concurrency changes remain deferred; setup retains prek's ordinary environment preparation.
Logs reside in `target/validation/prek-setup-complete.log`, `target/validation/prek-check.log`, `target/validation/prek-coverage.log`, `target/validation/prek-parity.log` and `target/validation/prek-backends.log`.
Coverage reports reside under `coverage/`; differential evidence and measured phase outcomes reside under `target/validation/`.
Linux-specific credential acceptance remains an obligation of the corresponding CI jobs.
The following sections retain acceptance results from preceding revisions.

## Expanded application unit boundary: 13 October 2026

The independent application unit run executes 57 tests using in-memory command, HTTP, filesystem, credential, terminal and clock boundaries.
Its inventory comprises 26 production modules and covers all 2,634 executable lines, 4,396 regions and 303 functions.
The independent versioning policy run retains complete coverage of 107 lines, 166 regions and 15 functions across two modules and 27 tests.
Owner-local submission tests exercise private state without widening production visibility; native adapters retain separate integration acceptance.
The versioning helper's mixed repository and version-orchestration modules remain outside the strict unit percentage.

Timeout values are validated once during argument parsing and retained in their integer representation.
Git remote-expression compilation resides in backend policy, and PR query construction validates the configured API host.
Native command, terminal, timing and credential implementations retain their existing execution contracts behind focused boundary modules; filesystem access is injected explicitly.
The additional unit assertions cover phase barriers, worker failure, rewritten selections, ref guards, PR lookup failures, historical descriptions, labels, stack consent and partial effects.

The `just check` recipe passes on macOS arm64 with 97 library tests, two CLI integration tests, 27 versioning unit tests, 14 version-automation integration tests, eight collector regressions, separate doctests, semantic checks and release compilation.
The reference comparison passes all 24 repository scenarios, comprising 36 submissions per implementation, and all 14 CLI cases.
The native git-branchless and Jujutsu backend recipe passes.
The `just coverage` recipe passes with 2,798 of 2,803 application lines, or 99.82%, and 382 of 390 version-helper lines, or 97.95%.
Execution logs reside in `target/validation/expanded-unit-*.log`, with independent unit reports under `coverage/unit/`.
Linux and platform-specific credential execution remain obligations of the corresponding CI jobs.
The following sections retain preceding acceptance records.

## Shared coverage policy: 13 October 2026

Both repositories now use the same explicit manifest schema, collectors and collector regression checks.
Every production source under the declared roots is classified exactly once, with a 95% package line minimum and complete independent coverage of declared unit sources.
The application unit run executes 26 selected tests; repository and external execution cannot contribute to its independent profiles.
The application unit boundary comprises 15 modules and covers all 997 lines, 1,614 regions and 149 functions.
Application package coverage is 2,748 of 2,780 lines, or 98.85%.
Versioning package coverage is 382 of 390 lines, or 97.95%; its two unit modules cover all 107 lines, 166 regions and 15 functions from 27 in-memory tests.
The eight collector regression tests, `just check` and `just coverage` pass on macOS arm64.
The coverage reports exclude test implementations and use integer acceptance counts rather than rounded display percentages.
The preceding package figures are therefore not directly comparable with historical reports which included inline test code.
JSON and HTML evidence resides under `coverage/`; execution logs reside in `target/validation/coverage-consistency-*.log`.
Linux and other platform-specific execution remain obligations of the corresponding CI jobs.

## Structural organisation: 13 October 2026

Submission orchestration now occupies a 142-line root module, with runtime construction, stack state, rewriting, pull-request operations, presentation and deterministic policy assigned to focused submodules.
Shared repository and pull-request values reside under `model/`; backend interpretation and decisions reside in the policy modules owned by Git, GitHub, Jujutsu, configuration and credentials.
Commit acceptance, draft creation, author exclusion and duplicate-reference decisions are independent of external execution, and recognised submission checkpoints are represented by an enumeration.
The reference-case mappings identify the relocated unit source files, and application coverage identifies nested modules by full repository-relative paths.

On macOS arm64, the independent application run executes 26 unit tests and covers all 997 lines, 1,614 regions and 149 functions in its 15 declared production modules.
The independent versioning run executes 27 unit tests and covers all 107 lines, 166 regions and 15 functions in its two declared production modules.
The `just check` recipe passes with 66 library tests, two CLI integration tests, 41 version-automation tests, separate doctests and release compilation.
The `just coverage` recipe passes with 2,748 of 2,780 application lines, or 98.85%, and 382 of 390 version-helper lines, or 97.95%.
The reference comparison passes all 24 repository scenarios, comprising 36 submissions per implementation, and all 14 CLI cases, preserving command sequencing, mutation ordering, trailer representation and deliberate waits.
The native git-branchless and Jujutsu backend recipe also passes.
Initial restricted execution could not resolve the advisory database host or bind loopback sockets; the completed acceptance runs used the network and socket permissions required by their isolated fixtures.
Reports reside in `coverage/unit/`, `coverage/coverage.json` and `coverage/versioning.json`; measured phase outcomes reside in `target/validation/timings.jsonl`.
Linux execution remains delegated to CI.
The following sections retain the acceptance evidence preceding this structural refactoring.

## Correctness repairs

The confirmed defects concerning failed worktree discovery, failed history reads, lost draft exclusions, failed PR lookups, non-Unicode process inputs and incomplete worker joining have been corrected.
Remote parsing now preserves complete aliases and dotted repository names; label reconciliation retains labels required by overlapping policy sources.
These changes intentionally supersede defective reference behaviour and are covered by unit assertions and disposable workflow regressions.
A source snapshot without `.git` compiles using locked, offline dependencies and reports `unknown` for absent Git version metadata.
Evidence resides in `target/validation/issue-fixes-check.log`, `target/validation/issue-fixes-coverage.log`, `target/validation/issue-fixes-parity.log`, `target/validation/issue-fixes-backends.log` and `target/validation/issue-fixes-archive.log`.
GitHub Enterprise Server remains outside the documented hosting contract.

## Independent unit attribution

The unit coverage boundary is defined in [coverage-policy.toml](../coverage-policy.toml) and justified in the [unit coverage specification](unit-coverage.md).
On macOS arm64, the independent application run executes 21 unit tests and covers all 938 lines, 1,536 regions and 141 functions in its nine declared production modules.
The independent versioning run executes 27 unit tests and covers all 107 lines, 166 regions and 15 functions in its two declared production modules.
These results use separate, freshly collected profiles and exclude test implementations, integration execution and external comparisons.
The generated versioning CLI schema retains explicit parser assertions but has no executable production coverage entry.
The mixed-module policy gaps recorded in the specification remain outside this immediately enforced percentage.

`just check` passes with 63 application tests, 41 version-automation tests, separate doctests and release compilation.
`just coverage` passes with 2,704 of 2,736 application lines, or 98.83%, and 382 of 390 version-helper lines, or 97.95%.
The version-helper package report now excludes test implementations; its percentage is therefore not directly comparable with the earlier records which included them.
The unit acceptance checks additionally reject uncovered integer counts despite a rounded percentage of 100%, missing modules, empty mappings, excluded modules in a report and unclassified production additions.
The reference comparison passes all 24 repository scenarios, comprising 36 submissions per implementation, and all 14 CLI cases.
The native git-branchless and Jujutsu backend gate also passes.
Reports reside in `coverage/unit/`, `coverage/coverage.json` and `coverage/versioning.json`; measured phase outcomes reside in `target/validation/timings.jsonl`.
Linux execution remains delegated to CI.
The following sections preserve preceding acceptance records.

## Dependency and tooling audit

Version 0.1.0 has been validated on macOS arm64 with Rust 1.99.0 and Python 3.15.0.
The application source digest is `45b870b19e0f7f3d41eec484ba1612a898432520ccd9a9263a896a479f27e9dc`, covering the manifest, lockfile, compiler declaration, build script and immediate application Rust files.
The direct dependency requirements and development-tool pins match their latest stable releases checked on 12 October 2026.
A registry-backed `cargo update --dry-run --verbose` reports no compatible lockfile updates.
Indirect dependencies retain upstream compatibility constraints; this result does not require incompatible major-version substitutions.

Plain `just check` passes with 47 application tests, 41 version-automation tests, separate doctests and release compilation.
Application coverage is 2,596 of 2,631 lines, or 98.67%, with complete line, region and function coverage in the five strict modules.
The version helper covers 588 of 596 lines, or 98.66%, with complete line, region and function coverage in its CLI, manifest and policy modules.
All 24 workflow scenarios, comprising 36 submissions per implementation, and all 14 CLI comparisons pass.
The workflow run was interrupted after 15 completed scenarios; the nine remaining scenarios were resumed and all 24 retained traces were rechecked.
The native git-branchless and Jujutsu backend gate passes.

The root package and version helper inherit the same workspace compiler metadata and Rust lint policy.
Setup installs the declared uv and Python versions locally; repeated `sh setup.sh` succeeds without a manual `PATH` override.
Evidence resides in `target/validation/dependency-check.log`, `target/validation/dependency-coverage.log`, `target/validation/dependency-parity-complete.json`, `target/validation/dependency-backends.log`, `target/validation/dependency-setup-repeat.log`, `target/validation/dependency-freshness-final.log` and `coverage/`.
Linux and hosted workflow execution remain delegated to CI.
The following sections record earlier application digests.

## Version automation

Version 0.1.0 has been validated on macOS arm64 with Rust 1.99.0.
The application source digest is `85549a8ef75eced3a18712ef7f4c02865432ccfb2686608dcab1300e82eaa386`, covering the manifest, lockfile, compiler declaration, build script and immediate application source files.
Plain `just check` passes with 47 application tests, 41 version-automation tests, separate doctests and release compilation.
Application coverage remains 2,596 of 2,631 lines, or 98.67%, with complete line, region and function coverage in the five strict modules.
The version helper covers 588 of 596 lines, or 98.66%, with complete line, region and function coverage in its CLI, manifest and policy modules.

An installed `prek` pre-push hook rejects a missing version increment and accepts corrected history against a private local bare remote.
The automated fixtures additionally cover initial commits, amendments, staged lockfile mismatches, unstaged changes, merge rejection, conflicting tags and publication retries.
The release workflow is configured to publish annotated version tags after successful main CI; hosted execution has not been performed locally.
No production tags have been published during acceptance.
The pinned executable specification downloads and extracts successfully without an adjacent checkout.
All 24 workflow scenarios, comprising 36 submissions per implementation, and all 14 CLI comparisons pass.
The native git-branchless and Jujutsu backend gate also passes.

Evidence resides in `target/validation/version-check.log`, `target/validation/version-coverage-complete.log`, `target/validation/version-installed-hooks.log`, `target/validation/version-parity-complete.log`, `target/validation/version-backends.log` and `coverage/`.
Failed attempts remain in the evidence record, including the sandbox's rejection of private test-server sockets before validation outside the sandbox.
Linux validation remains delegated to the configured CI jobs.

## Engineering baseline

The preceding application baseline follows the [engineering conventions](conventions.md).
Its application source digest is `4e0e7b9bf7297a9b5b2e6c4afa738d7e1b89785eadd448206d40a2336d5c8888`, covering the manifest, lockfile, compiler declaration, build script and immediate application source files.
The release profile uses optimisation level 3, fat LTO, one code-generation unit, aborting panics and stripped symbols.
Version output includes the package version, commit, dirty state, target, compiler and build timestamp.

Plain `just check` passes on macOS arm64 with 46 library tests, one CLI integration test, separate doctests and release compilation.
The native backend gate passes with the declared git-branchless and Jujutsu versions.
All 24 workflow scenarios, comprising 36 submissions per implementation, and all 14 direct CLI comparisons pass.
Six supplementary optimised workflow cases cover git-branchless and both Jujutsu layouts, with native stack management enabled and disabled, comprising 18 submissions per implementation.
These single-sample cases verify the release profile; they do not establish a new performance baseline.
Application coverage is 2,596 of 2,631 lines, or 98.67%, with complete line, region and function coverage in the five strict modules.
Initial and repeated setup, plain `prek` and the development-tool discovery require no manual `PATH` overrides.
All three hooks were installed for this baseline; the pre-push hook invoked `just check`.

The baseline executable occupies 5,509,088 bytes, compared with 9,333,664 bytes for the preceding default-profile executable.
Seven launch probes per executable are retained, including their first-launch outliers; these probes overlap development work and do not establish a runtime performance improvement.
The first release compilation after the profile and build-dependency changes takes 123.38 seconds; it shall not be compared directly with an unchanged incremental build.

Baseline evidence resides in `target/validation/engineering/` and `coverage/`.
Failed validation attempts remain alongside the successful final logs.
Linux and hosted GitHub Actions have not been executed for this candidate; CI is configured for Linux and macOS using the same setup and validation recipes.
The following sections concern earlier application digests and shall not be attributed to this candidate.

## Git configuration names

The preceding configuration-naming baseline uses camelCase Git configuration keys and kebab-case command-line flags.
The `defaultDraft` setting supplies the default for new PRs; `--draft` overrides that default for an invocation, whilst title patterns remain applicable.
Its application source digest is `d79179b7419ac89ec63588873e7ca506b5436fe5e092e3394264672d19874047`.
Git handles parsing, includes, Boolean conversion, case-insensitive section and variable names, and scope precedence.
Native configuration tests cover differently capitalised keys across scopes and within repeated settings.
The macOS `just check` gate passes with 46 library tests, one CLI integration test, separate doctests and the release build; the native backend gate also passes.
Coverage records 2,596 of 2,631 application lines, or 98.67%, with complete line, region and function coverage in all five strict modules.
All 24 workflow scenarios, comprising 36 submissions per implementation, and all 14 direct CLI comparisons pass.
The comparison harness declares the four renamed reference reads explicitly and rejects candidate reads using the earlier spellings; operation, request and mutation checks remain enforced.
Git parses all ten active settings in the README configuration example.
Evidence resides in `target/validation/default-draft-parity.json`, `target/validation/default-draft-guards.json`, the `target/validation/default-draft-` logs and `coverage/`.

The earlier managed-interpreter baseline provisioned Python 3.14.8 under `target/validation/tools/runtime` and selected it for Python tool installation, hooks and recipes.
Initial and repeated setup succeed, and both plain `prek run --all-files --show-diff-on-failure` and `just check` pass without an explicit `PATH` override or virtual-environment activation.
The corresponding logs are `target/validation/managed-python-setup.log`, `target/validation/managed-python-setup-repeat.log`, `target/validation/managed-python-prek.log` and `target/validation/managed-python-check.log`.

Linux validation remains unavailable because the Docker daemon cannot be reached.
The following records concern earlier application digests and shall not be attributed to this candidate.

## Native Git configuration baseline

The earlier configuration model delegates persistent application settings to Git, including file parsing, includes, Boolean conversion and scope precedence.
Its application source digest is `4799631c1f9bc47236b4655b011320a5a3160d40cd33586f7b68ad05c98156d3`.
The macOS `just check` gate passes with 46 library tests, one CLI integration test, separate doctests and the release build.
The native backend gate passes, including repository-local settings in a separate Jujutsu workspace.
Coverage records 2,596 of 2,631 application lines, or 98.67%, with complete line, region and function coverage in all five strict modules.
The README configuration example is parsed by Git and contains all ten documented active settings.
The nine reference test functions for standalone application configuration are expressly superseded; the case ledger records the replacement tests and retains all 312 captured production calls.
The differential harness permits exactly three additional configuration reads per completed settings load and Git Boolean conversion for two existing keys, whilst retaining all other command, request, mutation and phase checks.
Deliberately duplicated configuration reads, an undeclared configuration read and a missing HTTP request are rejected by the comparison guard checks.
The initial focused-test failures concerned an include condition using a non-canonical temporary path and incorrect expected output text; the corrected fixtures pass and the failed logs are retained.
The initial hygiene run corrected an extra final newline before the successful gate.
The workflow comparison run was interrupted after 18 passing scenarios when the Homebrew Git executable became temporarily unavailable.
Git subsequently returned at version 2.56.0, whilst the unqualified `python3` command resolved to the system's Python 3.9.6.
The six unfinished scenarios pass with the installed Python 3.14.8 selected explicitly in the process environment; both unsuccessful attempts remain in the retained logs.
All 24 scenarios, comprising 36 submissions per implementation, pass a final audit of their retained traces under the comparison rules recorded for that run; the 14 direct CLI comparisons also pass.
The combined record is `target/validation/git-config-trace-audit.json`, accompanied by the original and completion logs.

Linux validation of this candidate is unavailable because the Docker daemon cannot be reached.
The earlier Linux and timing results below concern their expressly identified application digests and shall not be attributed to this candidate.
Logs for that baseline use the `target/validation/git-config-` prefix.
The measured gate records 1.31 seconds for hygiene, 7.87 seconds for semantic checks, 69.54 seconds for tests, 2.61 seconds for doctests and 11.19 seconds for release compilation.
Coverage takes 111.00 seconds and the separate backend gate takes 39.96 seconds; these development measurements overlap other validation activity and are not submission benchmarks.

## Earlier baseline

Date: 12 October 2026.
Package: `git-pr` 0.1.0.
The application source digest for this earlier baseline, covering the manifest, lockfile, compiler declaration and application source files, is `91a771dc2c91787c8b532becd7ee3fbaabdf4a47ae4beb76516e8ff6c4d8f898`.
The executable specification is `duanyutong/git-pr-go` at `132dc66ea004d03606585c9bdfe95289bd0b9dcb`.
Test methodology and artefact locations are specified in [validation](testing.md).
This record distinguishes completed checks from outstanding platform acceptance; it does not certify an unexecuted CI run.

### Platforms and controls

The macOS host runs 26.6.2 on arm64, with Rust 1.99.0, Git 2.56.0 and Python 3.14.8.
Linux validation uses the official Rust 1.99.0 Debian Trixie image on aarch64, pinned to `sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181`.
Additional development and backend versions are fixed in `scripts/dev-tools.env`.

The final macOS candidate passes `just check`: hygiene, Clippy with warnings denied, current dependency advisories and policy, unused-dependency detection, Python checks, 45 library tests, the CLI integration test, separate doctests and release compilation.
No doctest examples are presently defined.
The final coverage gate records 2,635 of 2,670 application lines, or 98.69%, with complete line, region and function coverage in all five strict modules.
Real git-branchless and both Jujutsu layouts pass the backend gate and are exercised again by the final test suite.

The earlier Linux candidate passes Clippy, 43 library tests, the CLI integration test, separate doctests, release compilation and an additional native Secret Service test within a private D-Bus session.
Actual git-branchless and both Jujutsu layouts are included.
These results precede the final correction to nullable configuration decoding and two additional library tests; they are not represented as execution of the final candidate on Linux.

The case ledger accounts for all 74 reference test functions, 301 leaf cases and 312 captured production calls.
The complete differential baseline passes 24 workflow scenarios, comprising 36 submissions per implementation, and 14 direct CLI comparisons.
Four supplementary configuration comparisons pass after the final correction, giving 28 scenarios and 40 submissions per implementation in total.
They cover nullable output values and local overrides beside empty or null project configurations.
Repository identities, PR content and external-operation arguments agree; checks also verify per-invocation and per-phase operation counts.
The earlier scenarios do not contain the configuration documents affected by the correction.

Repeated setup succeeds with the declared tools already present.
An intentionally unused function in a disposable copy causes Clippy to fail and both `just lint` and `just check` to stop before testing.
A separate harness failure exercise confirms retention of the failing trace and closure of the loopback listener.
These deliberately failing exercises are acceptance checks for failure propagation, not successful application test runs.

A subsequent Linux run was interrupted when the host exhausted its available storage.
The Docker filesystem subsequently returned I/O errors, including when removal of the task-owned `git-pr-validation` container was attempted.
The final Linux rerun, additional Linux comparison and coverage jobs, and container removal therefore remain outstanding.
Restarting Docker would interrupt unrelated containers and requires the operator's approval.
Hosted GitHub Actions have not been executed during local development.
Full platform acceptance remains conditional upon those validation jobs completing successfully.

The reference checkout's pre-existing README modification, installed application and existing `git ss` alias remain unaffected.
Validation uses private repositories, local bare remotes and loopback services; no production PR is created or modified.

### Validation cost

Recorded cold Linux compilation takes 38 seconds for test artefacts and 74 seconds for the production release.
The Linux library tests subsequently take 4.53 seconds, the CLI test 0.28 seconds and the isolated credential test 2.29 seconds.
These measurements describe the container environment and shall not be compared directly with macOS timings.

The final macOS gate records 1.43 seconds for hygiene, 5.25 seconds for semantic checks, 43.88 seconds for tests, 0.86 seconds for separate doctests and 4.41 seconds for the release recipe.
This run reuses dependencies but recompiles the changed application.
A preceding unchanged release recipe takes 0.28 seconds.
Repeat setup takes 2.77 seconds.
The first coverage test phase takes 136.24 seconds, including 83 seconds of instrumentation compilation; external instrumented comparisons take another 44.00 seconds.
Final coverage takes 75.19 seconds overall, including 46.44 seconds for instrumented compilation and tests and 25.49 seconds for external comparisons.
The complete 24-scenario comparison takes 345.22 seconds; the dedicated backend gate takes 19.01 seconds.
These are development costs, distinct from submission latency.

Ten independent version-command launch probes record a reference median of 21.62 milliseconds and maximum of 28.72 milliseconds, and a candidate median of 18.42 milliseconds and maximum of 860.02 milliseconds.
The initial candidate outlier is retained without attributing an unverified cause.
Launching the debug test executable to list tests records a median of 41.71 milliseconds and maximum of 43.47 milliseconds.
These probes precede the configuration correction, whose branches are not exercised by either command.

### Submission measurements

The complete matrix comprises three samples for each combination of backend, stack size and native stack mode: 54 paired scenarios and 324 application invocations.
All behavioural comparisons, repository snapshots, operation counts and phase barriers pass.
Both executables use ordinary optimised builds and retain production waits.
The matrix and its first serial investigation use application digest `548cc4a751e8e6c653a8a247703341b92f6f7d37da8ddc7d189bdc77d5a051f6`, before the nullable configuration correction.
No timing workload contains a configuration document that exercises the corrected branches.

The committed [measurement dataset](benchmark-baseline.json) retains source and executable identities, raw samples, medians, maximum observed latency and subprocess/request counts.
Maximum observed latency is a descriptive tail measurement, not a percentile estimate.
Full command and HTTP traces, phase spans and repository snapshots remain in `target/validation/history/benchmark-matrix`, `target/validation/history/benchmark-follow-up` and the subsequent benchmark directories.
Three samples support investigation of repeatability; they do not establish a population-level latency guarantee.

The original matrix uses two concurrent scenarios, always executes the reference first, and uses an HTTP fixture listen backlog of five.
Initial small workloads overlap other validation activity.
Candidate/reference median ratios include 1.374 and 1.478 for the one-commit git-branchless repeat submissions, and 1.148 for the ten-commit colocated Jujutsu amended submission.
These observations warrant investigation; they are not discarded.

A serial investigation repeats both workloads three times, alternates implementation order and increases the fixture listen backlog to 128.
It performs no concurrent compilation.
The previously observed differences are not reproduced at their original magnitude:

| Workload | Submission | Reference median, seconds | Candidate median, seconds | Candidate/reference | Candidate maximum, seconds |
| --- | --- | ---: | ---: | ---: | ---: |
| git-branchless, one commit | First | 10.718 | 11.527 | 1.075 | 11.822 |
| git-branchless, one commit | Amended | 8.982 | 9.404 | 1.047 | 10.286 |
| git-branchless, one commit | Unchanged | 8.961 | 9.521 | 1.063 | 11.131 |
| Colocated Jujutsu, ten commits | First | 24.210 | 25.840 | 1.067 | 26.997 |
| Colocated Jujutsu, ten commits | Amended | 20.240 | 20.957 | 1.035 | 22.611 |
| Colocated Jujutsu, ten commits | Unchanged | 19.220 | 19.123 | 0.995 | 20.730 |

Native stack management is disabled in this investigation.
The full matrix includes both modes and both Jujutsu layouts.
The original matrix records the dominant four-second waiting interval; subsequent reports additionally account for the one-millisecond rewrite delays and distinguish concurrent waiting from the sum of scheduled per-push waits.
These reporting refinements do not change application waits or operation multiplicity.
No speed advantage over the reference is asserted.

The final application digest is additionally measured in one git-branchless sample and three colocated Jujutsu samples, using the same serial collection controls.
All comparisons pass.
The one-commit git-branchless candidate/reference ratios are 0.968, 1.039 and 0.998 for first, amended and unchanged submissions respectively; a single sample is not a repeatability claim.
Across the three ten-commit Jujutsu samples, reference medians are 24.267, 19.627 and 18.858 seconds, whilst candidate medians are 24.213, 19.296 and 19.197 seconds: ratios of 0.998, 0.983 and 1.018.
Candidate maxima are 34.461, 24.377 and 19.399 seconds.
The slower first sample remains in both the raw data and these maxima; subsequent samples do not reproduce its initial or amended latency.
Command counts, HTTP counts and waits remain identical.
No material reproducible regression is established by these measurements, although their limited sample size and local service boundary constrain the conclusion.

### Evidence retention

The principal final records are `target/validation/acceptance-corrected.log`, `target/validation/coverage-corrected.log`, `target/validation/parity-final-report.json`, `target/validation/config-parity-report.json`, `target/validation/backends-final.log` and `coverage/`.
Linux baseline and interrupted-run diagnostics are retained separately in `target/validation/linux-validation.log` and `target/validation/linux-final.log`.
Setup, deliberate failure exercises, launch probes and measured phase costs reside in `target/validation/setup-warm.log`, `target/validation/manual-failure.json`, `target/validation/harness-failure.json`, `target/validation/startup.json` and `target/validation/timings.jsonl`.
Local generated artefacts are excluded from Git; CI is configured to retain corresponding evidence even after failure.
