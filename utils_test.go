package main

import (
	"strings"
	"testing"
)

func TestFormatKey(t *testing.T) {
	out := formatKey("remote-ref")
	if out != "Remote-Ref" {
		t.Errorf("formatKey() = %v, want %v", out, "Remote-Ref")
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
			result := generateStackInfo(commits, tt.currentCommit, nil)

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

	result := generateStackInfo(commits, commits[0], nil)

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
			// Pass the historical PR numbers (101, 102, 103)
			allHistoricalPRs := []int{101, 102, 103}
			result := generateStackInfo(commits, tt.currentCommit, allHistoricalPRs)

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

