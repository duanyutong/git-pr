// git-pr submits the stack with each commit becomes a GitHub PR. It detects "Remote-Ref: <remote-branch>" from the
// commit message to know which remote branch to push to. It will attempt to create new "Remote-Ref" if not found.
//
// Usage: git pr -config=/path/to/config.json
package main

import (
	"fmt"
	"iter"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	KeyTags      = "tags"
	KeyRemoteRef = "remote-ref"
)

// Sentinel markers for identifying and replacing the git-pr stack info section
// in PR descriptions. These HTML comments are invisible in rendered markdown
// but allow us to reliably detect and replace our section even when other
// tools modify the PR description.
const (
	stackInfoStartMarker = "<!-- git-pr-stack-start -->"
	stackInfoEndMarker   = "<!-- git-pr-stack-end -->"
)

const bodyTemplate = `
# Summary





<br><br><br><br>
`

func main() {
	config = LoadConfig()

	// ensure no uncommitted changes
	if !validateGitStatusClean() {
		exitf(`ERROR: git status reports uncommitted changes

Hint: use "git add -A" and "git stash" to clean up the repository
`)
	}

	// checkpoint: validate
	if config.stopAfter == "validate" {
		printf("stopped after: validate\n")
		return
	}

	originMain := fmt.Sprintf("%v/%v", config.git.remote, config.git.remoteTrunk)
	originMainHash := must(resolveRef(originMain))

	// resolve the run's tip (top of the visible stack) and the user-selected
	// base (the lower exclusive endpoint of what we actually push).
	//
	// - no positional args: selectedBase == originMain, fullTip == HEAD/@-.
	// - range args:         selectedBase, fullTip come from config.commitRange.
	//
	// fullStack runs origin/<trunk>..fullTip and is used for stack-info
	// rendering and for resolving the bottom PR's base. stackedCommits is
	// what the rest of the pipeline operates on (validate → rewrite → push →
	// PR create/update).
	// multi-commit selection: the user named 2+ individual commits (possibly
	// non-contiguous, in any order). Resolve them against the visible stack into
	// a [lowest..highest] span and drive the rest of the pipeline through the
	// range-mode machinery; the in-span commits the user did NOT name are marked
	// Skip below so each PR links onto its nearest selected predecessor.
	var multiSelectedDepths []int       // depths-from-head of selected commits (multi mode, non-jj), for post-rewrite recovery
	var multiSelectedChangeIDs []string // jj change-ids of selected commits (multi mode, jj), for post-rewrite recovery
	if config.commitRange.IsMulti() {
		head := resolveStackHead()
		visible := must(getStackedCommits(originMain, head, false))
		lo, hi, err := orderSelection(visible, config.commitRange.Selected)
		if err != nil {
			exitf("ERROR: %v", err)
		}
		config.commitRange.TipRef = visible[hi].Hash
		if lo == 0 {
			config.commitRange.BaseRef = originMainHash
		} else {
			config.commitRange.BaseRef = visible[lo-1].Hash
		}
		// capture a stable identity per selected commit for post-rewrite recovery:
		// jj change-ids survive `jj describe`, so prefer them in jj mode; otherwise
		// fall back to depth-from-HEAD (git-branchless reword moves HEAD to track the
		// rewritten descendants, so depth is stable there).
		for _, h := range config.commitRange.Selected {
			if config.jj.enabled {
				multiSelectedChangeIDs = append(multiSelectedChangeIDs, must(jjGetChangeID(h)))
			} else {
				multiSelectedDepths = append(multiSelectedDepths, mustCommitCount(h, head))
			}
		}
	}

	// Capture the user's starting branch before any rewords so the run can
	// return to the invocation point.
	originalBranch, _ := git("branch", "--show-current")
	originalBranch = strings.TrimSpace(originalBranch)

	var selectedBase, fullTip string
	var rangeBaseDepth, rangeTipDepth int // depths from HEAD (non-jj), used to recover after rewrite
	var rangeTipChangeID string           // jj change-id of the selected tip (jj), used to recover after rewrite
	if config.commitRange.HasArg {
		selectedBase = config.commitRange.BaseRef
		fullTip = config.commitRange.TipRef
		if config.jj.enabled {
			// jj change-ids are stable across `jj describe`; recover the tip by
			// change-id so an explicit BASE..TIP / COMMIT... selection is honored
			// even when jj's @- points at a different sibling stack. selectedBase
			// needs no recovery: BASE is exclusive and below the reworded commits,
			// so it is never reworded and its hash does not drift.
			rangeTipChangeID = must(jjGetChangeID(fullTip))
		} else {
			preRewriteHead := resolveStackHead()
			rangeBaseDepth = mustCommitCount(selectedBase, preRewriteHead)
			rangeTipDepth = mustCommitCount(fullTip, preRewriteHead)
		}
	} else {
		selectedBase = originMain
		// HEAD is the submission ceiling for an implicit run. Descendants may
		// still be included later as description-only context.
		fullTip = resolveStackHead()
	}

	fullStack := must(getStackedCommits(originMain, fullTip, !config.commitRange.HasArg))
	if len(fullStack) == 0 {
		exitf("no commits to submit")
	}

	stackedCommits := fullStack
	if config.commitRange.HasArg {
		stackedCommits = sliceFromBase(fullStack, selectedBase, originMainHash)
		if len(stackedCommits) == 0 {
			exitf("no commits to submit in range %v..%v (base %v not found on trunk..tip path)",
				config.commitRange.BaseRef[:8], config.commitRange.TipRef[:8], config.commitRange.BaseRef[:8])
		}
	}
	// multi-commit selection: mark in-span commits the user did NOT name as Skip.
	// the selected hashes are valid pre-rewrite; they are recovered by depth after
	// the rewrite phase (see below) since rewriting changes hashes.
	if config.commitRange.IsMulti() {
		markUnselected(stackedCommits, sliceToSet(config.commitRange.Selected), true)
	}
	for _, commit := range stackedCommits {
		printf("%s\n", commit)
	}
	printf("\n")

	// filter draft commits based on configuration
	if shouldSkipDrafts() {
		for _, commit := range stackedCommits {
			if commit.Skip {
				continue // already skipped for other reasons
			}
			if matchAnyPattern(config.draftPatterns, commit.Title) {
				commit.Skip = true
				debugf("skipping draft commit %s: %s", commit.ShortHash(), shortenTitle(commit.Title))
				printf("skip draft \"%v\" (%v)\n", shortenTitle(commit.Title), commit.ShortHash())
			}
		}
	}

	// checkpoint: get-commits
	if config.stopAfter == "get-commits" {
		printf("stopped after: get-commits\n")
		return
	}

	// validate no duplicated remote ref
	mapRefs := map[string]*Commit{}
	for _, commit := range stackedCommits {
		remoteRef := commit.GetRemoteRef()
		if remoteRef == "" {
			continue
		}
		if last, ok := mapRefs[remoteRef]; ok {
			printf("[ERROR] DUPLICATED REMOTE REF: %q\n", remoteRef)
			printf("  Commit 1: %v - %v\n", last.ShortHash(), last.Title)
			printf("  Commit 2: %v - %v\n", commit.ShortHash(), commit.Title)
			printf("  This means a previous run didn't complete properly.\n")
			printf("  Delete the Remote-Ref from one of these commits and try again.\n")
			exitf("duplicated remote ref %q found for %q and %q", last.GetRemoteRef(), last.ShortHash(), commit.ShortHash())
		}
		mapRefs[remoteRef] = commit
	}

	// warn about merged PRs still present in the local stack — these usually
	// indicate the user hasn't rebased onto remote trunk after the PR landed,
	// and trying to push/update them downstream rarely does what you want.
	warnMergedPRsInStack(stackedCommits)

	// fill remote ref for each commit
	// For each commit without a remote-ref, find the local branch it's on
	// IMPORTANT: Collect all branch mappings FIRST, before any rewords.
	// After rewordCommit(), all hashes change and branches point to new commits.
	branchForCommit := map[string]string{} // commit hash -> branch name
	for _, commit := range stackedCommits {
		if commit.Skip || commit.GetRemoteRef() != "" {
			continue
		}
		localBranch, err := getLocalBranchForCommit(commit)
		if err != nil {
			exitf("failed to find local branch for commit %v: %v", commit.ShortHash(), err)
		}
		if localBranch == "" {
			printf("❌ ERROR: commit %v is not on any local branch\n", commit.ShortHash())
			printf("   Title: %v\n", commit.Title)
			printf("   Expected git-branchless to create a branch for this commit.\n")
			exitf("commit %v is not on any local branch", commit.ShortHash())
		}
		branchForCommit[commit.Hash] = localBranch
	}

	// Now apply the mappings (reword in reverse order: HEAD first)
	for i := len(stackedCommits) - 1; i >= 0; i-- {
		commit := stackedCommits[i]
		if commit.Skip || commit.GetRemoteRef() != "" {
			continue
		}

		remoteRef := branchForCommit[commit.Hash]
		commit.SetAttr(KeyRemoteRef, remoteRef)
		must(rewordCommit(commit, commit.FullMessage()))

		time.Sleep(time.Millisecond)
	}
	// rewords change commit_ids; in jj-workspace mode the captured `head` no
	// longer maps to the rewritten chain, so re-resolve before the re-read.
	// In range mode the user's baseRef/tipRef hashes also drift because the
	// rewritten chain has new hashes; recover the tip from its stable identity
	// captured pre-rewrite — jj change-id in jj mode (which survives `jj describe`
	// and stays correct even when @- is a different sibling stack), depth-from-HEAD
	// otherwise (git-branchless reword moves HEAD, so depth is stable there).
	postRewriteHead := resolveStackHead()
	depthResolver := func(head string, depth int) (string, error) {
		return resolveRef(fmt.Sprintf("%v~%v", head, depth))
	}
	if config.commitRange.HasArg {
		fullTip = must(recoverRangeTip(config.jj.enabled, rangeTipChangeID, postRewriteHead, rangeTipDepth, resolveJJRevision, depthResolver))
		if config.jj.enabled {
			// BASE is exclusive and never reworded, so its pre-rewrite hash is stable.
			selectedBase = config.commitRange.BaseRef
		} else {
			selectedBase = must(depthResolver(postRewriteHead, rangeBaseDepth))
		}
	} else {
		fullTip = postRewriteHead
	}
	fullStack = must(getStackedCommits(originMain, fullTip, !config.commitRange.HasArg))
	stackedCommits = fullStack
	if config.commitRange.HasArg {
		stackedCommits = sliceFromBase(fullStack, selectedBase, originMainHash)
	}
	// multi mode: recover the selected set and re-mark in-span skips, since the
	// rewrite phase gave every commit a new hash. recover from the same stable
	// identities captured pre-rewrite — jj change-ids in jj mode, depth-from-HEAD
	// otherwise (mirrors the tip recovery above).
	if config.commitRange.IsMulti() {
		selected := map[string]bool{}
		if config.jj.enabled {
			for _, id := range multiSelectedChangeIDs {
				selected[must(resolveJJRevision(id))] = true
			}
		} else {
			for _, d := range multiSelectedDepths {
				selected[must(depthResolver(postRewriteHead, d))] = true
			}
		}
		markUnselected(stackedCommits, selected, false)
	}

	// checkpoint: rewrite
	if config.stopAfter == "rewrite" {
		printf("stopped after: rewrite\n")
		return
	}

	// resolveBase determines the PR base branch for a given selected commit:
	//   1. the predecessor's Remote-Ref if a non-skipped predecessor exists
	//      in stackedCommits (normal stack-linkage case);
	//   2. otherwise walk fullStack to find the commit just before this one
	//      and use that commit's Remote-Ref if present (range-mode
	//      bottom-of-selection: anchor onto an existing PR);
	//   3. otherwise the remote trunk.
	resolveBase := func(commit *Commit) string {
		if prev := findPrevNonSkipped(stackedCommits, commit); prev != nil {
			return prev.GetRemoteRef()
		}
		return resolveBaseForBottom(fullStack, commit, config.git.remoteTrunk)
	}

	pushCommit := func(commit *Commit) (logs string, execFunc func() bool) {
		remoteRef := commit.GetAttr(KeyRemoteRef)
		if remoteRef == "" {
			exitf(`commit %v has no Remote-Ref after rewrite — refusing to push to empty branch

Title: %v
Hint: this usually means the local branch lookup or the reword silently dropped the
Remote-Ref trailer. Rerun with -verbose to see the branch lookup output, or add a
"Remote-Ref: <branch>" trailer to this commit's message manually.`, commit.ShortHash(), commit.Title)
		}
		if err := validateRemoteRef(remoteRef); err != nil {
			exitf(`commit %v has invalid Remote-Ref %q — refusing to push

Title: %v
Cause: %v
Hint: remove the bad Remote-Ref trailer from this commit and rerun git-pr, or
replace it with the intended local branch name.`, commit.ShortHash(), remoteRef, commit.Title, err)
		}
		args := fmt.Sprintf("%v:refs/heads/%v", commit.ShortHash(), remoteRef)
		logs = fmt.Sprintf("push -f %v %v", config.git.remote, args)
		if config.dryRun {
			logs = "[DRY-RUN] " + logs
			return logs, func() bool { return false } // no-op for dry-run
		}
		return logs, func() bool {
			out := must(git("push", "-f", config.git.remote, args))
			time.Sleep(1 * time.Second)
			needsPR := strings.Contains(out, "remote: Create a pull request")
			// Don't do any PR operations here - handle all PR creation/updates
			// sequentially after all pushes complete to ensure correct PR numbering
			return needsPR
		}
	}
	// mark commits we won't push (other authors, unless --include-other-authors)
	for _, commit := range stackedCommits {
		shouldPush := isMyOwnCommit(commit) || config.includeOtherAuthors
		if !shouldPush {
			commit.Skip = true
			author := coalesce(commit.AuthorEmail, "@unknown")
			printf("skip \"%v\" (%v)\n", shortenTitle(commit.Title), author)
		}
	}

	// fail loudly if any pushable commit slipped through the rewrite phase
	// without a Remote-Ref. without this guard, git rejects the empty refspec
	// inside a goroutine and the panic stack does not name the offending commit.
	if missing := validateRemoteRefsBeforePush(stackedCommits); len(missing) > 0 {
		exitf(`ERROR: %d commit(s) have no Remote-Ref after rewrite phase: %v

This usually means the in-memory view of the stack drifted from what jj/git
actually wrote. Re-run git-pr; if it recurs, file an issue with the output of
"git log -10" and the contents of .jj/repo/op_heads (if jj is in use).`,
			len(missing), strings.Join(missing, ", "))
	}

	// push commits, concurrently
	if config.dryRun {
		printf("[DRY-RUN] Would push commits:\n")
	}

	// Track which commits need PRs created (in order)
	type pushResult struct {
		commit  *Commit
		exec    func() bool
		needsPR bool
	}
	var pushResults []*pushResult
	for _, commit := range stackedCommits {
		if commit.Skip {
			continue
		}
		logs, execFn := pushCommit(commit)
		printf("%s\n", logs)
		if !config.dryRun {
			pushResults = append(pushResults, &pushResult{commit: commit, exec: execFn})
		}
	}
	parallelForEach(pushResults, func(result *pushResult) {
		result.needsPR = result.exec()
	})

	// Handle PRs: look up existing PRs in parallel, create missing ones serially, update bases in parallel
	if !config.dryRun {
		// Phase 1: Look up existing PR numbers in parallel for commits that weren't new pushes
		existingBranches := 0
		for _, result := range pushResults {
			if !result.needsPR {
				existingBranches++
			}
		}
		if existingBranches > 0 {
			printf("\nLooking up existing PRs...\n")
			var wg sync.WaitGroup
			for _, result := range pushResults {
				if result.needsPR {
					continue // new branch, will create PR
				}
				wg.Add(1)
				commit := result.commit
				go func() {
					defer wg.Done()
					prNumber, _ := githubFindPRNumberForCommit(commit)
					commit.PRNumber = prNumber
				}()
			}
			wg.Wait()
		}

		// Phase 2: Create PRs serially for commits that need them (in stack order)
		needsCreate := 0
		for _, result := range pushResults {
			if result.needsPR || result.commit.PRNumber == 0 {
				needsCreate++
			}
		}
		if needsCreate > 0 {
			printf("\nCreating %d PR(s)...\n", needsCreate)
			for _, result := range pushResults {
				commit := result.commit
				if result.needsPR || commit.PRNumber == 0 {
					// New branch or existing branch without PR - create PR
					printf("  %s\n", shortenTitle(commit.Title))
					mustE(githubCreatePRForCommit(commit, resolveBase(commit)))
				}
			}
		}

		// Phase 3: Update PR bases in parallel for existing PRs
		needsUpdate := 0
		for _, result := range pushResults {
			if !result.commit.NewlyCreated {
				needsUpdate++
			}
		}
		if needsUpdate > 0 {
			printf("Updating PR(s)...\n")
			var wg sync.WaitGroup
			for _, result := range pushResults {
				commit := result.commit
				if commit.NewlyCreated {
					continue // just created, base is already correct
				}
				wg.Add(1)
				go func() {
					defer wg.Done()
					err := githubPRUpdateBaseForCommit(commit, resolveBase(commit))
					if err == nil {
						commit.BaseUpdated = true
					} else {
						mustE(err)
					}
				}()
			}
			wg.Wait()
		}
		if config.noStack {
			warnStaleBlockedBases(stackedCommits)
		}
	}

	// checkpoint: push
	if config.stopAfter == "push" {
		printf("stopped after: push\n")
		return
	}

	// Return the user to where they started. With the stack-tip expansion above,
	// we may have rewritten descendant commits the user wasn't even on; checking
	// out the tip would silently move them. If we captured a starting branch,
	// land them back on it (branchless follows rewrites, so the branch points
	// at the rewritten commit). Otherwise, fall back to the tip.
	if !config.dryRun {
		switch {
		case config.jj.enabled:
			debugf("skipping git checkout in jj repo (jj manages working copy)")
		case config.commitRange.HasArg:
			// the user asked for a specific range; leave HEAD where it was so
			// we don't silently move HEAD below the original tip when the
			// selected tip is not at HEAD.
			debugf("skipping git checkout in range mode (preserving HEAD)")
		case originalBranch != "":
			must(git("checkout", originalBranch))
		default:
			must(git("checkout", stackedCommits[len(stackedCommits)-1].Hash))
		}
	}

	// wait for GitHub API to propagate before updating PR descriptions
	if !config.dryRun {
		printf("Waiting for GitHub to sync...\n")
		time.Sleep(3 * time.Second)
	}

	// update commits with PR numbers, concurrently
	if config.dryRun {
		printf("[DRY-RUN] Would update PR descriptions for:\n")
		for _, commit := range stackedCommits {
			if !commit.Skip {
				printf("  - %s: %s\n", commit.ShortHash(), commit.Title)
			}
		}
		return
	}
	var needsPRNumber []*Commit
	for _, commit := range stackedCommits {
		if commit.PRNumber == 0 {
			needsPRNumber = append(needsPRNumber, commit)
		}
	}
	parallelForEach(needsPRNumber, func(commit *Commit) {
		commit.PRNumber = must(githubGetPRNumberForCommit(commit, resolveBase(commit)))
	})

	if config.commitRange.HasArg {
		// in range mode, look up PR numbers for fullStack commits below the
		// selected range so stack-info bullets render existing PR links instead
		// of falling back to "no PR yet" formatting. lookup-only — never
		// creates a PR for a commit outside the selected range.
		var ancillary []*Commit
		for _, cm := range fullStack {
			if cm.PRNumber == 0 && cm.GetRemoteRef() != "" {
				ancillary = append(ancillary, cm)
			}
		}
		parallelForEach(ancillary, func(commit *Commit) {
			commit.PRNumber = githubLookupPRNumber(commit)
		})
	}

	// checkpoint: pr-create
	if config.stopAfter == "pr-create" {
		printf("stopped after: pr-create\n")
		return
	}

	// Print results in stack order
	printf("\n")
	stackCount := 0
	for _, commit := range stackedCommits {
		if !commit.Skip {
			stackCount++
		}
	}
	orderHint := "oldest at the top"
	if config.reverse {
		orderHint = "newest at the top"
	}
	printf("Stack of %d (%s):\n\n", stackCount, orderHint)
	printOrder := stackedCommits
	if config.reverse {
		printOrder = make([]*Commit, len(stackedCommits))
		for i, c := range stackedCommits {
			printOrder[len(stackedCommits)-1-i] = c
		}
	}
	first := true
	for _, commit := range printOrder {
		if commit.Skip {
			continue
		}
		if !first {
			printf("\n")
		}
		first = false
		prURL := fmt.Sprintf("https://%v/%v/pull/%v", config.git.host, config.git.repo, commit.PRNumber)
		status := ""
		if commit.NewlyCreated {
			status = " (created)"
		} else if commit.BaseUpdated {
			status = " (updated)"
		}
		printf("%s\n", commit.Title)
		printf("%s%s\n", prURL, status)
	}

	descriptionStack := fullStack
	if !config.commitRange.HasArg {
		descendants := fetchDescendantCommits(fullStack, originMain, fullTip)
		descriptionStack = append(slices.Clone(fullStack), descendants...)
	}

	// Collect PR body update targets. Results were already printed above in the
	// user's configured stack order, so this phase must not emit a second list.
	var prBodyTargets []*Commit
	for _, commit := range stackedCommits {
		if commit.Skip {
			continue
		}
		prBodyTargets = append(prBodyTargets, commit)
	}

	currentStackSet := make(map[int]bool)
	for _, commit := range descriptionStack {
		if commit.PRNumber != 0 {
			currentStackSet[commit.PRNumber] = true
		}
	}
	var allHistoricalPRs []PRHistoryEntry
	prHistoryMap := make(map[int]bool)
	mergedPRs := make(map[int]bool)
	for _, commit := range descriptionStack {
		if commit.PRNumber == 0 {
			continue
		}
		pr, err := githubGetPRByNumber(commit.PRNumber)
		if err != nil || pr == nil {
			continue
		}
		if pr.Merged {
			mergedPRs[commit.PRNumber] = true
		}
		for _, entry := range extractPRHistoryFromStackInfo(pr.Body) {
			if prHistoryMap[entry.Number] {
				continue
			}
			prHistoryMap[entry.Number] = true
			if !currentStackSet[entry.Number] {
				entry.IsMerged = true
			}
			allHistoricalPRs = append(allHistoricalPRs, entry)
		}
	}
	parallelForEach(prBodyTargets, func(commit *Commit) {
		pr := must(githubGetPRByNumber(commit.PRNumber))
		pullURL := ghAPIURL("pulls/%v", commit.PRNumber)

		// generate the PR body with stack info — render the full chain from
		// trunk up to the selected tip, so PR bodies of selected commits show
		// their position in the broader stack (not just the selected range).
		stackInfo := generateStackInfo(descriptionStack, commit, allHistoricalPRs, mergedPRs)
		body := generatePRBody(commit, pr.Body, stackInfo)

		// update the PR
		must(httpRequest("PATCH", pullURL, map[string]any{
			"title": commit.Title,
			"body":  body,
		}))
		// Draft status is set only when a PR is created; preserve the user's
		// subsequent draft/ready choice when updating an existing PR.
		if tags := commit.GetTags(config.tags...); len(tags) > 0 {
			must(gh("pr", "edit", strconv.Itoa(commit.PRNumber), "--add-label", strings.Join(tags, ",")))
		}
	})

	// create or update the native GitHub stack so the pushed PRs form one stack
	if !config.noStack {
		manageNativeStack(stackedCommits)
	}
}

