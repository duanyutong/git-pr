# Development requirements

Apply the shared coding-standards skill, the [Rust Style Guide](https://doc.rust-lang.org/style-guide/), [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/), and [Clippy guidance](https://doc.rust-lang.org/clippy/usage.html).
Keep business policy independent of command, HTTP, terminal and timing boundaries.
Production code shall not contain unsafe code, unchecked panics or unfinished paths.

Use `just check`, `just coverage` and `just backends` for acceptance.
See [development](docs/development.md) and [testing](docs/testing.md).
Tests shall use disposable repositories and isolated credentials.
Do not execute submission against a developer's working stack.
Preserve command sequencing and trailer representation.
Persistent application settings belong in Git configuration; Git shall handle file parsing, includes, Boolean conversion and scope precedence.
Configuration keys use camelCase; command-line flags use kebab-case.

Use Conventional Commits.
Create new branches under `dyt/` and new pull requests in draft.
Do not add co-authorship trailers.
Documentation shall use precise British English, with one sentence per source line.
Preserve paragraph boundaries; do not add Markdown hard-break markers.
PR descriptions shall not contain hard line breaks.
