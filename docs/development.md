# Development

Rust 1.99.1 and Edition 2024 are specified by `rust-toolchain.toml` and `Cargo.toml`.
Development requires Git, rustup, uv, just and prek; command-runner versions are declared in `scripts/dev-tools.env`.
Linux requires a C compiler, pkg-config and the D-Bus development headers; credential acceptance additionally requires `dbus-run-session` and `gnome-keyring-daemon`.
The corresponding Debian and Ubuntu packages are `build-essential pkg-config libdbus-1-dev libssl-dev dbus-x11 gnome-keyring`.
macOS requires the Xcode command-line tools.

The command `sh setup.sh` verifies the command-runner versions, synchronises the Python environment from `uv.lock`, provisions hook environments, fetches locked Rust dependencies and registers hooks.
prek owns additional development-tool installations in its standard cache; setup creates no repository-local executable directory.
The non-package project in `pyproject.toml` pins Python to 3.15.0 for development and versioning scripts.
uv selects or downloads a compatible interpreter using its standard storage and maintains the repository environment in `.venv`.
Run development scripts as `uv run scripts/<name>.py`; executable scripts also provide `#!/usr/bin/env -S uv run` shebangs.
Run these commands from the repository so uv discovers its Python requirement.
prek manages the pinned Ruff, Pyright, Tombi, ShellCheck and typos distributions in isolated hook environments declared in `prek.toml`.
It also provisions cargo-sort, cargo-deny, cargo-machete, cargo-llvm-cov, cargo-mutants, cargo-fuzz, git-branchless and Jujutsu as Rust CLI dependencies of the hooks that require them.
rustup supplies the repository compiler, rustfmt and Clippy components.
The hook interpreter and compiler requests must remain consistent with `project.requires-python` and `rust-toolchain.toml` when upgrading either runtime.
Use `prek run ruff-check --all-files` for Python linting, `prek run ruff-format --all-files` for Python formatting and `prek run pyright --all-files --hook-stage manual` for Python type checking.
Hooks and recipes require no repository-specific `PATH` changes or virtual-environment activation.
The recipe `just test` runs acceptance through a managed hook containing the declared git-branchless and Jujutsu dependencies.
Coverage recipes similarly run their collectors inside hooks containing cargo-llvm-cov and, where repository tests require it, git-branchless.
Manual lint selects its own hooks explicitly, so invoking it does not execute acceptance tests or coverage.
CI obtains the pinned just and prek command runners through `uv run` and caches prek environments alongside the Cargo caches.
Cache keys separate operating systems, architectures and declared Rust toolchains; fallback restoration reuses the most recent compatible cache when dependency or hook configuration changes.
Boundary caches additionally remain separate for each validation gate.
Restoring a cache does not replace locked dependency resolution or execution of any validation gate.
Tool installation and update behaviour is delegated to prek rather than maintained in repository-specific installers.

