package main

import (
	"strings"
	"testing"
)

func TestFormatKey(t *testing.T) {
	cases := []struct {
		in, want string
	}{
		{"remote-ref", "Remote-Ref"},
		{"", ""},
		{"single", "Single"},
		{"ALL-CAPS", "All-Caps"},
		{"a-b-c", "A-B-C"},
		{"trailing-", "Trailing-"},
		{"--double", "--Double"},
		{"MixedCase-key", "Mixedcase-Key"},
	}
	for _, tc := range cases {
		if got := formatKey(tc.in); got != tc.want {
			t.Errorf("formatKey(%q) = %q, want %q", tc.in, got, tc.want)
		}
	}
}

func TestGenerateStackInfo(t *testing.T) {
	// Setup test data
	commits := []*Commit{
		{Hash: "abc12345", Title: "chore: first commit", PRNumber: 101},
		{Hash: "def45678", Title: "feat: second commit", PRNumber: 102},
		{Hash: "ghi78901", Title: "fix: third commit", PRNumber: 103},
	}

	// Mock config
	oldReverse := config.reverse
	oldHost := config.git.host
	oldRepo := config.git.repo
	defer func() {
		config.reverse = oldReverse
		config.git.host = oldHost
		config.git.repo = oldRepo
	}()
	config.git.host = "github.com"
	config.git.repo = "user/repo"

	tests := []struct {
		name            string
		reverse         bool
		currentCommit   *Commit
		wantPosition    string
		wantOrder       string
		wantFirstInList string // What should appear first in the list
	}{
		{
			name:            "oldest commit, reverse=false",
			reverse:         false,
			currentCommit:   commits[0],
			wantPosition:    "This is PR **1 of 3** in a stack (oldest at the top)",
			wantOrder:       "oldest at the top",
			wantFirstInList: "#101", // oldest first (current commit shows with 👉)
		},
		{
			name:            "oldest commit, reverse=true",
			reverse:         true,
			currentCommit:   commits[0],
			wantPosition:    "This is PR **1 of 3** in a stack (newest at the top)",
			wantOrder:       "newest at the top",
			wantFirstInList: "#103", // newest first
		},
		{
			name:            "middle commit, reverse=false",
			reverse:         false,
			currentCommit:   commits[1],
			wantPosition:    "This is PR **2 of 3** in a stack (oldest at the top)",
			wantOrder:       "oldest at the top",
			wantFirstInList: "#101",
		},
		{
			name:            "middle commit, reverse=true",
			reverse:         true,
			currentCommit:   commits[1],
			wantPosition:    "This is PR **2 of 3** in a stack (newest at the top)",
			wantOrder:       "newest at the top",
			wantFirstInList: "#103",
		},
		{
			name:            "newest commit, reverse=false",
			reverse:         false,
			currentCommit:   commits[2],
			wantPosition:    "This is PR **3 of 3** in a stack (oldest at the top)",
			wantOrder:       "oldest at the top",
			wantFirstInList: "#101",
		},
		{
			name:            "newest commit, reverse=true",
			reverse:         true,
			currentCommit:   commits[2],
			wantPosition:    "This is PR **3 of 3** in a stack (newest at the top)",
			wantOrder:       "newest at the top",
			wantFirstInList: "#103",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			config.reverse = tt.reverse
			result := generateStackInfo(commits, tt.currentCommit, nil, nil)

			// Check position header
			if !strings.Contains(result, tt.wantPosition) {
				t.Errorf("generateStackInfo() position header = %v, want to contain %v", result, tt.wantPosition)
			}

			// Check order note
			if !strings.Contains(result, tt.wantOrder) {
				t.Errorf("generateStackInfo() order = %v, want to contain %v", result, tt.wantOrder)
			}

			// Check first item in list (verify reverse works)
			lines := strings.Split(result, "\n")
			var firstListItem string
			for _, line := range lines {
				if strings.HasPrefix(line, "* ") {
					firstListItem = line
					break
				}
			}
			if !strings.Contains(firstListItem, tt.wantFirstInList) {
				t.Errorf("generateStackInfo() first list item = %v, want to contain %v", firstListItem, tt.wantFirstInList)
			}
		})
	}
}