// manageNativeStack creates or updates the native GitHub stack so the pushed
// PRs (in `commits`, bottom→top) form a single stack. It prompts before doing
// anything; --yes auto-accepts and --no-stack skips this entirely (handled by
// the caller). gh-stack's `link` creates a stack when none exists and updates an
// existing one (correcting bases), but it never drops a PR — so if the local
// stack no longer contains a PR that is still in the native stack, `link` fails
// and we fall back to a dissolve+relink rebuild (after a second confirmation).
func manageNativeStack(commits []*Commit) {
	var branches []string
	for _, commit := range commits {
		if !commit.Skip {
			branches = append(branches, commit.GetRemoteRef())
		}
	}
	if len(branches) < 2 { // a single PR is not a stack
		return
	}
	if !confirm(fmt.Sprintf("Create/update the GitHub stack for %d PRs (%s)?",
		len(branches), strings.Join(branches, " "))) {
		printf("skipped native stack (base branches only)\n")
		warnStaleBlockedBases(commits)
		return
	}
	out, err := gh(append([]string{"stack", "link"}, branches...)...)
	switch {
	case err == nil:
		printf("native stack updated: %s\n", strings.Join(branches, " "))
	case isGhStackMissing(out, err):
		warnf("gh-stack extension not installed; skipping native stack.\n" +
			"  install: gh extension install github/gh-stack (or pass --no-stack)")
		warnStaleBlockedBases(commits)
	case isStackWouldRemove(out, err):
		stackNumber := githubStackNumberForCommits(commits)
		if stackNumber == 0 {
			exitf("ERROR: updating the native stack needs to drop a PR, but git-pr could not\n" +
				"find the stack number. Fix it manually with `gh stack modify`.")
		}
		warnf("updating the stack would drop PR(s) no longer in your local stack (stack #%d).\n"+
			"git-pr will dissolve and rebuild it as: %s", stackNumber, strings.Join(branches, " "))
		if confirm("Rebuild the GitHub stack now?") {
			if err := githubStackRealign(stackNumber, branches); err != nil {
				exitf("ERROR: failed to rebuild GitHub stack: %v", err)
			}
		} else {
			warnf("skipped; branches were pushed but the stack is unchanged.\n"+
				"To drop the PR, run `gh stack modify` (interactive, keeps the rest of the stack),\n"+
				"or rebuild the whole stack with: gh stack unstack %d && gh stack link %s",
				stackNumber, strings.Join(branches, " "))
		}
	default:
		exitf("ERROR: gh stack link failed: %v\n%s", err, out)
	}
}

