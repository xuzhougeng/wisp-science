# App updates

Wisp supports optional, signed in-app updates on macOS. Windows and Linux keep
the update check and **Open Releases** path until their installers are enabled
in a later change.

Manual installers are also available from the [guided download page](https://wispscience.com/#download)
through the Cloudflare mirror. See [website deployment and installer synchronization](cloudflare-downloads.md).
The website download manifest is separate from the signed Tauri updater manifest;
in-app updates continue using the existing GitHub endpoint.

## User flow

1. Wisp checks the signed `latest.json` manifest. The Tauri updater chooses
   `darwin-aarch64` or `darwin-x86_64` from the running binary's target.
2. The update dialog shows the release notes. Nothing downloads until the user
   selects **Download update**.
3. Wisp reports download progress and verifies the package signature. An
   invalid signature never reaches the install step.
4. A second dialog asks the user to select **Install and restart**.
5. Installation is refused while an agent turn, approval, review, or persisted
   Run Manager job is active.

The existing **Don't remind me**, **Later**, and **Open Releases** choices
remain available. Updates are never forced.

An Apple Silicon Mac running the Intel build under Rosetta receives the Intel
update. Installing the native Apple Silicon build once is required to switch
architectures.

## Failure and recovery behavior

- Network loss, an interrupted download, a missing architecture, and signature
  verification failure happen before installation and leave the current app
  untouched. The verified package is kept only in memory, so quitting or losing
  power during download simply requires downloading again.
- If installation reports an error, Wisp keeps the verified package available
  for a later retry and offers the GitHub Releases fallback.
- The Tauri macOS installer extracts the new app into a temporary directory
  before replacing the installed bundle. A power loss during the final bundle
  replacement cannot be recovered by an app that is no longer launchable. The
  recovery path is to download the matching `.dmg` from GitHub Releases and
  replace `wisp-science.app` in `/Applications`. Project data and settings live
  outside the application bundle and are not removed by this repair.

There is no background install or automatic downgrade. A rollback uses the same
manual `.dmg` path with the desired older release.

## Cutting a GitHub release

Agents follow **Cutting a release** in `AGENTS.md`: push the version-bump
commit, then `gh release create` so the GitHub Release (title + notes) exists
before CI starts. That command creates the `v*` tag; the tag push starts the
platform builds. Do not push the tag separately.

The workflows are two steps so platform builds cannot overwrite the notes.

1. **Create Release** (`release-create.yml`) is the fallback if someone only
   pushes a tag. It reads `.github/release-notes/<tag>.md` and publishes the
   GitHub Release title and body. An optional HTML comment sets the title:

   ```markdown
   <!-- release-title: v1.6.0: Theme -->
   ```

   Without that comment the title is the tag. If the release already exists
   (the usual `gh release create` path), this job leaves title and body
   unchanged unless `overwrite_notes` is checked.
2. **Platform workflows** (Windows, macOS, Linux) wait until that release
   exists, then attach installers. They do not set the release name or body.
   macOS still merges `latest.json` for both architectures. **Android Release**
   attaches `wisp-science_<version>_android.apk` the same way once
   `ANDROID_RELEASE_ENABLED` is set (see `apps/android/README.md`); the APK has
   no in-app updater, and users install new versions over the old one.

Manual reruns of a platform workflow require the release from step 1 to
already exist.

## Release configuration

Updater artifacts are enabled only for the macOS release workflow through
`src-tauri/tauri.updater.conf.json`; ordinary development and diagnostic builds
do not require a signing key. The release workflow requires these repository
secrets:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

The matching public key and the
`releases/latest/download/latest.json` endpoint are committed in
`src-tauri/tauri.conf.json`. The private key must never be committed or copied
into the application.

Each tagged macOS release uploads both `.app.tar.gz` archives, their `.sig`
files, and one merged `latest.json`. A follow-up workflow job verifies that the
manifest contains non-empty signatures and URLs for both `darwin-aarch64` and
`darwin-x86_64`.

The first updater-capable release must still be installed manually by users of
an older build whose release feed has no updater manifest. Updates between later
updater-capable releases use the in-app flow.

## Desktop frontend migration

SwiftUI and WinUI desktop builds have been retired. Their source, packaging
scripts and release jobs are removed. Existing published assets remain in
GitHub Releases; immutable download URLs remain readable for past installations.
New manifests and the download picker offer the supported WebView installers
and Android APK. GPUI is a source-only preview; see [GPUI desktop](gpui-desktop.md).
iOS and Android clients continue to use the existing remote-access contracts.

## Windows code signing

Windows Release Authenticode-signs with an
SSL.com Personal ID (IV) code signing certificate enrolled in eSigner. The
credential id is `44eb788c-2a80-4f18-ae58-912d3298c053`. That id is not a secret;
the values below exist only as GitHub Actions secrets:

| Secret | Role |
| --- | --- |
| `ESIGNER_CREDENTIAL_ID` | eSigner credential id for the Personal ID (IV) certificate |
| `ESIGNER_USERNAME` | SSL.com account username |
| `ESIGNER_PASSWORD` | SSL.com account password |
| `ESIGNER_TOTP_SECRET` | OAuth TOTP secret from eSigner enrollment |
| `ESIGNER_PIN` | 4-digit PIN created during eSigner enrollment |

CodeSignTool does not send `ESIGNER_PIN`. CI still requires the secret so the
enrollment PIN is stored with the other signing credentials. SSL.com asks for
that PIN before it shows the authenticator QR or TOTP secret again.

Before the first signed build, enroll the certificate in eSigner, choose an
authenticator app, create the PIN, and scan the QR code. Save the TOTP secret
as `ESIGNER_TOTP_SECRET`. Signing fails until that enrollment is finished.
CodeSignTool 1.3.2 then signs from GitHub-hosted Windows runners and timestamps
with SSL.com's RFC 3161 server, `http://ts.ssl.com`. Each signature consumes one
eSigner signing. The signer waits for a fresh TOTP between calls because eSigner
rejects a reused one-time password. CodeSignTool's `main` discards picocli's
exit code, so `java -jar` exits 0 on auth errors, invalid OTPs, and bad
arguments. A sign counts as success only when the process exits 0, the
captured output contains `Code signed successfully:`, and the signed file is
present. An `Error:` line, an exception, a non-zero exit, or a missing file
fails the job and prints sanitized stdout, stderr, and
`logs/code_signing_tool.log`. An
invalid OTP is retried up to four times into later TOTP windows even when the
process exits 0. Windows Release uses `WISP_SIGN_SLOT=windows`.


Tauri's `signCommand` runs on the main `wisp-science.exe` after the bundler
patches it and before that copy is packed, then on the MSI and the NSIS setup
executable. The loose `target/release/wisp-science.exe` is unsigned again after
packaging because Tauri restores the unpatched binary. NSIS plugin DLLs and WiX
extension DLLs are left unsigned; the signed installer covers those build-tool
binaries.

A tag push or a dispatch with `publish=true` fails the job when signing fails,
and does not upload unsigned installers. `SKIP_WINDOWS_SIGNING` is no longer
read. The manual **Windows Build** workflow stays unsigned; it does not publish
a release. **Windows Test Signing** is a manual Authenticode check against the
same production credential and does not publish.

Signing dry-run (artifacts only, no release upload):

```bash
gh workflow run "Windows Release" --ref main -f tag=vX.Y.Z -f publish=false
```

Publish after the GitHub Release exists:

```bash
gh workflow run "Windows Release" --ref main -f tag=vX.Y.Z -f publish=true
```

SmartScreen reputation is built from download volume. An IV certificate does not
skip that warning, and Microsoft no longer gives EV certificates an instant
SmartScreen bypass either. Keep signing later releases with this same
certificate so reputation can carry forward. A newly signed installer can still
show SmartScreen until enough people have installed it.

After a run, the job summary lists the Authenticode status and signer subject.
On a downloaded installer:

```powershell
Get-AuthenticodeSignature .\wisp-science_1.21.0_x64_en-US.msi | Format-List Status, SignerCertificate, TimeStamperCertificate
```

`Status` is `Valid` and `TimeStamperCertificate` is present when the RFC 3161
timestamp verified.

## Manual smoke test

1. Publish the next tagged release from an updater-capable build with both
   macOS targets (use a staging fork when the feed must not affect users).
2. Confirm `latest.json`, two `.app.tar.gz` files, and two `.sig` files are
   attached to the GitHub release.
3. Install the previous Apple Silicon build, check for updates, and verify the
   release notes and download progress.
4. Cancel once before downloading, then download and cancel once before
   installing; neither action should change the installed version.
5. Start an agent turn or managed run and confirm installation is blocked.
6. Finish the work, select **Install and restart**, and confirm the new version
   launches with existing projects and settings.
7. Repeat on an Intel Mac or an Intel runner/VM.
8. Temporarily test a manifest with a bad signature and one with the current
   architecture removed; both must show an error and **Open Releases**, without
   presenting the install confirmation.
