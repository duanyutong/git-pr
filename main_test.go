package main

import (
	"strings"
	"testing"
)

// TestGeneratePRBodyWithSentinels tests the new sentinel-based approach
// for detecting and replacing the git-pr stack info section.
func TestGeneratePRBodyWithSentinels(t *testing.T) {
	// Mock commit for testing
	commit := &Commit{
		Hash:    "abc12345",
		Message: "", // Empty means user manages via GitHub UI
	}

	stackInfo := "This is PR **1 of 3** in a stack (oldest at the top)\n\n* ◻️ #101\n* ◻️ #102\n* 🦊 #103"

	tests := []struct {
		name         string
		existingBody string
		wantContains []string
		wantNotContain []string
		description  string
	}{
		{
			name:         "first time - no existing section",
			existingBody: "# My PR Description\n\nThis is a great feature!",
			wantContains: []string{
				"My PR Description",
				"great feature",
				stackInfoStartMarker,
				"This is PR **1 of 3**",
				stackInfoEndMarker,
			},
			description: "Should append wrapped stack info to existing content",
		},
		{
			name: "update existing section with sentinels",
			existingBody: "# My PR Description\n\nThis is a great feature!\n\n---\n" +
				stackInfoStartMarker + "\nOld stack info\n* ◻️ #999\n" + stackInfoEndMarker,
			wantContains: []string{
				"My PR Description",
				"great feature",
				stackInfoStartMarker,
				"This is PR **1 of 3**",
				"#101",
				stackInfoEndMarker,
			},
			wantNotContain: []string{
				"Old stack info",
				"#999",
			},
			description: "Should replace existing git-pr section between sentinels",
		},
		{
			name: "preserve content after git-pr section",
			existingBody: "# My PR Description\n\n" +
				stackInfoStartMarker + "\nOld stack\n" + stackInfoEndMarker + "\n\n" +
				"---\nSome bot added this after",
			wantContains: []string{
				"My PR Description",
				stackInfoStartMarker,
				"This is PR **1 of 3**",
				stackInfoEndMarker,
				"Some bot added this after",
			},
			wantNotContain: []string{
				"Old stack",
			},
			description: "Should preserve content added by other bots after git-pr section",
		},
		{
			name: "multiple bot sections",
			existingBody: "# Description\n\n" +
				stackInfoStartMarker + "\nOld\n" + stackInfoEndMarker + "\n\n" +
				"---\nBot 1 section\n\n---\nBot 2 section",
			wantContains: []string{
				"Description",
				stackInfoStartMarker,
				"This is PR **1 of 3**",
				stackInfoEndMarker,
				"Bot 1 section",
				"Bot 2 section",
			},
			description: "Should preserve multiple bot sections after git-pr section",
		},
		{
			name:         "empty body",
			existingBody: "",
			wantContains: []string{
				stackInfoStartMarker,
				"This is PR **1 of 3**",
				stackInfoEndMarker,
			},
			description: "Should handle empty body gracefully",
		},
		{
			name: "backwards compatibility - old format without sentinels",
			existingBody: "# Description\n\n---\nThis is PR 1 of 3\n\n* ◻️ #101",
			wantContains: []string{
				"Description",
				stackInfoStartMarker,
				"This is PR **1 of 3**",
				stackInfoEndMarker,
			},
			description: "Should detect and replace old format (pattern matching) and add sentinels",
		},
		{
			name: "backwards compatibility - old format with bot content after",
			existingBody: "# Description\n\n---\n* ◻️ #101\n* ◻️ #102\n\n---\n> [!NOTE]\nBot added this",
			wantContains: []string{
				"Description",
				stackInfoStartMarker,
				"This is PR **1 of 3**",
				stackInfoEndMarker,
				"[!NOTE]",
				"Bot added this",
			},
			wantNotContain: []string{
				"* ◻️ #101\n* ◻️ #102\n\n---\n> [!NOTE]", // Old section should be replaced
			},
			description: "Should find and replace old git-pr section even when bot content comes after it",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := generatePRBody(commit, tt.existingBody, stackInfo)

			// Check that all expected strings are present
			for _, want := range tt.wantContains {
				if !strings.Contains(result, want) {
					t.Errorf("%s\nResult missing expected content: %q\nFull result:\n%s",
						tt.description, want, result)
				}
			}

			// Check that unwanted strings are not present
			for _, notWant := range tt.wantNotContain {
				if strings.Contains(result, notWant) {
					t.Errorf("%s\nResult contains unwanted content: %q\nFull result:\n%s",
						tt.description, notWant, result)
				}
			}

			// Verify sentinels are properly paired
			startCount := strings.Count(result, stackInfoStartMarker)
			endCount := strings.Count(result, stackInfoEndMarker)
			if startCount != 1 || endCount != 1 {
				t.Errorf("%s\nSentinel count mismatch: start=%d, end=%d (expected 1 each)\nFull result:\n%s",
					tt.description, startCount, endCount, result)
			}

			t.Logf("✓ %s", tt.description)
		})
	}
}

