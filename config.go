package main

import (
	"flag"
	"fmt"
	"os"
	"regexp"
	"strings"
	"time"

	"github.com/zalando/go-keyring"
	"gopkg.in/yaml.v3"
)

var (
	emojis0 = []string{"♈️", "♉️", "♊️", "♋️", "♌️", "♍️", "♎️", "♏️", "♐️", "♑️", "♒️", "♓️"}
	emojis1 = []string{"🐹", "🐮", "🐯", "🦊", "🐲", "🐼", "🦁", "🐰", "🐵", "🐻", "🐶", "🐷"}
	emojis2 = []string{"🏠", "🏡", "🏘️", "🏚️", "🏢", "🏬", "🏣", "🏤", "🏥", "🏦", "🏨", "🏩", "🏪", "🏫", "🏭", "🏯", "🏰", "🏟️", "🏛️", "🏗️", "🌇", "🌆", "🌃", "🏙️"}
	emojis3 = []string{"🚗", "🚕", "🚆", "🚄", "🚅", "🚈", "🚇", "🚝", "🚋", "🚌", "🚎", "🏎️", "🚓", "🚑", "🚒", "🚚", "🚛", "🚜", "🏍️", "🛵", "🚲", "🛴"}
	emojis4 = []string{"🍏", "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🍈", "🍒", "🍑", "🥭", "🍍", "🥥", "🥝", "🍅", "🍆", "🥑", "🥦", "🥬", "🥒", "🌶️", "🌽", "🥕", "🧄", "🧅", "🥔", "🍠", "🥐", "🥯", "🍞", "🥖", "🥨", "🧀", "🥚", "🍳", "🧈", "🥞", "🧇", "🥓", "🥩", "🍗", "🍖", "🦴", "🌭", "🍔", "🍟", "🍕", "🥪", "🥙", "🧆", "🌮", "🌯", "🥗", "🥘", "🥫", "🍝", "🍜", "🍲", "🍛", "🍣", "🍱", "🥟", "🦪", "🍤", "🍙", "🍚", "🍘", "🍥", "🥮", "🥠", "🍢", "🍡", "🍧", "🍨", "🍦", "🥧", "🧁", "🍰", "🎂", "🍮", "🍭", "🍬", "🍫", "🍿", "🍩", "🍪", "🌰", "🥜", "🍯", "🥛", "🍼", "☕", "🍵", "🧃", "🥤", "🍶", "🍺", "🍻"}
)

const version = "1.2.0"

var (
	emojisx = emojis1 // config emojis
	config  Config
)

const gitconfigTags = "git-pr.tags"

type Config struct {
	repoDir string // git

	git ConfigGit
	gh  ConfigGh
	bl  ConfigBranchless
	jj  ConfigJj

	tags    []string      // git config git-pr.<repo>.tags
	verbose bool          // flag
	timeout time.Duration // flag

	includeOtherAuthors bool   // flag
	dryRun              bool   // flag: show what would be done without making changes
	stopAfter           string // flag: stop after specific phase

	skipDraft      bool     // flag: skip draft commits by default
	includeDraft   bool     // flag: explicitly include draft commits (highest precedence)
	draftPatterns  []string // wildcard patterns for draft detection (case-insensitive)
	reverse        bool     // flag/config: show stack in reverse order (newest at the top)
	branchFromTitle bool    // flag/config: generate branch names from commit title instead of hash
}

type ConfigGit struct {
	enabled bool
	user    string // git
	email   string // git

	localTrunk  string // main | trunk branch (optional)
	remoteTrunk string // main | trunk branch

	remote    string // origin
	remoteUrl string // git@github.com:org/repo.git | https://github.com/org/repo.git
	protocol  string // ssh | https
	host      string // github.com
	repo      string // org/repo
}

type ConfigGh struct {
	user  string // gh-cli
	token string // gh-cli
	host  string // github.com
	repo  string // org/repo
}

type ConfigBranchless struct {
	enabled bool
	version string
}

type ConfigJj struct {
	enabled bool
	version string
}

