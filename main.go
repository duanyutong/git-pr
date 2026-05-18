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
	// in jj multi-workspace setups, the shared backing git repo's HEAD does not
	// follow the current workspace. resolve @- via jj instead so we walk the
	// stack of the workspace the user is actually in.
	head := "HEAD"
	if config.jj.enabled {
		out, err := jj("log", "-r", "@-", "--no-graph", "-T", "commit_id")
		if err != nil {
			exitf("ERROR: failed to resolve jj @-: %v", err)
		}
		head = strings.TrimSpace(out)
		debugf("resolved jj @- to %v", head)
	}

	// Capture the user's starting position so we can return them here after the run.
	originalBranch, _ := git("branch", "--show-current")
	originalBranch = strings.TrimSpace(originalBranch)

	// Submit only commits from trunk up to HEAD. Running git-pr at b2 in a stack
	// b1→b2→b3→b4 opens/updates PRs for b1 and b2 only — the user controls the
	// submission boundary by choosing where to check out.
	stackedCommits := must(getStackedCommits(originMain, head))
	if len(stackedCommits) == 0 {
		exitf("no commits to submit")
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
	// Re-fetch commits after rewords. Branchless moves the current branch pointer
	// to the rewritten commit, so HEAD still resolves to the right place.
	stackedCommits = must(getStackedCommits(originMain, head))

	// checkpoint: rewrite
	if config.stopAfter == "rewrite" {
		printf("stopped after: rewrite\n")
		return
	}

	prevCommit := func(commit *Commit) (prev *Commit) {
		for _, cm := range stackedCommits {
			if cm == commit {
				return prev
			}
			if cm.Skip {
				continue
			}
			prev = cm
		}
		panic("not found")
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
		args := fmt.Sprintf("%v:refs/heads/%v", commit.ShortHash(), remoteRef)
		logs = fmt.Sprintf("push -f %v %v", config.git.remote, args)
		if config.dryRun {
			logs = "[DRY-RUN] " + logs
			return logs, func() bool { return false } // no-op for dry-run
		}
		return logs, func() bool {
			out := must(git("push", "-f", config.git.remote, args))
			time.Sleep(1 * time.Second)
			// Return true if this is a new branch that needs a PR created
			needsPR := strings.Contains(out, "remote: Create a pull request")
			// Don't do any PR operations here - handle all PR creation/updates
			// sequentially after all pushes complete to ensure correct PR numbering
			return needsPR
		}
	}
	// push commits, concurrently
	if config.dryRun {
		printf("[DRY-RUN] Would push commits:\n")
	}
	
	// Track which commits need PRs created (in order)
	type pushResult struct {
		commit  *Commit
		needsPR bool
	}
	pushResults := make([]pushResult, 0, len(stackedCommits))
	var pushResultsMu sync.Mutex
	
	{
		var wg sync.WaitGroup
		for _, commit := range stackedCommits {
			// push my own commits
			// and include others' commits if "--include-other-authors" is set
			shouldPush := isMyOwnCommit(commit) || config.includeOtherAuthors
			if !shouldPush {
				commit.Skip = true
				author := coalesce(commit.AuthorEmail, "@unknown")
				printf("skip \"%v\" (%v)\n", shortenTitle(commit.Title), author)
				continue
			}
			wg.Add(1)
			logs, execFunc := pushCommit(commit)
			printf("%s\n", logs)
			commit := commit // capture for goroutine
			if !config.dryRun {
				go func() {
					defer wg.Done()
					needsPR := execFunc()
					pushResultsMu.Lock()
					pushResults = append(pushResults, pushResult{commit: commit, needsPR: needsPR})
					pushResultsMu.Unlock()
				}()
			} else {
				wg.Done()
			}
		}
		wg.Wait()
	}
	
	// Handle PRs: look up existing PRs in parallel, create missing ones serially, update bases in parallel
	if !config.dryRun {
		// Sort results by position in stackedCommits to ensure correct order
		commitOrder := make(map[string]int)
		for i, cm := range stackedCommits {
			commitOrder[cm.Hash] = i
		}
		slices.SortFunc(pushResults, func(a, b pushResult) int {
			return commitOrder[a.commit.Hash] - commitOrder[b.commit.Hash]
		})

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
					must(0, githubCreatePRForCommit(commit, prevCommit(commit)))
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
					base := xif(prevCommit(commit) != nil, prevCommit(commit).GetRemoteRef(), config.git.remoteTrunk)
					_, err := gh("pr", "edit", strconv.Itoa(commit.PRNumber), "--base", base)
					if err == nil {
						commit.BaseUpdated = true
					}
				}()
			}
			wg.Wait()
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
		if config.jj.enabled {
			debugf("skipping git checkout in jj repo (jj manages working copy)")
		} else if originalBranch != "" {
			must(git("checkout", originalBranch))
		} else {
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
	{
		var wg sync.WaitGroup
		for i := len(stackedCommits) - 1; i >= 0; i-- {
			commit := stackedCommits[i]
			if commit.PRNumber == 0 {
				wg.Add(1)
				go func() {
					defer wg.Done()
					var prev *Commit
					for j := 0; j < i; j++ {
						cm := stackedCommits[j]
						if !cm.Skip {
							prev = cm
							break
						}
					}
					commit.PRNumber = must(githubGetPRNumberForCommit(commit, prev))
				}()
			}
		}
		wg.Wait()
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
	
	// update PRs with review link, concurrently
	printf("\n")
	{
		// Build set of PR numbers in current local stack
		currentStackSet := make(map[int]bool)
		for _, commit := range stackedCommits {
			if commit.PRNumber != 0 {
				currentStackSet[commit.PRNumber] = true
			}
		}
		
		// First, collect all historical PR entries from existing PR bodies
		// This ensures we preserve the complete stack history AND detect merged PRs
		var allHistoricalPRs []PRHistoryEntry
		prHistoryMap := make(map[int]bool)
		mergedPRs := make(map[int]bool)

		// Fetch all existing PR bodies first (before concurrent updates)
		for _, commit := range stackedCommits {
			if commit.Skip {
				continue
			}
			pr, err := githubGetPRByNumber(commit.PRNumber)
			if err == nil && pr != nil {
				if pr.Merged {
					mergedPRs[commit.PRNumber] = true
				}
				entries := extractPRHistoryFromStackInfo(pr.Body)
				for _, entry := range entries {
					if !prHistoryMap[entry.Number] {
						prHistoryMap[entry.Number] = true
						// If this PR is not in current local stack, mark it as merged/closed
						if !currentStackSet[entry.Number] {
							entry.IsMerged = true
						}
						allHistoricalPRs = append(allHistoricalPRs, entry)
					}
				}
			}
		}
		
		var wg sync.WaitGroup
		for _, commit := range stackedCommits {
			if commit.Skip {
				continue
			}
			wg.Add(1)
			commit := commit
			go func() {
				defer wg.Done()

				pr := must(githubGetPRByNumber(commit.PRNumber))
				pullURL := fmt.Sprintf("https://api.%v/repos/%v/pulls/%v", config.git.host, config.git.repo, commit.PRNumber)

				// generate the PR body with stack info (pass accumulated history from all PRs)
				stackInfo := generateStackInfo(stackedCommits, commit, allHistoricalPRs, mergedPRs)
				body := generatePRBody(commit, pr.Body, stackInfo)

				// update the PR
				must(httpRequest("PATCH", pullURL, map[string]any{
					"title": commit.Title,
					"body":  body,
				}))
				// Note: We don't change draft status for existing PRs to preserve user's choice
				// Draft status is only set during PR creation
				if tags := commit.GetTags(config.tags...); len(tags) > 0 {
					must(gh("pr", "edit", strconv.Itoa(commit.PRNumber), "--add-label", strings.Join(tags, ",")))
				}
			}()
		}
		wg.Wait()
	}
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
			prNum, err := githubFindPRNumberForCommit(commit)
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
		stackSection = existingBody[startIdx+len(stackInfoStartMarker):endIdx]
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
	_, unstagedErr := _git("diff", "--quiet")
	_, stagedErr := _git("diff", "--cached", "--quiet")
	
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
