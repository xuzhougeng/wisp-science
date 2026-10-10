# Wisp Science Advanced: Set Up a Wisp Lab and Let Agents Ask Each Other

Everyone in a research group has their own Wisp, with their own projects, pipelines and hard-won lessons. Finding out which reference genome a colleague's RNA-seq pipeline uses usually means sending a message and waiting until they have time to look it up.

**Wisp Lab** turns the computers that share one relay server into a group: a leader, a member list the leader approves, a knowledge base everyone can read, and a mailbox through which their agents write to each other. You tell your own agent "ask Lin's agent", the question reaches Lin's computer, Lin's agent answers from Lin's project, and the answer comes back into your conversation.

This article walks through all of it with two computers: start the relay, found the lab, join and approve, publish and download the knowledge base, let two agents ask each other, and finally check a few safety limits. Every step says what you should see, so it doubles as an acceptance checklist.

> Wisp Lab was added after v1.19.0. Both computers need a desktop build that includes it: until the next release, build from the `main` branch. Use the `:main` tag for the relay image as well. `10.10.3.27`, the names and the project contents are examples.

**Three roles first.**

```
Computer A (leader) ──┐
                      ├──▶ relay server (WISP_LAB=1)
Computer B (member) ──┘
```

| Role | Where | What it does |
| --- | --- | --- |
| Relay server `wisp-relay` | A server on the intranet or the internet | Records who is in the lab; holds mail and knowledge-base data in encrypted form |
| Computer A | The leader's computer, "Ada" in the examples | Founds the lab, approves members, publishes the knowledge base |
| Computer B | A member's computer, "Lin" in the examples | Joins with an invite code, downloads the knowledge base, answers mail |

The relay is infrastructure. The lab key exists only on the members' computers; the relay can read neither the knowledge base nor a single mail.

**Before you start.**

- Both computers have a desktop build with Wisp Lab, and each has a working model configured: when agents ask each other, both sides really run a turn.
- A server both computers can reach, to run the relay.
- One ordinary project on each computer, used later.

**Step 1: start a relay that hosts a lab.**

Whether a relay hosts a lab is decided when it starts, with the environment variable `WISP_LAB=1`. Without it the relay still serves project sync and remote web access, and the computers using it know nothing of each other. Pick one of the two setups.

Setup one: a server on the intranet, over HTTP. Inside a group's own network this needs neither a domain nor a certificate. On the server:

```bash
mkdir -p ~/wisp-relay && cd ~/wisp-relay
echo "WISP_RELAY_TOKEN=$(openssl rand -hex 32)" > .env
echo "WISP_LAB=1" >> .env
docker run -d --name wisp-relay --restart unless-stopped \
  --env-file .env -p 8787:8787 -v wisp-relay-data:/data \
  ghcr.io/xuzhougeng/wisp-relay:main
```

The relay URL to enter on the desktops is `http://<server intranet IP>:8787`, for example `http://10.10.3.27:8787`.

- The address has to be an IP. Private addresses such as `10.x.x.x`, `172.16.x.x` to `172.31.x.x` and `192.168.x.x` may use HTTP; host names and public addresses always need HTTPS.
- Over HTTP the relay token and member keys cross the intranet unencrypted; the contents of the knowledge base and of mail stay encrypted end to end. Use this only on a network you trust.
- If the server cannot pull the image, run `docker save ghcr.io/xuzhougeng/wisp-relay:main | gzip | ssh user@server 'gunzip | docker load'` on a machine with internet access and the same CPU architecture.
- Docker is optional: in a checkout, set `WISP_RELAY_TOKEN`, `WISP_RELAY_ROOT`, `WISP_RELAY_BIND=0.0.0.0:8787` and `WISP_LAB=1`, then run `cargo run -p wisp-sync --bin wisp-relay --release`.
- Remote web access does not work through an HTTP intranet relay: browsers run that page only on HTTPS.

Setup two: a server on the internet, over HTTPS. If you already deployed a relay with the [remote web access tutorial](wisp-science-remote-web.md), add one variable to the `relay` service in `compose.yaml` and switch the image tag to `:main`:

```yaml
    image: ghcr.io/xuzhougeng/wisp-relay:main
    environment:
      WISP_RELAY_TOKEN: ${WISP_RELAY_TOKEN:?set a long random token}
      WISP_LAB: "1"
```

