# breeze-watch TUI Design

## Principle

The watch is a **status board**, not a log. Repos are the stable anchor. Status is the filter. The human's attention is the scarce resource — the UI must surface blockers first.

## Layout

```
┌─────────────────────────────────────────────────────────────┐
│  breeze                                 78 new · 0 wip · 0 human · 192 done │
│  ─────────────────────────────────────────────────────────  │
│                                                              │
│  BOARD (stable, deduped by repo+status)                     │
│                                                              │
│  ─ live ──────────────────────────────────────────────────  │
│                                                              │
│  LIVE FEED (transitions as they happen)                     │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

Two sections:
- **Board** (top, most of screen): current status of everything, grouped by repo+status
- **Live feed** (bottom): transitions as they happen, ephemeral

## Organization

**Primary:** by repo (stable anchor — repos don't change)
**Secondary:** by status within each repo, ordered by urgency:
1. HUMAN (blockers first)
2. WIP (in progress)
3. NEW (untouched)
4. DONE (collapsed)

Repos sorted by total non-done count (most active first).

## Color Scheme

Color = urgency for the human. Intensity maps to how blocking the item is.

| Status | Color | Meaning |
|--------|-------|---------|
| HUMAN | 🔴 red | **You are blocking work.** Fix now. |
| NEW | 🟠 orange | Needs triage, not yet touched. |
| WIP | 🔵 blue | Agent working, calm. |
| DONE | 🟢 green (dim) | Complete, background info. |

Rainbow reserved for decorative elements only (header banner).

## Board Layout Per Repo

```
### paperclip (62 open · 154 done)

  🔴 HUMAN (0)

  🔵 WIP (0)

  🟠 NEW (62)
    #3701  BLOCKED: SocialData API credits exhausted
    #3753  Can self-hosted auth via Claude?
    #3783  fix(server): respect configured host
    #3784  feat: implement multi-user access
    #3787  fix(server): serialize cursor timestamp
    ... 57 more

  🟢 DONE (154 — collapsed)
```

Rules:
- **Section labels** always shown (HUMAN, WIP, NEW, DONE) even when empty — makes scanning consistent across repos
- **Empty sections** shown as just `🔴 HUMAN (0)` — no expansion, one line
- **Non-empty sections** show up to 5 items, then "... N more"
- **DONE always collapsed** with count, expandable via command (`show paperclip done`)
- **Repo header** shows open count + done count: `paperclip (62 open · 154 done)`

## Live Feed

Real-time event stream. Appears below a `─── live ───` divider.

Only shows **transition events** — items changing status. No `poll` events, no re-adding of existing notifications.

```
─ live ──────────────────────────────────────────────────

19:15:30  🟠 → 🔵  paperclip-tree#295  sync: capability flags
19:16:00  🔵 → 🔴  paperclip-tree#295  sync: capability flags  (needs architecture decision)
19:18:45  🟠 → 🟢  paperclip#3748  feat: OpenRouter adapter  (merged)
```

Format: `time  FROM → TO  repo#number  title  (optional reason)`

Colors use the same status palette. Time is local.

## Status Changes in Board

When a status changes:
- The item moves to its new section within the repo
- The repo's counts update
- A line appears in the live feed
- No duplicate entries, no history shown in the board

## Interaction

This is a **read-only** view. Actions happen through `/breeze` in Claude Code or `gh` CLI directly. The watch is a passive monitor.

## Refresh

The file `~/.breeze/inbox.json` is updated by `breeze-poll` every ~90s. The watch re-renders the board whenever the file changes. Live feed appends events from `~/.breeze/activity.log` as they arrive.

## Header Status Bar

```
breeze    78 new · 0 wip · 0 human · 192 done    updated 19:20
```

One line. Counts in status colors. Last update timestamp in dim.

## Dividers

Rainbow horizontal rule between header and board.
Dim horizontal rule between board and live feed, labeled `─── live ───`.

## Empty States

- **No new/wip/human items:** "✨ inbox zero — all caught up. (192 done)"
- **No activity log yet:** "waiting for first transition..."
- **Stale poll (>10 min):** warn in header: "⚠️ poller not running"

## Non-Goals

- No inline timeline/history per item (use GitHub for that)
- No interactive controls (it's a monitor, not a TUI app)
- No filtering UI (commands via `/breeze`)
- No search