func TestGenerateStackInfoSingleCommit(t *testing.T) {
	// Single commit shouldn't show position header
	commits := []*Commit{
		{Hash: "abc12345", Title: "single commit", PRNumber: 101},
	}

	config.git.host = "github.com"
	config.git.repo = "user/repo"

	result := generateStackInfo(commits, commits[0], nil, nil)

	// Should NOT contain "This is PR"
	if strings.Contains(result, "This is PR") {
		t.Errorf("generateStackInfo() with single commit should not show position header, got: %v", result)
	}
}

func TestGenerateStackInfoPreservesMergedPRs(t *testing.T) {
	config.git.host = "github.com"
	config.git.repo = "user/repo"

	// Simulate scenario: originally had 3 PRs (#101, #102, #103), but PR #101 merged
	// Current stack only has 2 commits (PR #101 merged and removed from stack)
	commits := []*Commit{
		{Hash: "def67890", Title: "second commit", PRNumber: 102},
		{Hash: "ghi13579", Title: "third commit", PRNumber: 103},
	}

	tests := []struct {
		name                string
		reverse             bool
		currentCommit       *Commit
		wantPosition        string
		wantMergedPRPresent bool
		wantMergedPRFirst   bool // true if merged PR should be first in list
	}{
		{
			name:                "preserve merged PR #101, normal order",
			reverse:             false,
			currentCommit:       commits[0], // PR #102
			wantPosition:        "This is PR **2 of 3** in a stack",
			wantMergedPRPresent: true,
			wantMergedPRFirst:   true, // merged PR at top (oldest first)
		},
		{
			name:                "preserve merged PR #101, reverse order",
			reverse:             true,
			currentCommit:       commits[1], // PR #103
			wantPosition:        "This is PR **3 of 3** in a stack",
			wantMergedPRPresent: true,
			wantMergedPRFirst:   false, // merged PR at bottom (newest first)
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			config.reverse = tt.reverse
			// Pass the historical PR entries: #101 is merged, #102 and #103 are active
			allHistoricalPRs := []PRHistoryEntry{
				{Number: 101, IsMerged: true},  // was merged
				{Number: 102, IsMerged: false}, // active, in current stack
				{Number: 103, IsMerged: false}, // active, in current stack
			}
			result := generateStackInfo(commits, tt.currentCommit, allHistoricalPRs, nil)

			// Should preserve merged PR count in position header
			if !strings.Contains(result, tt.wantPosition) {
				t.Errorf("Want position %q in result:\n%v", tt.wantPosition, result)
			}

			// Should include merged PR with checkmark
			if tt.wantMergedPRPresent {
				if !strings.Contains(result, "✔️ #101") {
					t.Errorf("Want merged PR '✔️ #101' in result:\n%v", result)
				}
			}

			// Verify merged PR position relative to other PRs
			lines := strings.Split(result, "\n")
			var firstPRLine, lastPRLine string
			for _, line := range lines {
				if strings.HasPrefix(line, "* ") {
					if firstPRLine == "" {
						firstPRLine = line
					}
					lastPRLine = line
				}
			}

			if tt.wantMergedPRFirst {
				if !strings.Contains(firstPRLine, "✔️ #101") {
					t.Errorf("Want merged PR first (normal order), got first line: %v", firstPRLine)
				}
			} else {
				if !strings.Contains(lastPRLine, "✔️ #101") {
					t.Errorf("Want merged PR last (reverse order), got last line: %v", lastPRLine)
				}
			}
		})
	}
}

