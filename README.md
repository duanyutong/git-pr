# git-pr

`git-pr` pushes standalone Git branches and submits managed stacks as GitHub pull requests.
An ordinary invocation on an unmanaged standalone branch pushes that branch and sets its upstream, without requiring GitHub API authentication.
Explicit commit selections and existing publication associations retain the managed submission workflow.
In managed submissions, each selected commit corresponds to a pull request.
The first PR targets the stack's base branch; each subsequent PR targets the preceding commit's branch.
Subsequent submissions update PR titles, descriptions and labels, and keep the PRs linked in stack order.

## Integration Support

Git is required; managed pull-request submissions additionally require GitHub credentials.
Use [GitHub CLI](https://cli.github.com/) authentication to configure the supported credential source; submission uses Git and the GitHub API directly.
Native stack linking requires no GitHub CLI extension.

Supported VCS backends are:

- Git, optionally using [git-branchless](https://github.com/arxanas/git-branchless) to manage local stacks.
- [Jujutsu](https://jj-vcs.github.io/jj/), with either a colocated Git repository or a separate workspace backed by Git.

GitHub.com is the supported hosting target.
GitHub Enterprise Server requires changes to REST endpoint construction, as described under [hosting limitations](docs/configuration.md#authentication-and-hosting).

## Installation

Source installation requires Rust.

```sh
cargo install --path . --locked
```

Authentication, repository initialisation and submission examples are documented in [common commands](docs/commands.md).

Interactive status displays show completed and total items, current PR identities and live elapsed time during submission.
Description updates identify the PRs in each active batch; counts advance after confirmed completion or successful recovery.
Waits retain a countdown, and stage summaries retain final or partially completed counts.

## Configuration

Settings reside in the `[git-pr]` section of Git configuration.
Example `~/.gitconfig` section:

```gitconfig
[git-pr]
    defaultDraft = true
    stackUp = true
    githubStack = true
    stackEmoji = false
    skipDraft = false
    draftPattern = "wip:*,draft:*,*[wip]*,*[draft]*"
    tags =
    addLabel =
    addTipLabel =
    output = "{url} {title}"
```

Option definitions and precedence are documented in [configuration](docs/configuration.md#git-configuration).
Development instructions appear in [development](docs/development.md).
Workflow ownership, batching, recovery and performance limits are specified in [submission efficiency](docs/submission-efficiency.md).
