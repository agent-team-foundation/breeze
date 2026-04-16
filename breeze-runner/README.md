# breeze-runner

`breeze-runner` is the unified local service for breeze. One process does both:

1. **Inbox refresh** — polls the active `gh` account every 60s, enriches
   notifications with GitHub labels + PR/Issue state, and writes
   `~/.breeze/inbox.json` (consumed by the statusline and the `/breeze` skill).
2. **Agent dispatch** — turns actionable inbox items into isolated task
   workspaces and runs local `codex` and/or `claude` CLI agents against them.

A single `breeze-runner run` (or `breeze-runner start` for background mode)
covers both. No separate `breeze-poll` launchd job is required once you run
the daemon — the shell script remains available as a standalone debugging
tool and as a `breeze-runner poll` one-shot.

## Commands

```bash
cargo run --manifest-path breeze-runner/Cargo.toml -- doctor
cargo run --manifest-path breeze-runner/Cargo.toml -- run
cargo run --manifest-path breeze-runner/Cargo.toml -- run-once
cargo run --manifest-path breeze-runner/Cargo.toml -- poll
cargo run --manifest-path breeze-runner/Cargo.toml -- start
cargo run --manifest-path breeze-runner/Cargo.toml -- status
cargo run --manifest-path breeze-runner/Cargo.toml -- stop
```

| Command     | Purpose                                                         |
|-------------|-----------------------------------------------------------------|
| `doctor`    | Validate `gh` auth, PATH, local state dirs                      |
| `run`       | Foreground daemon — inbox refresh loop + dispatch loop          |
| `run-once`  | One-shot dispatch (no background thread); useful for CI / tests |
| `poll`      | One-shot inbox refresh (replaces `bin/breeze-poll`)             |
| `start`     | Background daemon via `nohup`, logs under `~/.breeze/runner`    |
| `status`    | Print lock, heartbeat, queue snapshot                           |
| `stop`      | Stop the background daemon for the active gh identity           |

## Behavior

- Reuses the active `gh` identity for the configured host.
- Refuses to start if another `breeze-runner` instance is already running for the same `host + login + profile`.
- Sweeps actionable notification threads from the last 24 hours on every poll, even if they are already marked read, and only uses GitHub search as a slower backfill path.
- Creates one isolated `git worktree` per scheduled task.
- Prepares a local snapshot for each task before the agent starts so the agent can inspect GitHub context without re-fetching it.
- Launches `codex` and/or `claude` in round-robin order with dangerous local permissions.
- Keeps local agent/worktree fan-out high while brokering all in-task `gh` commands through a single paced queue.
- Persists task state, lock state, logs, and workspaces under `~/.breeze/runner` by default, and writes the TUI inbox to `~/.breeze/inbox.json` (override via `BREEZE_DIR`).

## Notes

- Public agent replies are instructed to include a disclosure sentence.
- Brokered `gh` commands are serialized and mutating operations are spaced out to reduce rate-limit pressure.
- `run-once` is the safest way to validate the whole loop before `start`.
- Workspaces are kept temporarily for inspection and are cleaned by `cleanup`.
