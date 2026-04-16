---
name: breeze-watch
description: |
  Open a live activity log for breeze notifications. Shows all events
  (new notifications, status changes, agent actions) streaming in real time
  with clickable GitHub links. Runs in a new terminal window.
  Use when: "watch", "breeze watch", "show log", "activity log", "notification log".
allowed-tools:
  - Bash
---

# breeze-watch — live activity log

Opens a new terminal window streaming breeze activity with clickable GitHub links.

## Launch

```bash
BREEZE_WATCH=$(find ~/.claude/skills -name breeze-watch -path "*/bin/*" -type f 2>/dev/null | head -1)
[ -z "$BREEZE_WATCH" ] && BREEZE_WATCH=$(find ~/breeze -name breeze-watch -path "*/bin/*" -type f 2>/dev/null | head -1)

if [ -z "$BREEZE_WATCH" ] || [ ! -x "$BREEZE_WATCH" ]; then
  echo "BREEZE_WATCH_NOT_FOUND"
else
  echo "BREEZE_WATCH: $BREEZE_WATCH"
fi
```

If `BREEZE_WATCH_NOT_FOUND`: Tell the user "breeze-watch script not found. Run `cd ~/breeze && ./setup` first."

Otherwise, open a new terminal window running the watch script:

```bash
osascript -e "
  tell application \"Terminal\"
    activate
    do script \"$BREEZE_WATCH\"
  end tell
" 2>/dev/null || open -a Terminal "$BREEZE_WATCH" 2>/dev/null || echo "Could not open new terminal. Run this manually: $BREEZE_WATCH"
```

Tell the user: "Opened breeze-watch in a new terminal window. It streams all notification events with clickable GitHub links."
