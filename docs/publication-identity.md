# Publication identity and recovery

Submission records publication identity in repository-local Git configuration and preserves stored commit messages, parents, trees, signatures and object identities.
New changes require a maintained local branch or Jujutsu bookmark.
Existing `Remote-Ref` trailers remain supported, including legacy commits without a local anchor, but submission never inserts, removes or normalises a trailer.
Plain Git, git-branchless and Jujutsu use the same identity rules.

## Stored association

```gitconfig
[branch "dyt/example"]
    gitPrHead = "github.com/owner/repository refs/heads/dyt/example"
    gitPrPullRequest = "https://github.com/owner/repository/pull/123"
```

`gitPrHead` stores the publication repository and full branch ref in one value, separated by one ASCII space.
`gitPrPullRequest` identifies an adopted review; it is not a cache of whether that review remains open.
Jujutsu associations additionally record `gitPrChangeId` to establish continuity when a bookmark is renamed.
A revision hash identifies the submitted content, rather than the enduring review.
The publication head repository and the review repository must both match the selected repository in the currently supported workflow.
Publication requires one verified push destination; multiple push URLs must be disambiguated before submission.

Git parses configuration and reports scopes and source files.
Associations belong to the shared repository configuration, including local includes, rather than global or worktree overrides.
Equivalent repeated definitions are accepted; incompatible or malformed definitions require correction at their reported origins.
Ordinary writes fill missing records in the common repository configuration without changing editor-owned settings or included files.
Policy preferences retain ordinary Git scope precedence.

## Authority and reconciliation

Local associations constrain intended identity.
An existing trailer constrains the head ref and must agree with any stored head association.
Fresh GitHub observations establish the review's current identity, lifecycle and relationships; they cannot silently substitute a different review for a recorded PR.
A reused branch name therefore cannot redirect an adopted review.

Where an association is missing, verified editor PR metadata, Git publication mappings, maintained branch names and exact PR-head object relationships provide discovery evidence.
Upstream configuration supplies fallback evidence and cannot override an established binding.
A pull upstream naming the trunk or an unrelated topic does not establish publication identity.
Multiple credible candidates or competing bound anchors require explicit disambiguation.
Commit titles, approximate patches and stack positions never determine review identity.

Every run observes the selected changes, reconciles claims, resolves candidates, validates the complete publication plan, persists associations and applies verified operations.
Missing metadata is handled by this same procedure.
Confirmed absence, an unavailable observation and conflicting evidence have different consequences; a failed lookup does not authorise creating a replacement PR.
Dependency context outside the selection is read without migrating its configuration.

## Recovery and interrupted effects

Deleting some or all association keys is recoverable when surviving anchors and verifiable remote evidence determine one association.
Maintained publication names permit recovery after a rebase or amendment even when local and remote hashes differ.
A Git branch rename normally carries its configuration to the new local name whilst retaining the original publication head.
Copied bindings require disambiguation before submission.
If every relationship to an old publication name has been destroyed, the local graph alone cannot reconstruct that historical choice.

All required head associations are persisted before remote mutation.
Each push uses an explicit expected-object lease, including an absence lease for a new remote head.
A changed remote ref stops publication rather than being overwritten using an obsolete observation.
Successful earlier effects remain in place after a later failure; retry observes and completes the remaining work.
An unchanged successful run performs no redundant association writes or PR metadata changes.

Before creating a PR, submission writes a repository-local `git-pr.pendingCreate` value.
Its format is `v1 <attempt-id> <head-repository> <full-head-ref> <review-repository> <full-base-ref>`.
The UUID identifies a local attempt and is not a GitHub idempotency token.
An observed resulting review is adopted before the matching attempt record is removed.
An outstanding attempt whose outcome cannot be established prevents automatic creation on the next run.
An empty result while a request may still complete, or elapsed time alone, is insufficient evidence that creation failed.
Deleting both an outstanding attempt and all authoritative service evidence defeats that guarantee.

## Explicit association and diagnostics

After verifying the intended destination, an association can be established with Git:

```sh
git config --local branch.dyt/example.gitPrHead 'github.com/owner/repository refs/heads/published-name'
git config --local branch.dyt/example.gitPrPullRequest 'https://github.com/owner/repository/pull/123'
```

Use Git's origin reporting to locate conflicting or inherited definitions before changing them.
A supplied association still undergoes repository, uniqueness and lifecycle checks.
An incompatible existing trailer requires a separate, deliberate history edit; submission will not perform it.
For an uncertain creation, verify whether the original request produced a PR and associate that review where applicable.
Only after resolving the outstanding request should its exact `pendingCreate` value be removed with Git's literal-value matching operation.

Dry runs and the preparation checkpoint neither persist associations nor issue remote requests.
`--stop-after=prepare` names the preparation checkpoint; `--stop-after=rewrite` remains a compatibility alias and performs no rewriting.
Older executable versions do not understand these associations and may still insert trailers.
