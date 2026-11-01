# wisp-relay on Cloudflare

Remote web access without a server: the same rendezvous as the `wisp-relay`
binary, on Cloudflare Workers and Durable Objects (Workers Free plan).

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/xuzhougeng/wisp-science/tree/main/crates/wisp-sync/cloudflare)

Set `WISP_RELAY_TOKEN` when asked, then enter the Worker's URL and that token
in **Settings → Remote Access → Remote web access** on the desktop. Scan the
QR code it shows with your phone.

Serves remote access only; project sync and Wisp Lab need the binary relay.
Setup, limits and manual checks: [docs/remote-access.md](../../../docs/remote-access.md#deploy-to-cloudflare-no-server).

From a clone:

```bash
cd crates/wisp-sync/cloudflare
npm install
npx wrangler secret put WISP_RELAY_TOKEN
npx wrangler deploy
```