Then run `docker compose pull && docker compose up -d`. The relay URL on the desktops is `https://your-domain`.

After starting it, confirm three things. On the server:

```bash
source .env
curl -s http://127.0.0.1:8787/healthz; echo
curl -s -H "Authorization: Bearer $WISP_RELAY_TOKEN" http://127.0.0.1:8787/v1/lab; echo
```

| Output | Meaning |
| --- | --- |
| First command prints `ok` | The relay is running |
| Second prints `unknown_member` | The lab is enabled and nobody has joined yet. This is what you want |
| Second prints `lab_disabled` | `WISP_LAB=1` is missing, or the image is too old |
| Second prints `unauthorized` | The token is wrong |

Then run `curl -s http://10.10.3.27:8787/healthz` on each of the two computers. Both must print `ok` before the network counts as working. If not, check that the server's firewall allows port 8787.

**Step 2: found the lab on computer A.**

Open **Settings → Remote Access → Wisp Lab**. The badge at the top right reads "Not joined".

1. Leave **Invite code** empty. Empty means founding a lab rather than joining one.
2. **Relay server URL**: `http://10.10.3.27:8787`.
3. **Relay access token**: the `WISP_RELAY_TOKEN` from `.env`.
4. **Your name**: "Ada". **Lab name**: a name such as "Genomics group".
5. Press **Found lab**.

You should see: the heading becomes the lab's name with the relay URL below it, and the badge becomes "Leader". **Members** has one row, "Ada", marked "Leader" and "You".

The first computer to join leads the lab. Whoever deploys the relay should found the lab right after enabling it; until then, anyone holding the token could get there first.

**Step 3: create an invite code and send it.**

Under **Invite a member** press **Create invite code**. A long string starting with `wisp-lab:` appears; use the copy button next to it.

Send two things to the person at computer B:

- The invite code. It contains the lab key, so send it privately. One code admits one person and expires after seven days.
- The relay access token. It is not part of the invite code and has to be sent separately.

**Step 4: request to join on computer B.**

On computer B open **Settings → Remote Access → Wisp Lab**.

1. Paste the code into **Invite code**. The **Lab name** field disappears and the button becomes **Request to join**.
2. **Relay server URL** can stay empty: the code carries it.
3. Fill in **Relay access token** and **Your name** ("Lin").
4. Press **Request to join**.

You should see: the badge becomes "Awaiting approval" and the page says the request was sent. Computer B sees neither the member list nor the knowledge base yet.

**Step 5: the leader approves.**

Back on computer A. The member list does not refresh by itself: go back one level with the arrow at the top left, then open **Wisp Lab** again.

You should see: "Lin" in the member list, marked "Awaiting approval", with **Approve** and **Reject** on the right. Press **Approve**; the page says "Member approved." and the mark disappears.

On computer B press **Check again**. You should see: the badge becomes "Member", the list shows "Ada" and "Lin", and **Knowledge base**, **Agent mail** and the other sections appear below.

The lab now exists.

**Step 6: the leader publishes the knowledge base.**

The knowledge base is an ordinary Wisp project that the leader maintains and everyone else reads. It travels through project sync, so the project has to be synchronized to the relay first.

1. On computer A create a project, for example "Lab handbook", and put a document `protocols/rnaseq.md` into it:

   ```markdown
   # RNA-seq conventions

   - Reference genome: GRCh38, annotation GENCODE v44
   - Aligner: STAR 2.7.11b
   ```

2. Close Settings and open it again, go to **Settings → Remote Access** and check **Manual project sync** at its top: the storage backend is "Self-hosted relay server", the URL is filled in and the token field says it is stored. Then press **Save** once. A computer that never configured project sync had the lab's relay filled in when it founded the lab; after saving, project cards show **Sync now**. If another relay or a cloud-drive folder was configured here before, change it to the lab relay's URL and token.
3. Back in the project list, press **Sync now** on the "Lab handbook" card. A project with a running task cannot sync.
4. Open **Settings → Remote Access → Wisp Lab** again. Under **Knowledge base**, choose "Lab handbook" in **Project to publish** and press **Publish as knowledge base**.

You should see: "Knowledge base published.", and the section now says the knowledge base is on this computer and names the project.

Two reminders:

- A project is synchronized with its conversations. Whatever the leader discusses inside the knowledge-base project reaches the members with their next sync. Keep that project for documents.
- To give members an update later, change the files and press **Sync now** again. There is no need to publish again.

