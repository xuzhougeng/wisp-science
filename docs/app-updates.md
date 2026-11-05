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

<a id="swiftui-preview-alongside-the-macos-release"></a>

## SwiftUI alongside the macOS release

Starting with the next tag that includes this packaging change, **macOS Release**
also attaches separate SwiftUI installers and SHA-256 files without the Preview label:

- `Wisp-Science-SwiftUI_<version>_aarch64.dmg` for Apple Silicon.
- `Wisp-Science-SwiftUI_<version>_x86_64.dmg` for Intel.
- A matching `.dmg.sha256` file for each installer.

The SwiftUI app requires macOS 13 or later. Drag **Wisp Science SwiftUI.app** to
Applications; its name allows it to coexist with **wisp-science.app**. The
SwiftUI app uses the existing desktop project database, settings and keyring by
default, so installing a separate app does **not** isolate user data. Use the
documented QA build and isolated database for destructive testing.

When upgrading from **Wisp Science Preview.app**, quit the old app, install
**Wisp Science SwiftUI.app**, and remove the old app bundle to avoid duplicate
launchers. The existing `science.wisp-science.native-preview` bundle identifier
is retained to preserve preferences; no data migration is required.

SwiftUI updates are manual downloads from GitHub Releases. These installers do
not enter the stable Tauri `latest.json` feed, and the existing WebView installers
and updater remain available. SwiftUI publishing only attaches assets; it never
rewrites the release title or notes.

The release job builds optimized Swift and Rust executables for the same target,
then validates the shell/helper versions, clean source stamp and all three
binary architectures. It reuses the existing Developer ID certificate and
`APPLE_ID`, `APPLE_APP_PASSWORD`, and `APPLE_TEAM_ID` secrets. The service, embedded
host and shell are signed inside out with hardened runtime; the app and DMG are
notarized and stapled before upload. Any signing, notarization or validation
failure stops SwiftUI upload. Notarization results are retained as Actions
artifacts. The final verification job requires both SwiftUI installers and
checksums alongside the stable updater manifest.

For a local optimized build (ad-hoc signed, not a distributable release):

```bash
bash scripts/build_native_macos.sh --release --target aarch64-apple-darwin
# Or --target x86_64-apple-darwin, with that Rust target installed.
```

The app is written to `target/native-macos-release/<target>/Wisp Science SwiftUI.app`.
Omitting `--target` uses the host architecture and the parent output directory.
The debug app uses the same new name under `target/native-macos`; `--qa` keeps
its existing app name and isolated identifier. The native-preview CI
workflow uploads a separate, ad-hoc signed and **unnotarized** ZIP as an Actions
artifact; that ZIP is not the GitHub Release installer. Rebuilding an older tag
from the release workflow on `main` uses the tagged source's app and installer
names (including Preview for older tags), and skips SwiftUI packaging when that
tag lacks the packaging script.

For a published SwiftUI build, verify each downloaded checksum, run
`codesign --verify --deep --strict` and `spctl --assess --type execute` on the
installed app, and launch it from Finder on Apple Silicon and Intel. Confirm the
embedded helper starts, projects and settings load, and the WebView app remains
launchable. Finder and the main window should show **Wisp Science SwiftUI**;
the home footer should say **SwiftUI 原生版**. When upgrading from Preview,
confirm the saved database selection and appearance preferences are retained.
Actual Developer ID notarization and downloaded-app Gatekeeper
behavior require this release CI/manual validation; local ad-hoc builds and
mocked packaging tests do not establish them.

## Windows code signing

Windows Release and Windows WinUI Release Authenticode-sign with an
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
process exits 0. Windows Release (`WISP_SIGN_SLOT=windows`) and Windows WinUI
Release (`WISP_SIGN_SLOT=winui`) sign in parallel with the same
credential, so the WinUI job waits one extra TOTP window before its first
sign and on each OTP retry.

Tauri's `signCommand` runs on the main `wisp-science.exe` after the bundler
patches it and before that copy is packed, then on the MSI and the NSIS setup
executable. The loose `target/release/wisp-science.exe` is unsigned again after
packaging because Tauri restores the unpatched binary. NSIS plugin DLLs and WiX
extension DLLs are left unsigned; the signed installer covers those build-tool
binaries. The WinUI installer signs `Wisp.Science.Preview.exe`, `wisp-service.exe`,
and `settings-host/wisp-tauri.exe` before NSIS packages them, then signs each
setup executable. Microsoft runtime DLLs are not re-signed. The NSIS-generated
WinUI uninstaller is unsigned.

A tag push or a dispatch with `publish=true` fails the job when signing fails,
and does not upload unsigned installers. `SKIP_WINDOWS_SIGNING` is no longer
read. The manual **Windows Build** workflow stays unsigned; it does not publish
a release. **Windows Test Signing** is a manual Authenticode check against the
same production credential and does not publish.

Signing dry-run (artifacts only, no release upload):

```bash
gh workflow run "Windows Release" --ref main -f tag=vX.Y.Z -f publish=false
gh workflow run release-winui.yml --ref main -f tag=vX.Y.Z -f publish=false
```

Publish after the GitHub Release exists:

