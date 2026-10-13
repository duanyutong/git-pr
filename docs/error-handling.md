# Error handling and recovery

## Diagnostic contract

An anticipated failure shall identify the operation which failed and retain its underlying cause, except where disclosure could reveal credentials.
The process shall return a non-zero status for a failed submission.
Existing fatal-operation and command-line usage failures retain status 2; other operational failures use status 1, and handled terminal interruption uses status 130.
Help and version requests remain independent of repository discovery.
After stopping its owned commands, interruption permits at most 100 milliseconds for the diagnostic attempt.
A full stderr pipe or a held stderr lock shall not postpone process exit.

A failed observation shall not establish that configuration is absent, a repository is clean, a commit is non-empty, a PR is missing or publication succeeded.
Git configuration status 1 without a diagnostic denotes an absent key; a missing template file permits examination of the next template location.
Other access, execution and decoding failures shall retain a diagnostic rather than select a default silently.

| Boundary or condition | Required diagnostic and recovery behaviour |
| --- | --- |
| Invalid flags, environment encoding or command-line encoding | Identify the invalid input; usage errors retain the parser's existing help contract. |
| Deleted or inaccessible working directory | Identify directory access failure and advise changing to an existing, accessible repository directory. |
| Missing executable, denied execution or command startup failure | Identify the command and preserve the OS cause; advise checking installation, permissions and the working directory. |
| Non-zero command status, deadline, signal or invalid structured encoding | Identify the executable and operation, retain status or cause, and reject incomplete observations. |
| Repository or configuration discovery failure | Preserve the Git failure, including malformed configuration or access errors, rather than describe every failure solely as a non-repository directory. |
| Missing remote default-branch metadata | Name `refs/remotes/<remote>/HEAD`, preserve the Git cause and explain `git remote set-head <remote> --auto` or an explicitly verified default branch. |
| Missing author identity | Name the missing Git key and provide the corresponding repository-local `git config` command. |
| Jujutsu workspace with an unavailable executable | Identify the detected workspace and require installation or repair; do not silently select Git as its backend. |
| Failed or malformed cleanliness and commit-change observations | Stop before rewriting or publication; distinguish inaccessible state from actual uncommitted changes. |
| Unreadable or invalidly encoded PR template | Name the template path and access or encoding problem; do not replace it silently with the default template. |
| Missing, malformed or inaccessible GitHub credential configuration | Name its actual path and provide authentication guidance; omit YAML contents from parser diagnostics. |
| Locked or unavailable native credential store | Preserve the store failure and advise unlocking it or authenticating again; do not report merely that a token is absent. |
| HTTP initialisation, network, TLS, proxy or response-read failure | Use fallible client initialisation, identify the method and URL, and explain that a lost mutation response can leave its remote result unconfirmed. |
| HTTP authentication, permissions, absence, validation, rate-limit or service failure | Retain the status and a bounded server diagnostic; provide guidance specific to the status without asserting an unverified cause. |
| Invalid or incomplete GitHub JSON or GraphQL data | Identify the response or observation and reject incomplete results rather than cache them as authoritative absence. |
| Incomplete REST PR content or inconsistent creation identity | Reject missing reconciled fields and unexpected heads, bases or repositories before adopting an identity or changing cached content; an invalid creation response shall explain that a PR may already exist and require remote verification before retrying. |
| Overlapping or non-advancing REST association pages | Deduplicate matching PR identities; reject a full page with no new identities, identify the requested association and page, and leave the incomplete result uncached. |
| Inconsistent native-stack identity or membership | Identify the requested stack and conflicting observation; reject empty, duplicate or missing member identities before any native-stack mutation. |
| Existing PR base differs from the planned base | Permit recovery to the configured trunk only with verified merged-downstack identity and ancestry; otherwise stop before rewriting or publication, without offering a confirmation bypass. |
| Recovery retarget or publication is uncertain | Observe before proceeding, retain confirmed base changes, and stop if identity, head, membership or base differs; never reopen or replace a closed PR implicitly. |
| Native member requires base recovery | Require a separately verified native transition; native-mode preferences and confirmation flags shall not bypass this refusal. |
| Closed or merged publication identity | Identify the PR, lifecycle state and recorded base; stop without creating a replacement or assuming that the commits reached the trunk. |
| Unselected PR depends on a publication destination | Identify the dependent PR and branch; require an appropriately complete selection or a separately verified transition. |
| PR identity, lifecycle or base changes across publication | Retain pushed refs, stop subsequent PR creation and updates, and require inspection of the newly observed state. |
| Malformed or ambiguous managed-footer delimiters | Identify the affected PR, require exactly one correctly ordered delimiter pair or removal of all delimiters whilst preserving human text, and prevent updating the affected description. |
| Reserved footer delimiters in a description initialisation source | Identify the commit message or PR template, require removal of the delimiters or substitution of `&lt;` for their opening `<` in literal examples, and prevent the affected PR creation or description update. |
| Failed push followed by failed verification | Retain both failures and state that some remote refs may already have changed. |
| Failed PR creation followed by failed recovery lookup | Retain both failures and explain that a PR may already exist; do not retry creation blindly. |
| Failed ancestry verification during local ref reconciliation | Warn with the actual cause and leave the local ref unchanged; retain successful publication. |
| Failure during preparation, publication, PR content or native stack reconciliation | Identify the phase and explain that any completed local or remote effects remain. |
| Failed confirmation input | Identify the input failure and decline the operation. |
| Unavailable output stream | Retain a stream-write failure and return a non-zero status; explain completed effects through stderr where that stream remains usable. |
| Unexpected Rust panic at the executable boundary | Print the application version, source location and recovery guidance; omit the panic payload and stop. |

