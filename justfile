set export
set positional-arguments
RUST_TEST_THREADS := env("RUST_TEST_THREADS", "6")

default:
    @just --list

setup:
    sh setup.sh

fmt:
    prek run cargo-fmt cargo-sort tombi-format ruff-format --all-files

lint:
    uv run scripts/measure.py hygiene prek run --all-files --show-diff-on-failure
    uv run scripts/measure.py semantic prek run pyright cargo-clippy cargo-deny cargo-machete --all-files --hook-stage manual

typecheck:
    cargo check --workspace --all-targets --all-features --locked

test:
    uv run --locked scripts/test_version.py
    uv run scripts/test_coverage.py
    uv run scripts/test_assurance.py
    prek run acceptance-tests --all-files --hook-stage manual --verbose
    uv run scripts/measure.py doctests cargo test --workspace --doc --all-features --locked

check:
    just lint
    just test
    just build-release

test-unit:
    uv run scripts/unit_coverage.py --test-only

coverage:
    just coverage-unit
    just coverage-versioning
    prek run coverage-application --all-files --hook-stage manual --verbose

coverage-unit:
    prek run coverage-unit --all-files --hook-stage manual --verbose

coverage-branches:
    prek run coverage-branches --all-files --hook-stage manual --verbose

build-release:
    uv run scripts/measure.py release cargo build --release --locked

# Compare with the latest reachable release tag, or include the initial commit.
verify-versions base="0000000000000000000000000000000000000000" head="HEAD":
    uv run --locked scripts/version.py verify-range "$1" "$2"

# Explicit release command; CI invokes this only after successful main validation.
tag-versions base head:
    uv run --locked scripts/version.py tag-range "$1" "$2"

# Preserve the original parent as the version baseline when amending a commit.
amend *args:
    #!/bin/sh
    exec uv run --locked scripts/version.py amend "$@"

coverage-versioning:
    uv run --locked coverage erase
    uv run --locked coverage run scripts/test_version.py
    uv run --locked coverage combine
    uv run --locked coverage report --fail-under=95
    uv run --locked coverage json -o coverage/versioning.json
    uv run --locked coverage html -d coverage/versioning

parity:
    prek run parity --all-files --hook-stage manual --verbose

bench:
    prek run benchmark --all-files --hook-stage manual --verbose

backends:
    prek run backends --all-files --hook-stage manual --verbose

assurance:
    prek run assurance regression-sensitivity fuzz-smoke --all-files --hook-stage manual --verbose

mutants base="":
    GIT_PR_MUTATION_BASE="$1" prek run mutants --all-files --hook-stage manual --verbose

fuzz-smoke:
    prek run fuzz-smoke --all-files --hook-stage manual --verbose

fuzz seconds="60" seed="20261015":
    GIT_PR_FUZZ_SECONDS="$1" GIT_PR_FUZZ_SEED="$2" prek run fuzz --all-files --hook-stage manual --verbose

credentials:
    uv run scripts/measure.py credentials uv run scripts/secret_service.py
