# Wisp Lab

[中文说明](wisp-lab.zh-CN.md)

A `wisp-relay` serves any number of independent computers. Wisp Lab is an
optional layer on top of it: the computers sharing one relay form a research
group with a leader, an approved member list, a shared knowledge base and a
mailbox through which their agents write to each other.

The relay stays infrastructure. It stores who belongs to the lab and who wrote
to whom. The lab key never reaches it, so it cannot read the knowledge base
or a single mail.

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

### A relay inside the lab's network, over HTTP

A lab relay does not have to be on the internet. On a server inside the
group's own network it needs neither a domain nor a certificate:

```bash
echo "WISP_RELAY_TOKEN=$(openssl rand -hex 32)" > .env
echo "WISP_LAB=1" >> .env
docker run -d --name wisp-relay --restart unless-stopped \
  --env-file .env -p 8787:8787 -v wisp-relay-data:/data \
  ghcr.io/xuzhougeng/wisp-relay:main
```

On the desktops enter the server's address as the relay URL, for example
`http://10.10.3.27:8787`. Plain HTTP is accepted for private network
addresses written as IP literals (`10.x.x.x`, `172.16.x.x` to `172.31.x.x`,
`192.168.x.x`, link-local, and their IPv6 counterparts) and for `localhost`.
A host name always needs HTTPS.

Over HTTP the relay token and each member key cross that network unencrypted.
What they give access to stays encrypted end to end: the knowledge base, lab
mail and its attachments. Somebody who can read the network's traffic could
therefore act as a member towards the relay, for instance delete waiting
mail, but could not read or forge its contents. Use HTTP only on a network
you trust, and put a TLS proxy in front otherwise.

[Remote web access](remote-access.md) does not work through such a relay:
browsers run its page only on HTTPS.

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

## Found a lab

1. Deploy the relay with `WISP_LAB=1`.
2. On your desktop open **Settings → Remote Access → Wisp Lab**.
3. Leave **Invite code** empty. Enter the relay URL, the relay's
   `WISP_RELAY_TOKEN`, your name and a name for the lab, then press
   **Found lab**.

Your computer generates the lab key and its own member key and keeps both in
the OS keyring. You are now the leader.

To add someone, press **Create invite code**, send them the code privately and
send them the relay token separately. They appear in **Members** as *Awaiting
approval*; press **Approve** to let them in or **Reject** to turn them away.
**Remove** takes a member out later. Reject and Remove ask for a second click.

## Join a lab

1. Open **Settings → Remote Access → Wisp Lab**.
2. Paste the invite code. The relay URL is part of it, so that field can stay
   empty. Enter the relay token and your name, then press **Request to join**.
3. The page shows *Awaiting approval*. Once the leader has approved you,
   **Check again** shows the members and the knowledge base.

**Leave lab** removes you from the member list and deletes the lab key from
this computer. Rejoining needs a new invite code.

If the relay's operator changes `WISP_RELAY_TOKEN`, enter the new one under
**Relay access token** and press **Update token**. If the leader removed this
computer, the page says so and offers **Forget this lab**, which only clears
this computer's copy of the keys.

## Knowledge base

The knowledge base is an ordinary Wisp project that the leader keeps and
everyone else reads.

**Leader.** Put the group's protocols, reference notes and shared scripts into
a project, press **Sync now** on its project card so it exists on the relay,
then choose it under **Knowledge base** and press **Publish as knowledge
base**. From then on the relay accepts changes to that project from you only.
Press **Sync now** again whenever you want members to get an update.

**Members.** Press **Download knowledge base** and choose where to put it. It
becomes a project on your computer, and **Sync now** on its card pulls the
leader's later changes. To contribute, send the material to the leader.

Treat that project as read-only. Anything you change in it stays on your
computer, and a conversation held inside it counts as a change too: **Sync
now** then fails with a note that only the lab leader can update the
knowledge base. Once the leader has published a newer version, **Sync now**
reports a conflict instead, and **Use remote version** in that dialog brings
the leader's version back and discards your local changes. Ask your questions
about the knowledge base from another project; your agent reads the folder
from there.

A project is synchronized with its conversations. Whatever the leader
discusses inside the knowledge-base project reaches every member with the
next **Sync now**, so keep that project for documents.

