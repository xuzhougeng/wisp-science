# Execution-context terminals

Wisp can open an interactive terminal for a registered execution context from
the **Contexts** panel. The terminal opens as a resizable dock below the
conversation and right panel, keeping the active research context visible.

## Context behavior

- `local` starts the user's login shell on macOS/Linux or PowerShell on Windows.
- `wsl:<distro>` starts that distribution with `wsl.exe` and asks WSL to use the
  active project directory. On Windows, **Import WSL** registers the installed
  distributions before opening one.
- `ssh:<alias>` starts the system OpenSSH client with a remote PTY. OpenSSH
  continues to honor SSH config, ssh-agent, ProxyJump, host-key prompts, and
  interactive authentication.

One live terminal is reused for each project/context pair. Closing its panel
detaches the view without terminating the shell; choosing **Open terminal** for
the same context reattaches it. Use the terminal panel's **Terminate** action
to end the process explicitly. Terminal sessions and scrollback are ephemeral
and are not written to SQLite or included in project sync.

Interactive terminals are deliberately separate from Runs. Use a terminal for
human-driven exploration, editors, monitors, and debugging. Use the Run Manager
for durable computation that needs lifecycle status, cancellation, output
harvesting, or provenance.

## Current limitations

- SSH sessions do not survive a network disconnect or Wisp restart. A future
  optional tmux integration may provide remote reattachment without making
  tmux a requirement.
- The initial SSH terminal starts in the remote user's home directory because
  project-to-remote workspace bindings are not modeled yet.
- WSL path handling relies on `wsl.exe --cd`; custom automount layouts should be
  verified on the target Windows installation.
- Terminal tabs are not yet persisted across an application restart, and only
  one terminal panel is displayed at a time.

## Manual smoke checks

On Windows, verify local PowerShell and an installed WSL distribution can open
in the bottom dock, resize, accept input, run a full-screen application,
close/reopen without losing the shell, and terminate explicitly. For SSH, use a
test alias from SSH config and verify host-key/password prompts, resize,
`Ctrl+C`, and disconnect handling.
Automated tests must not require a real WSL distribution or SSH host.
