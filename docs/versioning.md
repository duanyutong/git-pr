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

Successful CI for a push to `main` triggers the release workflow.
The workflow verifies the complete untagged range before creating annotated `v<version>` tags, then publishes those tags in one atomic push.
Automatic release discovery uses semantic-version tags advertised by `origin`, including annotated-tag targets, to identify the latest published ancestor on the release history.
Local tags retained after a failed publication do not advance this baseline, and a subsequent invocation retries their publication without recreating them.
Only version tags derived from commits in the validated release range are published; unrelated local tags are retained locally.
Release publication explicitly disables Git's automatic annotation following, including when `push.followTags` is enabled in configuration.
After provisioning the Python versioning environment, the workflow refreshes `origin/main` and requires the validated revision to remain an ancestor of that reference before invoking tag publication.
An ordinary subsequent commit preserves eligibility; an amendment or history rewrite that removes the validated revision causes release failure.
This check reflects the remote history at fetch time and cannot prevent a concurrent rewrite after the check; published commits shall remain immutable.
Conflicting tags are rejected; existing tags are never moved.
PR validation and failed CI do not publish tags.
Tag publication does not create a GitHub Release, upload binaries or publish a crate.
The manual equivalent is `just tag-versions <base> <head>`; this command publishes tags to `origin`.

## Recovery

If push-triggered CI fails because of a transient runner, network or service error, rerun the original CI workflow after resolving the cause.
A successful rerun retains the original push event and revision and can trigger release publication.
If CI succeeded but release failed, rerun the failed release workflow; version publication is idempotent and retries do not move existing tags.
Use `gh run list --workflow ci.yml --branch main` and `gh run list --workflow release.yml` to identify the relevant run, then `gh run rerun <run-id>` to rerun it.
GitHub permits workflow reruns for up to 30 days after the initial run, as specified in its [workflow rerun documentation](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs).
Manually dispatching CI through `gh workflow run ci.yml --ref main` performs validation but does not publish tags, because release accepts only successful push-triggered CI.
Recovery shall use a validated revision that remains in current `main`; do not rerun release for a superseded amendment or repoint a published tag.
An actual code defect requires a corrective commit and a new push rather than a retry of the defective revision.
