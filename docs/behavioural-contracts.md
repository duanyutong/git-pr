# Behavioural contracts and independent evidence

## Recovery of obsolete downstack bases

An existing open PR may move directly to the configured trunk when a unique merged downstack identity and immutable Git ancestry establish that the downstack has landed there.
The original downstack head need not be an ancestor of trunk after a squash or rebase merge; the authentic merged result supplies the integration evidence.
The surviving published head must contain the original downstack head, and any still-existing obsolete ref must match that generation.
All affected dependencies and intermediate ref combinations must preserve the open PR invariant.

The complete recovery preflight precedes local rewriting, and fresh revalidation of the plan precedes any remote mutation.
Base changes are verified before publication, which uses explicit expected-object leases.
Post-publication reconciliation additionally verifies exact head identity and membership.
Uncertain effects require fresh observation; completed repairs are retained without automatic rollback, reopening or replacement creation.
Native members requiring an unsupported relationship change and arbitrary reorders remain refused.
Git leases and PR observations do not constitute a single transaction across Git and GitHub.

This specification defines the observable contracts prioritised by the behavioural assurance programme.
The production contracts and fixture obligations below govern their respective contexts; compatibility with an earlier implementation does not supersede them.
Passing a finite campaign establishes evidence within its declared inputs, transitions and failure positions, without establishing the absence of every possible defect.
The [assurance guide](assurance.md) specifies execution commands, retained artefacts and continuous integration obligations.

## Authorities and permissible normalisation