Once the knowledge base is on your computer, Wisp tells your agent where the
folder is in ordinary conversations, so it consults the group's protocols and
conventions before answering lab-specific questions.

The knowledge base travels through [project sync](project-sync.md), which
reads the relay token from **Settings → General → Manual project sync**. A
computer that never configured project sync gets the lab's relay URL and
token filled in there when it joins, so **Sync now** works right away. An
existing sync configuration is not changed: if it points at a different
relay, the knowledge base cannot be downloaded until it points at the lab's.

## Agent mail

Members' agents can ask each other questions. Nobody has to be online at the
same time: a message waits in the recipient's mailbox on the relay until
their computer fetches it.

**Receiving.** Under **Agent mail**, choose the **Project that answers lab
messages**. Until you do, a question sent to this computer gets an automatic
note that it does not take lab mail, so nobody waits for an answer that will
not come. Each sender gets one conversation in that project, named
*Lab: <name>*. When a message arrives, your agent runs one turn there and its
answer is mailed back automatically. Attached files are saved under
`uploads/lab/` in that project.

**Asking.** Your agent has two tools once this computer is in a lab:

- `lab_members` lists the members.
- `lab_send` sends a question to one member, optionally with up to four files
  from the current project. The other agent sees only that message and those
  files, not your conversation.

So you can write "ask Lin's agent which reference genome their RNA-seq
pipeline uses" in any project conversation. The answer arrives later in the
same conversation as a message starting with `[Lab reply from Lin]`, and your
agent continues from it.

Wisp checks the mailbox every 15 seconds while it is running. If your
computer was off, waiting messages are answered when it starts again.

### What a lab message may do

A message from another member's agent is a colleague's text, not yours. A
turn it starts, on either side, is held to these limits:

- **Changes ask first.** Writing or editing files, shell commands, kernels and
  every other changing tool need your approval on the desktop, even if your
  default is Allow. This is the same floor as for IM channels.
- **Reads stay inside the project.** The agent can read and search only the
  project the conversation belongs to: the one you chose for lab mail, or, for
  a reply, the one that asked.
- **Only a few tools run unasked.** Reading, searching and viewing the
  project's files, loading a skill, listing lab members and finishing the
  answer. Everything else asks first, including tools that only read: memory
  search, the browser, Run status and every connector, bundled or your own.
- **No global memory.** Your global memory is left out of that turn.
- **Further mail asks first.** An agent that wants to write to the lab from
  such a turn needs your approval, Full Permission or not.
- **Four in a row at most.** Without a person writing in between, an exchange
  can go ask, reply, ask, reply and then stops: the agent is told to summarize
  for its user instead. Since every further mail already needs approval, this
  is a second line behind that rule.

The automatic answer to a question is the one thing that leaves without your
review. It is your agent's reply text, written from the project you chose.
Choose a project whose contents you are willing to discuss with the lab.

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
- **Attachments are encrypted blobs.** A file sent with `lab_send` is
  encrypted with the lab key and stored like a sync blob; the mail carries
  its hash, size and name. The receiving desktop checks all three and writes
  the file under `uploads/lab/` with a name it sanitized itself. The relay
  keeps blobs indefinitely, as it does for project sync.
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
was interrupted is delivered again. The desktop acknowledges a mail once its
turn has finished, and remembers handled mail for as long as it runs, so a
lost acknowledgement is repeated rather than the turn. A desktop that quits
mid-turn answers that mail again after restarting. A reply whose turn could
not start, or an answer that could not be mailed back, also stays in the
mailbox and is tried again at the next start, so its text is not lost. A mail
is at most 256 KiB sealed and a mailbox holds 500; attachments travel as relay
blobs. The relay side is
`crates/wisp-sync/src/lab.rs`; the desktop side is
`src-tauri/src/channels/lab.rs` (membership) and `lab_mail.rs` (mail and the
two tools).

## Not yet included

- Leadership cannot be transferred, and the leader cannot leave.
- There are no group messages: `lab_send` writes to one member.
- The desktop polls for mail; nothing is pushed.
- A pending member learns of approval by pressing **Check again**.
- Members cannot push to the knowledge base; contributions go through the
  leader.
