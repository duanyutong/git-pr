package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestLoadPRTemplate(t *testing.T) {
	// Save original working directory
	originalDir, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	defer os.Chdir(originalDir)

	// Create a temporary directory for testing
	tmpDir, err := os.MkdirTemp("", "git-pr-test-*")
	if err != nil {
		t.Fatal(err)
	}
	defer os.RemoveAll(tmpDir)

	// Change to temp directory
	if err := os.Chdir(tmpDir); err != nil {
		t.Fatal(err)
	}

	tests := []struct {
		name           string
		setup          func() error
		wantContains   string
		wantIsDefault  bool
	}{
		{
			name: "load from .github/PULL_REQUEST_TEMPLATE.md",
			setup: func() error {
				if err := os.MkdirAll(".github", 0755); err != nil {
					return err
				}
				return os.WriteFile(".github/PULL_REQUEST_TEMPLATE.md", []byte("## Custom Template\n\nDescription here"), 0644)
			},
			wantContains:  "## Custom Template",
			wantIsDefault: false,
		},
		{
			name: "load from root PULL_REQUEST_TEMPLATE.md",
			setup: func() error {
				return os.WriteFile("PULL_REQUEST_TEMPLATE.md", []byte("# Root Template\n\nFill this out"), 0644)
			},
			wantContains:  "# Root Template",
			wantIsDefault: false,
		},
		{
			name: "use default when no template found",
			setup: func() error {
				return nil // no template file
			},
			wantContains:  "# Summary",
			wantIsDefault: true,
		},
		{
			name: "prefer .github over root",
			setup: func() error {
				if err := os.MkdirAll(".github", 0755); err != nil {
					return err
				}
				if err := os.WriteFile(".github/PULL_REQUEST_TEMPLATE.md", []byte("## GitHub Template"), 0644); err != nil {
					return err
				}
				return os.WriteFile("PULL_REQUEST_TEMPLATE.md", []byte("## Root Template"), 0644)
			},
			wantContains:  "## GitHub Template",
			wantIsDefault: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// Clean up between tests
			os.RemoveAll(filepath.Join(tmpDir, ".github"))
			os.Remove(filepath.Join(tmpDir, "PULL_REQUEST_TEMPLATE.md"))
			os.Remove(filepath.Join(tmpDir, "pull_request_template.md"))

			// Setup
			if err := tt.setup(); err != nil {
				t.Fatalf("setup failed: %v", err)
			}

			// Test
			result := loadPRTemplate()

			// Verify
			if !strings.Contains(result, tt.wantContains) {
				t.Errorf("loadPRTemplate() = %q, want to contain %q", result, tt.wantContains)
			}

			if tt.wantIsDefault {
				if result != bodyTemplate {
					t.Errorf("loadPRTemplate() should return default template, got %q", result)
				}
			} else {
				if result == bodyTemplate {
					t.Errorf("loadPRTemplate() should not return default template")
				}
			}
		})
	}
}

func TestGetPRTemplateCaching(t *testing.T) {
	// Save and restore config
	oldTemplate := config.prTemplate
	defer func() { config.prTemplate = oldTemplate }()

	// Reset cache
	config.prTemplate = ""

	// First call should load and cache
	template1 := getPRTemplate()
	if config.prTemplate == "" {
		t.Error("getPRTemplate() should cache the template")
	}

	// Second call should return cached value
	template2 := getPRTemplate()
	if template1 != template2 {
		t.Error("getPRTemplate() should return same cached value")
	}
}
