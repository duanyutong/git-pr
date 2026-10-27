# Validation evidence

Acceptance requires `just check`, `just coverage` and `just backends` against the complete source state being assessed.
Focused regressions supply evidence for their named contracts but do not replace these gates.
The [testing contract](testing.md) specifies fixture isolation, platform obligations, coverage attribution and behavioural expectations.
The [assurance guide](assurance.md) specifies additional generated campaigns, mutation analysis and semantic fuzzing.

## Provenance and retention

An acceptance record shall identify the source revision or source digest, whether the working tree contains uncommitted changes, the host operating system, the Rust compiler and the relevant development-tool versions.
Record each executed command, its exit status, elapsed duration and retained diagnostic location.
Where an external restriction prevents execution, identify the unexecuted obligation explicitly rather than representing a focused check as full acceptance.
A failed or interrupted attempt remains part of the evidence; a later successful run does not retrospectively establish that the earlier attempt passed.

Measured validation phases append timing and status records under `target/validation/`.
Rust unit coverage reports and the separate Python versioning sanity-guard evidence reside under `coverage/`, with isolated unit profiles and integer metric requirements as specified in the [coverage policy](coverage-policy.md).
Backend acceptance uses disposable repositories and isolated credentials; it shall not operate upon a developer's working stack or production pull requests.
Native results are attributable to the operating system on which they execute.
The CI platform matrix supplies additional macOS and Linux evidence without implying that every local execution covers both systems.

## Behavioural and performance claims

Behavioural conclusions shall identify the independently expected result, the observed repository or service effect and the relevant failure boundary.
A successful command alone does not establish preservation of pull-request identities, human descriptions, unrelated refs or partially completed remote effects.
Tests assert these postconditions directly using in-memory boundaries and native disposable fixtures.
Generated campaigns additionally report their seeds, dimensions and execution budgets.
Coverage, generated cases, mutation detection and native observations remain distinct forms of evidence with distinct limitations.

The [submission efficiency contract](submission-efficiency.md#verification-and-interpretation) defines enforceable operation budgets for repeated unchanged submissions.
Those budgets require zero ordinary pushes, metadata mutations, local ref synchronisations, rewrites or unconditional sleeps when authoritative observations establish that no change is required.
Fresh invocation-scoped destination observations and pull-request observations remain necessary; zero remote writes does not imply zero remote reads.
Record the selected stack size, backend, native-stack setting, cache state, request counts, subprocess counts and any retries when assessing that contract.

Elapsed submission measurements shall distinguish compilation and tool acquisition from application execution.
Record the build profile, workload, service environment, sample count and individual samples alongside any aggregate statistic.
Local service fixtures establish fixture-specific elapsed time and operation counts; they do not predict live GitHub latency.
A timing improvement does not authorise weakened identity, lease, freshness, recovery or publication safeguards.