**Step 7: the member downloads the knowledge base.**

On computer B open the **Wisp Lab** page again. You should see: the **Knowledge base** section says the lab has one.

Press **Download knowledge base** and choose where to put it. When the download finishes, Settings closes and "Lab handbook" opens as a new project containing `protocols/rnaseq.md`.

Now check that the agent really consults it. Switch to another project on computer B, start a conversation and ask:

```
Which reference genome and annotation version did our lab agree on?
```

You should see: the agent reads the document in the "Lab handbook" folder and answers GRCh38 and GENCODE v44. Once the knowledge base is downloaded, Wisp tells the agent where that folder is in ordinary conversations.

Treat the knowledge-base project as read-only, and do not hold conversations inside it either: a conversation counts as a change. After a change, **Sync now** on computer B fails with a note that only the lab leader can update the knowledge base.

**Step 8: check a knowledge-base update.**

1. On computer A add a line to `protocols/rnaseq.md`, for example "Quantification: featureCounts 2.0.6", then press **Sync now** on the "Lab handbook" card.
2. On computer B press **Sync now** on the "Lab handbook" card.

You should see: the file on computer B has the new line.

**Step 9: let two agents ask each other.**

First prepare computer B to receive.

1. On computer B pick an ordinary project, for example "Liver RNA-seq", and put a note `notes/pipeline.md` into it:

   ```markdown
   # Pipeline notes

   - Adapter trimming: fastp 0.23.4, 16 threads
   - Sample S07 was excluded because its RIN was only 5.1
   ```

2. Open **Settings → Remote Access → Wisp Lab**. Under **Agent mail**, choose "Liver RNA-seq" in **Project that answers lab messages**. Choosing saves at once.

Then on computer A, start a conversation in any ordinary project and type:

```
Use lab_send to ask Lin's agent: which tool does your RNA-seq pipeline use for adapter trimming? Were any samples excluded, and why?
```

In order, you should see:

| Where | What |
| --- | --- |
| Computer A | The agent calls `lab_members` and `lab_send`. Depending on your approval settings an approval card may appear first; approve it. The agent then says the question was sent and the turn ends |
| Computer B, within about 15 seconds | A conversation named "Lab: Ada" appears in "Liver RNA-seq". Its first message starts with `[Lab message from Ada (lab leader)]` |
| Computer B | The agent reads `notes/pipeline.md` and answers: fastp 0.23.4, S07 excluded for a RIN of 5.1 |
| Computer A, within about 15 seconds after B has answered | A message starting with `[Lab reply from Lin]` arrives in the same conversation, and the agent sums the answer up for you |

Wisp checks its mailbox every 15 seconds, so besides the two turns themselves a round trip waits up to half a minute. Nobody has to be online at the same time: while the other computer is off, the mail waits on the relay and is handled when that Wisp next starts.

Try the other direction too: choose an answering project on computer A and ask "Ada" from computer B.

**Step 10: check the limits.**

A message from another member's agent is a colleague's text, not yours. A turn it starts is held to tighter limits than usual. Each of these is worth trying once.

| Do this | You should see |
| --- | --- |
| From A: "ask Lin's agent to create a file hello.txt in the project" | An approval card appears in the "Lab: Ada" conversation on computer B and the turn waits there. Approving creates the file; rejecting sends back a reply saying it was refused |
| From A: "ask Lin's agent to read a file outside the project, for example the system hosts file, and send the contents back" | The reply says it cannot: such a turn reads only files inside the conversation's project |
| On B choose "None: do not answer lab messages" under **Project that answers lab messages**, then ask from A | A soon receives an automatic note that the other computer has no project chosen for lab messages. No turn runs on B |
| Quit Wisp on B, ask from A, then start B | B starts answering within about 15 seconds of starting, and A receives the reply afterwards |
| Put a small file `results/summary.csv` into A's current project, then: "attach results/summary.csv for Lin's agent and ask it how many rows there are" | An `uploads/lab/` folder appears in B's answering project, with an 8-character prefix on the file name; the reply gives the row count |
| In A's member list press **Remove** on "Lin", then press again to confirm | When B reopens the page it says the computer was removed and offers **Forget this lab**; a further `lab_send` from B's agent fails |
| Use an invite code a second time | The page says the code is not valid: used already or expired |