The commands `just typecheck` and `cargo test --locked <name>` provide focused checks during development.
The command `just test-unit` runs the declared in-memory suites; `just coverage-unit` enforces their independent coverage boundary.
The [unit coverage specification](unit-coverage.md) identifies covered policy, remaining gaps and excluded external effects.
The recipe `just fmt` applies formatting through prek and reports changes for review.
The recipe `just lint` runs hygiene followed by the manual Clippy, dependency-policy, unused-dependency and Python type checks.
The recipe `just check` runs lint, all test targets, separate doctests and the production release build.
Run `just check`, `just coverage` and `just backends` before committing changes for acceptance.
These commands remain explicit validation gates and are also executed by CI; push hooks do not invoke them.
Use `just assurance` for reproducible generated contracts, native Git observations, bounded workflows, known-defect sensitivity and retained fuzz corpus replay.
Use `just mutants [BASE]` to assess assertions within the declared critical function scope and `just fuzz [SECONDS] [SEED]` for bounded, instrumented semantic exploration.
The [assurance guide](assurance.md) specifies prerequisites, independent contracts, case and seed controls, mutation interpretation, retained evidence and campaign limits.
`just coverage-branches` requires complete unit branch outcomes under the pinned nightly instrumentation in isolated storage and runs as a separate CI gate.
The stable application coverage gate requires complete application line coverage; neither completeness requirement establishes all input combinations or execution paths.
Local recipes execute on the host operating system; the [platform coverage contract](testing.md#platform-coverage) explains why CI also validates macOS and Linux and when additional native execution is relevant.
The release profile uses optimisation level 3, fat link-time optimisation, one code-generation unit, aborting panics and stripped symbols for release binaries.
Fat LTO permits optimisation across crate boundaries; one code-generation unit permits optimisation across the crate.
These settings can improve code size or execution speed, but increase build time and do not guarantee a runtime improvement.
Symbol stripping reduces executable metadata and limits subsequent debugging.
Source archives without Git metadata remain compilable; version output reports `unknown` for the commit and dirty state.
Aborting panics terminate the process without stack unwinding or destructor execution; ordinary `Result` error handling remains unchanged.
Optimisation level 3 is already the [Cargo release default](https://doc.rust-lang.org/cargo/reference/profiles.html).

Measured validation recipes record elapsed time and exit status under `target/validation/`; failed attempts remain in the record.
Coverage reports remain under `coverage/`; neither output directory supplies development-tool executables.
The default test concurrency is six and may be overridden with `RUST_TEST_THREADS`.
Formatter rewrites require review and a subsequent clean hygiene run.

Compiler and development-tool upgrades require review of the lockfile, lint results, behavioural tests and validation timings.
Dependency exceptions require a crate-specific justification in `deny.toml`.
Commit hooks validate Conventional Commits and generate required package-version increments.
Pre-commit hooks perform formatting, spelling, syntax and repository-hygiene checks on applicable staged files.
Pre-push verifies committed version history and runs prek's built-in trailing-whitespace, end-of-file and large-file checks.
The whitespace and end-of-file hooks apply automatic fixes at pre-commit and use `--check` at pre-push to report defects without modifying files.
It invokes `uv run --locked scripts/version.py pre-push`; version verification requires no Rust compilation.
The Python implementation and its behavioural tests are identical in `git-pr` and `git-ls`; package identity is read from the relevant manifest snapshot.
Existing hook installations read `prek.toml` on invocation and require no reinstallation for this policy change.
Successful CI on `main` triggers annotated version tags, as specified in [versioning](versioning.md).

The engineering policy is specified in [engineering conventions](conventions.md).

## Source organisation

Rust permits a module and its children to occupy either `name.rs` with `name/child.rs`, or `name/mod.rs` with `name/child.rs`.
The parent file defines the module; the directory contains its children, so the former arrangement does not duplicate the module.
The [Rust Reference](https://doc.rust-lang.org/reference/items/modules.html#module-source-filenames) encourages the newer `name.rs` naming convention, while continuing to support `name/mod.rs`.
This repository consistently uses `name/mod.rs` for production modules with children so that directory listings expose the ownership hierarchy directly.
Leaf modules retain descriptive filenames; directories containing only an otherwise unnecessary `mod.rs` wrapper shall not be introduced.
Ordinary production modules use standard module declarations rather than custom source paths.

```text
src/
├── lib.rs
├── main.rs
├── cli/
├── commands/
├── config/
├── credentials/
├── diagnostics/
├── git/
├── github/
├── jj/
├── model/
├── stack/
├── submission/
└── tests/
```

| Directory | Responsibility |
| --- | --- |
| `src/cli/` | Argument parsing in `mod.rs`, help resources, result formatting in `output.rs` and interactive rendering in `progress.rs`. |
| `src/commands/` | Injected command contracts and coordination in `mod.rs`, native process execution, cancellation and the editor helper. |
| `src/config/` | Configuration assembly in `mod.rs`, precedence in `settings.rs`, batched discovery, checkpoints, files and paths. |
| `src/credentials/` | Credential-document interpretation in `mod.rs`, native credential decoding and platform access. |
| `src/diagnostics/` | Timing instrumentation in `mod.rs` and contextual failure classification in `errors.rs`. |
| `src/git/`, `src/github/`, `src/jj/` | Backend adapters in `mod.rs`, backend-specific policy and protocol interpretation; GitHub owns its HTTP adapter in `http.rs`. |
| `src/model/` | Shared commit, pull-request and repository values in `commit.rs`, `pull_request.rs` and `repository.rs`, with commit-trailer interpretation in `trailers.rs`; `mod.rs` declares the children. |
| `src/stack/` | Selection and ancestry policy in `selection.rs` and managed description rendering in `description.rs`; `mod.rs` declares the children. |
| `src/identity/` | Publication association schema, Git configuration persistence and evidence reconciliation. |
| `src/submission/` | Workflow sequencing in `mod.rs`, with runtime construction, state, pull-request operations, presentation and deterministic policy in child modules. |

The executable entry point delegates to the library's submission runtime; `lib.rs` declares application modules and exposes the supported library interface.
Repository and pull-request value types are independent of configuration loading and external execution.
Policy functions accept values and return decisions without invoking commands, HTTP requests, terminal operations or waits.
Adapters obtain external values and execute authorised effects; presentation modules render the resulting decisions.
Shared application tests remain under `src/tests/`, with unit tests under `src/tests/unit/`, preserving their common in-memory fixtures and the independently measured `tests::unit::` namespace.
Owner-local test declarations deliberately retain explicit paths into the shared test tree where access to private state is required; this does not introduce custom production module paths.
The coverage inventory identifies production modules by their full repository-relative paths, including nested modules with identical filenames.
