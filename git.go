package main

import (
	"fmt"
	"regexp"
	"strings"
	"time"
)

var (
	regexpCommitHash = regexp.MustCompile(`^commit ([0-9a-f]{40})$`)
	regexpAuthor     = regexp.MustCompile(`^Author: (.*) <(.*)>$`)
	regexpDate       = regexp.MustCompile(`^Date:\s+(.*)$`)

	// "key: value"  or  "key = value"
	// - must not start with space at the beginning of the line
	regexpKeyVal = regexp.MustCompile(`^([a-zA-Z0-9-]+)\s*:\s*([^ ].+)$`)
	dateLayouts  = []string{"Mon Jan _2 15:04:05 2006 -0700", "2006-01-02 15:04:05 -0700"}
)

func gitLogs(size int, extra ...string) (string, error) {
	args := []string{"log", fmt.Sprintf("-%v", size)}
	args = append(args, extra...)
	return git(args...)
}

func parseLogs(logs string) (out CommitList, _ error) {
	logs = strings.TrimSpace(logs)
	if logs == "" {
		return nil, nil
	}
	lines := strings.Split(logs, "\n")
	part := []string{}
	for _, line := range lines {
		if m := regexpCommitHash.FindStringSubmatch(line); m != nil {
			if len(part) > 0 {
				item, err := parseLogsCommit(part)
				if err != nil {
					return nil, err
				}
				out = append(out, item)
			}
			part = part[:0]
		}
		part = append(part, line)
	}
	item, err := parseLogsCommit(part)
	if err != nil {
		return nil, err
	}
	out = append(out, item)
	return out, err
}

func parseLogsCommit(lines []string) (*Commit, error) {
	if len(lines) == 0 {
		return nil, nil
	}
	backup := lines
	out := &Commit{}
	// parse header
	bodyStart := len(lines) // default: no body
	for i, line := range lines {
		if line == "" {
			bodyStart = i + 1
			break
		}
		if m := regexpCommitHash.FindStringSubmatch(line); m != nil {
			out.Hash = m[1]
		}
		if m := regexpAuthor.FindStringSubmatch(line); m != nil {
			out.AuthorName = m[1]
			out.AuthorEmail = m[2]
		}
		if m := regexpDate.FindStringSubmatch(line); m != nil {
			var date time.Time
			var err error
			for _, layout := range dateLayouts {
				date, err = time.Parse(layout, m[1])
				if err == nil {
					break
				}
			}
			if err != nil {
				return nil, errorf("failed to parse time from %q", m[1])
			}
			out.Date = date.UTC()
		}
	}
	// parse title and body
	bodyLines := lines[bodyStart:]
	if len(bodyLines) > 0 {
		out.Title = strings.TrimSpace(bodyLines[0])
		bodyLines = bodyLines[1:]
		// trim 4 spaces prefix from body lines before parsing trailers
		for i := 0; i < len(bodyLines); i++ {
			bodyLines[i] = strings.TrimPrefix(bodyLines[i], "    ")
		}
		out.Message, out.Attrs = parseTrailers(bodyLines)
	}
	// validate (allow empty title for jujutsu commits like "jj new")
	if out.Hash == "" || out.AuthorName == "" || out.AuthorEmail == "" {
		return nil, errorf("failed to parse commit with log:\n%v", strings.Join(backup, "\n"))
	}
	return out, nil
}

func parseTrailers(lines []string) (message string, attrs []KeyVal) {
	// skip empty lines
	for i := 0; i < len(lines); i++ {
		if strings.TrimSpace(lines[i]) != "" {
			lines = lines[i:]
			break
		}
	}
	for i := len(lines) - 1; i >= 0; i-- {
		if strings.TrimSpace(lines[i]) != "" {
			lines = lines[:i+1]
			break
		}
	}

	// parse trailer from bottom up
	i, line := 0, ""
	for i = len(lines) - 1; i >= 0; i-- {
		if m := regexpKeyVal.FindStringSubmatch(lines[i]); m != nil {
			key, val := strings.ToLower(m[1]), strings.TrimSpace(m[2])
			attrs = append(attrs, KeyVal{key, val})
		} else {
			line = lines[i]
			break
		}
	}

	// require: trailers must be separated from body by a blank line
	// stop at first non-trailer line, then validate the blank line above
	if len(attrs) > 0 && line == "" {
		if i >= 0 {
			lines = lines[:i] // exclude the blank line
		} else {
			lines = nil
		}
	} else {
		attrs = nil // no valid trailers
	}

	return strings.TrimSpace(strings.Join(lines, "\n")), attrs
}

