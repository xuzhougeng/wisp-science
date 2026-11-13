# SSH through jump hosts

In **Settings → Environments → Add SSH host** (or **Edit server**), fill in the
destination server's address, user, port, and authentication, then set
**Jump host (ProxyJump, optional)**. The jump route is saved with the host and
its execution context. Connection tests, probes, Runs, Python/R runtimes,
file browsing, local SCP/rsync transfers, and the interactive terminal all use
the same route through the shared `SshConnection`.

| Value | Behavior |
| --- | --- |
| `bastion` | Resolve this jump alias using the local OpenSSH config |
| `alice@bastion.example.org:2222` | Connect through this user, host, and port |
| `gateway,alice@login:2200` | Visit the two jump hosts in order |
| `alice@[2001:db8::1]:2222` | Use a bracketed IPv6 jump address |
| Empty | Inherit the route from the local SSH config, if configured |
| `none` | Explicitly disable ProxyJump for this destination |

Jump aliases refer to **OpenSSH config aliases**, not other names registered
only in Wisp. A jump host's user, port, and key are independent of those for
the destination. Wisp passes a validated `-J` argument to the installed
OpenSSH client; it does not concatenate login commands or copy private keys.
Up to 16 explicit hops are accepted. Whitespace between hops, shell commands,
and invalid ports are rejected before connecting.

## Existing SSH config and separate keys

The config file is `~/.ssh/config` on macOS/Linux and
`%USERPROFILE%\.ssh\config` on Windows. For example:

```sshconfig
Host bastion
    HostName bastion.example.org
    User gateway-user
    Port 2222
    IdentityFile ~/.ssh/id_ed25519_bastion
    IdentitiesOnly yes
    BatchMode yes

Host private-gpu
    HostName 10.0.0.5
    User researcher
    ProxyJump bastion
    IdentityFile ~/.ssh/id_ed25519_gpu
    IdentitiesOnly yes
```

Use **Import all from ~/.ssh/config** and select `private-gpu`, leaving its
address and jump field blank to retain the config's alias resolution. Or
register a destination address manually in Wisp and enter `bastion` as its
jump host. The destination identity file in Wisp does not select the bastion's
identity file. See the [OpenSSH ProxyJump reference](https://man.openbsd.org/ssh_config.5#ProxyJump)
for configuration precedence and per-hop settings.

## Passwords and interactive authentication

The destination can use Wisp's saved password while the jump hosts use keys or
an agent. Before creating the password helper, Wisp resolves the route with
the local client's `ssh -G` and requires **`BatchMode yes` on every jump host**,
including routes inherited from SSH config. This prevents a jump SSH process
from consuming the destination's password through its inherited ASKPASS helper.
Configuration errors stop the attempt before the password helper is created.
Destination passwords remain in the existing OS keyring; no per-hop passwords
are stored in the host registry or execution-context JSON.

Verify each jump host's fingerprint and configure its key/agent login first.
If its host key is not known yet, an administrator-approved interactive login
such as `ssh -o BatchMode=no bastion` can show the confirmation prompt. Verify
the fingerprint rather than disabling host-key checks. Then use **Test
connection** and **Probe** in Wisp.

Wisp does not automatically select separate jump-host passwords, answer MFA
challenges, or drive a bastion's interactive host-selection menu. For a manual
terminal session, the **SSH key / agent** authentication setting leaves
OpenSSH's interactive password/MFA prompts available; this does not make such
a route usable by unattended Runs. The gateway must allow TCP forwarding for
ProxyJump to reach the destination.

## Port knocking is a separate prerequisite

Port knocking temporarily opens a firewall port after an administrator-defined
packet sequence. It does not establish an SSH connection or replace a jump
host. Wisp does not send a guessed knock sequence or run a connection pre-hook.
If the server requires knocking or a VPN, follow the administrator's procedure
first, then test the configured SSH route while access is open. Confirm the
required protocol, port order, and access window with the administrator.

## Verification

Regression coverage uses argument construction, mocked `ssh -G` results,
temporary SQLite databases, DTO round trips, and the mocked Tauri bridge. It
requires no real SSH server, credentials, or network. Local test execution
requires the permission specified in `AGENTS.md`.

Manual smoke steps, when authorized, on both macOS and Windows with a current
OpenSSH client:

1. Register a private destination with a jump alias and verify **Test connection**
   and **Probe**. Open its terminal, browse files, and run a short managed command.
2. Transfer a small disposable file to/from the destination. Where rsync is
   installed on both endpoints, check that its explicit transport uses the route.
3. Edit the route, reopen the form, then clear it to restore SSH-config inheritance.
   Open a new host form and verify it starts with an empty jump field.
4. With a saved destination password, remove `BatchMode yes` from one jump's
   config and verify Wisp reports the configuration requirement. Restore it and
   verify connection succeeds with key authentication on that jump.
5. Open the host form above Settings and press Escape immediately: only the form
   closes. Settings stays open. Do not move focus before pressing Escape.
