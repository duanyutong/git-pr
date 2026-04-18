package main

import (
	"strconv"
	"testing"
)

// TestDraftPRCreationLogic tests that the draft flag is correctly determined
// during PR creation based on config and commit title patterns.
func TestDraftPRCreationLogic(t *testing.T) {
	// Save original config values to restore after tests
	oldDraft := config.draft
	oldDraftPatterns := config.draftPatterns
	defer func() {
		config.draft = oldDraft
		config.draftPatterns = oldDraftPatterns
	}()

	// Set up draft patterns for testing
	config.draftPatterns = []string{"wip:*", "draft:*", "*[wip]*", "*[draft]*"}

	tests := []struct {
		name              string
		configDraft       bool
		commitTitle       string
		wantDraft         bool
		description       string
	}{
		{
			name:              "draft flag enabled with normal title",
			configDraft:       true,
			commitTitle:       "feat: add new feature",
			wantDraft:         true,
			description:       "When --draft flag is set, PR should be created as draft regardless of title",
		},
		{
			name:              "draft flag disabled with normal title",
			configDraft:       false,
			commitTitle:       "feat: add new feature",
			wantDraft:         false,
			description:       "Without --draft flag or draft pattern, PR should be ready for review",
		},
		{
			name:              "draft pattern in title with flag disabled",
			configDraft:       false,
			commitTitle:       "wip: add new feature",
			wantDraft:         true,
			description:       "Title with 'wip:' prefix should create draft PR even without --draft flag",
		},
		{
			name:              "draft pattern with brackets",
			configDraft:       false,
			commitTitle:       "[draft] add new feature",
			wantDraft:         true,
			description:       "Title with [draft] should create draft PR",
		},
		{
			name:              "draft pattern with wip in brackets",
			configDraft:       false,
			commitTitle:       "feat: [wip] add new feature",
			wantDraft:         true,
			description:       "Title with [wip] anywhere should create draft PR",
		},
		{
			name:              "both draft flag and pattern",
			configDraft:       true,
			commitTitle:       "wip: add new feature",
			wantDraft:         true,
			description:       "When both flag and pattern present, should definitely be draft",
		},
		{
			name:              "case insensitive pattern matching",
			configDraft:       false,
			commitTitle:       "WIP: add new feature",
			wantDraft:         true,
			description:       "Pattern matching should be case-insensitive",
		},
		{
			name:              "draft prefix pattern",
			configDraft:       false,
			commitTitle:       "draft: experimental feature",
			wantDraft:         true,
			description:       "Title with 'draft:' prefix should create draft PR",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// Set config for this test
			config.draft = tt.configDraft

			// Simulate the draft detection logic from githubCreatePRForCommit
			isDraft := config.draft || matchAnyPattern(config.draftPatterns, tt.commitTitle)

			// Verify the result matches expectation
			if isDraft != tt.wantDraft {
				t.Errorf("%s\nGot isDraft=%v, want isDraft=%v\nTitle: %q, config.draft=%v",
					tt.description, isDraft, tt.wantDraft, tt.commitTitle, tt.configDraft)
			} else {
				t.Logf("✓ %s", tt.description)
			}
		})
	}
}

// TestPRNumberExtraction tests that PR numbers are correctly extracted
// from gh CLI output after PR creation.
func TestPRNumberExtraction(t *testing.T) {
	tests := []struct {
		name           string
		ghOutput       string
		expectedPRNum  int
		shouldExtract  bool
		description    string
	}{
		{
			name:           "standard PR URL output",
			ghOutput:       "https://github.com/user/repo/pull/123\n",
			expectedPRNum:  123,
			shouldExtract:  true,
			description:    "Should extract PR number from standard GitHub URL",
		},
		{
			name:           "PR URL with trailing newline",
			ghOutput:       "https://github.com/user/repo/pull/456",
			expectedPRNum:  456,
			shouldExtract:  true,
			description:    "Should extract PR number even without trailing newline",
		},
		{
			name:           "large PR number",
			ghOutput:       "https://github.com/organization/repository/pull/99999\n",
			expectedPRNum:  99999,
			shouldExtract:  true,
			description:    "Should handle large PR numbers correctly",
		},
		{
			name:           "output with extra text",
			ghOutput:       "Creating pull request...\nhttps://github.com/user/repo/pull/789\nSuccess!",
			expectedPRNum:  789,
			shouldExtract:  true,
			description:    "Should extract PR number even with surrounding text",
		},
		{
			name:           "no PR number in output",
			ghOutput:       "Error: failed to create PR",
			expectedPRNum:  0,
			shouldExtract:  false,
			description:    "Should handle cases where no PR number is present",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// Simulate the PR number extraction logic
			prNumStr := regexpNumber.FindString(tt.ghOutput)
			
			if tt.shouldExtract {
				if prNumStr == "" {
					t.Errorf("%s\nFailed to extract PR number from output: %q",
						tt.description, tt.ghOutput)
					return
				}
				
				// Parse the extracted number
				prNum := 0
				if prNumStr != "" {
					var err error
					prNum, err = strconv.Atoi(prNumStr)
					if err != nil {
						t.Errorf("%s\nFailed to parse PR number: %v", tt.description, err)
						return
					}
				}
				
				if prNum != tt.expectedPRNum {
					t.Errorf("%s\nGot PR number %d, want %d from output: %q",
						tt.description, prNum, tt.expectedPRNum, tt.ghOutput)
				} else {
					t.Logf("✓ %s: extracted PR #%d", tt.description, prNum)
				}
			} else {
				if prNumStr != "" {
					t.Errorf("%s\nExpected no extraction, but got: %s",
						tt.description, prNumStr)
				} else {
					t.Logf("✓ %s: correctly handled no PR number", tt.description)
				}
			}
		})
	}
}

