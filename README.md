# breeze

GitHub notifications inside Claude Code. See what needs your attention in the statusline, browse your inbox grouped by project, get AI-powered summaries and suggested actions, and respond without leaving your terminal.

```
/breeze: 52 PRs · 3 issues · 1 discussions (+2 new)
```

## What it does

1. **Polls GitHub** every 60 seconds for all your notifications (PRs, issues, discussions, review requests, mentions)
2. **Shows a summary** in your Claude Code statusline with a terminal bell on new items
3. **Type `/breeze`** to see your inbox grouped by project with clickable GitHub links
4. **Pick a notification** and the agent summarizes the context, suggests an action with a confidence level
5. **Act on it** in natural language ("approve this PR", "snooze for 3 days", "close as duplicate of #42")

breeze manages its own 5-state status system — the statusline count only changes when you or your agent act, not when GitHub marks something as read.

## Install

```bash
git clone https://github.com/agent-team-foundation/breeze-demo.git
cd breeze-demo
./setup
```

The setup script:
- Creates `~/.breeze/` with a default config
- Installs a launchd plist (macOS) or crontab entry (Linux) to poll every 60 seconds
- Symlinks the `/breeze` skill into `~/.claude/skills/`
- Chains breeze into your existing Claude Code statusline (doesn't replace it)
- Runs an initial poll

### Prerequisites

- [GitHub CLI](https://cli.github.com/) (`gh`) installed and authenticated
- [jq](https://jqlang.github.io/jq/) installed
- Claude Code

## Usage

In Claude Code, type `/breeze` to open your inbox.

```
/breeze inbox — 15 open · 3 claimed · 5 pending · 1 snoozed

## serenakeyitan/paperclip-tree (10)
  1. [PR] feat: add OAuth support (review_requested)
     https://github.com/serenakeyitan/paperclip-tree/pull/305
  2. [PR] fix: remove hardcoded JWT (author)
     https://github.com/serenakeyitan/paperclip-tree/pull/227

## agent-team-foundation/first-tree (3)
  1. [Issue] bug: extractOwnersFromCodeowners (mention)
     https://github.com/agent-team-foundation/first-tree/issues/90

## paperclipai/paperclip (2)
  1. [Discussion] PR review agent for paperclip-tree (participating)
     https://github.com/serenakeyitan/paperclip-tree/discussions/287
```

Pick a number. The agent loads the full context (PR diff, comment thread, issue body), summarizes it, and suggests an action. Tell it what to do in plain English.

## Notification Status

breeze tracks its own status per notification, independent of GitHub's read/unread:

| Status | Meaning |
|--------|---------|
| **open** | Needs action, no one's on it (shows in statusline) |
| **claimed** | Agent or human is actively working on it (locked) |
| **pending** | Acted on, waiting for someone else to respond |
| **snoozed** | Deferred — comes back after timer or new activity |
| **resolved** | Done, no more action needed |

The statusline only counts **open** notifications. This means the number is stable across all your terminals — it only changes when you or your agent take action, not when GitHub randomly marks something as read.

### Status commands

- `"resolve #3"` — mark as done
- `"snooze #5 for 3 days"` — hide until then (or until new activity)
- `"show pending"` — see what's waiting on others
- `"show resolved"` — see what was handled recently

### Agent claim locks

When an agent starts working on a notification, it claims it with an atomic filesystem lock. Other agents see the claim and skip it. Claims auto-expire after 5 minutes if the agent crashes.

## Config

Edit `~/.breeze/config.yaml`:

```yaml
repos:
  - all                    # or list specific repos: owner/repo1, owner/repo2
poll_interval: 60          # seconds between polls
footer: true               # append "sent via breeze" to comments
auto_mark_read: false      # mark notifications as read after polling
```

## How it works

```
Poller (launchd/cron)  →  inbox.json  ←→  status.json
         ↑                    ↓                ↑
     gh api              Statusline        /breeze skill
   /notifications       (open count)     (dashboard + actions)
                                               ↓
                                        gh pr review
                                        gh issue comment
                                        gh pr merge
                                               ↓
                                    claims/<id>/  (agent locks)
```

## Vision

breeze v1 is the read path: see notifications, act on them with AI help.

The north star is full agent autonomy. The agent handles 90% of your GitHub interactions, only escalates to you when human judgment is needed. High-confidence actions (docs PRs, duplicates) are auto-handled with an undo option. Medium-confidence actions get a suggestion. Low-confidence items are escalated with full context.

See [DESIGN.md](DESIGN.md) for the full architecture and status system design.

## License

MIT
