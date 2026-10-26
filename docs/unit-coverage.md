# Unit coverage specification

## Attribution and acceptance

Unit coverage shall be attributed exclusively to tests which call production functions using values or in-memory doubles.
Such tests shall not execute external commands, inspect repositories, open sockets, access native credential stores or perform wall-clock waits.
The application suite is selected by `::unit::`, comprising the shared `tests::unit::` tests and owner-local `submission::unit::` and `github::unit::` tests.
Rust's placement of a test inside a source module does not, by itself, establish that the test is a unit test.

The authoritative inventory is [coverage-policy.toml](../coverage-policy.toml).
The shared [coverage policy](coverage-policy.md) specifies the manifest, source classifications, profile isolation and package acceptance rules used by both `git-pr` and `git-ls`.
Every production Rust module shall be classified as unit, mixed, boundary, declarations or generated; absent, duplicate and obsolete classifications constitute acceptance failures.
New classifications require review of the functions and effects in the relevant module.
Reclassifying uncovered policy as an external boundary solely to satisfy a percentage is inadmissible.

The unit boundary requires zero uncovered executable lines, regions and functions in every named production module.
Acceptance compares integer counts rather than rounded percentages and rejects missing or empty report entries.
Test implementation files are excluded from the report.
Python versioning tests reside separately in `scripts/test_version.py` and do not contribute to production coverage.

## Enforced application and policy boundary

| Area | Production modules and required cases |
| --- | --- |
| Command-line parsing | `cli/mod.rs`: defaults, supplied values, aliases, repetition, positional boundaries, Boolean forms, integer syntax, overflow and invalid arguments. |
| Command coordination | `commands/mod.rs`: result normalisation, verbose diagnostics, argument rendering and joining every worker before returning an error. |
| Configuration assembly | `config/mod.rs` and `config/batch.rs`: repository discovery, configuration reads, precedence, host documents, templates and revision-selection failures through in-memory boundaries. |
| Credential documents | `credentials/mod.rs`: host-document interpretation, absent and empty tokens, fallback failure and diagnostic redaction. |
| Configuration precedence | `config/settings.rs`: setting-specific CLI, environment and Git precedence; absence, empty values, label replacement and Unicode environment boundaries. |
| Commit representation | `model/commit.rs`: attributes, tags, deterministic deduplication, message serialisation and display. |
| Selection and ref policy | `stack/selection.rs`: ranges, explicit selections, skipped predecessors, repeatable draft exclusions, recovery callbacks, branch matching, exact remote postconditions and ancestry decisions. |
| Trailer interpretation | `model/trailers.rs`: malformed input, footer boundaries, multiline messages, dates and representation-preserving serialisation. |
| Description preservation | `stack/description.rs`: sentinel sections, legacy sections, historical membership, ordering, duplicates, compact navigation and content outside the managed section. |
| Output and diagnostics | `cli/output.rs` and `diagnostics/errors.rs`: presets, templates, validation, substitution, absent Git version metadata and contextual exit-status classification. |
| Git value interpretation and acceptance | `git/policy.rs`: complete remote names, dotted repository names, short hashes, absent descriptions, empty commits and missing titles. |
| Git operations | `git/mod.rs`: discovery, commit acceptance, rewrite prerequisites, push failure reconciliation, local ref guards and bookmark reconciliation. |
| Structured snapshots | `git/snapshot.rs`: framed records, full object identities, complete histories, malformed observations and verified linear depth reuse. |
| Batched PR observation | `github/observation.rs`: complete identities, pagination, labels, schema fallback, selective expiry and rejection of incomplete observations. |
| GitHub resilience | `github/content.rs`, `github/dispatch.rs` and `github/limits.rs`: bounded batches, partial-update reconciliation, concurrent edits, retry classification, server deadlines and conservative local write budgets. |
| Native topology | `github/native.rs`: idempotence, contiguous selections, append, consent, explicit declined outcomes, guarded rebuilding and retained-member failures. |
| GitHub operations | `github/mod.rs`: failed lookups, creation, base updates, consent, native-stack transitions, observation age, encoded content budgets and partial failures through scripted command and transport responses. |
| Jujutsu operations | `jj/mod.rs`: revision cardinality, change-ID recovery, working-copy assembly and bookmark targets. |
| Submission workflows | `submission/mod.rs`, `submission/state.rs`, `submission/recovery.rs`, `submission/pull_requests.rs` and `submission/presentation.rs`: phase barriers, publication recovery, author and draft filtering, explicit selections, PR completion, history, labels and partial failures. |
| GitHub decisions | `github/policy.rs`: draft creation, label overlap reconciliation, PR selection and URLs, and stack failure classification. |
| Jujutsu value interpretation | `jj/policy.rs`: working-copy status and description interpretation. |
| Configuration paths and checkpoints | `config/paths.rs` and `config/checkpoint.rs`: home expansion, recognised phase names, absence and invalid checkpoints. |
| Credential value interpretation | `credentials/keychain.rs`: supported native credential encodings and malformed values. |
| Submission invariants | `submission/policy.rs`: author exclusions, preservation of existing exclusions and the first duplicate remote reference. |