// jjGetChangeID returns the jj change ID for a git commit hash
func jjGetChangeID(gitHash string) (string, error) {
	if !config.jj.enabled {
		return "", nil
	}
	output, err := jj("log", "-r", gitHash, "--no-graph", "-T", "change_id")
	if err != nil {
		return "", err
	}
	// jj output may include status messages before the actual change ID
	// get the last non-empty line
	lines := strings.Split(strings.TrimSpace(output), "\n")
	for i := len(lines) - 1; i >= 0; i-- {
		line := strings.TrimSpace(lines[i])
		if line != "" {
			return line, nil
		}
	}
	return "", errorf("failed to parse change ID from jj output: %s", output)
}

// parseJJWorkingCopy parses jujutsu working copy output into a Commit.
// checkOutput format: "EMPTY|HAS-DESC" or "NONEMPTY|NO-DESC"
// infoOutput format: "changeID|commitID|description"
func parseJJWorkingCopy(checkOutput, infoOutput string) (*Commit, error) {
	lines := strings.Split(strings.TrimSpace(checkOutput), "\n")
	lastLine := lines[len(lines)-1]
	parts := strings.Split(lastLine, "|")

	if len(parts) != 2 {
		return nil, nil
	}

	isEmpty := parts[0] == "EMPTY"
	hasDesc := parts[1] == "HAS-DESC"

	// skip if no description at all
	if !hasDesc {
		return nil, nil
	}

	// skip empty commits (no changes)
	if isEmpty {
		return nil, nil
	}

	// include only non-empty commits with description

	// parse info output
	lines = strings.Split(strings.TrimSpace(infoOutput), "\n")
	firstLine := lines[0]
	parts = strings.Split(firstLine, "|")
	if len(parts) < 3 {
		return nil, errorf("unexpected jj @ output: %s", firstLine)
	}

	changeID := parts[0]
	commitID := parts[1]
	// full description starts after "changeID|commitID|"
	descriptionBody := strings.TrimPrefix(firstLine, changeID+"|"+commitID+"|")
	if len(lines) > 1 {
		// description spans multiple lines
		descriptionBody = descriptionBody + "\n" + strings.Join(lines[1:], "\n")
	}

	// parse description like a commit body
	descLines := strings.Split(descriptionBody, "\n")
	title := ""
	if len(descLines) > 0 {
		title = strings.TrimSpace(descLines[0])
	}
	message, attrs := parseTrailers(descLines[1:])

	// create commit struct
	commit := &Commit{
		Hash:        commitID,
		ChangeID:    changeID,
		Title:       title,
		Message:     message,
		Attrs:       attrs,
		AuthorEmail: config.git.email,
		AuthorName:  config.git.user,
	}
	return commit, nil
}

// jjGetWorkingCopy returns the working copy commit if it's non-empty with description
func jjGetWorkingCopy() (*Commit, error) {
	if !config.jj.enabled {
		return nil, nil
	}

	// check if @ is non-empty with description
	checkOutput, err := jj("log", "-r", "@", "--no-graph", "-T",
		"if(empty, \"EMPTY\", \"NONEMPTY\") ++ \"|\" ++ if(description, \"HAS-DESC\", \"NO-DESC\")")
	if err != nil {
		return nil, err
	}

	// get full info including description body
	infoOutput, err := jj("log", "-r", "@", "--no-graph", "-T",
		"change_id ++ \"|\" ++ commit_id ++ \"|\" ++ description")
	if err != nil {
		return nil, err
	}

	return parseJJWorkingCopy(checkOutput, infoOutput)
}

// resolveStackTip walks descendants of `target` (typically HEAD) along local
// branch refs and returns the topmost commit reachable through them. This lets
// git-pr operate on the full stack even when the user has checked out a middle
// commit of the stack. Returns `target` unchanged if there are no descendants
// on local branches, or if the descendants diverge into multiple unrelated
// chains — in which case we can't unambiguously pick a single tip.
func resolveStackTip(target string) string {
	targetHash, err := git("rev-parse", target)
	if err != nil {
		return target
	}
	targetHash = strings.TrimSpace(targetHash)

	output, err := git("for-each-ref", "--contains", target,
		"--format=%(objectname)", "refs/heads/")
	if err != nil {
		return target
	}

	var tips []string
	seen := map[string]bool{}
	for _, line := range strings.Split(strings.TrimSpace(output), "\n") {
		h := strings.TrimSpace(line)
		if h == "" || h == targetHash || seen[h] {
			continue
		}
		seen[h] = true
		tips = append(tips, h)
	}

	leaf, ok := pickStackLeaf(tips, isAncestorCommit)
	if !ok {
		debugf("warning: %v has multiple unrelated descendant branches; operating on %v..HEAD only", target, target)
		return target
	}
	if leaf == "" {
		return target
	}
	return leaf
}