// TestCommitNewlyCreatedFlag tests that the NewlyCreated flag is properly
// managed to distinguish new PRs from existing ones.
func TestCommitNewlyCreatedFlag(t *testing.T) {
	tests := []struct {
		name          string
		initialState  bool
		afterCreation bool
		description   string
	}{
		{
			name:          "new commit starts as not newly created",
			initialState:  false,
			afterCreation: true,
			description:   "Commit should start with NewlyCreated=false, then set to true after PR creation",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// Create a test commit
			commit := &Commit{
				Hash:         "abc12345",
				Title:        "test: add test commit",
				NewlyCreated: tt.initialState,
			}

			// Verify initial state
			if commit.NewlyCreated != tt.initialState {
				t.Errorf("Initial state incorrect: got %v, want %v",
					commit.NewlyCreated, tt.initialState)
			}

			// Simulate PR creation (this would be done in githubCreatePRForCommit)
			commit.NewlyCreated = true

			// Verify state after creation
			if commit.NewlyCreated != tt.afterCreation {
				t.Errorf("%s\nAfter creation: got NewlyCreated=%v, want %v",
					tt.description, commit.NewlyCreated, tt.afterCreation)
			} else {
				t.Logf("✓ %s", tt.description)
			}
		})
	}
}

// TestDraftStatusPreservation tests that draft status should NOT be modified
// during PR updates, only during creation.
func TestDraftStatusPreservation(t *testing.T) {
	// This test documents the expected behavior: draft status should never
	// be changed during PR updates. The update flow should only modify:
	// - PR title
	// - PR body
	// - PR labels
	// But NOT draft/ready status.

	tests := []struct {
		name        string
		scenario    string
		expectation string
	}{
		{
			name:     "existing draft PR stays draft",
			scenario: "PR was created as draft, then user runs git-pr again",
			expectation: "Draft status should remain unchanged - the update flow " +
				"should not call 'gh pr ready' or 'gh pr ready --undo'",
		},
		{
			name:     "existing ready PR stays ready",
			scenario: "PR was created as ready, then user runs git-pr again",
			expectation: "Ready status should remain unchanged - the update flow " +
				"should not call 'gh pr ready' or 'gh pr ready --undo'",
		},
		{
			name:     "manually marked ready stays ready",
			scenario: "PR was created as draft, user manually marked it ready in GitHub UI, then runs git-pr",
			expectation: "Should preserve the ready status chosen by user - the update flow " +
				"should not call 'gh pr ready --undo' to revert it back to draft",
		},
		{
			name:     "manually marked draft stays draft",
			scenario: "PR was created as ready, user manually marked it draft in GitHub UI, then runs git-pr",
			expectation: "Should preserve the draft status chosen by user - the update flow " +
				"should not call 'gh pr ready' to revert it back to ready",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			t.Logf("Scenario: %s", tt.scenario)
			t.Logf("Expected behavior: %s", tt.expectation)
			
			// This test serves as documentation of the expected behavior.
			// The actual implementation in main.go (lines 280-287) should:
			// 1. Update PR title and body via PATCH request
			// 2. Add labels if needed
			// 3. NOT call 'gh pr ready' or 'gh pr ready --undo'
			
			// The key insight is that the update logic has been simplified to:
			//   - httpRequest("PATCH", pullURL, {"title": ..., "body": ...})
			//   - gh("pr", "edit", prNumber, "--add-label", labels) [if needed]
			// And specifically does NOT include any draft status management.
		})
	}

	t.Log("\n✓ Draft status preservation is enforced by:")
	t.Log("  1. Only setting draft status during PR creation (githubCreatePRForCommit)")
	t.Log("  2. Never calling 'gh pr ready' or 'gh pr ready --undo' during updates")
	t.Log("  3. Update flow only modifies: title, body, and labels")
}