func TestExtractPRNumbersFromStackInfo(t *testing.T) {
	tests := []struct {
		name       string
		body       string
		wantPRNums []int
	}{
		{
			name: "extract from body with sentinels",
			body: `Description

---
` + stackInfoStartMarker + `
This is PR **2 of 3** in a stack

* ◻️ #101
* 👉 #102
* ◻️ #103
` + stackInfoEndMarker,
			wantPRNums: []int{101, 102, 103},
		},
		{
			name: "extract from old format without sentinels",
			body: `Description

---
This is PR **1 of 2** in a stack

* ◻️ #100
* 🦊 #200
`,
			wantPRNums: []int{100, 200},
		},
		{
			name: "extract with merged PRs",
			body: `---
` + stackInfoStartMarker + `
* ✔️ #50
* ✔️ #51
* ◻️ #52
` + stackInfoEndMarker,
			wantPRNums: []int{50, 51, 52},
		},
		{
			name:       "empty body",
			body:       "",
			wantPRNums: nil,
		},
		{
			name:       "no stack info in body",
			body:       "Just a description\n\nNo PRs here",
			wantPRNums: nil,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := extractPRNumbersFromStackInfo(tt.body)
			
			if len(result) != len(tt.wantPRNums) {
				t.Errorf("extractPRNumbersFromStackInfo() = %v, want %v", result, tt.wantPRNums)
				return
			}
			
			for i, prNum := range result {
				if prNum != tt.wantPRNums[i] {
					t.Errorf("extractPRNumbersFromStackInfo()[%d] = %v, want %v", i, prNum, tt.wantPRNums[i])
				}
			}
		})
	}
}

func TestGenerateStackInfoInsertsNewPRInMiddle(t *testing.T) {
	// Simulate inserting a new commit/PR in the middle of an existing stack
	// Current stack (oldest to newest): #101, #104 (NEW), #102, #103
	commits := []*Commit{
		{Hash: "abc12345", PRNumber: 101, Title: "First commit"},
		{Hash: "new99999", PRNumber: 104, Title: "Inserted in middle"}, // NEW PR
		{Hash: "def67890", PRNumber: 102, Title: "Second commit"},
		{Hash: "ghi13579", PRNumber: 103, Title: "Third commit"},
	}
	
	// Historical (from existing PRs): only has #101, #102, #103
	allHistoricalPRs := []PRHistoryEntry{
		{Number: 101, IsMerged: false},
		{Number: 102, IsMerged: false},
		{Number: 103, IsMerged: false},
	}
	
	config.reverse = false
	result := generateStackInfo(commits, commits[1], allHistoricalPRs, nil)
	
	// The new PR #104 should be inserted between #101 and #102
	// Expected order (oldest first): #101, #104, #102, #103
	lines := strings.Split(result, "\n")
	var prOrder []string
	for _, line := range lines {
		if strings.HasPrefix(line, "* ") {
			// Extract PR number
			if idx := strings.Index(line, "#"); idx >= 0 {
				prNum := ""
				for i := idx + 1; i < len(line) && line[i] >= '0' && line[i] <= '9'; i++ {
					prNum += string(line[i])
				}
				prOrder = append(prOrder, prNum)
			}
		}
	}
	
	expectedOrder := []string{"101", "104", "102", "103"}
	if len(prOrder) != len(expectedOrder) {
		t.Fatalf("Expected %d PRs, got %d: %v\nResult:\n%s", len(expectedOrder), len(prOrder), prOrder, result)
	}
	
	for i, want := range expectedOrder {
		if prOrder[i] != want {
			t.Errorf("Position %d: want PR #%s, got #%s\nFull order: %v\nResult:\n%s", i, want, prOrder[i], prOrder, result)
		}
	}
}

func TestExtractPRHistoryNormalizesReversedOrder(t *testing.T) {
	// When the stored order is "newest at the top", extractPRHistoryFromStackInfo
	// should normalize it back to internal order (oldest first)
	
	tests := []struct {
		name     string
		body     string
		wantNums []int // Expected PR numbers in internal order (oldest first)
	}{
		{
			name: "normalizes newest-at-top to oldest-first",
			body: `---
` + stackInfoStartMarker + `
This is PR **2 of 3** in a stack (newest at the top)

* ⬛ #103
* 🐼 #102 👈 This PR
* ⬛ #101
` + stackInfoEndMarker,
			wantNums: []int{101, 102, 103}, // Should be reversed to oldest-first
		},
		{
			name: "keeps oldest-at-top as-is",
			body: `---
` + stackInfoStartMarker + `
This is PR **2 of 3** in a stack (oldest at the top)

* ⬛ #101
* 🐼 #102 👈 This PR
* ⬛ #103
` + stackInfoEndMarker,
			wantNums: []int{101, 102, 103}, // Already in correct order
		},
		{
			name: "legacy format without order note stays as-is",
			body: `---
` + stackInfoStartMarker + `
This is PR **2 of 3** in a stack

* ⬛ #101
* 🐼 #102
* ⬛ #103
` + stackInfoEndMarker,
			wantNums: []int{101, 102, 103}, // Legacy = oldest-first (default)
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			entries := extractPRHistoryFromStackInfo(tt.body)
			
			if len(entries) != len(tt.wantNums) {
				t.Fatalf("Got %d entries, want %d", len(entries), len(tt.wantNums))
			}
			
			for i, entry := range entries {
				if entry.Number != tt.wantNums[i] {
					t.Errorf("Position %d: got PR #%d, want #%d", i, entry.Number, tt.wantNums[i])
				}
			}
		})
	}
}

