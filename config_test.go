package main

import (
	"testing"
)

func TestMatchWildcard(t *testing.T) {
	t.Run("exact matches", func(t *testing.T) {
		tests := []struct {
			name     string
			pattern  string
			text     string
			expected bool
		}{
			{name: "identical strings", pattern: "hello", text: "hello", expected: true},
			{name: "case insensitive uppercase pattern", pattern: "HELLO", text: "hello", expected: true},
			{name: "case insensitive lowercase pattern", pattern: "hello", text: "HELLO", expected: true},
			{name: "case insensitive mixed case", pattern: "HeLLo", text: "hElLO", expected: true},
			{name: "different strings", pattern: "hello", text: "world", expected: false},
			{name: "empty pattern and text", pattern: "", text: "", expected: true},
			{name: "empty pattern non-empty text", pattern: "", text: "hello", expected: false},
			{name: "non-empty pattern empty text", pattern: "hello", text: "", expected: false},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchWildcard(tt.pattern, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchWildcard(%q, %q) = %v, want %v",
						tt.pattern, tt.text, result, tt.expected)
			})
		}
	})

	t.Run("asterisk wildcard", func(t *testing.T) {
		tests := []struct {
			name     string
			pattern  string
			text     string
			expected bool
		}{
			{name: "only asterisk matches empty", pattern: "*", text: "", expected: true},
			{name: "only asterisk matches anything", pattern: "*", text: "hello world", expected: true},
			{name: "asterisk at start", pattern: "*world", text: "hello world", expected: true},
			{name: "asterisk at end", pattern: "hello*", text: "hello world", expected: true},
			{name: "asterisk in middle", pattern: "hello*world", text: "hello beautiful world", expected: true},
			{name: "asterisk matches zero chars", pattern: "hello*world", text: "helloworld", expected: true},
			{name: "asterisk matches single char", pattern: "hello*world", text: "hello world", expected: true},
			{name: "asterisk matches multiple chars", pattern: "hello*world", text: "hello big beautiful world", expected: true},
			{name: "asterisk at start no match", pattern: "*world", text: "hello", expected: false},
			{name: "asterisk at end no match", pattern: "hello*", text: "world", expected: false},
			{name: "multiple asterisks", pattern: "a*b*c", text: "aXXbYYc", expected: true},
			{name: "consecutive asterisks", pattern: "a**b", text: "aXXXb", expected: true},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchWildcard(tt.pattern, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchWildcard(%q, %q) = %v, want %v",
						tt.pattern, tt.text, result, tt.expected)
			})
		}
	})

	t.Run("question mark wildcard", func(t *testing.T) {
		tests := []struct {
			name     string
			pattern  string
			text     string
			expected bool
		}{
			{name: "question matches one char", pattern: "test?", text: "test1", expected: true},
			{name: "question matches letter", pattern: "test?", text: "testa", expected: true},
			{name: "question not match empty", pattern: "test?", text: "test", expected: false},
			{name: "question not match multiple", pattern: "test?", text: "test12", expected: false},
			{name: "multiple questions", pattern: "test???", text: "test123", expected: true},
			{name: "multiple questions no match", pattern: "test???", text: "test12", expected: false},
			{name: "question in middle", pattern: "te?t", text: "test", expected: true},
			{name: "question at start", pattern: "?est", text: "test", expected: true},
			{name: "consecutive questions", pattern: "a??b", text: "aXYb", expected: true},
			{name: "consecutive questions no match", pattern: "a??b", text: "aXb", expected: false},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchWildcard(tt.pattern, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchWildcard(%q, %q) = %v, want %v",
						tt.pattern, tt.text, result, tt.expected)
			})
		}
	})

	t.Run("combined wildcards", func(t *testing.T) {
		tests := []struct {
			name     string
			pattern  string
			text     string
			expected bool
		}{
			{name: "asterisk and question", pattern: "a*b?c", text: "aXXXbYc", expected: true},
			{name: "question and asterisk", pattern: "a?b*c", text: "aXbYYYc", expected: true},
			{name: "multiple of each", pattern: "a*b?c*d", text: "aXbYcZZd", expected: true},
			{name: "complex pattern match", pattern: "a??b*c*d", text: "aXYbZZcWWd", expected: true},
			{name: "complex pattern no match", pattern: "a??b*c*d", text: "aXbZZcWWd", expected: false},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchWildcard(tt.pattern, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchWildcard(%q, %q) = %v, want %v",
						tt.pattern, tt.text, result, tt.expected)
			})
		}
	})

	t.Run("backtracking scenarios", func(t *testing.T) {
		tests := []struct {
			name     string
			pattern  string
			text     string
			expected bool
		}{
			{name: "simple backtrack", pattern: "a*a", text: "aaa", expected: true},
			{name: "simple backtrack no match", pattern: "a*a", text: "abb", expected: false},
			{name: "multiple asterisk backtrack", pattern: "a*b*c", text: "aXXbYYc", expected: true},
			{name: "greedy then backtrack", pattern: "a*ab", text: "aaab", expected: true},
			{name: "complex backtrack", pattern: "*ab*cd*", text: "XXabYYcdZZ", expected: true},
			{name: "worst case backtrack", pattern: "a*a*a*a*b", text: "aaaaaaaaac", expected: false},
			{name: "pattern longer than text", pattern: "abcdefghijk", text: "abc", expected: false},
			{name: "text longer than pattern", pattern: "abc", text: "abcdefghijk", expected: false},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchWildcard(tt.pattern, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchWildcard(%q, %q) = %v, want %v",
						tt.pattern, tt.text, result, tt.expected)
			})
		}
	})

	t.Run("production draft pattern", func(t *testing.T) {
		tests := []struct {
			name     string
			pattern  string
			text     string
			expected bool
		}{
			{name: "draft at end", pattern: "*[draft]*", text: "feat: add feature [draft]", expected: true},
			{name: "draft at start", pattern: "*[draft]*", text: "[draft] work in progress", expected: true},
			{name: "draft in middle", pattern: "*[draft]*", text: "some [draft] commit", expected: true},
			{name: "draft uppercase", pattern: "*[draft]*", text: "feat: [DRAFT] feature", expected: true},
			{name: "draft mixed case", pattern: "*[draft]*", text: "feat: [DrAfT] feature", expected: true},
			{name: "draft only", pattern: "*[draft]*", text: "[draft]", expected: true},
			{name: "no brackets", pattern: "*[draft]*", text: "draft without brackets", expected: false},
			{name: "partial match no brackets", pattern: "*[draft]*", text: "draft", expected: false},
			{name: "no draft word", pattern: "*[draft]*", text: "feat: completed feature", expected: false},
			{name: "draft random pattern", pattern: "*[draft]*", text: "[draft][random] this is an example", expected: true},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchWildcard(tt.pattern, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchWildcard(%q, %q) = %v, want %v",
						tt.pattern, tt.text, result, tt.expected)
			})
		}
	})

	t.Run("new default patterns", func(t *testing.T) {
		tests := []struct {
			name     string
			pattern  string
			text     string
			expected bool
		}{
			// wip: prefix pattern
			{name: "wip prefix match", pattern: "wip:*", text: "wip: working on feature", expected: true},
			{name: "wip prefix uppercase", pattern: "wip:*", text: "WIP: Feature", expected: true},
			{name: "wip prefix no match", pattern: "wip:*", text: "feat: wip inside", expected: false},

			// draft: prefix pattern
			{name: "draft prefix match", pattern: "draft:*", text: "draft: new feature", expected: true},
			{name: "draft prefix uppercase", pattern: "draft:*", text: "DRAFT: Feature", expected: true},
			{name: "draft prefix no match", pattern: "draft:*", text: "feat: draft inside", expected: false},

			// [wip] bracket pattern
			{name: "wip bracket at start", pattern: "*[wip]*", text: "[wip] something", expected: true},
			{name: "wip bracket in middle", pattern: "*[wip]*", text: "feat [wip] something", expected: true},
			{name: "wip bracket at end", pattern: "*[wip]*", text: "feat: something [wip]", expected: true},
			{name: "wip bracket uppercase", pattern: "*[wip]*", text: "feat [WIP] something", expected: true},
			{name: "wip no brackets", pattern: "*[wip]*", text: "wip something", expected: false},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchWildcard(tt.pattern, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchWildcard(%q, %q) = %v, want %v",
						tt.pattern, tt.text, result, tt.expected)
			})
		}
	})

	t.Run("realistic patterns", func(t *testing.T) {
		tests := []struct {
			name     string
			pattern  string
			text     string
			expected bool
		}{
			{name: "wip prefix", pattern: "wip:*", text: "wip: working on feature", expected: true},
			{name: "wip uppercase", pattern: "wip:*", text: "WIP: Working On Feature", expected: true},
			{name: "wip no match", pattern: "wip:*", text: "feat: new feature", expected: false},
			{name: "todo anywhere", pattern: "*TODO*", text: "fix: resolve TODO in code", expected: true},
			{name: "skip ci pattern", pattern: "*[skip ci]*", text: "docs: update README [skip ci]", expected: true},
			{name: "draft prefix", pattern: "draft*", text: "draft: new feature", expected: true},
			{name: "draft suffix", pattern: "*-draft", text: "feature-draft", expected: true},
			{name: "emoji in text", pattern: "*[draft]*", text: "🛠️ [draft] feature", expected: true},
			{name: "emoji in pattern", pattern: "🛠️*", text: "🛠️ fix something", expected: true},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchWildcard(tt.pattern, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchWildcard(%q, %q) = %v, want %v",
						tt.pattern, tt.text, result, tt.expected)
			})
		}
	})

	t.Run("edge cases", func(t *testing.T) {
		tests := []struct {
			name     string
			pattern  string
			text     string
			expected bool
		}{
			{name: "special chars literal", pattern: "a.b", text: "a.b", expected: true},
			{name: "special chars no regex", pattern: "a.b", text: "aXb", expected: false},
			{name: "brackets literal", pattern: "[abc]", text: "[abc]", expected: true},
			{name: "brackets not character class", pattern: "[abc]", text: "a", expected: false},
			{name: "hyphen literal", pattern: "a-z", text: "a-z", expected: true},
			{name: "hyphen not range", pattern: "a-z", text: "m", expected: false},
			{name: "plus literal", pattern: "a+", text: "a+", expected: true},
			{name: "parens literal", pattern: "(test)", text: "(test)", expected: true},
			{name: "long pattern", pattern: "a*b*c*d*e*f*g", text: "aXbXcXdXeXfXg", expected: true},
			{name: "many wildcards", pattern: "????????", text: "12345678", expected: true},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchWildcard(tt.pattern, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchWildcard(%q, %q) = %v, want %v",
						tt.pattern, tt.text, result, tt.expected)
			})
		}
	})
}

