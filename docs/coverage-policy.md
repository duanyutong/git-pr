# Coverage policy

The authoritative acceptance inventory is [coverage-policy.toml](../coverage-policy.toml).
This is repository development policy; Cargo does not interpret the manifest.
The principal coverage measure is independently attributed coverage of the core Rust implementation by its declared unit tests.
Executable lines, regions and functions shall each be completely covered within that scope.

## Source classification

Every Rust source file under a declared `source_root` shall appear exactly once in the inventory, except test implementation files under `tests/` or named `tests.rs`.
Missing files, obsolete entries, duplicate classifications and unknown manifest fields constitute acceptance failures.
The inventory uses complete repository-relative paths, including where several modules share a basename such as `mod.rs` or `policy.rs`.
Moving a module requires updating its inventory path without changing its classification or weakening its acceptance obligation.

| Classification | Obligation |
| --- | --- |
| `unit` | Complete executable line, region and function coverage from the declared unit suite alone. |
| `mixed` | Policy and effects share a module; behavioural acceptance remains required. |
| `boundary` | External execution or resource access; native behavioural acceptance remains required. |
| `declarations` | Module declarations, type declarations, constants or re-exports without handwritten executable behaviour. |
| `generated` | Compiler or derive-generated implementation without a handwritten executable coverage entry; explicit behavioural assertions remain required where applicable. |

Classification shall reflect the module's actual responsibilities.
Reclassifying uncovered policy solely to satisfy acceptance is inadmissible.
Mixed and boundary classifications exclude sources from the strict unit denominator without exempting them from behavioural acceptance.
Deterministic decisions and orchestration through injected boundaries belong in the unit scope; native process, filesystem, HTTP, terminal, credential and clock adapters retain their integration-test obligations.

## Attribution and thresholds

The application declares its unit target, optional test filter and optional test-name exclusions through `target`, `test_filter` and `test_skip`.
Unit tests shall exercise production behaviour using values and in-memory doubles, without external commands, repository access, sockets, native credentials or wall-clock waits.
Repository tests shall remain part of ordinary acceptance, regardless of their placement within the Rust module tree.
Test implementations and test-only helper definitions shall reside in separate test source files and shall not contribute to the reported production metrics.
Production generic functions instantiated with in-memory test doubles remain valid production coverage evidence.

Each unit suite uses a freshly cleaned profile directory under `target/unit-coverage/<suite>/`.
Repository, backend, native-credential and other integration execution shall not contribute profiles to unit acceptance.
Unit reports shall contain exactly the declared unit files, with non-zero counts and no uncovered executable lines, regions or functions.
Acceptance uses integer covered and total counts; rounded display percentages cannot establish compliance.
Missing or duplicate files, foreign source paths and invalid metric counts constitute acceptance failures.
The collector clears the current summary before validating or collecting evidence so that a failed attempt cannot retain an earlier successful result.
Exclusion patterns are anchored to the repository layout; a checkout ancestor named `tests` shall not exclude production sources.

Aggregate application coverage, branch coverage, generic-instantiation coverage and MC/DC are outside the acceptance metrics and routine summary.
They carry no percentage targets.
Ordinary validation and coverage acceptance use the pinned stable compiler and require no nightly compiler.

## Commands and evidence

`just test-unit` executes the manifest's declared unit suites without instrumentation.
`just coverage-unit` collects and enforces independent unit coverage for the Rust application.
`just coverage-versioning` measures the Python versioning implementation, including command subprocesses, and retains a 95% statement-coverage sanity guard.
`just coverage` runs the strict Rust unit gate and the separate versioning sanity guard.
Passing output identifies `Rust unit coverage: 100% lines, regions and functions` and reports the versioning guard separately as passed.
A passing versioning guard requires no additional work to reach 100%; a failed guard retains detailed percentage and missing-statement diagnostics and fails acceptance.
Ordinary repository and backend tests remain required through `just check` and `just backends`; Linux additionally requires native credential acceptance through `just credentials`.

Unit JSON, HTML and text reports reside under `coverage/unit/<suite>/`; combined unit counts reside in `coverage/unit/summary.json`.
The retained unit evidence identifies the local source scope, stable compiler and test selection.
Versioning JSON, HTML and diagnostic text reside under `coverage/versioning.json`, `coverage/versioning/` and `coverage/versioning-summary.txt`.
Collector regression checks exercise inventory rejection, basename collisions, exclusion boundaries, invalid and incomplete reports, independent unit selection and rejection of stale successful summaries.
These checks form part of `just test`.

Complete unit line, region and function coverage does not establish complete input-combination or native-path coverage, the correctness of assertions or the absence of defects.
Platform-specific execution remains an obligation of the corresponding CI platform.