```bash
gh workflow run "Windows Release" --ref main -f tag=vX.Y.Z -f publish=true
gh workflow run release-winui.yml --ref main -f tag=vX.Y.Z -f publish=true
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

<a id="winui-3-preview-alongside-the-windows-release"></a>

## WinUI alongside the Windows WebView release

**Windows WinUI Release** (`release-winui.yml`) builds one standalone x64
installer on `v*` tag pushes, alongside the existing Windows WebView EXE/MSI.
Its naming follows macOS SwiftUI:

- `Wisp-Science-WinUI_<version>_x64-setup.exe`, for example
  `Wisp-Science-WinUI_1.24.0_x64-setup.exe`.
- A matching `.exe.sha256`, calculated **after** signing.
- The `.size.json` report stays in Actions artifacts, rather than adding another
  download choice to the GitHub release.

Only the self-contained variant is built and published by PR/release CI. The
framework-dependent variant is no longer a release download. Package sizes vary
with the source and dependencies; the v1.24.0 self-contained payload produced an
approximately 58 MiB installer, not a fixed 50 MB size guarantee.

The WinUI installer works independently: it includes the native executable and
managed files, `wisp-service.exe`, its own `settings-host/wisp-tauri.exe`, host
resources, rich-preview assets, .NET 8, Windows App SDK and app-local MSVC DLLs.
Users do **not** need to download or install the WebView edition. Packaging fails
if required app files, either helper, bundled runtimes or host resources are
missing. Windows 10 1809+ x64 is required. Setup detects Microsoft WebView2
Evergreen Runtime in the machine and current-user registry locations and reuses
an installed runtime. If absent, it automatically runs the bundled Microsoft
bootstrapper to download and install the runtime. **Internet is required when
WebView2 is missing**, but users only download the WinUI installer themselves.
No separate Wisp WebView application, .NET SDK or .NET runtime download is needed.

CI downloads the small official Evergreen bootstrapper and verifies its Microsoft
Authenticode signature; packaging rechecks the exact staged bytes before embedding
them. The full WebView2 runtime is not bundled, keeping the installer small.
Setup rechecks runtime availability after installation and stops with a clear
error before touching an existing WinUI installation if setup fails. Uninstall
leaves the shared Microsoft runtime in place. This follows Microsoft's
[WebView2 distribution guidance](https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution).
Historical Preview installers do not gain this behavior unless rebuilt with the
current packager.

Fresh installs use `%LOCALAPPDATA%/Programs/Wisp Science WinUI` and the Start menu
entry **Wisp Science WinUI**. An existing WinUI Preview installation is upgraded
in its existing directory using its own uninstall manifest. Its stable uninstall
registry key and internal executable name are retained for compatibility; the
display name and shortcut no longer say Preview. The WebView installation is not
used or removed. Both editions share the desktop database/settings/keyring.
Uninstall removes packaged files and the shortcut/registration, preserving user
data. Close WinUI and its helpers before installing an update.

Updates remain manual. The workflow uploads assets without creating releases,
rewriting title/notes or changing `latest.json`. All three product executables
are signed before packaging, the installer is signed afterward, and final
signatures/checksums are verified before upload. Signing failures block release.
See [Windows code signing](#windows-code-signing).

The build explicitly uses Release, x64 and `-RuntimeMode SelfContained` for .NET
and Windows App SDK. Rust helpers use `opt-level=s`, LTO, one codegen unit and
stripped symbols. .NET disables ReadyToRun/debug symbols. Trimming, single-file
publishing and NativeAOT remain disabled. Solid LZMA compression excludes symbols
and Python caches. The regression ceilings remain **100 MiB installer / 350 MiB
installed payload**; review the size report before changing them.

To rebuild an existing tag, dispatch `release-winui.yml` **from main** as shown
above. `publish=false` keeps signed output in Actions without publishing it.
The requested tag supplies all application binaries and its version; the workflow
revision supplies the packaging/naming policy. `--source-root .` checks the tag
against that source's `Cargo.toml`, so an older tag can receive the formal filename
without being rebuilt from current-main app code. Tags predating native packaging
are skipped. Historical Preview release assets are not deleted automatically.

The build locates MSVC redistributables with `vswhere`; custom toolchains can pass
`-VCRedistDirectory`. Local packaging requires Python 3.11+ and NSIS 3:

```powershell
pwsh -File scripts/build_native_windows.ps1 -Configuration Release -RuntimeMode SelfContained
powershell -NoProfile -File scripts/prepare_native_windows_webview2.ps1 -Path target/MicrosoftEdgeWebview2Setup.exe
python scripts/package_native_windows.py target/native-windows-release --tag vX.Y.Z --makensis 'C:/Program Files (x86)/NSIS/makensis.exe' --webview2-bootstrapper target/MicrosoftEdgeWebview2Setup.exe
```

Framework-dependent/Both modes remain available for local development, with
distinct filenames. CI does not build or publish them.

Clean-machine acceptance: on a Windows x64 account **without WebView2, either
Wisp edition or .NET/Windows App SDK development tools**, verify SHA-256 and
Authenticode, connect to the internet and install this EXE alone. Confirm WebView2
is installed automatically, launch from Start, create/open a project, load
settings, and open a rich preview. Then exit and uninstall, preserving user data
and WebView2. Separately verify an account with existing WebView2 can install
offline; a missing-runtime offline failure must leave the previous WinUI intact.
Upgrade an old Preview and verify the formal shortcut/display name. Mock packaging
tests do not prove this clean-machine acceptance or signing.

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