// fetchDescendantCommits returns commits that exist above HEAD in the local DAG
// but were not submitted in this run (i.e. the user ran git-pr from a middle
// branch). Their PR numbers are looked up concurrently so the caller can
// include them in the stack description without pushing them.
func fetchDescendantCommits(stackedCommits []*Commit, originMain string, head string) []*Commit {
	tip := resolveStackTip(head)
	if tip == head {
		return nil // HEAD is already the tip; no descendants
	}
	fullStack, err := getStackedCommits(originMain, tip, false)
	if err != nil {
		debugf("warning: failed to get full stack for descendant lookup: %v", err)
		return nil
	}

	submittedHashes := make(map[string]bool, len(stackedCommits))
	for _, cm := range stackedCommits {
		submittedHashes[cm.Hash] = true
	}

	var descs []*Commit
	for _, cm := range fullStack {
		if !submittedHashes[cm.Hash] {
			descs = append(descs, cm)
		}
	}
	if len(descs) == 0 {
		return nil
	}

	var wg sync.WaitGroup
	for _, cm := range descs {
		if cm.GetRemoteRef() == "" {
			continue
		}
		wg.Add(1)
		go func() {
			defer wg.Done()
			prNum, err := githubFindPRNumberForHeadRef(cm.GetRemoteRef(), "open")
			if err == nil && prNum != 0 {
				cm.PRNumber = prNum
			}
		}()
	}
	wg.Wait()
	return descs
}