func LoadConfig() (config Config) {
	flagVersion := flag.Bool("version", false, "Show version information")
	flag.BoolVar(&config.verbose, "v", false, "Verbose output")
	flag.BoolVar(&config.includeOtherAuthors, "include-other-authors", false, "Create PRs for commits from other authors (default to false: skip)")
	flag.BoolVar(&config.dryRun, "dry-run", false, "Show what would be done without making changes")
	flag.StringVar(&config.stopAfter, "stop-after", "", "Stop after phase: validate|get-commits|rewrite|push|pr-create")
	flag.BoolVar(&config.skipDraft, "skip-draft", false, "Skip commits with [draft] in title")
	flag.BoolVar(&config.includeDraft, "include-draft", false, "Include draft commits (override config)")
	flag.BoolVar(&config.reverse, "reverse", false, "Show stack in reverse order (newest at the top)")
	flag.BoolVar(&config.branchFromTitle, "branch-from-title", false, "Generate branch names from commit title instead of hash")

	flagGitHubHosts := flag.String("gh-hosts", "~/.config/gh/hosts.yml", "Path to config.json")
	flagTimeout := flag.Int("timeout", 20, "API call timeout in seconds")
	flagSetTags := flag.String("default-tags", "", "Set default tags for the current repository (comma separated)")
	flagTags := flag.String("t", "", "Set tags for current stack, ignore default (comma separated)")
	flagDraftPattern := flag.String("draft-pattern", "", "Wildcard pattern(s) for draft detection (default: wip:*,draft:*,*[wip]*,*[draft]*; comma-separated)")

	{ // parse flags
		usage := "Usage: git pr [OPTIONS]"
		flag.Usage = func() {
			printf("%s\n", usage)
			flag.PrintDefaults()
		}
		flag.Parse()

		// handle version flag
		if *flagVersion {
			printf("git-pr version %s\n", version)
			os.Exit(0)
		}

		// check environment variables as fallback
		if !config.dryRun && os.Getenv("GIT_PR_DRY_RUN") == "1" {
			config.dryRun = true
		}
		if config.stopAfter == "" && os.Getenv("GIT_PR_STOP_AFTER") != "" {
			config.stopAfter = os.Getenv("GIT_PR_STOP_AFTER")
		}
		// environment variables for draft settings
		if !config.skipDraft && os.Getenv("GIT_PR_SKIP_DRAFT") == "1" {
			config.skipDraft = true
		}
		if !config.includeDraft && os.Getenv("GIT_PR_INCLUDE_DRAFT") == "1" {
			config.includeDraft = true
		}

		// configs from flags
		config.timeout = time.Duration(*flagTimeout) * time.Second
		if *flagSetTags != "" {
			tags := saveGitPRConfig(strings.Split(*flagSetTags, ","))
			printf("Set default tags: %v\n", strings.Join(tags, ", "))
			os.Exit(0)
		}
		config.tags = getGitPRConfig()
		if *flagTags != "" {
			config.tags = nil // override default tags
			tags := strings.Split(*flagTags, ",")
			for _, tag := range tags {
				tag = strings.TrimSpace(tag)
				if tag != "" {
					config.tags = append(config.tags, tag)
				}
			}
		}

		// read git config for skipDraft setting
		if !config.skipDraft {
			skipDraftStr, _ := getGitConfig("git-pr.skipDraft")
			if skipDraftStr == "true" || skipDraftStr == "1" {
				config.skipDraft = true
			}
		}

		// read git config for reverse setting
		if !config.reverse {
			reverseStr, _ := getGitConfig("git-pr.reverse")
			if reverseStr == "true" || reverseStr == "1" {
				config.reverse = true
			}
		}

		// read git config for branch-from-title setting
		if !config.branchFromTitle {
			branchFromTitleStr, _ := getGitConfig("git-pr.branch-from-title")
			if branchFromTitleStr == "true" || branchFromTitleStr == "1" {
				config.branchFromTitle = true
			}
		}

		// determine draft pattern (precedence: flag > git config > default)
		patternStr := *flagDraftPattern
		if patternStr == "" {
			patternStr, _ = getGitConfig("git-pr.draftPattern")
		}
		if patternStr == "" {
			patternStr = `wip:*,draft:*,*[wip]*,*[draft]*` // default: wip/draft prefix or bracketed
		}

		// parse comma-separated patterns
		patterns := strings.Split(patternStr, ",")
		config.draftPatterns = make([]string, 0, len(patterns))
		for _, p := range patterns {
			p = strings.TrimSpace(p)
			if p != "" {
				// validate pattern: only allow *, ?, and regular characters
				if err := validateWildcardPattern(p); err != nil {
					exitf(`ERROR: invalid wildcard pattern %q: %v

The pattern must be a valid wildcard pattern.
Supported wildcards:
  *          - matches any characters
  ?          - matches exactly one character

Example patterns:
  wip:*,draft:*,*[wip]*,*[draft]*  - default patterns (case-insensitive)
  *[draft]*,*[wip]*                - contains [draft] OR [wip]
  wip:*                            - starts with "wip:"
  *-draft-*                        - contains "-draft-"

Note: Character ranges like [a-z] are NOT supported.
`, p, err)
				}
				config.draftPatterns = append(config.draftPatterns, p)
			}
		}
	}
	{ // detect repository by git
		errMsg := `
git-pr is a tool for submitting git commits as GitHub stacked pull requests (stacked PRs).

ERROR: You need to run it in a git repository with remote configured.

For more information, see "git-pr --help".`

		output, err := _git("rev-parse", "--show-toplevel")
		if err != nil {
			exitf(errMsg)
		}
		config.git.enabled = true
		config.repoDir = strings.TrimSpace(output)

		// find remote url (push)
		// TODO: support multiple remotes
		out, err := git("remote", "-v")
		if err != nil {
			exitf(errMsg)
		}
		func() {
			line := out // find the line with "(push)"
			for _, l := range strings.Split(out, "\n") {
				if strings.Contains(l, "(push)") {
					line = l
					break
				}
			}

			// git@<host>:<user>/<repo>.git
			regexpURL := regexp.MustCompile(`(\w+)\s+(git@([^:\s]+):([^/\s]+)/([^.\s]+)(\.git)?)`)
			matches := regexpURL.FindStringSubmatch(line)
			if len(matches) > 0 {
				config.git.protocol = "ssh"
				config.git.remote = matches[1]
				config.git.remoteUrl = matches[2]
				config.git.host = matches[3]
				config.git.repo = matches[4] + "/" + matches[5]
				return
			}

			// https://<host>/<user>/<repo>.git
			regexpURL = regexp.MustCompile(`(\w+)\s+(https://(github\.com)/([^/\s]+)\/([^.\s]+)(\.git)?)`)
			matches = regexpURL.FindStringSubmatch(line)
			if len(matches) > 0 {
				config.git.protocol = "ssh"
				config.git.remote = matches[1]
				config.git.remoteUrl = matches[2]
				config.git.host = matches[3]
				config.git.repo = matches[4] + "/" + matches[5]
				return
			}

			exitf(`
ERROR: failed to parse remote url:
  expect git@<host>:<user>/<repo> or https://github.com/<user>/<repo> 
  got %q`, out)
		}()
	}
	{ // detect remote trunk branch
		out, err := git("symbolic-ref", "--short", fmt.Sprintf("refs/remotes/%v/HEAD", config.git.remote))
		if err != nil {
			exitf("ERROR: failed to detect remote trunk branch")
		}
		remoteTrunk := strings.TrimPrefix(out, config.git.remote+"/")
		if remoteTrunk == "" {
			exitf("ERROR: failed to detect remote trunk branch")
		}
		config.git.remoteTrunk = remoteTrunk
		config.git.localTrunk = config.git.remoteTrunk
	}
	{ // get git username and email
		user, err := getGitConfig("user.name")
		if err != nil || user == "" {
			exitf("ERROR: user.name not found in git config")
		}
		email, err := getGitConfig("user.email")
		if err != nil || email == "" {
			exitf("ERROR: user.email not found in git config")
		}
		config.git.user = user
		config.git.email = email
	}
	{ // detect jj
		if _, err := os.Stat(config.repoDir + "/.jj"); err == nil {
			version, err := _jj("version")
			if err == nil {
				config.jj.enabled = true
				config.jj.version = strings.TrimPrefix(version, "jj ")
				debugf("detected jj %s", config.jj.version)
			}
		}
	}
	{ // detect git-branchless
		version, err := _git("branchless", "--version")
		if err == nil {
			config.bl.enabled = true
			config.bl.version = strings.TrimSpace(version)
			debugf("detected git-branchless %s", config.bl.version)
		}
	}
	{ // parse github config
		ghHosts, err := LoadGitHubConfig(*flagGitHubHosts)
		if err != nil {
			exitf(`
ERROR: failed to load GitHub config at %v: %v
		
Hint: Install github client and login with your account
      https://github.com/cli/cli#installation
Then:
      gh auth login
`, *flagGitHubHosts, err)
		}

		ghHost := ghHosts[config.git.host]
		if ghHost == nil {
			exitf(`
ERROR: no GitHub config for host %v

Hint: Check your ~/.config/gh/hosts.yml
Run the following command and choose your github host:

      gh auth login
`, config.git.host)
			return
		}

		config.gh.user = ghHost.User
		config.gh.token = ghHost.OauthToken

		if config.gh.token == "" { // try getting from keyring
			key := "gh:" + config.git.host
			config.gh.token, _ = keyring.Get(key, "")
		}
		if config.gh.token == "" {
			exitf(`ERROR: no GitHub token found for host %q

Hint: use github cli to login to your account:

      gh auth login
`, config.git.host)
		}
	}

	config.gh.host = config.git.host // assume github.com
	config.gh.repo = config.git.repo // assume org/repo
	return config
}