// pickStackLeaf returns the unique tip in `tips` that has every other tip as
// an ancestor (i.e. the leaf of a linear stack of descendants). Returns
// (leaf, true) on success. Returns ("", true) when `tips` is empty (no
// descendants — caller should keep their target). Returns ("", false) when
// descendants diverge into multiple unrelated chains. The `isAncestor`
// argument lets tests substitute a fake without invoking git.
func pickStackLeaf(tips []string, isAncestor func(ancestor, descendant string) bool) (string, bool) {
	if len(tips) == 0 {
		return "", true
	}
	for _, candidate := range tips {
		isLeaf := true
		for _, other := range tips {
			if candidate == other {
				continue
			}
			if !isAncestor(other, candidate) {
				isLeaf = false
				break
			}
		}
		if isLeaf {
			return candidate, true
		}
	}
	return "", false
}

// isAncestorCommit reports whether `ancestor` is an ancestor of `descendant`.
// Uses `git merge-base --is-ancestor`, which exits 0 for true and 1 for false.
func isAncestorCommit(ancestor, descendant string) bool {
	_, err := git("merge-base", "--is-ancestor", ancestor, descendant)
	return err == nil
}

func getStackedCommits(base, target string) ([]*Commit, error) {
	logs, err := gitLogs(100, fmt.Sprintf("%v..%v", base, target))
	if err != nil {
		return nil, wrapf(err, "failed to find common ancestor for %v and %v", base, target)
	}
	list, err := parseLogs(logs)
	if err != nil {
		return nil, err
	}

	// filter out empty commits (no title and no message)
	filtered := make([]*Commit, 0, len(list))
	for _, commit := range list {
		if commit.Title != "" || commit.Message != "" {
			filtered = append(filtered, commit)
		}
	}
	list = filtered

	// populate jj change IDs if in jj repo
	if config.jj.enabled {
		for _, commit := range list {
			changeID, err := jjGetChangeID(commit.Hash)
			if err != nil {
				debugf("warning: failed to get change ID for %s: %v", commit.ShortHash(), err)
			} else {
				commit.ChangeID = changeID
			}
		}
	}

	// sort from oldest to newest
	result := revert(list)

	// append jj working copy at the end (newest) if applicable
	if config.jj.enabled {
		workingCopy, err := jjGetWorkingCopy()
		if err != nil {
			debugf("warning: failed to get jj working copy: %v", err)
		} else if workingCopy != nil {
			debugf("including jj working copy in stack: %s", workingCopy.Title)
			result = append(result, workingCopy)
		}
	}

	// validate commits and collect warnings/errors
	var warnings []string
	var errors []string
	filtered = result[:0] // reuse filtered slice for non-skipped commits

	for _, commit := range result {
		isEmpty := isEmptyCommit(commit)
		hasEmptyTitle := commit.Title == ""

		if hasEmptyTitle && isEmpty {
			// warn: empty title + no file changes
			warnings = append(warnings, fmt.Sprintf("⚠️  commit %s has empty title and no file changes, skipping", commit.ShortHash()))
			commit.Skip = true
			continue
		} else if hasEmptyTitle {
			// error: empty title + has file changes
			errors = append(errors, fmt.Sprintf("❌ commit %s has empty title but contains file changes (fix required)", commit.ShortHash()))
			commit.Skip = true
			continue
		} else if isEmpty {
			// warn: no file changes
			warnings = append(warnings, fmt.Sprintf("⚠️  commit %s %q has no file changes, skipping", commit.ShortHash(), shortenTitle(commit.Title)))
			commit.Skip = true
			continue
		}

		filtered = append(filtered, commit)
	}
	result = filtered

	// print warnings and errors
	for _, msg := range warnings {
		printf("%s\n", msg)
	}
	for _, msg := range errors {
		printf("%s\n", msg)
	}

	// return error if any validation errors
	if len(errors) > 0 {
		return nil, errorf("validation failed, please fix the commits above")
	}

	return result, nil
}