func TestGenerateStackInfoMarksDownstackAsMerged(t *testing.T) {
	// When a PR is not in the current local stack but appears in history,
	// it should be marked as merged (✔️) ONLY if it's downstack (before current PR)
	
	// Setup: current stack has #102, #103 but history has #101, #102, #103, #104
	// #101 is downstack (should be ✔️), #104 is upstack (should stay ⬛)
	commits := []*Commit{
		{Hash: "def67890", PRNumber: 102, Title: "Second commit"},
		{Hash: "ghi13579", PRNumber: 103, Title: "Third commit"},
	}
	
	// Historical PRs include #101 (not in stack anymore) and #104 (upstack, also not in stack)
	allHistoricalPRs := []PRHistoryEntry{
		{Number: 101, IsMerged: true},  // downstack, marked as merged during accumulation
		{Number: 102, IsMerged: false}, // in current stack
		{Number: 103, IsMerged: false}, // in current stack
		{Number: 104, IsMerged: true},  // upstack - wrongly marked during accumulation, should be fixed
	}
	
	// Save and restore config
	oldReverse := config.reverse
	oldHost := config.git.host
	oldRepo := config.git.repo
	defer func() {
		config.reverse = oldReverse
		config.git.host = oldHost
		config.git.repo = oldRepo
	}()
	config.reverse = false
	config.git.host = "github.com"
	config.git.repo = "user/repo"
	
	// Current commit is #102
	result := generateStackInfo(commits, commits[0], allHistoricalPRs, nil)
	
	// #101 should have ✔️ (downstack, merged)
	if !strings.Contains(result, "✔️ #101") {
		t.Errorf("Downstack PR #101 should be marked as merged (✔️)\nResult:\n%s", result)
	}
	
	// #104 should have ⬛ (upstack, NOT marked as merged even though not in local stack)
	if strings.Contains(result, "✔️ #104") {
		t.Errorf("Upstack PR #104 should NOT be marked as merged\nResult:\n%s", result)
	}
	if !strings.Contains(result, "⬛ #104") {
		t.Errorf("Upstack PR #104 should have ⬛ marker\nResult:\n%s", result)
	}
}

func TestGenerateStackInfoMarksMergedInLocalStack(t *testing.T) {
	// When a PR is still in the current local stack but already merged on GitHub
	// (e.g. user hasn't pulled/rebased since merge), it should render as ✔️ rather than ⬛.
	commits := []*Commit{
		{Hash: "aaa11111", PRNumber: 101, Title: "First"},
		{Hash: "bbb22222", PRNumber: 102, Title: "Second (merged on GitHub)"},
		{Hash: "ccc33333", PRNumber: 103, Title: "Third (current)"},
	}
	allHistoricalPRs := []PRHistoryEntry{
		{Number: 101, IsMerged: false},
		{Number: 102, IsMerged: false},
		{Number: 103, IsMerged: false},
	}
	mergedPRs := map[int]bool{102: true}

	oldReverse := config.reverse
	oldHost := config.git.host
	oldRepo := config.git.repo
	defer func() {
		config.reverse = oldReverse
		config.git.host = oldHost
		config.git.repo = oldRepo
	}()
	config.reverse = false
	config.git.host = "github.com"
	config.git.repo = "user/repo"

	result := generateStackInfo(commits, commits[2], allHistoricalPRs, mergedPRs)

	if !strings.Contains(result, "✔️ #102") {
		t.Errorf("Merged PR #102 in local stack should be marked ✔️\nResult:\n%s", result)
	}
	if strings.Contains(result, "⬛ #102") {
		t.Errorf("Merged PR #102 should not have ⬛ marker\nResult:\n%s", result)
	}
}