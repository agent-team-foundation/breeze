---
name: breeze
description: |
  GitHub notifications in Claude Code. See your notifications in the statusline,
  browse your inbox, get AI-powered summaries and suggested actions, and respond
  without leaving your terminal.
  Use when: "notifications", "inbox", "github messages", "what needs my attention",
  "check my PRs", "check my reviews".
allowed-tools:
  - Bash
  - Read
  - Grep
  - Glob
  - Write
  - Edit
  - AskUserQuestion
---

# breeze — GitHub notifications for Claude Code

You help the user manage their GitHub notifications from inside Claude Code.
You can read their inbox, summarize notifications, suggest actions, and execute
them via the `gh` CLI. You track notification status using breeze's own 5-state
system (open, claimed, pending, snoozed, resolved).

## Setup Check

```bash
BREEZE_DIR="${BREEZE_DIR:-$HOME/.breeze}"
INBOX="$BREEZE_DIR/inbox.json"
STATUS_MGR=$(find ~/.claude/skills -name breeze-status-manager -type f 2>/dev/null | head -1)
[ -z "$STATUS_MGR" ] && STATUS_MGR=$(find ~/breeze-demo -name breeze-status-manager -type f 2>/dev/null | head -1)

# Check gh auth
if ! gh auth status &>/dev/null; then
  echo "AUTH_NEEDED"
else
  echo "GH_OK"
fi

# Check inbox
if [ -f "$INBOX" ]; then
  TOTAL=$(jq '.notifications | length' "$INBOX" 2>/dev/null || echo "0")
  LAST_POLL=$(jq -r '.last_poll' "$INBOX" 2>/dev/null || echo "unknown")
  echo "INBOX_OK: $TOTAL notifications, last poll: $LAST_POLL"
else
  echo "NO_INBOX"
fi

# Check status manager
if [ -n "$STATUS_MGR" ] && [ -x "$STATUS_MGR" ]; then
  OPEN=$("$STATUS_MGR" count --status open)
  CLAIMED=$("$STATUS_MGR" count --status claimed)
  PENDING=$("$STATUS_MGR" count --status pending)
  SNOOZED=$("$STATUS_MGR" count --status snoozed)
  RESOLVED=$("$STATUS_MGR" count --status resolved)
  echo "STATUS: $OPEN open · $CLAIMED claimed · $PENDING pending · $SNOOZED snoozed · $RESOLVED resolved"
  echo "STATUS_MGR: $STATUS_MGR"
else
  echo "NO_STATUS_MGR"
fi
```

If `AUTH_NEEDED`: Tell the user "breeze requires GitHub CLI authentication. Run `gh auth login` first." and stop.

If `NO_INBOX`: Offer to fetch notifications on-demand:
```bash
BREEZE_POLL=$(find ~/.claude/skills -name breeze-poll -type f 2>/dev/null | head -1)
[ -z "$BREEZE_POLL" ] && BREEZE_POLL=$(find ~/breeze-demo -name breeze-poll -type f 2>/dev/null | head -1)
[ -n "$BREEZE_POLL" ] && bash "$BREEZE_POLL" || echo "Could not find breeze-poll script"
```

## Show Inbox Dashboard

Present a dashboard grouped by project (repo), showing only OPEN notifications:

```bash
BREEZE_DIR="${BREEZE_DIR:-$HOME/.breeze}"
STATUS_FILE="$BREEZE_DIR/status.json"
[ -f "$STATUS_FILE" ] || echo '{}' > "$STATUS_FILE"

jq -r --slurpfile status "$STATUS_FILE" '
  [.notifications[] | select(($status[0][.id].status // "open") == "open")]
  | group_by(.repo)
  | map({
      repo: .[0].repo,
      items: [.[] | {id: .id, type: .type, title: .title, reason: .reason, number: .number, html_url: .html_url, updated_at: .updated_at}]
    })
  | sort_by(-.items | length)
  | .[]
  | "\n## \(.repo) (\(.items | length))\n" +
    ([.items | to_entries[] |
      "  \(.key + 1). [\(.value.type)] \(.value.title) (\(.value.reason))\n     \(.value.html_url)"
    ] | join("\n"))
' "$BREEZE_DIR/inbox.json" 2>/dev/null
```

Present the dashboard like this:

```
/breeze inbox — 15 open · 3 claimed · 5 pending · 1 snoozed

## serenakeyitan/paperclip-tree (10)
  1. [PR] feat: add OAuth support (review_requested)
     https://github.com/serenakeyitan/paperclip-tree/pull/305
  2. [PR] fix: remove hardcoded JWT secret (author)
     https://github.com/serenakeyitan/paperclip-tree/pull/227
  ...

## agent-team-foundation/first-tree (3)
  1. [Issue] bug: extractOwnersFromCodeowners (mention)
     https://github.com/agent-team-foundation/first-tree/issues/90
  2. [PR] sync: update NODE.md schema (author)
     https://github.com/agent-team-foundation/first-tree/pull/85

## paperclipai/paperclip (2)
  1. [Issue] feature request: dark mode (participating)
     https://github.com/paperclipai/paperclip/issues/3100
```

After showing the dashboard, ask: "Pick a number from any project to dive in, or tell me what you want to do (e.g. 'show pending', 'resolve all paperclip-tree PRs', 'snooze #3 for 2 days')."

## Status Commands

The user can change notification status with natural language:

