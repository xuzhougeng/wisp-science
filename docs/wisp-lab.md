# Wisp Lab

A `wisp-relay` serves any number of independent computers. Wisp Lab is an
optional layer on top of it: the computers sharing one relay form a research
group with a leader, an approved member list, a shared knowledge base and a
mailbox through which their agents write to each other.

The relay stays infrastructure. It stores who belongs to the lab and who wrote
to whom. The lab key never reaches it, so it cannot read the knowledge base
or a single mail.

> This page currently covers the relay side. The desktop screens for joining a
> lab, reviewing members, publishing the knowledge base and agent mail are
> added in follow-up changes and documented here as they land.

## Enable it on the relay

The lab is chosen when the relay is deployed. Add one variable to the
[relay setup](remote-access.md#deploy-the-relay):

```bash
export WISP_LAB=1
```

With Docker Compose, add `WISP_LAB: "1"` under the relay's `environment`.

Without the variable every `/v1/lab/*` route answers 404, nothing is written
under `lab/`, and the computers using the relay know nothing of each other.
Project sync and remote web access behave the same either way.

Lab data lives in `lab/` inside `WISP_RELAY_ROOT`: `state.json` holds the
members, outstanding invites and the knowledge-base pointer, and `inbox/`
holds undelivered mail. Back it up with the rest of the relay directory.
Turning the variable off later hides the lab without deleting it.

## Roles

- **The first computer to join founds the lab and leads it.** Whoever deploys
  the relay should join right after enabling the lab: until then, anyone
  holding the relay token could take the leader's place.
- **Everyone else needs an invite code from the leader**, and stays *pending*
  until the leader approves them. A pending member sees the lab's name and
  nothing else: no member list, no knowledge base, no mail.
- **The leader** issues invite codes, approves or rejects pending members,
  removes members and publishes the knowledge base.
- **A member** can leave. The leader cannot, and leadership cannot be handed
  over yet.

An invite code admits one member and expires after seven days. It contains
the lab key, so treat it like a password. It does not contain the relay
token: hand that over separately, as with a project device code.

## Security model

- **Two secrets per member.** The relay token proves a computer may use the
  relay at all. A member key, 256 random bits generated on the desktop and
  kept in the OS keyring, proves which member it is. The relay stores only
  the key's SHA-256.
- **The relay never sees the lab key.** It travels inside invite codes, from
  person to person. The knowledge-base pointer and every mail are sealed with
  it (AES-256-GCM) before they leave a desktop.
- **Invites are registered as hashes.** The leader's desktop generates the
  invite nonce and registers its SHA-256. Somebody who reads the relay's disk
  cannot join with a hash.
- **Mail is bound to both ends.** The relay stamps each mail with the sender
  it authenticated, and the mail's authentication data names sender and
  recipient. A relay that delivers a mail to someone else, or renames its
  sender, produces a mail that fails to open.
- **Only the leader writes the knowledge base.** It is an ordinary
  [synced project](project-sync.md). Once the leader registers it, the relay
  refuses commits to that project from anyone else. Members read it with the
  project key they receive through the sealed pointer.
- **Removal revokes the member key, not the lab key.** A removed member can no
  longer list members, read new mail or send any. They still hold the lab key
  and the relay token, so they could keep pulling knowledge-base revisions.
  To cut that off, rotate `WISP_RELAY_TOKEN` and hand the new one to the
  remaining members.

## Relay API (for contributors)

Every route needs the relay bearer token. All but `join` also need the member
key in `x-wisp-member`. A refusal carries a short code as its body, such as
`lab_disabled`, `unknown_member`, `not_active`, `not_leader`,
`invite_required` or `inbox_full`.

| Route | Who | Purpose |
| --- | --- | --- |
| `POST /v1/lab/join` | anyone with the token | Found the lab, or join it with an invite |
| `GET /v1/lab` | member | The lab as this member may see it |
| `POST /v1/lab/invites` | leader | Register the hash of an invite nonce |
| `POST /v1/lab/members/{id}/approve` | leader | Activate a pending member |
| `DELETE /v1/lab/members/{id}` | leader, or the member itself | Remove, reject or leave |
| `PUT /v1/lab/knowledge-base` | leader | Publish the sealed pointer |
| `POST /v1/lab/members/{id}/mail` | active member | Leave a sealed mail for a member |
| `GET /v1/lab/mail` | active member | The oldest waiting mails |
| `DELETE /v1/lab/mail/{id}` | active member | Acknowledge one mail |

Mail stays in the mailbox until it is acknowledged, so a mail whose delivery
was interrupted is delivered again. A mail is at most 256 KiB sealed and a
mailbox holds 500; attachments travel as relay blobs. The implementation is
`crates/wisp-sync/src/lab.rs`.
