# Versioning and release tags

Both repositories use identical `scripts/version.py` and `scripts/test_version.py` files.
The helper derives package identity from `Cargo.toml` and requires no Rust compilation.

Every commit after the initial commit shall contain the package version required by its Conventional Commit message.
The initial commit establishes the starting version.
`feat` requires a minor increment; `!`, `BREAKING CHANGE:` or `BREAKING-CHANGE:` requires a major increment; all other accepted types require a patch increment.
The manifest and lockfile shall record the same package version.
Release history shall remain linear; merge commits are rejected by version verification.

The commit-message hook generates an omitted increment in `Cargo.toml` and `Cargo.lock`, then stops the commit.
The generated changes require review and staging before the commit is repeated.
Existing unstaged manifest or lockfile changes are preserved and require resolution before automatic generation.
An incorrect staged increment is rejected without replacement.
The pre-push hook and CI independently verify committed version history.
Pre-push combines this version-policy check with lightweight, read-only built-in file checks; full acceptance remains an explicit development command and a CI requirement, as specified in [development](development.md).

```sh
git add Cargo.toml Cargo.lock
git commit

# Amendment uses the existing commit's parent as the version baseline.
just amend --no-edit

# Verification covers commits after the latest reachable version tag.
just verify-versions
```

Git does not supply an unambiguous amendment flag to commit-message hooks.
The `just amend` recipe invokes `git commit --amend` with the correct version baseline; additional arguments are forwarded to Git.
A change to the commit type may require a corresponding correction to the staged version.

The `Release` job belongs to the push-triggered CI workflow and depends on both complete validation matrices.
Its outcome contributes to the same workflow result as the six acceptance checks; eligible CI remains incomplete while publication is queued or running.
Only successful pushes to `main` can publish; pull requests and manual CI dispatches remain validation-only.
The workflow verifies the complete untagged range before creating annotated `v<version>` tags, then publishes those tags in one atomic push.
Automatic release discovery uses semantic-version tags advertised by `origin`, including annotated-tag targets, to identify the latest published ancestor on the release history.
Local tags retained after a failed publication do not advance this baseline, and a subsequent invocation retries their publication without recreating them.
Only version tags derived from commits in the validated release range are published; unrelated local tags are retained locally.
Release publication explicitly disables Git's automatic annotation following, including when `push.followTags` is enabled in configuration.
The checkout and `VALIDATED_HEAD` retain the original push's `github.sha`, including during reruns.
After provisioning the Python versioning environment, the job refreshes `origin/main` and requires the validated revision to remain an ancestor of that reference before invoking tag publication.
An ordinary subsequent commit preserves eligibility; an amendment or history rewrite that removes the validated revision causes release failure.
This check reflects the remote history at fetch time and cannot prevent a concurrent rewrite after the check; published commits shall remain immutable.
Conflicting tags are rejected; existing tags are never moved.
PR validation and failed CI do not publish tags.
Tag publication does not create a GitHub Release, upload binaries or publish a crate.
The manual equivalent is `just tag-versions <base> <head>`; this command publishes tags to `origin`.

## Publication credentials and concurrency

Release uses the built-in `GITHUB_TOKEN` with `contents: write` granted only to the dependent publication job.
Validation retains read-only Contents access.
No dedicated App, repository secret or personal access token is required for routine publication.

Checkout credentials are not persisted.
Only the publication step explicitly receives the built-in token through `GH_TOKEN`; its command-scoped Git configuration clears inherited credential helpers and delegates GitHub authentication to `gh auth git-credential`.
These credentials are not written into repository configuration or supplied to validation jobs.
GitHub may require additional Workflows permission when a pending historical tag targets workflow definitions which differ from the current default branch.
The built-in token cannot acquire that permission; `actions: write` is not a substitute.
Such a backlog requires an authorised one-off invocation of `just tag-versions <base> <validated-head>` using an existing login with Contents and Workflows write authority, or an OAuth token with `repo` and `workflow` scopes.
Use a disposable checkout, verify that the validated revision remains in remote `main`, and retain the helper's conflict checks and atomic publication.
Do not copy the operator's credential into CI, skip rejected historical tags, move existing tags or tag a superseded amendment.
Once the backlog is published, routine publication resumes with the built-in token.
Later workflow changes or queued releases targeting older workflow definitions can encounter the same restriction and require the same recovery procedure.

Validation supersession is scoped by workflow, event, ref and matrix entry.
Manual validation cannot cancel push validation, matrix peers do not cancel each other, and newer validation does not automatically cancel a running publisher.
Release uses the shared `release-tags` concurrency group with cancellation disabled and GitHub's [bounded pending queue](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency).
Queue ordering is not a publication authority; the helper continues to verify remote progress and tag identities before publishing.
Manual cancellation, runner failure and queue exhaustion remain recoverable failures.

## Recovery

If push-triggered CI fails because of a transient runner, network or service error, rerun the original CI workflow after resolving the cause.
A successful rerun retains the original push event and revision and can enable the dependent Release job.
If the acceptance jobs passed but Release failed, correct the evidenced cause and rerun the failed jobs within that original CI run.
For a historical-target permission refusal, complete the authorised backfill described above before retrying; rerunning with the same insufficient token does not change its authority.
Version publication is idempotent and retries do not move existing tags.
Use `gh run list --workflow ci.yml --branch main` to identify the run, then `gh run rerun <run-id> --failed` to retry the failed publication job.
Rerun the complete original workflow when acceptance evidence itself requires renewal.
GitHub permits workflow reruns for up to 30 days after the initial run, as specified in its [workflow rerun documentation](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs).
Manually dispatching CI through `gh workflow run ci.yml --ref main` performs validation but does not publish tags, because Release accepts only successful push-triggered validation.
Recovery shall use a validated revision that remains in current `main`; do not rerun release for a superseded amendment or repoint a published tag.
An actual code defect requires a corrective commit and a new push rather than a retry of the defective revision.