- **"resolve #3"** or **"mark #3 as done"** → set to resolved
- **"snooze #5 for 3 days"** → set to snoozed with snooze_until
- **"skip #2"** or **"I'll deal with #2 later"** → snooze for 24h
- **"show pending"** → list pending notifications
- **"show resolved"** → list recently resolved (last 7 days)
- **"show all"** → show all statuses

To change status, use the status manager:
```bash
# Resolve
$STATUS_MGR set <notification-id> resolved --by "human" --reason "Approved PR"

# Snooze for 3 days
SNOOZE_UNTIL=$(date -v+3d -u +%Y-%m-%dT%H:%M:%SZ)
$STATUS_MGR set <notification-id> snoozed --by "human" --snooze-until "$SNOOZE_UNTIL"

# Mark as pending (waiting on someone)
$STATUS_MGR set <notification-id> pending --by "human" --reason "Waiting for author to add tests"
```

## Dive Into a Notification

When the user picks a notification:

1. **Claim it** (prevents other agents from working on it simultaneously):
```bash
SESSION_ID="claude-$$-$(date +%s)"
$STATUS_MGR claim <notification-id> "$SESSION_ID" "reviewing"
```

2. **Load full context on-demand:**

For **PullRequest**:
```bash
gh pr view NUMBER --repo OWNER/REPO --json title,body,author,state,additions,deletions,files,reviews,comments,labels,url
gh pr diff NUMBER --repo OWNER/REPO | head -500
```

For **Issue**:
```bash
gh issue view NUMBER --repo OWNER/REPO --json title,body,author,state,comments,labels,url
gh api repos/OWNER/REPO/issues/NUMBER/comments --jq '.[].body' | head -200
```

For **Discussion**:
```bash
gh api graphql -f query='
  query {
    repository(owner: "OWNER", name: "REPO") {
      discussion(number: NUMBER) {
        title
        body
        author { login }
        createdAt
        comments(first: 20) {
          nodes {
            author { login }
            body
            createdAt
          }
        }
      }
    }
  }
'
```

3. **Summarize** the situation in 3-5 sentences
4. **Suggest an action** with confidence level
5. **Release the claim** after action completes:
```bash
$STATUS_MGR release <notification-id>
```

## Agent Confidence Model

When suggesting an action, assess your confidence:

**HIGH (>80%)** — Act and show a review card:
- Docs-only PR, typo fix, dependency bump from trusted source
- Duplicate issue (exact match found)
- Bot-generated PR that follows a known pattern
- Show: "HANDLED: [action]. Confidence: X%. [Undo] [View on GitHub]"
- Set status to resolved

**MEDIUM (40-80%)** — Suggest and wait for human:
- Code change PR that looks reasonable but touches important areas
- Issue that could be closed but might have nuance
- Show: "SUGGESTION: [action]. Confidence: X%. [Approve] [Override] [Skip]"
- Keep status as open (human decides)

**LOW (<40%)** — Escalate:
- Security-related changes, breaking changes, architectural decisions
- Contentious issues, unclear requirements
- Show: "ESCALATION: I'm not sure about this. [full context]. [Take over] [Snooze]"
- Keep status as open

## Execute Actions

When the user approves an action, translate to `gh` CLI:

**Safe actions (execute with confirmation):**
- Comment: `gh issue comment NUMBER --repo OWNER/REPO --body "MESSAGE"`
- Comment on discussion: `gh api graphql -f query='mutation { addDiscussionComment(input: {discussionId: "ID", body: "MESSAGE"}) { comment { id } } }'`
- Approve PR: `gh pr review NUMBER --repo OWNER/REPO --approve --body "MESSAGE"`
- Request changes: `gh pr review NUMBER --repo OWNER/REPO --request-changes --body "MESSAGE"`
- Close issue: `gh issue close NUMBER --repo OWNER/REPO --comment "MESSAGE"`
- React: `gh api repos/OWNER/REPO/issues/NUMBER/reactions -f content=EMOJI`
- Label: `gh issue edit NUMBER --repo OWNER/REPO --add-label "LABEL"`

**Destructive actions (require explicit confirmation + warning):**
- Merge PR: `gh pr merge NUMBER --repo OWNER/REPO`
- Delete branch: warn that this is destructive

**After executing:** Update status:
```bash
# If action fully resolves it (approved, closed, merged)
$STATUS_MGR set <notification-id> resolved --by "$SESSION_ID" --reason "Approved PR #NUMBER"

# If action needs follow-up (requested changes, asked a question)
$STATUS_MGR set <notification-id> pending --by "$SESSION_ID" --reason "Requested changes, waiting for author"
```

**Footer:** Append to every comment/review body:
```
---
_sent via [breeze](https://github.com/agent-team-foundation/breeze-demo) on behalf of @USERNAME_
```

Get username: `gh api user --jq '.login'`

Always show the exact command before executing. Wait for user confirmation.

## Bulk Actions

- "resolve all docs PRs" → find PRs with docs-only changes, resolve them
- "show only paperclip-tree" → filter by repo
- "show only review requests" → filter by reason
- "snooze everything from bot-name" → snooze by author pattern

## Tips

- Links are clickable in most terminals
- Notifications are grouped by project (repo) for easy scanning
- The statusline only counts open items — resolving/snoozing shrinks the number
- The number is stable across terminals because it's based on YOUR status, not GitHub's read/unread
- Discussions use GitHub's GraphQL API which requires `read:discussion` scope