// warnMergedPRsInStack looks up the PR state for each commit that already has
// a Remote-Ref and prints a warning if any of them are merged. Merged PRs in
// the local stack typically mean the user hasn't synced with remote trunk, and
// continuing the push will recreate deleted branches and confuse later steps.
// We warn but don't exit — the user may have a reason to keep pushing.
func warnMergedPRsInStack(stackedCommits []*Commit) {
	if config.dryRun {
		return
	}

	type result struct {
		commit *Commit
		prNum  int
		merged bool
	}
	results := make([]result, len(stackedCommits))

	var wg sync.WaitGroup
	for i, commit := range stackedCommits {
		if commit.Skip || commit.GetRemoteRef() == "" {
			continue
		}
		wg.Add(1)
		go func() {
			defer wg.Done()
			prNum, err := githubFindPRNumberForHeadRef(commit.GetRemoteRef(), "all")
			if err != nil || prNum == 0 {
				return
			}
			pr, err := githubGetPRByNumber(prNum)
			if err != nil || pr == nil {
				return
			}
			results[i] = result{commit: commit, prNum: prNum, merged: pr.Merged}
		}()
	}
	wg.Wait()

	hasMerged := false
	for _, r := range results {
		if !r.merged {
			continue
		}
		if !hasMerged {
			printf("⚠️  merged PR(s) found in your local stack:\n")
			hasMerged = true
		}
		printf("  #%v (%v) %v\n", r.prNum, r.commit.ShortHash(), shortenTitle(r.commit.Title))
	}
	if hasMerged {
		printf("  Hint: run \"git fetch %v && git rebase %v/%v\" to drop merged commits from your local stack.\n\n",
			config.git.remote, config.git.remote, config.git.remoteTrunk)
	}
}