// TestGeneratePRBodyWithCommitMessage tests that commit messages override
// the entire PR body (with wrapped stack info).
func TestGeneratePRBodyWithCommitMessage(t *testing.T) {
	commit := &Commit{
		Hash:    "abc12345",
		Message: "This is my commit message\n\nWith multiple paragraphs.",
	}

	stackInfo := "* ◻️ #101\n* ◻️ #102"
	existingBody := "Old PR description from GitHub UI"

	result := generatePRBody(commit, existingBody, stackInfo)

	// Should contain commit message
	if !strings.Contains(result, "This is my commit message") {
		t.Errorf("Missing commit message in result: %s", result)
	}

	// Should contain wrapped stack info
	if !strings.Contains(result, stackInfoStartMarker) {
		t.Errorf("Missing start sentinel in result: %s", result)
	}
	if !strings.Contains(result, stackInfoEndMarker) {
		t.Errorf("Missing end sentinel in result: %s", result)
	}

	// Should NOT contain old PR description (commit message overrides)
	if strings.Contains(result, "Old PR description") {
		t.Errorf("Should not contain old PR description when commit has message: %s", result)
	}

	t.Log("✓ Commit message correctly overrides PR body with wrapped stack info")
}

// TestGeneratePRBodyPreservesExistingWhenSentinelsPresent verifies that when the
// remote PR description already has sentinel markers, only the section between
// them is updated — even if the commit has a message body. The remote
// description (which the user may have filled out via GitHub UI, a PR template,
// or another tool) is the source of truth once it has markers.
func TestGeneratePRBodyPreservesExistingWhenSentinelsPresent(t *testing.T) {
	commit := &Commit{
		Hash:    "abc12345",
		Message: "Stale commit body that should NOT replace the rich PR description",
	}

	stackInfo := "* ◻️ #101\n* ◻️ #102"
	existingBody := "## Motivation\n\nDetailed rationale a human wrote.\n\n## Test plan\n\n- [x] Unit tests\n\n---\n" +
		stackInfoStartMarker + "\nOld stack\n* ◻️ #999\n" + stackInfoEndMarker

	result := generatePRBody(commit, existingBody, stackInfo)

	// Rich existing description must be preserved verbatim.
	for _, want := range []string{"## Motivation", "Detailed rationale a human wrote.", "## Test plan", "- [x] Unit tests"} {
		if !strings.Contains(result, want) {
			t.Errorf("Existing description content %q was not preserved.\nResult:\n%s", want, result)
		}
	}

	// Commit message body must NOT replace the existing description.
	if strings.Contains(result, "Stale commit body") {
		t.Errorf("Commit message body should not override existing PR body when sentinels are present.\nResult:\n%s", result)
	}

	// Stack info between markers must be updated.
	if !strings.Contains(result, "#101") || !strings.Contains(result, "#102") {
		t.Errorf("New stack info missing.\nResult:\n%s", result)
	}
	if strings.Contains(result, "#999") || strings.Contains(result, "Old stack") {
		t.Errorf("Old stack info still present.\nResult:\n%s", result)
	}

	// Exactly one pair of sentinels.
	if got := strings.Count(result, stackInfoStartMarker); got != 1 {
		t.Errorf("Expected exactly 1 start marker, got %d", got)
	}
	if got := strings.Count(result, stackInfoEndMarker); got != 1 {
		t.Errorf("Expected exactly 1 end marker, got %d", got)
	}
}

