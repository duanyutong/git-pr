# Submission efficiency

## Ownership and execution

Git owns publication in every mode.
PR observation, creation, base verification, descriptions, labels and native topology use one application HTTP adapter and one reusable client.
Native linking never delegates publication or PR reconciliation to an extension.
The implementation follows GitHub's [stack API](https://docs.github.com/en/rest/pulls/stacks), including ordered membership, append and the distinction between complete and partial unstacking.

The workflow retains the following dependency order:

1. Load effective Git configuration, validate the working tree and discover the selected history.
2. Apply selection, draft and author exclusions; validate references before publication and before inserting new reference trailers.
3. Infer missing branch names in memory, calculate predecessor bases once, and verify existing PR identities, lifecycle states, base relationships and incoming dependencies before inserting trailers.
4. Insert missing trailers in a backend transaction, verify recovered selection identities, publish the selected refs, invalidate affected observations and confirm pinned PR identities before creating any new PRs.
5. Require existing PR bases to remain unchanged, gather complete description context, reconcile changed content and labels, then reconcile native topology if enabled and print the completion listing.

Local validation checkpoints and dry runs do not require credentials.
Dry runs compute missing trailer assignments in memory and report proposed operations; they perform no rewrite, publication, configuration write or Jujutsu working-copy snapshot.
The historical `push` checkpoint includes PR creation and verification of existing bases.
No checkpoint authorises later phases after an earlier failure.

