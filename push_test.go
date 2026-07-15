package main

import (
	"errors"
	"reflect"
	"testing"
)

func TestPushRemoteRef(t *testing.T) {
	const (
		remote       = "origin"
		remoteRef    = "alice/topic"
		fullRef      = "refs/heads/alice/topic"
		sourceHash   = "01234567"
		expectedHash = "0123456789abcdef0123456789abcdef01234567"
		otherHash    = "abcdef0123456789abcdef0123456789abcdef01"
	)
	pushErr := errors.New("cannot lock ref")
	verifyErr := errors.New("network unavailable")

	tests := []struct {
		name         string
		responses    []commandResponse
		wantOutput   string
		wantErr      error
		wantCommands [][]string
	}{
		{
			name:       "successful push",
			responses:  []commandResponse{{output: "pushed"}},
			wantOutput: "pushed",
			wantCommands: [][]string{
				{"push", "-f", remote, sourceHash + ":" + fullRef},
			},
		},
		{
			name: "failed push whose postcondition is already satisfied",
			responses: []commandResponse{
				{err: pushErr},
				{output: expectedHash + "\t" + fullRef + "\n"},
			},
			wantCommands: [][]string{
				{"push", "-f", remote, sourceHash + ":" + fullRef},
				{"ls-remote", "--refs", remote, fullRef},
			},
		},
		{
			name: "failed push with divergent remote ref",
			responses: []commandResponse{
				{err: pushErr},
				{output: otherHash + "\t" + fullRef + "\n"},
			},
			wantErr: pushErr,
			wantCommands: [][]string{
				{"push", "-f", remote, sourceHash + ":" + fullRef},
				{"ls-remote", "--refs", remote, fullRef},
			},
		},
		{
			name: "failed push and failed verification",
			responses: []commandResponse{
				{err: pushErr},
				{err: verifyErr},
			},
			wantErr: pushErr,
			wantCommands: [][]string{
				{"push", "-f", remote, sourceHash + ":" + fullRef},
				{"ls-remote", "--refs", remote, fullRef},
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var commands [][]string
			nextResponse := 0
			run := func(args ...string) (string, error) {
				commands = append(commands, append([]string(nil), args...))
				response := tt.responses[nextResponse]
				nextResponse++
				return response.output, response.err
			}

			output, err := pushRemoteRef(run, remote, sourceHash, expectedHash, remoteRef)
			if output != tt.wantOutput {
				t.Fatalf("output = %q, want %q", output, tt.wantOutput)
			}
			if !errors.Is(err, tt.wantErr) {
				t.Fatalf("error = %v, want %v", err, tt.wantErr)
			}
			if !reflect.DeepEqual(commands, tt.wantCommands) {
				t.Fatalf("commands = %#v, want %#v", commands, tt.wantCommands)
			}
			if nextResponse != len(tt.responses) {
				t.Fatalf("used %d responses, want %d", nextResponse, len(tt.responses))
			}
		})
	}
}

type commandResponse struct {
	output string
	err    error
}

func TestRemoteRefMatchesRequiresExactHashAndRef(t *testing.T) {
	const hash = "0123456789abcdef0123456789abcdef01234567"
	const ref = "refs/heads/alice/topic"
	output := hash + "\t" + ref + "\n" +
		"abcdef0123456789abcdef0123456789abcdef01\trefs/heads/alice/other\n"

	if !remoteRefMatches(output, ref, hash) {
		t.Fatal("expected exact hash and ref to match")
	}
	if remoteRefMatches(output, ref, hash[:8]) {
		t.Fatal("short hash must not match")
	}
	if remoteRefMatches(output, "refs/heads/alice/other", hash) {
		t.Fatal("different ref must not match")
	}
}