func findCommitsWithoutRemoteRef(commits []*Commit) iter.Seq[*Commit] {
	commits = slices.Clone(commits)
	slices.Reverse(commits)
	return func(yield func(*Commit) bool) {
		for _, commit := range commits {
			if commit.Skip {
				continue
			}
			if commit.GetRemoteRef() == "" {
				yield(commit)
			}
		}
	}
}

// findPrevNonSkipped returns the most-recent non-skipped commit strictly before
// target in commits (ordered oldest→newest). Returns nil if no eligible
// predecessor exists, including the case where target is not in commits.
func findPrevNonSkipped(commits []*Commit, target *Commit) (prev *Commit) {
	for _, cm := range commits {
		if cm == target {
			return prev
		}
		if cm.Skip {
			continue
		}
		prev = cm
	}
	return nil
}

// sliceFromBase returns the suffix of fullStack strictly after the commit with
// hash == baseHash. When baseHash equals originMainHash, the entire fullStack
// is returned (origin/<trunk> is exclusive and thus never appears in
// fullStack). Returns nil if baseHash is neither originMainHash nor present in
// fullStack — that case signals "user-supplied base is not on the trunk..tip
// path".
func sliceFromBase(fullStack []*Commit, baseHash, originMainHash string) []*Commit {
	if baseHash == originMainHash {
		return fullStack
	}
	for i, cm := range fullStack {
		if cm.Hash == baseHash {
			return fullStack[i+1:]
		}
	}
	return nil
}

// orderSelection locates each selected commit hash within stack (ordered
// oldest→newest) and returns the indices of the lowest (lo) and highest (hi)
// selected commits — the span the multi-commit form operates on. Selected may be
// given in any order; lo/hi reflect actual stack position. It errors if any
// selected hash is absent from the stack (e.g. not an ancestor of the tip).
func orderSelection(stack []*Commit, selected []string) (lo, hi int, err error) {
	pos := make(map[string]int, len(stack))
	for i, cm := range stack {
		pos[cm.Hash] = i
	}
	lo, hi = -1, -1
	for _, h := range selected {
		i, ok := pos[h]
		if !ok {
			return 0, 0, errorf("commit %v is not in the current stack", h[:min(8, len(h))])
		}
		if lo == -1 || i < lo {
			lo = i
		}
		if hi == -1 || i > hi {
			hi = i
		}
	}
	if lo == -1 {
		return 0, 0, errorf("no commits selected")
	}
	return lo, hi, nil
}

// sliceToSet returns a set (map to true) of the given hashes.
func sliceToSet(hashes []string) map[string]bool {
	set := make(map[string]bool, len(hashes))
	for _, h := range hashes {
		set[h] = true
	}
	return set
}

// markUnselected sets Skip on every commit whose hash is not in selected. Used
// by the multi-commit form to exclude in-span commits the user did not name
// while leaving the existing skip/base-linkage machinery to do the rest. announce
// prints a line per newly-skipped commit (true for the pre-rewrite pass, false
// for the post-rewrite re-mark which would otherwise duplicate the output).
func markUnselected(commits []*Commit, selected map[string]bool, announce bool) {
	for _, cm := range commits {
		if cm.Skip {
			continue
		}
		if !selected[cm.Hash] {
			cm.Skip = true
			if announce {
				printf("skip \"%v\" (%v) not selected\n", shortenTitle(cm.Title), cm.ShortHash())
			}
		}
	}
}

// resolveBaseForBottom returns the branch the bottommost selected PR should
// base on. It walks fullStack to find the commit immediately before bottom and
// returns that commit's Remote-Ref if it has one (i.e., the parent is already a
// PR), otherwise the remote trunk. Used to preserve stack relationships when
// pushing a partial range over an existing stack.
func resolveBaseForBottom(fullStack []*Commit, bottom *Commit, trunk string) string {
	var prev *Commit
	for _, cm := range fullStack {
		if cm.Hash == bottom.Hash {
			break
		}
		prev = cm
	}
	if ref := prev.GetRemoteRef(); ref != "" {
		return ref
	}
	return trunk
}

