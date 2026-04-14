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
them via the `gh` CLI. You track which notifications the user has seen (read/unread).

## Setup Check

```bash
BREEZE_DIR="${BREEZE_DIR:-$HOME/.breeze}"
INBOX="$BREEZE_DIR/inbox.json"
SEEN="$BREEZE_DIR/seen.json"

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

# Check seen tracker
if [ -f "$SEEN" ]; then
  SEEN_COUNT=$(jq '.seen_ids | length' "$SEEN" 2>/dev/null || echo "0")
  echo "SEEN_OK: $SEEN_COUNT seen"
else
  echo '{"seen_ids":[],"seen_at":{}}' > "$SEEN"
  echo "SEEN_CREATED"
fi
```

If `AUTH_NEEDED`: Tell the user "breeze requires GitHub CLI authentication. Run `gh auth login` first." and stop.

If `NO_INBOX`: The poller hasn't run yet. Offer to fetch notifications on-demand:
"No inbox file found. The breeze poller may not be running. Want me to fetch your notifications now?"
If yes, run the poller inline:
```bash
BREEZE_POLL=$(find ~/.claude/skills -name breeze-poll -type f 2>/dev/null | head -1)
[ -z "$BREEZE_POLL" ] && BREEZE_POLL=$(find ~/breeze-demo -name breeze-poll -type f 2>/dev/null | head -1)
[ -n "$BREEZE_POLL" ] && bash "$BREEZE_POLL" || echo "Could not find breeze-poll script"
```

## Show Inbox

Read the inbox and present a numbered list with read/unread status:

```bash
BREEZE_DIR="${BREEZE_DIR:-$HOME/.breeze}"
SEEN="$BREEZE_DIR/seen.json"
SEEN_IDS=$(jq -c '.seen_ids // []' "$SEEN" 2>/dev/null || echo "[]")

jq -r --argjson seen "$SEEN_IDS" '
  .notifications
  | map(. + {is_seen: (.id | IN($seen[]))})
  | to_entries
  | map(
      (if .value.is_seen then "   " else " * " end)
      + "\(.key + 1). [\(.value.type)] \(.value.repo) — \(.value.title)"
      + " (\(.value.reason), \(.value.updated_at | split("T")[0]))"
      + "\n      \(.value.html_url)"
    )
  | join("\n")
' "$BREEZE_DIR/inbox.json" 2>/dev/null
```

Present this list to the user. Each notification shows:
- `*` marker for unread, blank for read
- Number for selection
- Type (PullRequest, Issue, Discussion)
- Repo name
- Title
- Reason (review_requested, mention, assign, etc.)
- Date
- Clickable GitHub link

Show a summary line at the top: "X unread, Y read"

Ask: "Pick a number to dive in, or tell me what you want to focus on (e.g. 'show unread only', 'show PRs only')."

## Mark as Seen

When the user picks a notification to dive into, mark it as seen:

```bash
BREEZE_DIR="${BREEZE_DIR:-$HOME/.breeze}"
SEEN="$BREEZE_DIR/seen.json"
NOTIF_ID="THE_NOTIFICATION_ID"
TIMESTAMP=$(date -u +%Y-%m-%dT%H:%M:%SZ)

# Add to seen list (idempotent)
TMP_SEEN=$(mktemp)
jq --arg id "$NOTIF_ID" --arg ts "$TIMESTAMP" '
  .seen_ids = ((.seen_ids // []) + [$id] | unique) |
  .seen_at[$id] = $ts
' "$SEEN" > "$TMP_SEEN" && mv "$TMP_SEEN" "$SEEN"
```

Also support bulk mark-as-seen:
- "mark all as read" — adds all current notification IDs to seen.json
- "mark 1-5 as read" — adds those specific notifications to seen.json

## Dive Into a Notification

When the user picks a notification, mark it as seen (above), then load the full context on-demand.

For **PullRequest** notifications:
```bash
# Substitute OWNER, REPO, NUMBER from the selected notification
gh pr view NUMBER --repo OWNER/REPO --json title,body,author,state,additions,deletions,files,reviews,comments,labels,url
gh pr diff NUMBER --repo OWNER/REPO | head -500
```

For **Issue** notifications:
```bash
gh issue view NUMBER --repo OWNER/REPO --json title,body,author,state,comments,labels,url
gh api repos/OWNER/REPO/issues/NUMBER/comments --jq '.[] | {author: .user.login, body: .body, created_at: .created_at}' | head -200
```

