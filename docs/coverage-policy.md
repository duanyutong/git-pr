# Coverage policy

The authoritative acceptance inventory is [coverage-policy.toml](../coverage-policy.toml).
Both `git-pr` and `git-ls` use the same manifest schema, unit collector and package collector.
This is repository development policy; Cargo does not interpret the manifest.

## Source classification

Every Rust source file under a declared `source_root` shall appear exactly once in the inventory, except test implementation files under `tests/` or named `tests.rs`.
Missing files, obsolete entries, duplicate classifications and unknown manifest fields constitute acceptance failures.
The inventory uses complete repository-relative paths, including where several modules share a basename such as `mod.rs` or `policy.rs`.
Moving a module requires updating its inventory path without changing its classification or weakening its acceptance obligation.

| Classification | Obligation |
| --- | --- |
| `unit` | Complete executable line, region and function coverage from the declared unit suite alone. |
| `mixed` | Policy and effects share a module; package coverage and behavioural tests remain required. |
| `boundary` | External execution or resource access; package coverage and behavioural tests remain required. |
| `declarations` | Module declarations, type declarations, constants or re-exports without handwritten executable behaviour. |
| `generated` | Compiler or derive-generated implementation without a handwritten executable coverage entry; explicit behavioural assertions remain required where applicable. |

Classification shall reflect the module's actual responsibilities.
Reclassifying uncovered policy solely to satisfy acceptance is inadmissible.
Mixed and boundary classifications exclude sources from the strict unit percentage, while retaining their contribution to package acceptance.

## Attribution and thresholds

Each package declares its unit target, optional test filter and optional test-name exclusions through `target`, `test_filter` and `test_skip`.
Unit tests shall exercise production behaviour using values and in-memory doubles, without external commands, repository access, sockets, native credentials or wall-clock waits.
Repository tests shall remain part of ordinary and package acceptance, regardless of their placement within the Rust module tree.
Test implementations shall reside in separate test source files and shall not contribute to the reported production metrics.

Each unit suite uses a freshly cleaned profile directory under `target/unit-coverage/<suite>/`.
Package collectors use separate instrumentation directories and shall not contribute profiles to unit acceptance.
Unit reports shall contain exactly the declared unit files, with non-zero counts and no uncovered executable lines, regions or functions.
Package reports shall include every unit, mixed and boundary source, meet the declared `minimum_lines`, and retain complete coverage of unit sources.
Package line minima are specified by each package's manifest entry; this repository requires 100% for the application.
Acceptance uses integer covered and total counts; rounded display percentages cannot establish compliance.

## Commands and evidence

`just test-unit` executes the manifest's declared unit suites without instrumentation.
`just coverage-unit` collects and enforces independent unit coverage for the Rust application.
`just coverage-versioning` measures the Python versioning implementation, including command subprocesses, and requires at least 95% line coverage.
`just coverage` requires the unit and versioning gates before collecting application package coverage.
The application collector may retain repository-specific external fixtures where these are necessary to exercise production boundaries.

Unit JSON, HTML and text reports reside under `coverage/unit/<suite>/`; combined unit counts reside in `coverage/unit/summary.json`.
The application report resides in `coverage/coverage.json`; coverage.py retains versioning JSON and HTML under `coverage/versioning.json` and `coverage/versioning/`.
Collector regression checks exercise inventory rejection, basename collisions, missing and incomplete reports, unit test selection and integer threshold decisions.
These checks form part of `just test`.

Complete line, region and function coverage does not establish complete branch or input-combination coverage, nor does it establish the correctness of assertions.
Platform-specific execution remains an obligation of the corresponding CI platform.
