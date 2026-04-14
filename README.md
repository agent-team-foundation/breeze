# breeze

GitHub notifications inside Claude Code. See what needs your attention in the statusline, browse your inbox, get AI-powered summaries and suggested actions, and respond without leaving your terminal.

```
breeze: 3 PRs · 1 issue · 2 mentions
```

## What it does

1. **Polls GitHub** every 3 minutes for your notifications (PRs, issues, discussions, review requests, mentions)
2. **Shows a summary** in your Claude Code statusline
3. **Type `/breeze`** to see your inbox with clickable GitHub links
4. **Pick a notification** and the agent summarizes the context, suggests an action
5. **Act on it** in natural language ("approve this PR", "comment asking for tests", "close as duplicate of #42")

The agent posts on your behalf via the `gh` CLI with a footer noting it was sent via breeze.

## Install

```bash
git clone https://github.com/agent-team-foundation/breeze-demo.git
cd breeze-demo
./setup
```

The setup script:
- Creates `~/.breeze/` with a default config
- Installs a launchd plist (macOS) or crontab entry (Linux) to poll every 3 minutes
- Symlinks the `/breeze` skill into `~/.claude/skills/`
- Runs an initial poll

### Prerequisites

- [GitHub CLI](https://cli.github.com/) (`gh`) installed and authenticated
- [jq](https://jqlang.github.io/jq/) installed
- Claude Code

## Usage

In Claude Code, type `/breeze` to open your inbox.

```
1. [PullRequest] owner/repo — Fix auth token refresh (review_requested, 2026-04-14)
   https://github.com/owner/repo/pull/47
2. [Issue] owner/repo — Login page broken on mobile (mention, 2026-04-14)
   https://github.com/owner/repo/issues/123
3. [Discussion] owner/repo — RFC: new API versioning (participating, 2026-04-13)
   https://github.com/owner/repo/discussions/89
```

Pick a number. The agent loads the full context (PR diff, comment thread, issue body), summarizes it, and suggests an action. Tell it what to do in plain English.

## Config

Edit `~/.breeze/config.yaml`:

```yaml
repos:
  - all                    # or list specific repos: owner/repo1, owner/repo2
poll_interval: 180         # seconds between polls
footer: true               # append "sent via breeze" to comments
auto_mark_read: false      # mark notifications as read after polling
```

## How it works

```
Poller (launchd/cron)  →  ~/.breeze/inbox.json  →  Statusline
         ↑                        ↓
     gh api                  /breeze skill
   /notifications         (summarize + act)
                                  ↓
                           gh pr review
                           gh issue comment
                           gh pr merge
```

## Vision

breeze v1 is the read path: see notifications, act on them with AI help.

The north star is full agent autonomy. The agent handles 90% of your GitHub interactions, only escalates to you when human judgment is needed. Think: an always-on open source maintainer that reviews PRs, responds to issues, and participates in discussions on your behalf.

## License

MIT