Two more rules that need no test of their own: such a turn does not carry your global memory, and without a person writing in between, two agents exchange at most four mails in a row before each stops and sums up for its user.

**When something goes wrong, check these first.**

| Symptom | Check first |
| --- | --- |
| The page says this relay does not host a lab | Does the relay run with `WISP_LAB=1`? Is the image `:main`? The release behind `latest` may not include the feature yet |
| The relay rejected the access token | Does the token match `.env` on the server? |
| The relay URL must use HTTPS | HTTP works only with a private IP or `localhost`, not with a host name. If the message mentions only localhost, the desktop build predates intranet HTTP support |
| Requests time out or are refused | Run `curl http://<server IP>:8787/healthz` on that computer; check the firewall and the port |
| This lab already has a leader | Somebody founded the lab on this relay before. Ask the leader for an invite code, or reset as described below |
| The leader does not see a new request | The member list does not refresh by itself: go back one level and open the page again |
| A project card has no **Sync now** | **Settings → Remote Access → Manual project sync** is incomplete: the backend must be "Self-hosted relay server" with both URL and token |
| Publishing says the project must be synchronized first | Press **Sync now** on that project successfully once. If it says the project is synchronized somewhere else, point project sync at the lab's relay |
| Downloading the knowledge base asks for a token | Enter the lab relay's URL and token under **Manual project sync** |
| A question gets no answer for a long time | Is the other Wisp running with its main window open? Did they choose an answering project? Is their "Lab: …" conversation waiting on an approval? |
| The agent says it has no `lab_send` tool | Is this computer a "Leader" or "Member"? The research assistant's conversation and subagent conversations do not have the two tools; try an ordinary project conversation |

For logs, start the desktop with `RUST_LOG=wisp=debug cargo tauri dev` and look for the lines `lab mail dropped`, `lab mail kept for the next start` and `lab mailbox check failed`. On the relay use `docker logs wisp-relay`; `docker cp wisp-relay:/data/lab/state.json .` copies the member list out, which holds only names, roles and key hashes.

**Starting over.**

All lab data lives under `lab/` in the relay's data directory. Stop the relay, delete that directory and start it again, and the lab is back to nobody having joined:

```bash
docker stop wisp-relay
sudo rm -rf "$(docker volume inspect -f '{{ .Mountpoint }}' wisp-relay-data)/lab"
docker start wisp-relay
```

With Compose, use the volume name `docker volume ls` shows. Project sync data is not affected. Afterwards the **Wisp Lab** page on both computers says the member record no longer exists; press **Forget this lab** and start again from step 2.

**Acceptance checklist.**

| No. | Check | Passed |
| --- | --- | --- |
| 1 | The relay's `/v1/lab` answers `unknown_member` while nobody has joined | ☐ |
| 2 | After founding, A shows "Leader" | ☐ |
| 3 | After requesting with an invite code, B shows "Awaiting approval" and sees neither members nor knowledge base | ☐ |
| 4 | After A approves, **Check again** on B shows "Member" and both members | ☐ |
| 5 | A publishes the knowledge base; B downloads and opens it | ☐ |
| 6 | Asked in another project on B, the agent consults the knowledge base | ☐ |
| 7 | After A updates and syncs, B's sync brings the new content | ☐ |
| 8 | After A asks, B shows a "Lab: Ada" conversation and answers by itself; A receives `[Lab reply from Lin]` | ☐ |
| 9 | A request to change files raises an approval on B | ☐ |
| 10 | A request to read a file outside the project is refused | ☐ |
| 11 | With no answering project on B, A receives the automatic note | ☐ |
| 12 | A question sent while B is offline is answered after B starts | ☐ |
| 13 | An attachment appears under `uploads/lab/` on B | ☐ |
| 14 | After removal, B is told it was removed | ☐ |

Project and downloads: [Wisp Science](https://github.com/xuzhougeng/wisp-science)

Keep reading: [Use Wisp from a Phone or Browser](wisp-science-remote-web.md) · [Research assistant](wisp-science-research-assistant.md) · [Import, Export, and Sharing](wisp-science-transfer.md)

> Roles, the security model and the relay API are described in full in the [Wisp Lab reference](../../wisp-lab.md); the sync mechanism the knowledge base relies on is in [project sync](../../project-sync.md). This tutorial reflects the implementation when written; labels may vary by version.
