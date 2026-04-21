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
			exitf("duplicated remote ref %q found for %q and %q", last.GetRemoteRef(), last.ShortHash(), commit.ShortHash())
		}
		mapRefs[remoteRef] = commit
	}

	// fill remote ref for each commit
	for commitWithoutRemoteRef := range findCommitsWithoutRemoteRef(stackedCommits) {
		// Try to find an existing branch for this commit
		existingBranch, err := findBranchForCommit(commitWithoutRemoteRef)
		if err != nil {
			debugf("warning: failed to check for existing branch: %v", err)
		}
		
		var remoteRef string
		if existingBranch != "" {
			// Use the existing branch name
			remoteRef = existingBranch
			debugf("found existing branch %v for %v", remoteRef, commitWithoutRemoteRef.ShortHash())
		} else {
			// Generate new branch name
			if config.branchFromTitle {
				// Generate from commit title
				sanitized := sanitizeBranchName(commitWithoutRemoteRef.Title)
				remoteRef = fmt.Sprintf("%v/%v", config.gh.user, sanitized)
			} else {
				// Generate from hash (default behavior)
				remoteRef = fmt.Sprintf("%v/%v", config.gh.user, commitWithoutRemoteRef.ShortHash())
			}
			debugf("creating remote ref %v for %v", remoteRef, commitWithoutRemoteRef.Title)
		}
		
		commitWithoutRemoteRef.SetAttr(KeyRemoteRef, remoteRef)
		must(rewordCommit(commitWithoutRemoteRef, commitWithoutRemoteRef.FullMessage()))

		time.Sleep(time.Millisecond)
	}
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
	pushCommit := func(commit *Commit) (logs string, execFunc func()) {
		args := fmt.Sprintf("%v:refs/heads/%v", commit.ShortHash(), commit.GetAttr(KeyRemoteRef))
		logs = fmt.Sprintf("push -f %v %v", config.git.remote, args)
		if config.dryRun {
			logs = "[DRY-RUN] " + logs
			return logs, func() {} // no-op for dry-run
		}
		return logs, func() {
			out := must(git("push", "-f", config.git.remote, args))
			time.Sleep(1 * time.Second)
			if strings.Contains(out, "remote: Create a pull request") {
				must(0, githubCreatePRForCommit(commit, prevCommit(commit)))
			} else {
				must(0, githubPRUpdateBaseForCommit(commit, prevCommit(commit)))
			}
		}
	}
	// push commits, concurrently
	if config.dryRun {
		printf("[DRY-RUN] Would push commits:\n")
	}
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
			if !config.dryRun {
				go func() {
					defer wg.Done()
					execFunc()
				}()
			} else {
				wg.Done()
			}
		}
		wg.Wait()
	}

	// checkpoint: push
	if config.stopAfter == "push" {
		printf("stopped after: push\n")
		return
	}

	// checkout the latest stacked commit
	if !config.dryRun {
		if config.jj.enabled {
			debugf("skipping git checkout in jj repo (jj manages working copy)")
		} else {
			must(git("checkout", stackedCommits[len(stackedCommits)-1].Hash))
		}
	}

	// wait for 5 seconds
	if !config.dryRun {
		printf("waiting a bit...\n")
		time.Sleep(5 * time.Second)
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
		
		// Fetch all existing PR bodies first (before concurrent updates)
		for _, commit := range stackedCommits {
			if commit.Skip {
				continue
			}
			pr, err := githubGetPRByNumber(commit.PRNumber)
			if err == nil && pr != nil {
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
			prURL := fmt.Sprintf("https://%v/%v/pull/%v", config.git.host, config.git.repo, commit.PRNumber)
			printf("%v\n", prURL)
			go func() {
				defer wg.Done()

				pr := must(githubGetPRByNumber(commit.PRNumber))
				pullURL := fmt.Sprintf("https://api.%v/repos/%v/pulls/%v", config.git.host, config.git.repo, commit.PRNumber)

				// generate the PR body with stack info (pass accumulated history from all PRs)
				stackInfo := generateStackInfo(stackedCommits, commit, allHistoricalPRs)
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
func generateStackInfo(stackedCommits []*Commit, currentCommit *Commit, allHistoricalPRs []PRHistoryEntry) string {
	var stackB strings.Builder
	sprf := func(msg string, args ...any) { fprintf(&stackB, msg, args...) }

	// Build map of current stack: PR number -> commit + index (for ordering)
	type commitWithIndex struct {
		commit *Commit
		index  int
	}
	currentStackMap := make(map[int]commitWithIndex)
	for i, cm := range stackedCommits {
		if cm.PRNumber != 0 {
			currentStackMap[cm.PRNumber] = commitWithIndex{commit: cm, index: i}
		}
	}
	
	// Build set of historical PR numbers
	historicalPRs := make(map[int]bool)
	for _, entry := range allHistoricalPRs {
		historicalPRs[entry.Number] = true
	}
	
	// Find new PRs in current stack that aren't in history
	var newPRs []commitWithIndex
	for i, cm := range stackedCommits {
		if cm.PRNumber != 0 && !historicalPRs[cm.PRNumber] {
			newPRs = append(newPRs, commitWithIndex{commit: cm, index: i})
		}
	}
	
	// Build the final list in internal order (oldest first)
	// We need to merge historical entries with new PRs based on stack position
	type stackEntry struct {
		prNumber int
		commit   *Commit   // non-nil if in current stack
		isMerged bool      // only used if commit is nil (historical only)
		index    int       // position in current stack (-1 if not in current stack)
	}
	var entries []stackEntry
	
	// First, add historical entries in order (will be replaced/updated where applicable)
	for _, hist := range allHistoricalPRs {
		if cwi, ok := currentStackMap[hist.Number]; ok {
			// This PR is in current stack - use the commit
			entries = append(entries, stackEntry{prNumber: hist.Number, commit: cwi.commit, index: cwi.index})
		} else {
			// This PR is not in current stack - preserve historical marker
			entries = append(entries, stackEntry{prNumber: hist.Number, isMerged: hist.IsMerged, index: -1})
		}
	}
	
	// Insert new PRs at correct positions based on their index in stackedCommits
	// Find where each new PR should go relative to existing entries
	for _, newPR := range newPRs {
		// Find insertion point: after the last entry with index < newPR.index
		insertAt := 0
		for i, e := range entries {
			if e.index >= 0 && e.index < newPR.index {
				insertAt = i + 1
			}
		}
		// Insert at the found position
		newEntry := stackEntry{prNumber: newPR.commit.PRNumber, commit: newPR.commit, index: newPR.index}
		entries = append(entries[:insertAt], append([]stackEntry{newEntry}, entries[insertAt:]...)...)
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
	
	// Don't mark upstack PRs (after current) as merged - they haven't been pruned locally yet
	// Only mark downstack PRs (before current) as merged if they're not in current stack
	for i := currentPosition; i < len(entries); i++ {
		entries[i].isMerged = false // Upstack PRs stay as they were
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
			renderCommit(&stackB, e.commit, currentCommit)
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
func renderCommit(stackB *strings.Builder, cm *Commit, currentCommit *Commit) {
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
	if cm.Hash == currentCommit.Hash {
		sprf("* " + emojisx[currentCommit.PRNumber%len(emojisx)])
	} else {
		sprf("* ⬛")
	}
	sprf(" %v\n", cmRef)
}

// generatePRBody generates the PR body based on commit message and existing PR body
// If commit has a message, it overrides the entire PR body
// If commit has no message (GitHub UI user), it preserves existing content and only updates stack info
func generatePRBody(commit *Commit, existingBody string, stackInfo string) string {
	// normalize line endings from GitHub (may have \r\n)
	existingBody = strings.ReplaceAll(existingBody, "\r\n", "\n")

	// Wrap stack info with sentinel markers for reliable detection and replacement
	wrappedStackInfo := fmt.Sprintf("%s\n%s\n%s", stackInfoStartMarker, stackInfo, stackInfoEndMarker)

	if commit.Message != "" {
		// user manages via git commits - override entire PR body
		return fmt.Sprintf("%s\n\n---\n%s", commit.Message, wrappedStackInfo)
	}

	// user manages via GitHub UI - preserve their edits, only update stack info
	// Check if we have existing git-pr section marked with sentinels
	startIdx := strings.Index(existingBody, stackInfoStartMarker)
	endIdx := strings.Index(existingBody, stackInfoEndMarker)

	if startIdx >= 0 && endIdx >= 0 && endIdx > startIdx {
		// Found existing git-pr section - replace it
		// Keep everything before start marker and everything after end marker
		before := existingBody[:startIdx]
		after := existingBody[endIdx+len(stackInfoEndMarker):]
		
		// Trim trailing whitespace from before and leading whitespace from after
		before = strings.TrimRight(before, " \t\n")
		after = strings.TrimLeft(after, " \t\n")
		
		// Reconstruct: before + wrapped stack info + after (if not empty)
		if after != "" {
			return before + "\n\n" + wrappedStackInfo + "\n\n" + after
		}
		return before + "\n\n" + wrappedStackInfo
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