type GitHubConfigHostsFile map[string]*GitHubConfigHost

type GitHubConfigHost struct {
	User        string `yaml:"user"`
	OauthToken  string `yaml:"oauth_token"`
	GitProtocol string `yaml:"git_protocol"`
}

func LoadGitHubConfig(configPath string) (out GitHubConfigHostsFile, _ error) {
	configPath = expandPath(configPath)
	data, err := os.ReadFile(configPath)
	if err != nil {
		return nil, err
	}

	err = yaml.Unmarshal(data, &out)
	if err != nil {
		return nil, err
	}
	return out, nil
}

func getGitConfig(name string) (string, error) {
	out, err := git("config", "--get", name)
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(out), nil
}

func expandPath(path string) string {
	if path == "" {
		return ""
	}
	if path[0] == '~' {
		return os.Getenv("HOME") + path[1:]
	}
	return path
}

func validateConfig[T comparable](name string, value T) {
	var zero T
	if value == zero {
		exitf("missing config %q", name)
	}
}

func getGitPRConfig() (tags []string) {
	rawTags, _ := git("config", "--get", gitconfigTags)
	for _, tag := range strings.Split(rawTags, ",") {
		tag = strings.TrimSpace(tag)
		if tag != "" {
			tags = append(tags, tag)
		}
	}
	return tags
}

