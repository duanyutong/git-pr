# Engineering conventions

This document specifies the engineering requirements for this repository.
The checkout provisions its own development tools.

| Area | Convention |
| --- | --- |
| Compiler | Rust 1.99.1, Edition 2024, minimal toolchain with rustfmt, Clippy, LLVM tools and rust-analyzer. |
| Rust policy | Workspace-wide compiler metadata and lints, inherited by the application; unsafe code is forbidden. |
| Module layout | Production modules with children use `name/mod.rs`; leaf modules retain descriptive filenames, and ordinary production declarations use no custom source paths. |
| Release profile | Optimisation level 3, fat LTO, one code-generation unit, aborting panics and stripped symbols. |
| Build identity | Version output records the package version, commit, dirty state, target, compiler and build timestamp. |
| Dependencies | Committed Cargo lockfile; locked builds and tests; current advisory, licence, source and duplicate-version checks. |
| Setup | `sh setup.sh` or `just setup`; prek-managed hook environments and the uv-managed Python 3.15.0 project environment. |
| Formatting | Rustfmt, cargo-sort, offline Tombi, Ruff, spelling and repository hygiene. |
| Hooks | Pre-commit formatting and hygiene, Conventional Commit and version validation at commit time, version-history verification and lightweight, read-only built-in file checks before push. |
| Validation | `just lint`, `just typecheck`, `just test`, `just coverage` and `just build-release`; `just check` combines lint, tests, doctests and release compilation. |
| Coverage | Complete application line coverage, complete unit branch outcomes under pinned nightly instrumentation and complete coverage within the declared pure-policy boundaries. |
| CI | Linux and macOS, pinned action revisions, repository setup and local recipes, bounded jobs and retained evidence. |
| Configuration | Git owns parsing, includes, precedence and Boolean conversion; keys use camelCase and flags use kebab-case. |
| Release policy | Commit hooks generate required version increments; successful main CI triggers annotated version tags. |
| Documentation | Concise README, copyable Git configuration, separate command and development references, British English and one sentence per source line. |

Dependencies, backend fixtures and coverage boundaries remain explicit.
The [source organisation](development.md#source-organisation) assigns directories to responsibilities and specifies the intentional placement of shared and owner-local tests.
Licence allowlists, duplicate-version exceptions and Clippy exceptions require justification for the actual dependency graph or code surface.
The release procedure is specified in [versioning](versioning.md).
