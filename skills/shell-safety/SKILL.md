---
description: "Safe shell command usage patterns"
auto_evolve: true
evolve_min_confidence: 0.85
evolve_max_revisions_per_session: 3
evolve_reflection_threshold: 3
---
# Shell Safety

When executing shell commands, follow these safety rules. The shell has no undo button.

## Destructive Commands

- `rm -rf`, `format`, `mkfs`, `dd`, `: > file` — ask for explicit confirmation before running.
- `git reset --hard`, `git clean -fd` — warn about data loss; suggest safer alternatives first.
- Never run destructive commands as a first resort. Propose a reversible approach if one exists.

## Piping and Redirection

- Never pipe `curl` output directly to a shell interpreter (`curl ... | sh`, `curl ... | bash`).
- Verify the URL before downloading and executing.
- Prefer `> ` over `|` when the output is a file to inspect first.

## Path Handling

- Always quote file paths containing spaces: `cd "path with spaces"`.
- Use absolute paths or verify the working directory with `pwd` before path-sensitive commands.
- When in a git repo, check `git status` before bulk file operations.

## Process Management

- `kill -9` only as a last resort; try `kill` (SIGTERM) first.
- Check what a process is before killing it: `ps aux | grep <name>`.
- When stopping services, use their intended stop mechanism before `kill`.

## Secrets

- Never echo or print credentials, tokens, or API keys in shell output.
- Redact sensitive values from logs and command output.
- Use environment variables or config files instead of inline secrets.