## Remote default-branch discovery

A remote-tracking default-branch reference records which remote branch submission shall use as its trunk.
Its absence does not prevent ordinary Git commits on a local branch, including `main`.
It prevents submission configuration from determining a trunk reliably.
The tool shall not infer that the default branch is `main`, nor modify this metadata automatically.
Fetch the remote where necessary and establish its actual default branch before recording the reference locally.

## Partial effects and retries

Submission comprises ordered Git or Jujutsu operations and GitHub requests; it is not a transaction spanning those systems.
A failed preparation operation may already have rewritten local history.
A failed publication batch may already have updated some refs.
Subsequent PR or native-stack failures retain successful publication and any completed service changes.
Inspect the reported operation, repository status and relevant remote state before retrying.

A lost transport response does not establish that a mutation failed remotely.
The existing bounded observation and recovery paths shall determine confirmed effects where possible.
Retry policy remains restricted to explicitly recognised throttling responses; these diagnostic changes do not introduce blind mutation retries.
Successful invocation budgets and completion barriers remain subject to the [submission efficiency contract](submission-efficiency.md).

The final PR listing is emitted only after description reconciliation and native linking have completed successfully.
Errors after publication shall retain their phase-specific partial-effect diagnostics without printing that completion listing.
An HTTP 502 during a description mutation may represent partial application; this release does not retry that mutation blindly or treat its absence from the completion listing as proof of rollback.

## Confidentiality and limits

Credential wrappers and HTTP request debug output redact authentication tokens; native credential commands suppress output logging.
HTTPS remote discovery separates embedded user information from the hostname before credential lookup and API construction.
Remote-parser diagnostics, verbose traces, command-failure diagnostics and the final operational-error display redact URL user information.
HTTPS remote components shall not contain raw whitespace, query or fragment delimiters; path separators and other reserved characters in user information require percent-encoding.
Diagnostic redaction treats an authority-like prefix containing whitespace, raw `?` or raw `#` before `@` conservatively, including malformed URLs rejected by discovery.
A colon within that prefix can introduce a password containing raw path separators; this ambiguous form is redacted through the diagnostic's final `@`, including intervening slashes and nested scheme text.
That conservative rule can also suppress a port-bearing URL, intervening URLs or adjacent prose containing `@`; such text does not establish a reliable credential boundary.
Without a colon in the authority-like prefix, an explicit path separator limits redaction, retaining subsequent path and query text.
The final `@` is located once, and conservative redaction then terminates scheme scanning, preserving a linear bound in diagnostic length.
Successful command observations retain their original bytes for parsing; redaction applies to their diagnostic representation.
Malformed credential YAML diagnostics omit source contents because parser messages can quote a token-bearing line.
Command-failure context names the executable, its initial operation and working directory without repeating arbitrary arguments or standard input.
Subprocess and server diagnostics may contain repository details; review such details before attaching logs to an issue.
HTTP status diagnostics display at most 2,048 server-response characters and explicitly identify truncation.

The executable installs an unexpected-failure hook even in release builds, where panics abort without unwinding.
This hook provides an explanation; it does not make an internal panic recoverable or guarantee execution of cleanup destructors.
Forced termination, memory exhaustion and unavailable diagnostic streams can prevent an explanation from being emitted.
Accordingly, the supported contract covers anticipated failures and diagnosable unexpected panics, rather than claiming that every possible process termination can produce a message.

## Verification

Regressions exercise the failure conditions through in-memory adapters, disposable repositories, isolated process boundaries and local HTTP fixtures.
They verify retained causes, credential redaction, non-zero statuses, blocked subsequent mutations and preserved completed effects.
The three acceptance gates are `just check`, `just coverage` and `just backends`.
The [testing contract](testing.md) specifies platform-specific obligations and the isolation required for these gates.