Git is authoritative for accepted literal ref names, symbolic ref targets, revision identities and resolved push destinations.
Native contract tests use [`git check-ref-format`](https://git-scm.com/docs/git-check-ref-format), [`git symbolic-ref`](https://git-scm.com/docs/git-symbolic-ref), [`git remote get-url`](https://git-scm.com/docs/git-remote) and observations of disposable repositories as independent oracles.
The shared command fixture is subject to the same contracts and shall not invent an `origin` remote when another publication remote is selected.
Object identities and ref names require literal equality; Unicode case folding, separator removal, abbreviation and prefix equality are not permissible substitutes.
Published metadata and ancestry shall correspond to the stored Git objects transferred to the remote.
Invocation-scoped command settings disable local replacement refs and legacy graft views throughout discovery, rewriting and publication, without changing repository settings or the caller's environment.
Existing `GIT_NO_REPLACE_OBJECTS` values retain their presence semantics; an inherited custom graft file shall not override the stored publication ancestry.

Trailer interpretation is an application contract stated in the following section, rather than a claim of complete equivalence with [`git interpret-trailers`](https://git-scm.com/docs/git-interpret-trailers).
Git comparisons apply only to the explicitly shared subset of ordinary, single-line `key: value` trailers.
Configurable Git trailer separators, command execution, continuation lines and acceptance of empty values are outside that subset.
Structured metadata follows the application's declared NUL-delimited field order using Git's [`pretty-formats`](https://git-scm.com/docs/pretty-formats) placeholders, rather than parsing Git's human-readable log presentation.

PR identity, permitted mutations, dry-run behaviour and reconciliation after uncertainty are application contracts specified here and in the [error-handling contract](error-handling.md) and [submission efficiency contract](submission-efficiency.md).
Tests construct the expected PR identities, bases, labels, human text and remote ref states independently of the production planning and rendering functions.
Managed descriptions canonicalise each run of one or more carriage returns immediately preceding LF to a single LF, both when analysing observed text and when composing new output.
This fixed-point normalisation includes line endings newly formed where trailing carriage returns meet an inserted newline separator.
Carriage returns outside such a run remain unchanged; the normalisation does not authorise arbitrary removal of human characters.

## Trailer requirements

A trailer key shall contain one or more ASCII letters, digits or hyphens, and shall be compared after ASCII lower-case normalisation.
Spaces and tabs may indent a trailer line, whitespace preceding the colon is discarded, and surrounding Unicode whitespace is removed from its value.
A value containing one Unicode scalar, including a single ASCII character, is sufficient; an empty value is not a trailer.
The parser recognises a terminal block of consecutive valid trailers separated from human body text by a blank line, or a message consisting entirely of trailers.
An invalid nonblank line encountered whilst inspecting that terminal block leaves the complete trimmed input as human text and produces no attributes.
The parser trims surrounding message whitespace and normalises line endings through line-based parsing, while preserving internal human text.
Unknown valid keys shall be retained alongside application-owned metadata.

The final occurrence of a normalised key on disk shall determine its effective value.
For example, `Tags: 🦀` followed by `Tags: x` shall yield the effective label `x`.
Likewise, `Remote-Ref: first` followed by `Remote-Ref: second` shall identify `second`.
The internal attribute representation places the winning occurrence first.
Production submission preserves stored commit messages and their trailer representation.

The [test fixture builder](../src/tests/model_helpers.rs) additionally serialises model values when constructing disposable commits.
Its serialisation shall preserve duplicate precedence, the established title and body separation, aligned title-case keys, lexical key ordering and final `Remote-Ref` group.
Unique-key fixture representation shall remain unchanged by the duplicate-precedence correction.

## Contract inventory

Test names below identify executable evidence rather than additional normative authorities.
The [generated property suite](../src/tests/unit/properties.rs), [native assurance suite](../src/tests/assurance.rs), [bounded workflow suite](../src/tests/unit/assurance.rs) and [deterministic regressions](../src/tests/regressions.rs) retain the corresponding assertions.

| Contract | Independent expectation and permitted normalisation | Observable invariant | Principal executable evidence |
| --- | --- | --- | --- |
| Trailer grammar and metadata retention | The explicit requirements above determine normalised keys, trimmed values and source-order precedence. | A single-character or unknown trailer shall not discard `Remote-Ref`, labels or human text; malformed footer text shall not be consumed. | `footer_semantics_follow_the_independent_source_order`; `empty_or_invalid_footer_lines_preserve_the_entire_human_body`; `a_missing_blank_boundary_cannot_consume_human_prose`; `single_character_tags_preserve_the_published_branch_and_label` |
| Fixture trailer round trips | Effective values are calculated directly from source-order examples, independently of the parser and fixture serialiser. | Fixture serialisation shall preserve every effective key and reach a stable form, including repeated keys. | `duplicate_trailer_winners_survive_rewriting`; `rewriting_preserves_all_effective_trailers` |
| Structured snapshots | Tests construct complete NUL-delimited records and expected fields, dates and linear ancestry directly. | Metadata shall retain record boundaries, author identity, SHA-1 or SHA-256 identities, messages and verified depths; truncation shall fail before a subsequent boundary operation. | `structured_metadata_preserves_independently_framed_records`; `truncated_structured_records_fail_before_any_effect` |
| Literal branch identity | Git determines whether generated names are admissible; application inventory separators carry no authority over names. | A pipe or Unicode character in a valid branch name shall survive discovery, publication and recovery unchanged; the first eligible alias for an identity is retained. | `branch_inventories_preserve_literal_names_and_the_first_alias`; `generated_literal_refs_agree_with_git_and_preserve_discovery_publication_and_recovery`; `literal_ref_acceptance_matches_git_at_reserved_and_punctuation_boundaries`; `pipe_branch_is_synchronised_without_moving_the_checked_out_branch` |
| Default branch discovery | Git supplies the full symbolic target within the selected remote namespace. | A colliding local branch or tag shall not change the trunk; nested default branch names and alternate publication remotes shall retain their exact identities. | `trunk_discovery_ignores_shadowing_local_branches_and_tags`; `nested_default_branches_and_alternate_remotes_preserve_literal_identity`; `alternate_remote_mock_answers_and_publication_follow_the_selected_name`; `in_memory_remote_head_observations_match_real_git_for_each_supported_remote_name` |
| Push destination selection | Git resolves explicit push URLs and URL rewriting; the expected postcondition is the complete ref and object identity at every resolved destination. | A failed or uncertain push shall not be declared successful solely because the fetch destination or one push destination matches. | `observed_publication_refs_require_complete_literal_identity`; `real_push_url_rewriting_and_explicit_push_urls_recover_lost_acknowledgements`; `every_partial_push_destination_prefix_remains_uncertain_until_all_destinations_match` |
| Selection and predecessor bases | Generated stack positions, membership sets, existing exclusion flags and verified ancestry determine expected bounds and predecessors. | Argument order shall not change stack order; exclusions shall remain excluded; filtered commits shall not invalidate verified range boundaries; selected PR bases shall retain the intended predecessor context. | `selected_bounds_follow_stack_order_and_ignore_argument_order`; `selection_never_clears_existing_exclusions`; `filtered_range_boundaries_obey_verified_linear_ancestry`; `bases_use_live_selected_predecessors_and_preserve_context` |
| Ref recovery authority | Identity equality, verified fast-forward ancestry or the same backend change identity determine the permitted action. | An unrelated local ref shall not move merely because publication completed elsewhere. | `ref_recovery_requires_identity_or_authorised_ancestry`; `generated_literal_refs_agree_with_git_and_preserve_discovery_publication_and_recovery` |
| Human description preservation | Independently constructed text before and after the managed section remains authoritative, subject only to the specified fixed-point line-ending normalisation of observed or newly composed output and whitespace adjoining the replaced section. | Managed updates shall retain human sections, use one delimiter pair, preserve the first historical occurrence of each PR identity and avoid a further update at the fixed point. | `managed_updates_preserve_human_sections_and_reach_a_fixed_point`; `unmanaged_human_text_survives_adoption_and_repeated_updates`; `history_retains_first_identity_occurrence_and_its_state`; `malformed_managed_markers_are_rejected_without_rendering_human_text`; `rendered_descriptions_canonicalise_new_line_endings_before_first_write`; `rendering_preserves_bare_carriage_returns_without_line_feed` |
| Dry-run, refused preflight and unrelated local state | Initial local and remote observations constitute the complete expected state when submission is unauthorised or refused, and unrelated worktree state remains outside the authorised submission effects. | Preview or refused preflight shall have no submission effect; unrelated refs, staged index contents, untracked files and linked worktree state shall survive an authorised submission. | `all_three_step_workflow_sequences_preserve_identity_and_human_text`; `nested_default_branches_and_alternate_remotes_preserve_literal_identity`; `refused_dirty_worktrees_preserve_index_unrelated_refs_and_all_remote_effects`; `successful_submission_preserves_unrelated_worktree_index_and_human_files` |
| Retry reconciliation and PR identity | The expected model retains completed effects and the existing PR identities; it does not assume rollback of remote effects. | A retry shall observe the current state, converge on intended heads, bases, labels and content, retain human text and avoid duplicate creation or redundant metadata mutation after convergence; changed remote bases or lifecycle states shall stop unsafe replay. | `before_effect_and_lost_acknowledgement_failures_converge_without_duplicate_prs`; `every_partial_prefix_recovers_without_repeating_completed_writes_or_publication`; `all_three_step_workflow_sequences_preserve_identity_and_human_text`; `remote_base_and_lifecycle_changes_stop_publication_before_any_further_pr_effects` |

## Interactive result presentation

The interactive-result contract preserves every visible PR, complete title and configured orientation, with non-zero outcome counts in the heading.
Every row carries a leading outcome glyph: `+` for created, `~` for updated and `=` for unchanged, including when all outcomes match.
Colour supplements these distinct glyphs; mixed outcomes retain stack order.
Hyperlink mode resolves from the final explicit CLI value, effective Git setting and `auto` default, in that order.
Hyperlinks remain restricted to eligible rich results; redirected and custom output retain their established representations under every mode.
Automatic hyperlinks follow rich-output eligibility independently of terminal identity or connection metadata; explicit `never` retains visible URLs.
The `terminal_results` unit cases, native configuration scope tests and pseudo-terminal progress tests exercise these contracts independently of live PRs.

## Bounded generated dimensions

The core property suite executes seventeen generated properties with 128 cases per property and a fixed default seed of `5138697615354058066`.
`PROPTEST_CASES` and `PROPTEST_RNG_SEED` permit larger, independently seeded campaigns without modifying the suite.
The shrink budget is 4,096 iterations per failure, and Proptest replays retained failure seeds before generating further cases.
Retained seeds reside in [proptest-regressions/tests/unit/properties.txt](../proptest-regressions/tests/unit/properties.txt), alongside explicit deterministic examples which state the required behaviour independently of generator internals.
A seed is reproducible with the recorded dependency version and generator definition; it does not replace the permanently retained behavioural example when either changes.

Trailer generation includes one-character ASCII and Unicode values, punctuation and embedded separators, repeated and unknown keys, variable indentation, key padding, alternate source order, LF and CRLF input, invalid keys and empty values.
Generated snapshot campaigns contain one through nine independently framed records with SHA-1 or SHA-256 identities, Unicode metadata, trailer-bearing messages and fixed offset dates; truncation campaigns remove required fields.
Selection campaigns use stacks of up to 31 commits, duplicate and reordered arguments, independent exclusion decisions, filtered ancestry boundaries and every Boolean combination governing ref synchronisation.
Description campaigns vary Unicode human text, Markdown punctuation, unrelated HTML comments, managed section placement, line endings, repeated history identities and six malformed delimiter arrangements.
These bounds concern represented inputs; they do not imply exhaustive enumeration of their Cartesian product.

## Transient terminal screen contract

The renderer shall distinguish durable output and operational facts from descriptive detail which is eligible for interactive ellipsis.
Descriptions supplied through the generic detail boundary shall receive the same display budget as request destinations, independently of message wording or the producing workflow.
No animation glyph or transient description shall remain on the visible screen after its live region is cleared for completion, failure, durable output, warnings, confirmation or destruction.
Ordinary fallback notices may preserve oversized operational facts, but shall not retain loading glyphs or descriptive detail.
Clearing shall preserve preceding durable output and shall not consume subsequent output.

The terminal-screen suite uses an independent `vt100` screen model rather than treating emitted erase sequences as evidence that erasure succeeded.
It obtains detail from real application workflows running against in-memory command and HTTP fixtures, including native-stack creation and unchanged reconciliation.
The deterministic matrix uses 40, 80 and 124 columns with colour enabled and disabled; a native pseudo-terminal matrix additionally replays actual Console bytes and newline processing at those widths.
The generated screen property ordinarily executes 128 cases with independently varied initial and resized widths from 40 through 179 columns, zero through 149 Unicode or control-character tokens, colour, completion outcome and clearing boundary.
It honours `PROPTEST_CASES` and `PROPTEST_RNG_SEED`, and retains minimised failure seeds under `proptest-regressions/tests/terminal_screen.txt`.
The screen model covers cursor movement, line erasure, wrapping and its defined resize semantics; graphical-terminal-specific reflow and joined-emoji shaping remain outside that model.
Separate grapheme-width assertions retain joined-emoji coverage without attributing graphical shaping support to the emulator.
Sensitivity experiments must reject the restored unbounded-detail decision and a restored permanent loading glyph through executed screen assertions.

## Bounded state transitions and failure positions

The workflow model enumerates all 125 ordered sequences of three actions drawn from preview, unchanged submission, tip amendment, tip selection and human editing, following an initial three-commit submission.
Each sequence checks the retained PR count, identities, expected predecessor bases and human notes, followed by a final reconciliation of publication heads and content.
Preview additionally checks unchanged commits, PRs and remote refs and the absence of traced submission effects.
The model uses isolated in-memory command and HTTP boundaries; native worktree and index preservation require the separate disposable-repository assertions.

The fault campaign exercises discovery, rewriting, push, PR creation, content reconciliation, labels and native topology boundaries.
For each represented boundary it injects both a failure before the operation and a lost acknowledgement after the fixture has completed the operation.
PR creation and label operations are targeted at each of three positions, and the remaining categories at their first matching operation.
These dimensions form 22 fault configurations, supplemented by the existing partial content-batch tests and separate native push-destination prefix tests.
The fixture records which fault was reached, retains completed effects and requires subsequent convergence, followed by a repetition without duplicate PR creation or unnecessary metadata mutation.
Remote base changes and closed or merged lifecycle states are separately injected before or after publication observation for each of three PR identities, forming eighteen refusal configurations.
Base changes require refusal before a further PR mutation; the campaign does not assume that the application is authorised to retarget an externally changed base.
The campaign does not enumerate every interleaving of concurrent external actors, every network retry schedule or every backend operation position.

## Interpretation of results

Line execution, generated cases, native observations, transition enumeration, mutation outcomes and fuzzing budgets measure different aspects of the evidence and shall be reported separately.
The independent footer expectations detected an additional defect whereby rewriting reversed duplicate-key precedence, despite the pre-existing high line coverage.
The retained regression and generated property both failed before the correction and passed afterwards.
Semantic fuzz replay independently detected a second defect class in which newly composed description output manufactured CRLF and consequently changed again on repetition.
A further two-carriage-return counterexample established that one replacement pass was insufficient where overlapping CRLF sequences remained.
Retained byte inputs and deterministic regressions distinguish fixed-point line-ending canonicalisation from preservation of unrelated carriage returns.
A larger local campaign using seed `20261015` passed 4,096 cases for each of the seventeen generated properties, corresponding to 69,632 newly generated cases in addition to corpus replay.
That result establishes the stated finite campaign only; changes to contracts, generators, dependencies or production behaviour require renewed execution.
External hosting behaviour, undocumented backend changes and scenarios outside the declared bounds remain explicit limitations of this assurance argument.