// blockedPRNumbers renders the PR numbers of commits whose base change was
// blocked by a native stack, e.g. "#123, #456".
func blockedPRNumbers(commits []*Commit) string {
	var parts []string
	for _, commit := range commits {
		if commit.PRNumber != 0 {
			parts = append(parts, fmt.Sprintf("#%d", commit.PRNumber))
		}
	}
	if len(parts) == 0 {
		return "(unknown)"
	}
	return strings.Join(parts, ", ")
}

// warnStaleBlockedBases warns when a base edit was blocked (the PR is already in
// a native stack) and the stack was not rebuilt to fix it — so the user knows
// the reparented PR still points at its old base. No-op when nothing was blocked.
func warnStaleBlockedBases(commits []*Commit) {
	var blocked []*Commit
	for _, commit := range commits {
		if commit.BaseBlocked && !commit.Skip {
			blocked = append(blocked, commit)
		}
	}
	if len(blocked) == 0 {
		return
	}
	warnf("PR(s) %s are in a GitHub native stack and could not be retargeted; their base is stale.\n"+
		"Fix with `gh stack modify`, or let git-pr rebuild the stack (accept the prompt / drop --no-stack).",
		blockedPRNumbers(blocked))
}

// parallelForEach runs fn concurrently for each item, waiting for all goroutines
// to finish before returning. An empty slice is a no-op.
func parallelForEach[T any](items []T, fn func(T)) {
	var wg sync.WaitGroup
	for _, item := range items {
		wg.Add(1)
		go func() {
			defer wg.Done()
			fn(item)
		}()
	}
	wg.Wait()
}

// resolveStackHead returns the commit_id git-log should walk from when computing
// the stack. In jj mode it queries jj's @- each time so the value reflects any
// rewords/rebases performed since the previous call — in jj-workspace setups the
// shared backing .git/HEAD does not follow the workspace, and the @- commit_id
// captured before a reword pass becomes stale once jj rewrites the chain. In
// plain git / git-branchless mode "HEAD" is sufficient because reword updates
// HEAD to track the rewritten descendants.
func resolveStackHead() string {
	if !config.jj.enabled {
		return "HEAD"
	}
	out, err := jj("log", "-r", "@-", "--no-graph", "-T", "commit_id")
	if err != nil {
		exitf("ERROR: failed to resolve jj @-: %v", err)
	}
	head := strings.TrimSpace(out)
	debugf("resolved jj @- to %v", head)
	return head
}

// recoverRangeTip recomputes the selected tip's commit hash after the reword
// phase rewrote commit hashes. In jj mode the tip's change-id is stable across
// `jj describe`, so it resolves the tip by change-id — honoring an explicit
// BASE..TIP / COMMIT... selection even when jj's @- points at a *different*
// sibling stack (the @-+depth path silently pushed the @- stack instead, ignoring
// the positional range). In git-branchless mode reword moves HEAD to track the
// rewritten descendants, so depth-from-HEAD against postRewriteHead is correct.
// resolveChangeID/resolveDepth are injected so the decision is unit-testable
// without invoking jj/git.
func recoverRangeTip(
	jjEnabled bool,
	changeID string,
	postRewriteHead string,
	tipDepth int,
	resolveChangeID func(string) (string, error),
	resolveDepth func(head string, depth int) (string, error),
) (string, error) {
	if jjEnabled {
		return resolveChangeID(changeID)
	}
	return resolveDepth(postRewriteHead, tipDepth)
}

// validateRemoteRefsBeforePush returns the ShortHash of each non-skipped commit
// that has no Remote-Ref attribute. An empty result means every pushable commit
// has a ref and the push phase is safe to start.
func validateRemoteRefsBeforePush(commits []*Commit) []string {
	var missing []string
	for _, c := range commits {
		if c.Skip {
			continue
		}
		if c.GetAttr(KeyRemoteRef) == "" {
			missing = append(missing, c.ShortHash())
		}
	}
	return missing
}

// rewordCommit updates a commit's message using jj describe or git reword
func rewordCommit(commit *Commit, message string) (string, error) {
	if config.jj.enabled {
		// use jj change ID to avoid creating divergent commits
		if commit.ChangeID == "" {
			return "", errorf("commit %s has no change ID", commit.ShortHash())
		}
		debugf("using jj describe with change ID %s", commit.ChangeID[:12])
		return jj("describe", "-r", commit.ChangeID, "-m", message)
	}
	if config.bl.enabled {
		debugf("using git branchless reword to reword commit")
		return git("reword", commit.Hash, "-m", message)
	}

	exitf(`ERROR: neither jj nor git-branchless is available

This tool requires either:
  1. Jujutsu (jj) - install from https://martinvonz.github.io/jj/
     OR
  2. git-branchless - install from https://github.com/arxanas/git-branchless
     Then run: git branchless init

After installation, try again.`)
	return "", nil // unreachable
}

// PRHistoryEntry represents a PR from the historical stack with its status
type PRHistoryEntry struct {
	Number   int
	IsMerged bool // true if marked with ✔️, false if ⬛ or other emoji
}

// extractPRHistoryFromStackInfo extracts PR numbers and their status from existing stack info.
// Always returns entries normalized to internal order (oldest first).
// Note: The IsMerged status here reflects what was stored previously; it gets updated during
// accumulation if the PR is no longer in the local stack (see: allHistoricalPRs accumulation).
func extractPRHistoryFromStackInfo(existingBody string) []PRHistoryEntry {
	if existingBody == "" {
		return nil
	}

	var entries []PRHistoryEntry

	// First try to find content within sentinel markers
	startIdx := strings.Index(existingBody, stackInfoStartMarker)
	endIdx := strings.Index(existingBody, stackInfoEndMarker)

	var stackSection string
	if startIdx >= 0 && endIdx >= 0 && endIdx > startIdx {
		// Extract content between markers
		stackSection = existingBody[startIdx+len(stackInfoStartMarker) : endIdx]
	} else {
		// Fall back to searching all sections for the stack info pattern
		parts := strings.Split(existingBody, "\n---\n")
		stackInfoPattern := regexp.MustCompile(`(?m)^\* .* #\d+`)
		for _, part := range parts {
			if stackInfoPattern.MatchString(part) {
				stackSection = part
				break
			}
		}
	}

	if stackSection == "" {
		return nil
	}

	// Detect display order from the stored section:
	//   "newest at the top" = reverse=true (natural git order)
	//   "oldest at the top" or absent = reverse=false (legacy/default)
	// We need to normalize to internal order (oldest first) for consistent processing.
	isNaturalOrder := strings.Contains(stackSection, "newest at the top")

	// Extract PR numbers with their markers
	// Match lines like "* ✔️ #123" or "* ⬛ #456" or "* 🐻 #789"
	linePattern := regexp.MustCompile(`(?m)^\* ([^\s]+) #(\d+)`)
	matches := linePattern.FindAllStringSubmatch(stackSection, -1)

	seen := make(map[int]bool) // deduplicate
	for _, match := range matches {
		if len(match) > 2 {
			prNum := must(strconv.Atoi(match[2]))
			if !seen[prNum] {
				emoji := match[1]
				isMerged := emoji == "✔️"
				entries = append(entries, PRHistoryEntry{Number: prNum, IsMerged: isMerged})
				seen[prNum] = true
			}
		}
	}

	// Normalize to internal order (oldest first)
	// If the section was displayed in natural git order (newest at top), reverse it
	if isNaturalOrder && len(entries) > 1 {
		slices.Reverse(entries)
	}

	return entries
}