// TestSentinelMarkerStructure tests the sentinel marker format and positioning.
func TestSentinelMarkerStructure(t *testing.T) {
	commit := &Commit{Hash: "abc12345", Message: ""}
	stackInfo := "Stack content here"
	existingBody := "Some description"

	result := generatePRBody(commit, existingBody, stackInfo)

	// Extract the section between sentinels
	startIdx := strings.Index(result, stackInfoStartMarker)
	endIdx := strings.Index(result, stackInfoEndMarker)

	if startIdx < 0 || endIdx < 0 {
		t.Fatalf("Sentinels not found in result: %s", result)
	}

	if endIdx <= startIdx {
		t.Fatalf("End marker before start marker: start=%d, end=%d", startIdx, endIdx)
	}

	// Extract content between markers (including the markers themselves)
	section := result[startIdx : endIdx+len(stackInfoEndMarker)]

	// Verify structure: should be start marker, newline, content, newline, end marker
	expectedStart := stackInfoStartMarker + "\n"
	expectedEnd := "\n" + stackInfoEndMarker

	if !strings.HasPrefix(section, expectedStart) {
		t.Errorf("Section doesn't start correctly.\nExpected prefix: %q\nGot: %q",
			expectedStart, section[:len(expectedStart)])
	}

	if !strings.HasSuffix(section, expectedEnd) {
		t.Errorf("Section doesn't end correctly.\nExpected suffix: %q\nGot: %q",
			expectedEnd, section[len(section)-len(expectedEnd):])
	}

	// Verify content is between the markers
	content := section[len(expectedStart) : len(section)-len(expectedEnd)]
	if !strings.Contains(content, stackInfo) {
		t.Errorf("Content between markers missing stack info.\nExpected: %q\nGot: %q",
			stackInfo, content)
	}

	t.Log("✓ Sentinel marker structure is correct")
	t.Logf("  Start marker: %s", stackInfoStartMarker)
	t.Logf("  End marker: %s", stackInfoEndMarker)
	t.Logf("  Content properly wrapped")
}

// TestSentinelReplacement tests that repeated updates don't create duplicates.
func TestSentinelReplacement(t *testing.T) {
	commit := &Commit{Hash: "abc12345", Message: ""}

	// Simulating multiple git-pr runs
	rounds := []struct {
		stackInfo string
		round     int
	}{
		{"Stack v1\n* ◻️ #101", 1},
		{"Stack v2\n* ◻️ #101\n* ◻️ #102", 2},
		{"Stack v3\n* ◻️ #101\n* ◻️ #102\n* ◻️ #103", 3},
	}

	previousBody := "# My Feature\n\nInitial description"

	for _, round := range rounds {
		result := generatePRBody(commit, previousBody, round.stackInfo)

		// Count sentinels - should always be exactly 1 of each
		startCount := strings.Count(result, stackInfoStartMarker)
		endCount := strings.Count(result, stackInfoEndMarker)

		if startCount != 1 || endCount != 1 {
			t.Errorf("Round %d: Sentinel duplication detected! start=%d, end=%d\nResult:\n%s",
				round.round, startCount, endCount, result)
		}

		// Verify current stack info is present
		if !strings.Contains(result, round.stackInfo) {
			t.Errorf("Round %d: Missing current stack info\nExpected: %s\nResult:\n%s",
				round.round, round.stackInfo, result)
		}

		// Verify old stack info from previous rounds is NOT present
		for i := 1; i < round.round; i++ {
			oldInfo := rounds[i-1].stackInfo
			// Check for the specific version marker to ensure it's replaced
			if strings.Contains(result, "Stack v"+string(rune('0'+i))) {
				t.Errorf("Round %d: Old stack info still present: %s\nResult:\n%s",
					round.round, oldInfo, result)
			}
		}

		// Original description should still be present
		if !strings.Contains(result, "My Feature") {
			t.Errorf("Round %d: Original description lost\nResult:\n%s",
				round.round, result)
		}

		t.Logf("✓ Round %d: Correctly replaced stack info without duplication", round.round)

		// Use this result as input for next round
		previousBody = result
	}

	t.Log("✓ Multiple updates correctly replace without creating duplicates")
}

