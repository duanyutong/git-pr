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
			wantPosition:    "This is PR **1 of 3** in a stack (oldest on top)",
			wantOrder:       "oldest on top",
			wantFirstInList: "#101", // oldest first (current commit shows with 👉)
		},
		{
			name:            "oldest commit, reverse=true",
			reverse:         true,
			currentCommit:   commits[0],
			wantPosition:    "This is PR **1 of 3** in a stack (newest on top)",
			wantOrder:       "newest on top",
			wantFirstInList: "#103", // newest first
		},
		{
			name:            "middle commit, reverse=false",
			reverse:         false,
			currentCommit:   commits[1],
			wantPosition:    "This is PR **2 of 3** in a stack (oldest on top)",
			wantOrder:       "oldest on top",
			wantFirstInList: "#101",
		},
		{
			name:            "middle commit, reverse=true",
			reverse:         true,
			currentCommit:   commits[1],
			wantPosition:    "This is PR **2 of 3** in a stack (newest on top)",
			wantOrder:       "newest on top",
			wantFirstInList: "#103",
		},
		{
			name:            "newest commit, reverse=false",
			reverse:         false,
			currentCommit:   commits[2],
			wantPosition:    "This is PR **3 of 3** in a stack (oldest on top)",
			wantOrder:       "oldest on top",
			wantFirstInList: "#101",
		},
		{
			name:            "newest commit, reverse=true",
			reverse:         true,
			currentCommit:   commits[2],
			wantPosition:    "This is PR **3 of 3** in a stack (newest on top)",
			wantOrder:       "newest on top",
			wantFirstInList: "#103",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			config.reverse = tt.reverse
			result := generateStackInfo(commits, tt.currentCommit)

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

	result := generateStackInfo(commits, commits[0])

	// Should NOT contain "This is PR"
	if strings.Contains(result, "This is PR") {
		t.Errorf("generateStackInfo() with single commit should not show position header, got: %v", result)
	}
}