Ordinary configuration discovery uses three grouped Git queries: integer deadlines, string values and Boolean values.
The string query also acquires publication destination settings without a separate configuration process.
Remote selection follows [Git configuration precedence](https://git-scm.com/docs/git-config): the current branch's `pushRemote`, `remote.pushDefault`, the current branch's `remote`, then `origin` or the sole configured remote.
Branch subsection names retain their case-sensitive identity, and the current branch is read only when branch-specific destination settings exist.
Detached checkouts disregard settings for named branches.
Ambiguous destinations, missing configured remotes and multiple push URLs fail before publication, with instructions for selecting one named remote.
The selected remote's push URL determines both publication and the GitHub repository; alphabetical remote ordering has no influence.
Git retains responsibility for includes, conditional includes, parsing, typed conversion and scope ordering; NUL-framed output preserves value boundaries.
Explicitly overridden typed settings are excluded where the existing configuration contract permits this.
A failed typed batch is resolved through Git scalar queries, preserving native diagnostics and conversion behaviour, including rejection of malformed superseded values.
Literal branch validation performs linear work in the reference bytes without launching Git for each reference.
Checkout-history expressions such as `@{-1}` are rejected as publication destinations rather than expanded.
A native Git comparison corpus verifies the literal naming rules.
The configured remote trunk is rejected as a publication destination, including destinations inferred from local branches.
Inferred destinations undergo duplicate validation before rewriting, and publication preflights the complete destination set before its first batch.

Existing trailers bypass rewriting and the corresponding recovery reads.
Author exclusions are applied before rewriting and publication observations, then reapplied after changed commit identities are recovered.
Related commits may remain necessary as description context even when they are not publication targets.

## Observations and mutation planning

Git history is acquired through a NUL-framed protocol carrying full object identities, author metadata, trees, parents and messages.
Structured history and tree queries explicitly disable signature display, preventing presentation settings from contaminating metadata or launching signature verifiers.
The former 100-commit truncation is removed.
Merge commits in the discovered history are rejected before remote observation or local rewriting, because submission requires a linear stack.
Commits without a message remain subject to tree validation; a changed commit without a title prevents submission.
Known parent trees establish ordinary single-parent emptiness without per-commit subprocesses; unknown boundary trees retain an authoritative Git comparison.
The initial visible snapshot also supplies explicit selection and verified linear depths.
Its base commit/tree observation supplies the resolved trunk identity, avoiding a separate resolution command.
Descendant discovery consumes the immutable observed tip directly rather than resolving that identity again.
After rewriting, explicit selection reuses depths from the refreshed snapshot or Jujutsu change identities from the refreshed commit inventory.
Missing or ambiguous recovered identities prevent publication.
Verified single-parent paths supply depths from their actual fork point, including when the remote trunk has advanced independently.
The excluded range base remains unchanged when its selected descendants are rewritten and therefore requires no recovery query.
Git recovery follows the local branch at the selected tip where available, including submissions from another checkout.
Inventories retain literal branch names, and recovery resolves fully qualified branch refs so equally named tags cannot redirect it.
Without a tip branch, recovery uses the current checkout only after verifying the selected tip's exact first-parent position; an unverifiable or vanished selection fails explicitly.
Empty commits remain valid range boundaries after their exclusion from the displayed stack.
Unverified paths retain bounded recovery queries with complete identity validation.
Initial explicit Git arguments are deduplicated before resolution and resolved in batches of at most 128 revisions and 64 KiB of revision arguments.
The 1,000-commit selection regression requires eight initial resolution commands and no per-commit depth commands for a verified linear path.

Jujutsu takes one deliberate initial working-copy snapshot.
Its cleanliness protocol contains fixed status markers; description text cannot introduce delimiters or impersonate an absent description.
Working-copy metadata retains the commit's actual author identity, and an incomplete observation prevents submission.
Existing batched change-identity and working-copy reads also retain explicit Jujutsu conflict markers, without additional subprocesses.
Unresolved conflicts in a selected publication head's ancestry prevent rewriting and publication, including where the conflicted ancestor is excluded from PR creation.
An excluded conflicted descendant outside that ancestry does not prevent submission of its resolved ancestors.
An explicit list may include a described, non-empty working-copy commit; its discovery uses the same number of Jujutsu queries as an ordinary explicit list.
Discovery reads suppress incidental snapshots, and bulk change-ID queries use an explicit operation identity.
Explicit Jujutsu argument lists are deduplicated and resolved in batches of at most 128 arguments, with a combined revision-set and template budget of 64 KiB.
One operation observation pins all batches to an immutable repository state.
Template literals preserve Jujutsu's escape syntax, including raw control characters for which JSON Unicode escapes would be invalid.
Each returned commit identifies its original arguments through Jujutsu's [`contained_in` template method](https://github.com/jj-vcs/jj/blob/main/docs/templates.md), and every argument must independently match exactly one complete commit identity.
Resolution retains caller order and permits distinct aliases for the same commit, which selection subsequently deduplicates.
A failed batch retains the existing scalar Jujutsu and Git fallback diagnostics.
The 100-argument regression requires two initial Jujutsu processes, including operation acquisition, and the 1,000-argument regression requires nine.
Template membership evaluation is bounded by the batch cardinality; this reduces process launches without treating aggregate revision-set size as proof of individual cardinality.
Explicit Jujutsu ranges and lists reuse the tip identity from that same observation, without a second scalar change-ID query.
The observation retains an empty tip's identity before presentation filtering, so exclusion from the displayed stack does not require another lookup.
An adapter observation which cannot associate its requested tip with an observed commit retains a fallible scalar identity lookup.
Publication is followed by an explicit Git import before bookmark reconciliation.
Conflicted, unrelated and protected checked-out refs are preserved, and ordinary Git updates use compare-and-swap preconditions.

Invocation-scoped PR observations include identity, title, body, base, head repository, labels and native membership.
GraphQL head decoding transfers owned node fields into the pull-request deserialiser; normalising field names does not clone description strings.
Malformed connection shapes and missing required content remain fallible observations and cannot populate the cache.
Individual REST observations require explicit identity, description, title, label, state, merge-status and reference fields before entering the cache.
An explicitly null description denotes empty content; an omitted description cannot establish that existing human text is empty.
REST repository identities are validated independently of PR numbers, and successful creation responses must identify the requested head and base in the selected repository.
An invalid creation response leaves its remote effect unconfirmed and reports that a PR may already exist before advising verification prior to another submission.
Head queries contain at most 25 aliases.
Incomplete connections retain their initial GraphQL results and advance only unfinished connections in groups of at most 25 aliases.
PR and label cursors advance independently; missing or repeated cursors, malformed pages and identity mismatches prevent publication of an incomplete observation.
Foreign and deleted source repositories are excluded before PR selection, including in REST commit-association responses; missing repository identity is an error.
An unsupported GraphQL stack field is recorded once per invocation, after which reduced queries retain the same 25-head batching.
Reduced observations retain complete description and label content but explicitly leave native membership unknown.
Native topology reconciliation requires an additional authoritative membership observation when that field is unavailable.
Descendant history is read only beyond the immutable tip identity retained by the initial or post-rewrite observation.
These description-only reads omit recovery depths, Jujutsu operation identities and change-ID queries, which are required only for selected commits subject to rewriting.
The submitted prefix is reused, including when an empty tip has been excluded from presentation or a live checkout reference subsequently moves.
Description context borrows submitted commits and owns only newly discovered descendants; collecting context references does not copy commit messages.
Descendant description context is observed collectively before its cached identities are consumed, without one request or worker per descendant.
Failed descendant reference, ancestry or history reads stop description updates; incomplete local context cannot establish that a historical PR has merged.
Where merge markers are displayed, historical PRs outside the current selection use their observed GitHub merge status.
Fresh cached observations are reused; remaining historical identities are deduplicated and queried by PR number in batches of at most 25.
Description-history extraction refreshes each group of at most 25 cached PR identities immediately before reading their bodies and merge state.
History rows accumulate directly into one invocation-wide vector and identity set, without retaining a separate parsed list for each PR.
Source-order deduplication preserves the first observed row's merged state; newest-first descriptions reverse only the newly accumulated suffix, preserving earlier observations.
This refresh also applies where branch observations remain fresh; expiry shall not introduce an individual REST request for each retained PR.
Original PR numbers remain authoritative, and a PR absent from its previously observed branch retains the individual identity-read fallback.
The 51-PR expiry regressions require three refresh requests, whilst fresh repeated history extraction requires none.
Publication preflight refreshes selected branch observations in groups of at most 25.
A single open identity may be selected notwithstanding older closed history; multiple open identities or exclusively closed or merged history prevent publication.
The planner pins PR numbers and node identities, and requires each existing base to equal its intended predecessor unless the merged-downstack recovery contract below is satisfied.
A separate paginated query observes open PRs targeting each publication destination and rejects dependants outside the verified selection.
The publication plan retains a separate acquisition timestamp for each group of at most 25 destinations, using the invocation's monotonic clock.
Each timestamp precedes acquisition of its group's first page, so request delays and pagination contribute to the observation age.
Before publication, dependant observations older than 30 seconds are reacquired; fresh groups require no additional request.
Slow acquisition of a later group does not renew the earlier groups, and failed reacquisition prevents publication without renewing the affected observation.
These queries use GitHub's documented [`baseRefName` filter](https://docs.github.com/en/graphql/reference/repos#repository).
After publication, affected head and PR observations are invalidated regardless of their age; the same pinned identities and bases must be confirmed before any creation or update.
The ordinary post-publication reconciliation verifies bases and does not retarget existing PRs.
Branch publication explicitly disables automatic annotation following, so `push.followTags=true` cannot add unrelated annotated tags to the selected destination refs.
These observations provide conservative containment for published-stack reorders; they do not constitute an atomic transaction or a proof covering arbitrary concurrent remote operations.
General reordering and temporary-base staging remain unsupported.

An obsolete base may be repaired before publication when its unique merged PR has a verified merge result in the live configured trunk, its original head is present in the surviving published history, and any remaining obsolete ref still identifies that generation.
Recovery obtains strict individual PR observations, complete historical identity observations and live refs from a single resolved push destination matching the observed hosting repository.
Required Git objects shall be available locally; failed ancestry reads stop recovery and require synchronisation rather than an inferred negative result.
The original published heads and the prepared heads must remain outside their intended bases' ancestry throughout every old/new ref combination, and prepared heads must contain their intended bases.
Deleted obsolete refs are permitted, whilst reused branches, ambiguous historical identities and unselected dependants are refused.
Native members requiring retargeting remain blocked until a supported transition can be verified; an ordinary PR above a merged native stack is eligible.

The recovery plan remains read-only until local preparation completes.
All observations and dependencies are reacquired before mutation, and each retarget must be freshly confirmed before any publication.
Recovery publication uses explicit expected-object leases for every selected ref, including an absence lease for a new branch.
Post-publication observation additionally checks exact heads and native membership.
A lost retarget acknowledgement requires observation rather than replay; a later publication failure retains the repaired base and existing PR identity.
Git leases protect refs but do not make PR metadata and branch publication atomic, so concurrent metadata changes remain a detected stopping condition where observable.
Unchanged submissions do not acquire recovery evidence or incur its additional requests.
Missing identities, malformed statuses and failed requests stop description mutation.
Compact footers and presentation without merge markers require no additional historical-status queries.
Lookup failure, ambiguous identity and malformed observations cannot establish absence and therefore cannot authorise creation.
Commit-associated PR lookup retains its results; title similarity alone is not an identity criterion.
Explicit-selection context first reuses an observed open PR for its remote branch, including when the local commit is no longer associated with that PR.
Without an open branch match, commit-associated lookup retains the historical identity rule.
Complete fallback results, including absence, are cached for 30 seconds by commit identity; failed requests are not cached.
Commit-association decoding retains only PR numbers, state and head names after validating source repositories; descriptions, titles, labels and other unrelated fields are skipped during deserialisation.
The immutable association result is shared between the cache and its consumers without cloning the vector or its owned strings.
Overlapping REST pages retain each matching PR identity once, preserving first-observation order.
A full page containing no previously unseen identity is rejected as non-advancing pagination, including when its PRs belong to excluded source repositories.

Content and label changes are derived from observed differences.
Label differences are calculated after description updates, immediately before applying each PR's label policy.
Label refresh preflight checks each target once; a fresh observation requires no scan of the remaining targets.
An expired target triggers refresh of the remaining suffix of its current group of at most 25 PRs, permitting slow preceding mutations without retaining an expired plan.
Successful additions and removals update the invocation cache individually, so a later failure does not cause a retry to repeat confirmed changes.
Adopting an existing PR preserves its description even when the commit supplies a non-empty message.
Legacy stack sections are replaced whilst surrounding human and bot content is retained; commit messages and templates initialise empty or unchanged placeholder bodies only.
Legacy migration and historical-identity extraction share a conservative section recogniser.
Each exact observed body is analysed once within description reconciliation; cached section boundaries serve both history extraction and replacement.
Cached PR snapshots, their readers and body analysis share immutable description storage rather than copying complete bodies.
Full-stack renderers share the non-current rows and expose borrowed fragments plus the current header and row.
Managed-footer comparison consumes those fragments directly, including the delimiters, without materialising either a complete footer or a wrapped footer.
A changed description is assembled once into its final buffer; exact unchanged content retains its observed storage.
Required line-ending normalisation still produces replacement text.
Confirmed metadata changes replace the affected snapshot whilst retaining shared body bytes where the description is unchanged, so earlier readers remain valid.
Line-ending normalisation is retained with that analysis, and history rows are inspected only when full-stack presentation requires them.
A refreshed observation containing different body bytes replaces the analysis before rendering, preserving newer human edits.
Every non-empty line must match a generated stack header or PR row, with a recognised marker, stack header or current-PR annotation establishing ownership.
This recogniser accepts historical footers without emoji but preserves ambiguous plain PR lists and sections containing human prose, including issue-closing references.
Explicit managed-footer delimiters remain authoritative.
Delimiter analysis accepts absence or exactly one correctly ordered pair; unmatched, reversed, nested and duplicate delimiters produce an actionable error identifying the PR.
Malformed observations are not cached as valid bodies, adopted as legacy sections or repaired by appending another footer.
Rendering failures propagate before the affected content mutation is dispatched, preventing repeated-footer growth and unintended removal of human text.
Commit messages and templates used to initialise descriptions must not contain reserved footer delimiters, including delimiters quoted in fenced examples.
Initialisation rejects such inputs before the affected PR creation or description mutation and explains how to remove or escape literal delimiters without discarding the text.
Unused messages and templates do not constrain adoption of an existing human description.
Description bodies are rendered from the PR observation obtained when preparing each mutation batch, including any content refreshed after cache expiry.
A refreshed human edit therefore participates in rendering instead of being replaced by a previously prepared body.
The resulting title and body are compared again before dispatch, so a refresh which establishes the desired state requires no mutation.
Content mutations contain at most five aliases and fit within a 1 MiB encoded request budget, including conservative query overhead.
JSON escaping and UTF-8 bytes contribute to that budget; an individually oversized update fails explicitly.
A content update deferred at the byte limit retains its rendered body, encoded input and calculated size for the following request.
Preparation is reused only when the complete refreshed PR observation equals the original observation; changed observations require rendering again before any mutation.
This retention is limited to one deferred update and does not bypass observation expiry, human-edit preservation or unchanged-content detection.
Confirmed successful responses are retained when another alias fails.
GraphQL batches are not atomic.
A newly created PR normally requires a subsequent body update because its final full-stack footer depends on server-assigned PR numbers.

PR and head observations expire individually after 30 seconds from acquisition.
Before description planning, only expired, previously observed heads are refreshed; fresh entries survive slow invocations.
Before each content mutation batch, expired cached PRs are refreshed collectively through their observed heads, even where the head observation itself remains fresh.
Native topology reconciliation uses the same refresh mechanism in groups of at most 25 selected PRs before reading their membership.
Base verification likewise refreshes each group immediately before checking its relationships, retaining bounded workers and the original PR identities.
Fresh cached identity lookups and unchanged base comparisons execute directly without worker creation.
Base comparison borrows cached metadata; unchanged base reconciliation requires neither copying descriptions nor cloning commits.
Only unresolved operations use workers, and the original bounded completion barriers remain applicable.
Each such refresh retains the 25-head bound and complete pagination semantics.
The subsequent lookup retains the original PR number; absent cached identities and PRs no longer present under their observed heads require an individual authoritative read.
Partial mutations update known fields without renewing the acquisition age of unrelated fields.
This policy does not provide a distributed lock or prevent an edit between observation and mutation.
No persistent cache is trusted across invocations.
Cached head selection consumes borrowed PR observations in a single pass, with constant auxiliary space and no copying of bodies or labels.
It stops at the first matching open PR and otherwise retains the first matching historical identity in observation order.

Native reconciliation distinguishes unchanged membership, a contiguous selected slice, an append and a restructuring operation.
Resolved native-stack enablement authorises routine creation and extension without an additional confirmation, including with redirected standard input.
Unchanged topology requires no confirmation or mutation.
Restructuring retains confirmation and refuses to remove unselected members or combine multiple observed stacks.
Only a complete unstack response authorises rebuilding; retained members stop the workflow.
After unstacking, PR observations are invalidated and refreshed before verifying that the original identities remain open with the intended bases.
Changed bases are rejected rather than repaired through an additional PATCH request.
Native reconciliation distinguishes updated, unchanged and declined outcomes; declining a rebuild cannot produce an updated announcement.
Declined operations and single-PR selections retain warnings about blocked base changes.
Native reconciliation failures propagate to the invocation's exit status; completed ordinary PR updates remain effective.
HTTP 404 is a resource-lookup failure rather than evidence that native stacking is unavailable, including during membership reads, topology reads, appends and rebuilds.
This interpretation follows the [GitHub stacks API contract](https://docs.github.com/en/rest/pulls/stacks).

## Publication, recovery and boundaries

Publication batches contain at most 128 full-object refspecs and at most 64 KiB of refspec arguments.
Git performs its normal hooks and transport negotiation once per batch.
Progress output retains one logical publication line per ref; verbose command traces expose the actual batched invocation.
An up-to-date push is retained as the normal fresh publication check; a separate unconditional `ls-remote` preflight would add a transport to changed submissions.
After a failed push, one targeted remote observation identifies incomplete refs without replaying known completed publication.

Missing trailers ordinarily use one backend bulk editor transaction.
The editor receives task-owned message files as literal data, indexes commit-hash prefixes, and rejects ambiguous or incomplete editor sections.
Single-message changes and messages containing reserved backend delimiters use the literal-message compatibility path.
Every rewritten message is serialised once; scalar commands reuse the prepared bytes in the existing reverse execution order.
Bulk transactions preserve published commit contents and identities while changing backend undo granularity and unpublished intermediate recovery objects.

Explicit rate-limit rejection may be retried at most three times, subject to a fifteen-minute cumulative server-wait budget per request.
Retry-After and exhausted primary-reset metadata are honoured, including GraphQL errors conveyed by HTTP 200.
Without a supplied secondary deadline, backoff begins at sixty seconds and increases exponentially.
Primary quotas are observed separately for REST and GraphQL without additional polling.
Conservative rolling content budgets count individual aliases, with sixty writes per minute and 450 per hour.
Transient reads may be retried; uncertain content mutations instead trigger forced observation and bounded reconciliation of outstanding fields.
Long pre-dispatch waits invalidate prepared content and require re-rendering from fresh observations.
Confirmed writes are omitted from recovery, and the write-batch limit decreases following uncertain failure.
Ambiguous creation failure triggers an identity observation before any later attempt can create a duplicate.
Ordinary transport failures and failed mutations are not blindly replayed.
No transaction spans Git, Jujutsu and GitHub; a failure can leave completed effects in place.
The next invocation reconciles fresh state.

Native topology observations shall return the requested stack identity, a non-empty base and member list, and unique non-zero PR identities.
Topology members retain only their required PR numbers; descriptions, labels and other unrelated member fields are not deserialised into application values.
Selected PRs which claim native membership shall appear in that stack's member list.
Duplicate selected PR identities, invalid observed stack identities and inconsistent topology prevent native append, dissolution or recreation.
Valid selected slices preserve surrounding members under the existing reconciliation rules.

All HTTP dispatch is serialised, with elapsed-time-aware spacing between mutation requests and handling for service-specified retry delays.
This follows GitHub's [REST guidance](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api); removing visibility sleeps does not remove legitimate mutation pacing.
Unchanged submissions require no pacing because they make no mutations.

Structured processes retain binary stdout separately from diagnostics, while legacy commands preserve combined diagnostic order.
Concurrent readers drain socket pairs into bounded memory, avoiding output-file growth and stream back-pressure deadlocks.
Structured input uses a finite temporary file.
Structured commands default to a 120-second deadline; legacy backend and publication commands default to a 600-second deadline.
Git configuration and CLI flags may override these positive per-command deadlines.
Expired process groups are terminated and reaped.
Terminal interruption is forwarded to every registered child group, with a shared two-second grace period before forced termination; direct children are reaped before exit status 130.
Child registration and shutdown share a lock, preventing a spawn from escaping cancellation.
Captured command output and HTTP bodies exceeding 64 MiB fail explicitly rather than producing a partial successful observation.
Output limits are enforced during capture; oversized output terminates the owned process group.
Direct-child exit is observed without reaping the leader, retaining its process-group identity and cancellation registration until capture completes.
A blocking exit observer avoids imposing a polling interval on successful short commands.
After direct-child completion, a one-second drain grace period applies independently of whether further bytes arrive; a surviving stream fails explicitly rather than retaining the invocation indefinitely.
Capture failure terminates the retained process group before reaping and unregistering its leader, preventing descendants from continuing effects after failure.
Worker batches contain at most eight operations, resolve cached results directly, and join all started workers before propagating failure.

## Algorithmic bounds

Let N denote visible commits, S explicit selections, A distinct revision arguments before identity deduplication, B candidate branch tips, H retained history entries, and U descriptions requiring changes.
The bounds below exclude Git's own graph traversal and the bytes that must be read or written.

| Operation | Implemented bound or strategy |
| --- | --- |
| Literal branch validation | O(R) for R reference bytes, without subprocesses. |
| Configuration discovery | Three ordinary queries, independent of stack size; failed typed conversions retain scalar diagnostic fallback. |
| Selection and identity deduplication | Expected O(N + S), using indices and sets; initial Git resolution uses bounded batches rather than one process per selection. |
| Predecessor/base resolution | O(N), using one prefix pass before rewriting; PR creation, number recovery and native management share the resulting bases. |
| Local branch-name assignment | Expected O(B + N), using one branch inventory and an identity index. |
| Descendant-tip choice | One `merge-base --independent` call for at most 128 candidates; larger inventories require at most ceil((B − 1)/127) + ceil(B/127) bounded calls. Failed discovery commands stop description updates. |
| Rewrite message lookup | O(LN) prefix indexing, where L is the bounded object-identity length. |
| Required-label membership | Expected linear work in observed and desired labels, preserving deterministic output order. |
| Label refresh preflight | One check per target; only an expired target requires a bounded suffix refresh. |
| Description context | O(N + H) auxiliary storage for borrowed context, shared history and positions; extraction reads the observed footer bytes, and rendering performs work proportional to compared or emitted bodies. |
| Initial Jujutsu explicit-list resolution | One operation read and at least ceil(A/128) bounded queries, with additional groups when the 64 KiB command budget applies; at most 128 membership predicates per returned record, with independent cardinality validation. |
| Cached description storage | One shared immutable body per observation; unchanged rendering allocates no complete footer, wrapped footer or replacement body. Changed descriptions use one final output buffer. |
| Output formatting | One template parse followed by work proportional to emitted output. |
| Draft matching | Patterns compiled once into an anchored byte matcher; title classifications shared across selection, rewrite recovery and creation. |

Large descendant inventories first reduce bounded candidate groups, then verify the remaining candidate against every input group.
A unique global leaf, if present, wins its group and all subsequent groups; otherwise verification detects an incomparable discarded candidate.
The 129-tip regression requires four ancestry commands, compared with 256 in the former scalar fallback.

Full-stack footers necessarily emit O(U(N + H)) entries.
Unnumbered commits remain visible as escaped commit links with their author identity, including commits excluded from publication by author or draft policy.
Their links use full commit identities, and their inclusion does not create PRs or publish additional branch destinations.
PR-number deduplication applies only to numbered entries; distinct unnumbered commits shall not collapse into a shared zero identity.
Where unnumbered context is present, the header reports an entry position rather than implying that every entry is a PR.
Changing every PR in a large stack therefore remains quadratic in body bytes under the existing representation.
The implementation removes repeated context construction without changing that representation or silently truncating history.
Optional compact footers build an O(N) navigation index and emit at most three PR links per description, giving O(N + U) navigation work and entries.
Historical lists are deliberately omitted in compact mode; full footers remain the default.
A persistent cache, daemon and embedded VCS implementation remain conditional design options requiring separate evidence.

## Verification and interpretation

The strict unit gate covers every declared application policy module completely for lines, regions and functions.
Scale tests exercise 1, 5, 10, 50, 100, 101 and 1,000 commits, including full discovery, bounded request/publication counts and a mutation-free unchanged pass.
Regression counters enforce one body analysis per unchanged observation and one predecessor-base calculation per active submission in both native modes, including rewritten selections and compact presentation.
Protocol tests cover pagination, malformed identities, lost responses, partial mutation results, stale observations, ref guards, literal editor inputs and process deadlines.
Disposable backend tests and differential fixtures verify published objects and completed remote effects.
The [validation contract](testing.md) enumerates permitted differences from the inefficient reference workflow.

For N unchanged conventional PRs with complete observations, ordinary-size names, no additional descendants and a submission shorter than 30 seconds, the normal budget is 3 × ceil(N/25) observation requests, comprising initial head observations, incoming-dependency observations and fresh post-publication head observations, and ceil(N/128) up-to-date push batches, with zero metadata mutations, rewrites or sleeps.
Native mode adds one topology read for the selected existing stack.
Pagination, capability fallback, descendant context, recovery and long-submission refresh are explicit additional work.
These bounds establish removal of identified duplication; they do not establish a universal optimum or a live-service latency guarantee.
Measured acceptance and benchmark evidence is recorded in [the acceptance record](validation.md).

## Preparation and byte reuse

Request bodies are encoded before dispatch and retained unchanged for diagnostics and explicit rate-limit retries.
Content inputs are encoded once and measured by their encoded byte length; GraphQL envelopes incorporate those fragments without serialising their descriptions again.
REST content updates likewise reuse their measured bytes for dispatch.
The complete request remains subject to the 1 MiB content limit, and changed observations invalidate deferred preparation.

Draft patterns retain Unicode lowercasing followed by byte-oriented wildcard semantics.
Literal patterns use direct equality or anchored prefix and suffix comparisons followed by ordered KMP searches of their interior sections.
Prefix tables are prepared once, and each interior search consumes the title through its earliest complete match; literal sections cannot overlap.
For one literal pattern, preparation and matching take O(P + T) time and O(P) prepared storage, where P is pattern length and T is title length.
Only patterns containing `?` require an anchored, non-backtracking [byte regex set](https://docs.rs/regex/latest/regex/bytes/struct.RegexSet.html); literals are escaped, `?` consumes one byte and `*` includes newline bytes.
Their matching retains the dependency's O(P × T) worst-case bound for compiled pattern size P and title length T.
Raw pattern bytes are limited to 1 MiB in aggregate, and regex compilation additionally retains the dependency's resource limits.
Excessive patterns produce configuration guidance before rewriting, publication or PR mutation.
Each distinct original title is classified once per invocation, with results shared across configuration clones and rewrite recovery.