// extractPRNumbersFromStackInfo extracts PR numbers from existing stack info section
// Returns a slice of PR numbers in the order they appear
func extractPRNumbersFromStackInfo(existingBody string) []int {
	entries := extractPRHistoryFromStackInfo(existingBody)
	var prNumbers []int
	for _, e := range entries {
		prNumbers = append(prNumbers, e.Number)
	}
	return prNumbers
}

// generateStackInfo generates the stack info section showing all PRs in the stack.
//
// Internal order: always works with oldest-first order for consistency.
// Display order: applies reverse flag at render time.
//   - reverse=false (default/legacy): oldest at top, newest at bottom
//   - reverse=true: newest at top, oldest at bottom (natural git log order)
//
// Historical PRs not in current stack are preserved with their original markers.
func generateStackInfo(stackedCommits []*Commit, currentCommit *Commit, allHistoricalPRs []PRHistoryEntry, mergedPRs map[int]bool) string {
	var stackB strings.Builder
	sprf := func(msg string, args ...any) { fprintf(&stackB, msg, args...) }

	// Build set of current stack PR numbers for fast lookup
	currentStackSet := make(map[int]bool)
	for _, cm := range stackedCommits {
		if cm.PRNumber != 0 {
			currentStackSet[cm.PRNumber] = true
		}
	}

	// Build the final list in internal order (oldest first):
	//   1. Historical-only PRs (merged/closed, not in current stack) first — these are the
	//      older PRs that have already landed. Preserve their relative order from history.
	//   2. Current stack PRs in chronological order from stackedCommits.
	// Reversing this for newest-at-top display puts the current stack on top
	// (newest first) with merged history at the bottom — a continuous stack.
	type stackEntry struct {
		prNumber int
		commit   *Commit // non-nil if in current stack
		isMerged bool    // only used if commit is nil (historical only)
		index    int     // position in current stack (-1 if not in current stack)
	}
	var entries []stackEntry

	seen := make(map[int]bool)
	for _, hist := range allHistoricalPRs {
		if currentStackSet[hist.Number] || seen[hist.Number] {
			continue
		}
		seen[hist.Number] = true
		entries = append(entries, stackEntry{prNumber: hist.Number, isMerged: hist.IsMerged, index: -1})
	}
	for i, cm := range stackedCommits {
		if cm.PRNumber == 0 || seen[cm.PRNumber] {
			continue
		}
		seen[cm.PRNumber] = true
		entries = append(entries, stackEntry{prNumber: cm.PRNumber, commit: cm, isMerged: mergedPRs[cm.PRNumber], index: i})
	}

	// Calculate total count and current PR position (based on internal/chronological order)
	totalCount := len(entries)
	currentPosition := 0
	for i, e := range entries {
		if e.commit != nil && e.commit.Hash == currentCommit.Hash {
			currentPosition = i + 1 // 1-indexed, based on chronological position
			break
		}
	}

	// Create display order from internal order:
	//   reverse=false (legacy): keep as-is (oldest at top)
	//   reverse=true: flip to newest at top (natural git order)
	renderEntries := entries
	if config.reverse {
		renderEntries = make([]stackEntry, len(entries))
		for i, e := range entries {
			renderEntries[len(entries)-1-i] = e
		}
	}

	// Add stack position header
	if totalCount > 1 {
		orderNote := "oldest at the top"
		if config.reverse {
			orderNote = "newest at the top"
		}
		sprf("This is PR **%d of %d** in a stack (%s)\n\n", currentPosition, totalCount, orderNote)
	}

	// Render entries in display order
	for _, e := range renderEntries {
		if e.commit != nil {
			// Current stack PR - render with commit info
			renderCommit(&stackB, e.commit, currentCommit, e.isMerged)
		} else {
			// Historical PR not in current stack - preserve marker
			if e.isMerged {
				sprf("* ✔️ #%v\n", e.prNumber)
			} else {
				sprf("* ⬛ #%v\n", e.prNumber)
			}
		}
	}

	return stackB.String()
}

// renderCommit renders a single commit line to the stack info
func renderCommit(stackB *strings.Builder, cm *Commit, currentCommit *Commit, isMerged bool) {
	sprf := func(msg string, args ...any) { fprintf(stackB, msg, args...) }

	var cmRef string
	cmURL := fmt.Sprintf("https://%v/%v/commit/%v", config.git.host, config.git.repo, cm.ShortHash())
	switch {
	case cm.PRNumber != 0 && cm.Hash == currentCommit.Hash:
		cmRef = fmt.Sprintf("#%v 👈 This PR (%v)", cm.PRNumber, cm.ShortHash())
	case cm.PRNumber != 0:
		cmRef = fmt.Sprintf("#%v", cm.PRNumber)
	default:
		first, last := splitEmail(cm.AuthorEmail)
		formattedEmail := first + "&#x200B;" + last // zero-width space to prevent creating email link
		cmRef = fmt.Sprintf(`&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;<b>[%v (%v)](%v)</b>&nbsp;&nbsp; ${\textsf{\color{lightblue}· %v}}$`, cm.Title, cm.ShortHash(), cmURL, formattedEmail)
	}
	switch {
	case cm.Hash == currentCommit.Hash:
		sprf("* " + emojisx[currentCommit.PRNumber%len(emojisx)])
	case isMerged:
		sprf("* ✔️")
	default:
		sprf("* ⬛")
	}
	sprf(" %v\n", cmRef)
}

