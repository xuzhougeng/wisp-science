# Deploy Wisp Remote Web Access on Sealos

With the Wisp relay deployed on Sealos, you can reach the Wisp on your computer from a phone or a browser: read conversations, send messages, preview files and answer approvals. This article starts from creating the app and goes through deploying the relay, connecting the desktop app and checking the result in a browser.

The connection looks like this:

```text
phone / browser ←→ relay on Sealos ←→ Wisp on your computer
```

Sealos serves the web page and forwards encrypted data. The agent, the model calls and the computation still run on your computer. So while you use it remotely, **the computer has to stay online and awake, and Wisp has to keep running**. The computer dials out to the relay, so it needs no public IP address and no inbound port opened on a router.

> This article follows a deployment done on October 8, 2026. The Sealos screenshot shows the console in its Chinese interface, with the domain masked; the Wisp settings screenshot is the demonstration capture from the repository. Field names for the Sealos console follow its English documentation, and the console's own wording may differ slightly. The desktop app needs remote web access, which was added after v1.18.0.

**Step 1: create the app.**

Sign in to Sealos, open **App Launchpad** and create a new app. Fill it in as below; for where each field is, see [Deploy Your First App](https://sealos.io/docs/guides/app-deploy/first-deploy/) in the Sealos documentation.

| Setting | Value |
| --- | --- |
| App name | `wisp-relay` |
| Image source | Public |
| Image | `ghcr.io/xuzhougeng/wisp-relay:main` |
| Deploy mode | Fixed instances |
| Instance count | `1` |
| CPU | `0.2` cores |
| Memory | `256 M` |

When this deployment was checked, the registry only had the `main` tag, and `latest` failed with `not found`. This tutorial therefore uses the `main` image, which was verified to pull. `main` follows the development branch; once a release image is available, you can pin an explicit version tag instead.

The resources in the screenshot are a starting point for personal use; adjust them later to what you actually use. Keep the instance count at **1**: the relay pairs each computer with its browsers inside one process, and with several instances the two sides may land on different processes.

**Step 2: configure networking.**

In the network section, enter:

| Setting | Value |
| --- | --- |
| Container port | `8787` |
| Public Access | Enabled |
| Protocol | `https://` |

![Sealos app settings: the public main image, one fixed instance, 0.2 CPU cores, 256 M of memory and an HTTPS public entry on port 8787](../../assets/tutorials/en/sealos-remote-web/01-deployment.png)

*Figure 1: The basic and network settings of this deployment, in the console's Chinese interface. From top to bottom: the app name, the image, the deploy mode with its instance count, CPU and memory, then the container port and public access. The public domain is masked.*

Sealos assigns a public domain, which goes into Wisp later. This deployment used the **HTTPS entry** shown in the screenshot and connected successfully, so keep that setting. Sealos provides the HTTPS entry; there is no Caddy to deploy and no certificate to configure by hand.

Remote web access uses an HTTPS page and a long-lived WSS connection. If you add your own reverse proxy later, make sure WebSocket requests on the same domain are also forwarded to port `8787` of the container.

**Step 3: set the environment variables.**

Open the advanced section and leave the startup command and its arguments empty, so the image starts the way it was built to.

Add these environment variables:

| Key | Value |
| --- | --- |
| `WISP_RELAY_TOKEN` | A long random token that you generate |
| `WISP_RELAY_BIND` | `0.0.0.0:8787` |
| `WISP_RELAY_ROOT` | `/data` |

With OpenSSL installed, generate the token in a terminal:

```bash
openssl rand -hex 32
```

On Windows without OpenSSL, PowerShell can generate it too:

```powershell
$bytes = New-Object byte[] 32
$rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
$rng.GetBytes($bytes)
$rng.Dispose()
([System.BitConverter]::ToString($bytes)).Replace('-', '').ToLowerInvariant()
```

Copy the output into the value of `WISP_RELAY_TOKEN` and keep it somewhere safe: the desktop app needs the same token.

**For remote web access alone, do not add a storage mount at `/data`.** Keeping the `WISP_RELAY_ROOT=/data` environment variable is enough; the program uses the writable directory that already exists in the image.

Remote web access keeps no data. An extra storage volume can replace the directory permissions that come with the image, and the container then fails to create its directories at startup. If you need project sync later, configure persistent storage separately and make sure the container user `10001:10001` can write to it.

**Step 4: deploy and check.**

Deploy the app and wait until the instance is running.

Say the address Sealos assigned is `https://your-app-domain`. First open this in a browser:

```text
https://your-app-domain/healthz
```

If it shows `ok`, the relay is reachable. Then open:

```text
https://your-app-domain/remote
```

You should see the remote web access page. Note that **the page lives at `/remote`**; do not judge whether the service works from the root address alone.

**Step 5: connect the Wisp on your computer.**

Open Wisp, go to **Settings → Remote Access → Remote web access**, and enter:

| Setting | Value |
| --- | --- |
| Relay server URL | `https://your-app-domain` |
| Relay access token | The `WISP_RELAY_TOKEN` you set when deploying |
| Enable remote web access | Checked |

The relay server URL is the domain only; **do not add `/remote`**. Press **Save**.

![Wisp remote web access settings: the relay URL, the access token, the enable switch and the connection code](../../assets/tutorials/en/remote-web/01-settings.png)

*Figure 2: The desktop settings, as an illustration. The domain and the connection code are demonstration values.*

Once connected, the right side reads **“Running”**, and a connection code and a remote access link appear below.

The two credentials do different jobs:

- **Relay access token**: lets the Wisp on your computer connect to the relay. You set it when deploying.
- **Connection code**: lets a browser connect to the Wisp on this computer. The desktop app generates it.

**Step 6: use it from a phone or a browser.**

Press **Copy link** in Wisp and open the link on a phone or in another browser. The link has this form:

```text
https://your-app-domain/remote#your-connection-code
```

You can also open the `/remote` page and type the connection code. When the desktop app shows **“Browsers connected: 1”**, one page has joined.

Now open a conversation in the page and send a message to confirm that the reply shows up. Writing files, editing files and running commands that were started remotely still need approval, which you can give in the page.

The connection code and the full link grant access, so keep them like a password. Before sharing a screenshot, cover the token, the connection code and everything after `#` in the link. If the link leaks, press **Reset code** in the desktop app and copy the new link: the old link stops working and connected pages are disconnected.

**Troubleshooting.**

| Symptom | What to check |
| --- | --- |
| Pulling the image fails with `latest: not found` | Use `ghcr.io/xuzhougeng/wisp-relay:main` from this tutorial, or confirm that the version tag you want has been published |
| `Back-off restarting failed container` | Read the app logs. The message means the container keeps exiting, and the logs give the actual reason; if the console offers the logs of the previous run, read those first |
| The logs say the token is missing or empty | Check that `WISP_RELAY_TOKEN` is filled in and saved |
| The logs say `Permission denied` | Check the permissions of the `/data` mount. A new app with no sync data that only serves remote web access can drop the mount; with existing sync data, keep the storage and fix its permissions |
| `/healthz` works but Wisp cannot connect | Check the relay URL, the token, and the network path from the computer to that domain; with a proxy of your own, check that it forwards WebSocket |
| The page says the computer is offline | Make sure the computer is awake, Wisp is running and remote access reads “Running” |

Keep reading: [Use Wisp from a Phone or Browser](wisp-science-remote-web.md) · [Remote web access deployment reference](../../remote-access.md) · [Project sync](../../project-sync.md)