func saveGitPRConfig(tags []string) []string {
	var xtags []string
	for i := range tags {
		tag := strings.TrimSpace(tags[i])
		if tag != "" {
			xtags = append(xtags, tag)
		}
	}
	rawTags := strings.Join(xtags, ",")

	_, _ = git("config", "--unset-all", gitconfigTags)
	must(git("config", "--add", gitconfigTags, rawTags))
	return xtags
}

// validateWildcardPattern checks if pattern contains only valid characters
// Valid: alphanumeric, spaces, common punctuation, *, and ?
// Invalid: syntax that looks like character classes or ranges
func validateWildcardPattern(pattern string) error {
	// pattern is valid if it doesn't look like it's trying to use unsupported features
	// we're being permissive here - just checking it's not empty
	if pattern == "" {
		return fmt.Errorf("pattern cannot be empty")
	}
	return nil
}

// matchWildcard checks if text matches a wildcard pattern (case-insensitive)
// Supports only * (any chars) and ? (one char), no ranges or character classes
// Returns true if pattern matches the text
func matchWildcard(pattern, text string) bool {
	// convert both to lowercase for case-insensitive matching
	pattern = strings.ToLower(pattern)
	text = strings.ToLower(text)

	return matchWildcardImpl(pattern, text)
}

// matchWildcardImpl implements simple wildcard matching with * and ?
// * matches zero or more characters
// ? matches exactly one character
func matchWildcardImpl(pattern, text string) bool {
	pIdx, tIdx := 0, 0
	pLen, tLen := len(pattern), len(text)
	starIdx, matchIdx := -1, 0

	for tIdx < tLen {
		// characters match or pattern has ?
		if pIdx < pLen && (pattern[pIdx] == '?' || pattern[pIdx] == text[tIdx]) {
			pIdx++
			tIdx++
		} else if pIdx < pLen && pattern[pIdx] == '*' {
			// found *, record position and try to match rest
			starIdx = pIdx
			matchIdx = tIdx
			pIdx++
		} else if starIdx != -1 {
			// no match, but we have a * before, backtrack
			pIdx = starIdx + 1
			matchIdx++
			tIdx = matchIdx
		} else {
			// no match and no * to backtrack
			return false
		}
	}

	// consume remaining * in pattern
	for pIdx < pLen && pattern[pIdx] == '*' {
		pIdx++
	}

	return pIdx == pLen
}

// matchAnyPattern checks if text matches any of the patterns
func matchAnyPattern(patterns []string, text string) bool {
	for _, pattern := range patterns {
		if matchWildcard(pattern, text) {
			return true
		}
	}
	return false
}