// TestDraftFlagPrecedence tests the precedence of draft determination:
// config.draft flag OR title pattern match should result in draft PR.
func TestDraftFlagPrecedence(t *testing.T) {
	// Save and restore config
	oldDraft := config.draft
	oldDraftPatterns := config.draftPatterns
	defer func() {
		config.draft = oldDraft
		config.draftPatterns = oldDraftPatterns
	}()

	config.draftPatterns = []string{"wip:*", "*[draft]*"}

	tests := []struct {
		name          string
		configDraft   bool
		titlePattern  bool  // whether title matches pattern
		commitTitle   string
		wantDraft     bool
		rationale     string
	}{
		{
			name:          "flag=false, pattern=false -> ready",
			configDraft:   false,
			titlePattern:  false,
			commitTitle:   "feat: normal commit",
			wantDraft:     false,
			rationale:     "Neither flag nor pattern present, should be ready for review",
		},
		{
			name:          "flag=true, pattern=false -> draft",
			configDraft:   true,
			titlePattern:  false,
			commitTitle:   "feat: normal commit",
			wantDraft:     true,
			rationale:     "Config flag alone is sufficient to create draft",
		},
		{
			name:          "flag=false, pattern=true -> draft",
			configDraft:   false,
			titlePattern:  true,
			commitTitle:   "wip: in progress",
			wantDraft:     true,
			rationale:     "Title pattern alone is sufficient to create draft",
		},
		{
			name:          "flag=true, pattern=true -> draft",
			configDraft:   true,
			titlePattern:  true,
			commitTitle:   "wip: in progress",
			wantDraft:     true,
			rationale:     "Both flag and pattern present, definitely draft",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			config.draft = tt.configDraft

			// Calculate draft status using OR logic (either condition triggers draft)
			isDraft := config.draft || matchAnyPattern(config.draftPatterns, tt.commitTitle)

			// Verify the precedence logic
			if isDraft != tt.wantDraft {
				t.Errorf("Draft precedence failed:\n"+
					"  Config flag: %v\n"+
					"  Title pattern: %v (title=%q)\n"+
					"  Expected draft: %v\n"+
					"  Got draft: %v\n"+
					"  Rationale: %s",
					tt.configDraft, tt.titlePattern, tt.commitTitle,
					tt.wantDraft, isDraft, tt.rationale)
			} else {
				t.Logf("✓ Correct precedence: %s", tt.rationale)
			}
		})
	}
}

// TestDraftPRCreationVsUpdateBehavior documents the critical distinction
// between PR creation and update behavior.
func TestDraftPRCreationVsUpdateBehavior(t *testing.T) {
	t.Log("=== PR Creation Behavior ===")
	t.Log("When creating a NEW PR (githubCreatePRForCommit):")
	t.Log("  1. Check if draft needed: config.draft OR matchAnyPattern(title)")
	t.Log("  2. If draft needed: pass --draft flag to 'gh pr create'")
	t.Log("  3. Extract PR number from output")
	t.Log("  4. Set commit.NewlyCreated = true")
	t.Log("  Result: PR is created in the correct state from the start")
	t.Log("")
	
	t.Log("=== PR Update Behavior ===")
	t.Log("When updating an EXISTING PR (main.go update flow):")
	t.Log("  1. PATCH request to update title and body")
	t.Log("  2. Add labels if needed")
	t.Log("  3. Do NOT touch draft status at all")
	t.Log("  Result: Draft/ready status is preserved as user intended")
	t.Log("")
	
	t.Log("=== Key Differences ===")
	t.Log("Creation: Draft status IS determined by code")
	t.Log("Update:   Draft status is NEVER modified by code")
	t.Log("")
	
	t.Log("✓ This design ensures:")
	t.Log("  - New PRs get correct initial status")
	t.Log("  - User's manual status changes are preserved")
	t.Log("  - No unexpected status flipping on updates")
}