Pure backend interpretation is separated from command and credential adapters and is exercised through values and in-memory doubles.

## External boundaries

The enforced inventory comprises the application modules declared in `coverage-policy.toml`.
Application business decisions, including orchestration through injected boundaries, are covered by the independent unit run.
The shared in-memory fixture records commands, HTTP requests, file reads, credential requests, terminal output and scheduled waits; unexpected operations fail without invoking native adapters.
Owner-local tests exercise private submission state without broadening its production visibility.
In-process worker threads remain part of the unit suite; subprocesses, sockets, filesystem fixtures and native credentials do not.

`commands/cancellation.rs`, `commands/system.rs`, `config/files.rs`, `credentials/system.rs`, `github/http.rs`, `main.rs`, `cli/progress.rs` and `submission/runtime.rs` contain native execution, filesystem access, process input or runtime construction.
These modules retain separate package and integration acceptance.
`diagnostics/mod.rs` samples the monotonic system clock and writes timing records through the terminal boundary; it contains no submission policy.
Its event pairing, disabled behaviour, forwarding and payload-omission contracts are tested with in-memory adapters, whilst native CLI acceptance verifies runtime integration.
It is classified as a boundary and remains included in package coverage.

The review identified contracts which percentages alone do not establish: refusal to move refs after failed worktree discovery; preservation of history after failed reads; retention of draft filtering after rewriting; refusal to interpret a failed PR lookup as absence; explicit handling of non-Unicode process input; and joining every worker before returning an error.
These defects have been corrected, together with truncated remote names, overlapping label removal and compilation without Git metadata.
Pure decisions have unit assertions; the worker coordination test uses in-process threads and no external resources.
Disposable-repository regressions additionally verify worktree and index preservation, transient history failures and successful retries, refusal to create duplicate PRs, overlapping labels and draft filtering across a rewritten-stack reread.
Those repository and subprocess tests do not contribute to the strict unit percentage.

`lib.rs`, `model/mod.rs` and `stack/mod.rs` contain declarations and re-exports rather than executable policy.
The shared value module `model/repository.rs` contains declarations and derived implementations.
Handwritten PR decoding and state interpretation in `model/pull_request.rs` belong to the strict unit boundary.
Boundary or mixed classification excludes a module only from the strict unit percentage; it does not exempt that module from package acceptance or from behavioural obligations.

## Evidence and commands

```sh
just test-unit
just coverage-unit
```

The collector cleans an independent profile directory for each package and executes only its declared unit suite.
Integration, backend and credential runs cannot contribute profiles to those directories.
Reports reside in `coverage/unit/application/` and `coverage/unit/summary.json`.
The recipe `just coverage` requires the unit gate before collecting the existing package and versioning acceptance evidence.
CI invokes that same recipe.

Rust provides [compiler instrumentation](https://doc.rust-lang.org/rustc/instrument-coverage.html).
The repository uses the pinned [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) tool to collect profiles, identify uncovered code and enforce the declared boundary.
Complete line, region and function coverage does not establish complete branch, combination or generic-instantiation coverage, nor does it prove that assertions describe the correct behaviour.
Branch instrumentation currently requires nightly and is not part of this stable-toolchain gate.

Python versioning uses the independent `just coverage-versioning` gate described in [coverage policy](coverage-policy.md).