// isEmptyCommit checks if a commit has no file changes
func isEmptyCommit(commit *Commit) bool {
	// use git to check if commit has file changes
	output, err := git("diff-tree", "--no-commit-id", "--name-only", "-r", commit.Hash)
	if err != nil {
		debugf("warning: failed to check if commit is empty: %v", err)
		return false // assume not empty on error
	}

	return strings.TrimSpace(output) == ""
}

func shortenTitle(title string) string {
	const Max = 36
	if len(title) <= Max {
		return title
	}
	title = title[:Max]
	idx := strings.LastIndexByte(title, ' ')
	if idx == -1 {
		return title + "..."
	} else {
		return title[:idx] + " ..."
	}
}

func deleteBranch(branch string) error {
	branches, err := git("branch")
	if err != nil {
		return err
	}
	if strings.Contains(branches, branch+"\n") {
		_, err = git("branch", "-D", branch) // delete branch
	}
	return err
}

// findBranchForCommit finds existing local or remote branch that POINTS TO the given commit
// (not just contains it in history)
func findBranchForCommit(commit *Commit) (string, error) {
	// Get all branches with their HEAD commit
	output, err := git("branch", "-a", "--format=%(refname)|%(objectname)")
	if err != nil {
		return "", nil // error listing branches, not a fatal error
	}
	
	lines := strings.Split(strings.TrimSpace(output), "\n")
	if len(lines) == 0 || lines[0] == "" {
		return "", nil
	}
	
	var localBranch string
	var remoteBranch string
	
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		
		parts := strings.Split(line, "|")
		if len(parts) != 2 {
			continue
		}
		
		refName := parts[0]
		commitHash := parts[1]
		
		// Only match if branch points EXACTLY to this commit
		if commitHash != commit.Hash {
			continue
		}
		
		// Skip HEAD and main/master branches
		if strings.Contains(refName, "HEAD") {
			continue
		}
		if strings.HasSuffix(refName, "/main") || strings.HasSuffix(refName, "/master") {
			continue
		}
		if refName == "refs/heads/main" || refName == "refs/heads/master" {
			continue
		}
		
		// Prefer local branches
		if strings.HasPrefix(refName, "refs/heads/") {
			localBranch = strings.TrimPrefix(refName, "refs/heads/")
			break // found local branch, use it
		}
		
		// Track remote branches as fallback
		if strings.HasPrefix(refName, "refs/remotes/"+config.git.remote+"/") {
			remoteBranch = strings.TrimPrefix(refName, "refs/remotes/"+config.git.remote+"/")
		}
	}
	
	// Prefer local branch, fallback to remote
	if localBranch != "" {
		return localBranch, nil
	}
	return remoteBranch, nil
}

// getLocalBranchForCommit returns the local branch that points to this commit
// Used when branches are pre-created (e.g., by git-branchless)
func getLocalBranchForCommit(commit *Commit) (string, error) {
	// Get all local branches with their HEAD commit
	output, err := git("branch", "--format=%(refname:short)|%(objectname)")
	if err != nil {
		return "", err
	}
	
	lines := strings.Split(strings.TrimSpace(output), "\n")
	
	debugf("[getLocalBranchForCommit] looking for commit %v", commit.Hash)
	
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		
		parts := strings.Split(line, "|")
		if len(parts) != 2 {
			continue
		}
		
		branchName := parts[0]
		commitHash := parts[1]
		
		// Skip main/master
		if branchName == "main" || branchName == "master" {
			continue
		}
		
		debugf("  checking %v -> %v (match: %v)", branchName, commitHash[:8], commitHash == commit.Hash)
		
		// Check for exact match
		if commitHash == commit.Hash {
			debugf("  FOUND: %v", branchName)
			return branchName, nil
		}
	}
	
	// No branch found - log debug info
	printf("[BRANCH-NOT-FOUND] commit %v not on any local branch\n", commit.Hash[:8])
	printf("  Commit title: %v\n", commit.Title)
	printf("  Available local branches:\n")
	for _, line := range lines {
		parts := strings.Split(line, "|")
		if len(parts) == 2 && parts[0] != "main" && parts[0] != "master" {
			printf("    %v -> %v\n", parts[0], parts[1][:8])
		}
	}
	return "", nil
}