func TestMatchAnyPattern(t *testing.T) {
	t.Run("empty patterns", func(t *testing.T) {
		result := matchAnyPattern([]string{}, "test")
		assert(t, result == false).
			Errorf("matchAnyPattern([], %q) = %v, want false", "test", result)
	})

	t.Run("single pattern", func(t *testing.T) {
		tests := []struct {
			name     string
			patterns []string
			text     string
			expected bool
		}{
			{name: "match", patterns: []string{"*draft*"}, text: "[draft] feature", expected: true},
			{name: "no match", patterns: []string{"*draft*"}, text: "feat: feature", expected: false},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchAnyPattern(tt.patterns, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchAnyPattern(%v, %q) = %v, want %v",
						tt.patterns, tt.text, result, tt.expected)
			})
		}
	})

	t.Run("multiple patterns OR logic", func(t *testing.T) {
		tests := []struct {
			name     string
			patterns []string
			text     string
			expected bool
		}{
			{name: "first pattern matches", patterns: []string{"wip:*", "draft:*"}, text: "wip: feature", expected: true},
			{name: "second pattern matches", patterns: []string{"wip:*", "draft:*"}, text: "draft: feature", expected: true},
			{name: "no pattern matches", patterns: []string{"wip:*", "draft:*"}, text: "feat: feature", expected: false},
			{name: "middle pattern matches", patterns: []string{"wip:*", "*[draft]*", "todo:*"}, text: "[draft] something", expected: true},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchAnyPattern(tt.patterns, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchAnyPattern(%v, %q) = %v, want %v",
						tt.patterns, tt.text, result, tt.expected)
			})
		}
	})

	t.Run("full default pattern", func(t *testing.T) {
		// test the actual default pattern: wip:*,draft:*,*[wip]*,*[draft]*
		patterns := []string{"wip:*", "draft:*", "*[wip]*", "*[draft]*"}

		tests := []struct {
			name     string
			text     string
			expected bool
		}{
			// should match
			{name: "wip prefix", text: "wip: working on feature", expected: true},
			{name: "WIP uppercase prefix", text: "WIP: Feature", expected: true},
			{name: "draft prefix", text: "draft: new API", expected: true},
			{name: "DRAFT uppercase prefix", text: "DRAFT: New Feature", expected: true},
			{name: "wip bracket", text: "[wip] something", expected: true},
			{name: "draft bracket", text: "[draft] something", expected: true},
			{name: "wip bracket middle", text: "feat: [wip] feature", expected: true},
			{name: "draft bracket end", text: "fix: bug [draft]", expected: true},

			// should not match
			{name: "completed feature", text: "feat: completed feature", expected: false},
			{name: "fix without markers", text: "fix: bug in code", expected: false},
			{name: "wip inside word", text: "feat: swipe gesture", expected: false},
			{name: "draft inside word", text: "feat: redraft document", expected: false},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				result := matchAnyPattern(patterns, tt.text)
				assert(t, result == tt.expected).
					Errorf("matchAnyPattern(%v, %q) = %v, want %v",
						patterns, tt.text, result, tt.expected)
			})
		}
	})
}