// generatePRBody generates the PR body based on commit message and existing PR body
// If the existing PR body already has sentinel markers, only the section between
// them is replaced — the rest of the description is preserved verbatim. This lets
// users edit the PR description on GitHub (or have other tools/templates fill it
// in) without git-pr clobbering their work.
// Otherwise, if the commit has a message, it overrides the entire PR body.
// If the commit has no message (GitHub UI user), existing content is preserved
// and stack info is appended.
func generatePRBody(commit *Commit, existingBody string, stackInfo string) string {
	// normalize line endings from GitHub (may have \r\n)
	existingBody = strings.ReplaceAll(existingBody, "\r\n", "\n")

	// Wrap stack info with sentinel markers for reliable detection and replacement
	wrappedStackInfo := fmt.Sprintf("%s\n%s\n%s", stackInfoStartMarker, stackInfo, stackInfoEndMarker)

	// If the existing body already has sentinels, only update the section between
	// them and preserve the surrounding content — regardless of whether this commit
	// has a message body. The remote description is the source of truth once it's
	// been set up with markers.
	startIdx := strings.Index(existingBody, stackInfoStartMarker)
	endIdx := strings.Index(existingBody, stackInfoEndMarker)

	if startIdx >= 0 && endIdx >= 0 && endIdx > startIdx {
		before := existingBody[:startIdx]
		after := existingBody[endIdx+len(stackInfoEndMarker):]

		before = strings.TrimRight(before, " \t\n")
		after = strings.TrimLeft(after, " \t\n")

		if after != "" {
			return before + "\n\n" + wrappedStackInfo + "\n\n" + after
		}
		return before + "\n\n" + wrappedStackInfo
	}

	if commit.Message != "" {
		// user manages via git commits - override entire PR body
		return fmt.Sprintf("%s\n\n---\n%s", commit.Message, wrappedStackInfo)
	}

	// No sentinel markers found - fall back to old detection logic for backwards compatibility
	// Search through ALL sections separated by "---" to find the old git-pr section
	parts := strings.Split(existingBody, "\n---\n")

	if len(parts) > 1 {
		// check if ANY section is stack info (has bullets with PR numbers)
		// This handles cases where other bots added content after the old git-pr section
		stackInfoPattern := regexp.MustCompile(`(?m)^\* .* #\d+`)
		foundStackInfoIndex := -1

		// Search through all parts to find the git-pr section
		for i, part := range parts {
			if stackInfoPattern.MatchString(part) {
				foundStackInfoIndex = i
				break
			}
		}

		if foundStackInfoIndex >= 0 {
			// Replace the old stack info section with sentinels
			parts[foundStackInfoIndex] = wrappedStackInfo
			return strings.Join(parts, "\n---\n")
		}

		// no stack info found in any section, append it
		return existingBody + "\n\n---\n" + wrappedStackInfo
	}

	// no separator found
	if existingBody == "" || existingBody == bodyTemplate || existingBody == getPRTemplate() {
		// empty or template only, use template
		return getPRTemplate() + "\n---\n" + wrappedStackInfo
	}
	// has content but no separator, append stack info
	return existingBody + "\n\n---\n" + wrappedStackInfo
}

func validateGitStatusClean() bool {
	if config.jj.enabled {
		// check jj working copy status: empty|nonempty + description
		output, err := jj("log", "-r", "@", "--no-graph", "-T",
			"if(empty, \"EMPTY\", \"NONEMPTY\") ++ \"|\" ++ if(description, description.first_line(), \"NO-DESC\")")
		if err != nil {
			debugf("warning: failed to check jj status: %v", err)
			// fallback to git status check
		} else {
			// parse output: "EMPTY|desc" or "NONEMPTY|NO-DESC" or "NONEMPTY|desc"
			lines := strings.Split(strings.TrimSpace(output), "\n")
			lastLine := lines[len(lines)-1] // get last line (actual output)
			parts := strings.Split(lastLine, "|")
			if len(parts) == 2 {
				isEmpty := parts[0] == "EMPTY"
				hasDesc := parts[1] != "NO-DESC"

				if isEmpty {
					debugf("jj working copy is empty, proceeding normally")
					return true
				}
				if !isEmpty && hasDesc {
					debugf("jj working copy has changes with description, will include in stack")
					return true
				}
				// not empty and no description - error
				return false
			}
		}
	}

	// for git repos or jj fallback
	if config.jj.enabled {
		// in jj mode the jj-path above returned earlier on success; getting here
		// means the jj template failed. don't silently swap to `git status` —
		// in a jj workspace GIT_DIR points outside the worktree and the output
		// would be meaningless. surface the failure instead.
		return false
	}

	// Check for uncommitted changes to tracked files (ignoring untracked files)
	// git diff checks unstaged changes, git diff --cached checks staged changes
	_, unstagedErr := git("diff", "--quiet")
	_, stagedErr := git("diff", "--cached", "--quiet")

	// Both commands exit with 0 if clean (nil error), non-zero if there are changes
	return unstagedErr == nil && stagedErr == nil
}

func isMyOwnCommit(commit *Commit) bool {
	return commit.AuthorEmail == config.git.email
}

func splitEmail(email string) (string, string) {
	if idx := strings.Index(email, "@"); idx >= 0 {
		return email[:idx], email[idx:]
	}
	return email, ""
}

func coalesce(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

// sanitizeBranchName converts a commit title into a valid git branch name
// by replacing spaces and illegal characters with hyphens
func sanitizeBranchName(title string) string {
	// Convert to lowercase
	s := strings.ToLower(title)

	// Remove common prefixes that are redundant in branch names
	prefixes := []string{"feat:", "fix:", "chore:", "docs:", "style:", "refactor:", "perf:", "test:", "build:", "ci:", "revert:"}
	for _, prefix := range prefixes {
		if strings.HasPrefix(s, prefix) {
			s = strings.TrimSpace(strings.TrimPrefix(s, prefix))
			break
		}
	}

	// Replace spaces and illegal characters with hyphens
	// Git branch names can't contain: space, ~, ^, :, ?, *, [, \, .., @{, trailing dot
	s = strings.ReplaceAll(s, " ", "-")
	s = strings.ReplaceAll(s, "_", "-")

	// Remove or replace illegal characters
	illegals := []string{"~", "^", ":", "?", "*", "[", "]", "\\", "..", "@{", "}", "/"}
	for _, illegal := range illegals {
		s = strings.ReplaceAll(s, illegal, "")
	}

	// Replace multiple consecutive hyphens with single hyphen
	for strings.Contains(s, "--") {
		s = strings.ReplaceAll(s, "--", "-")
	}

	// Trim leading/trailing hyphens and dots
	s = strings.Trim(s, "-.")

	// Limit length to reasonable size (50 chars)
	if len(s) > 50 {
		s = s[:50]
		s = strings.TrimRight(s, "-.")
	}

	// If empty after sanitization, use a fallback
	if s == "" {
		s = "unnamed-branch"
	}

	return s
}

// shouldSkipDrafts determines if draft commits should be skipped
// based on flags and config with proper precedence
func shouldSkipDrafts() bool {
	// --include-draft flag overrides everything (highest precedence)
	if config.includeDraft {
		return false
	}
	// --skip-draft flag or config setting
	return config.skipDraft
}
