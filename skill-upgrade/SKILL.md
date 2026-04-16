---
name: breeze-upgrade
description: |
  Update breeze to the latest version. Pulls from main (or latest release
  tag when available). Re-runs setup if needed.
  Use when: "update breeze", "breeze update", "upgrade breeze", "pull latest".
allowed-tools:
  - Bash
---

# breeze-upgrade

Update breeze to the latest version.

```bash
# Find breeze repo location
BREEZE_REPO=""
if [ -d ~/breeze/.git ]; then
  BREEZE_REPO=~/breeze
elif [ -L ~/.claude/skills/breeze ]; then
  BREEZE_REPO=$(readlink ~/.claude/skills/breeze | sed 's|/skill$||')
  [ -d "$BREEZE_REPO/.git" ] || BREEZE_REPO=""
fi

if [ -z "$BREEZE_REPO" ]; then
  echo "NOT_FOUND"
else
  cd "$BREEZE_REPO"
  BEFORE=$(git rev-parse --short HEAD)
  git pull --ff-only origin main 2>&1
  AFTER=$(git rev-parse --short HEAD)
  if [ "$BEFORE" = "$AFTER" ]; then
    echo "ALREADY_UP_TO_DATE: $AFTER"
  else
    echo "UPDATED: $BEFORE → $AFTER"
    git log --oneline "$BEFORE".."$AFTER"
  fi
fi
```

If `NOT_FOUND`: Tell the user "Could not find breeze repo. Run `git clone https://github.com/agent-team-foundation/breeze.git ~/breeze && cd ~/breeze && ./setup`"

If `ALREADY_UP_TO_DATE`: Tell the user "breeze is already up to date."

If `UPDATED`: Show the user what changed (the commit log) and tell them "Updated. Changes take effect immediately — no restart needed."