// TestSentinelWithBotInterference tests handling of other bots modifying the PR.
func TestSentinelWithBotInterference(t *testing.T) {
	commit := &Commit{Hash: "abc12345", Message: ""}

	// Simulate workflow:
	// 1. git-pr creates PR with stack info
	// 2. Bot adds content after git-pr section
	// 3. git-pr updates (should preserve bot content)
	// 4. Another bot adds more content
	// 5. git-pr updates again (should preserve all bot content)

	// Round 1: Initial creation
	initialStack := "Stack v1\n* ◻️ #101"
	result1 := generatePRBody(commit, "# Feature", initialStack)

	// Round 2: Bot adds content
	botContent1 := "\n\n---\n## CI/CD Status\n✓ All checks passed"
	result2WithBot := result1 + botContent1

	// Round 3: git-pr updates
	updatedStack := "Stack v2\n* ◻️ #101\n* ◻️ #102"
	result3 := generatePRBody(commit, result2WithBot, updatedStack)

	// Verify: should have updated stack but preserved bot content
	if !strings.Contains(result3, "Stack v2") {
		t.Errorf("Updated stack info missing: %s", result3)
	}
	if strings.Contains(result3, "Stack v1") {
		t.Errorf("Old stack info still present: %s", result3)
	}
	if !strings.Contains(result3, "CI/CD Status") {
		t.Errorf("Bot content lost after git-pr update: %s", result3)
	}

	// Round 4: Another bot adds more content
	botContent2 := "\n\n---\n## Performance Metrics\nLoad time: 120ms"
	result4WithBot := result3 + botContent2

	// Round 5: git-pr updates again
	finalStack := "Stack v3\n* ◻️ #101\n* ◻️ #102\n* ◻️ #103"
	result5 := generatePRBody(commit, result4WithBot, finalStack)

	// Verify: should have final stack and ALL bot content
	if !strings.Contains(result5, "Stack v3") {
		t.Errorf("Final stack info missing: %s", result5)
	}
	if !strings.Contains(result5, "CI/CD Status") {
		t.Errorf("First bot content lost: %s", result5)
	}
	if !strings.Contains(result5, "Performance Metrics") {
		t.Errorf("Second bot content lost: %s", result5)
	}
	if strings.Contains(result5, "Stack v1") || strings.Contains(result5, "Stack v2") {
		t.Errorf("Old stack versions still present: %s", result5)
	}

	// Verify only one git-pr section exists
	startCount := strings.Count(result5, stackInfoStartMarker)
	if startCount != 1 {
		t.Errorf("Multiple git-pr sections found: %d\nResult:\n%s", startCount, result5)
	}

	t.Log("✓ Successfully handled multiple bot interferences")
	t.Log("  - git-pr section correctly updated")
	t.Log("  - All bot content preserved")
	t.Log("  - No duplication of git-pr sections")
}