For **Discussion** notifications:
```bash
# GitHub Discussions use the GraphQL API (not available via gh issue/pr)
# Fetch discussion details via GraphQL
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

If the discussion number is not available from the notification URL, fall back to:
```bash
# Get the discussion URL from the notification thread
gh api "/notifications/threads/THREAD_ID" --jq '.subject.url'
# Then fetch via the API URL
gh api "SUBJECT_URL" 2>/dev/null || echo "Discussion details not available via REST API"
```

For **other types** (Release, CheckSuite, etc.):
```bash
gh api "/notifications/threads/THREAD_ID" --jq '{type: .subject.type, title: .subject.title, url: .subject.url, reason: .reason}'
```

After loading context, **summarize** the situation in 3-5 sentences:
- What's happening (PR change summary, issue description, discussion topic, comment thread)
- Who's involved and what they're asking
- What action seems needed

Then **suggest an action**:
- "This looks like a straightforward docs fix. Suggest: approve and merge."
- "The reviewer is asking for test coverage on the new endpoint. Suggest: comment acknowledging and ask for a timeline."
- "This issue is a duplicate of #42. Suggest: close with a link to the original."
- "This discussion is an RFC asking for feedback on API versioning. Suggest: comment with your position."

## Execute Actions

When the user tells you what to do, translate to `gh` CLI commands.

**Safe actions (execute with confirmation):**
- Comment on issue/PR: `gh issue comment NUMBER --repo OWNER/REPO --body "MESSAGE"`
- Comment on discussion: `gh api graphql -f query='mutation { addDiscussionComment(input: {discussionId: "ID", body: "MESSAGE"}) { comment { id } } }'`
- Approve PR: `gh pr review NUMBER --repo OWNER/REPO --approve --body "MESSAGE"`
- Request changes: `gh pr review NUMBER --repo OWNER/REPO --request-changes --body "MESSAGE"`
- Close issue: `gh issue close NUMBER --repo OWNER/REPO --comment "MESSAGE"`
- React with emoji: `gh api repos/OWNER/REPO/issues/NUMBER/reactions -f content=EMOJI`
- Label: `gh issue edit NUMBER --repo OWNER/REPO --add-label "LABEL"`

**Destructive actions (require explicit confirmation + warning):**
- Merge PR: `gh pr merge NUMBER --repo OWNER/REPO`
- Delete branch: warn that this is destructive
- Force push: refuse unless user insists

**Footer:** Append to every comment/review body:
```
---
_sent via [breeze](https://github.com/agent-team-foundation/breeze-demo) on behalf of @USERNAME_
```

Get the username:
```bash
gh api user --jq '.login'
```

Always show the exact command before executing. Wait for user confirmation.

## Bulk Actions

If the user wants to handle multiple notifications:
- "Mark all as read": add all notification IDs to seen.json
- "Mark all as read on GitHub too": `gh api -X PUT /notifications` (marks read on GitHub's side)
- "Show only unread": filter the inbox list by seen status
- "Show only PRs": filter by type
- "Show only review requests": filter by reason

## Cleanup

When notifications are no longer in the GitHub notifications list (resolved, merged, closed),
they'll naturally disappear from inbox.json on the next poll. The seen.json file may accumulate
stale IDs over time. Clean up periodically:

```bash
BREEZE_DIR="${BREEZE_DIR:-$HOME/.breeze}"
# Remove seen IDs that are no longer in the inbox
CURRENT_IDS=$(jq -c '[.notifications[].id]' "$BREEZE_DIR/inbox.json" 2>/dev/null || echo "[]")
TMP_SEEN=$(mktemp)
jq --argjson current "$CURRENT_IDS" '
  .seen_ids = [.seen_ids[] | select(. as $id | $current | index($id))] |
  .seen_at = (.seen_at | with_entries(select(.key as $k | $current | index($k))))
' "$BREEZE_DIR/seen.json" > "$TMP_SEEN" && mv "$TMP_SEEN" "$BREEZE_DIR/seen.json"
```

## Tips

- Links in the notification list are clickable in most terminals
- `*` means unread, blank means you've already looked at it
- The user can say things like "approve the first 3 PRs" or "close all stale issues"
- If the user asks about a repo not in their notifications, use `gh` to look it up directly
- Discussions use GitHub's GraphQL API which requires the `read:discussion` scope on the gh token
