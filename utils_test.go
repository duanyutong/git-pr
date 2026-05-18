package main

import "testing"

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
