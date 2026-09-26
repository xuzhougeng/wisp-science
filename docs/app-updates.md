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
   macOS still merges `latest.json` for both architectures.

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

## SwiftUI Preview alongside the macOS release

Starting with the next tag that includes this packaging change, **macOS Release**
also attaches separate SwiftUI Preview installers and SHA-256 files:

- `Wisp-Science-SwiftUI-Preview_<version>_aarch64.dmg` for Apple Silicon.
- `Wisp-Science-SwiftUI-Preview_<version>_x86_64.dmg` for Intel.
- A matching `.dmg.sha256` file for each installer.

The preview requires macOS 13 or later. Drag **Wisp Science Preview.app** to
Applications; its name allows it to coexist with **wisp-science.app**. The
preview uses the existing desktop project database, settings and keyring by
default, so installing a separate app does **not** isolate user data. Use the
documented QA build and isolated database for destructive testing.

Preview updates are manual downloads from GitHub Releases. These installers do
not enter the stable Tauri `latest.json` feed, and the existing WebView installers
and updater remain available. Preview publishing only attaches assets; it never
rewrites the release title or notes. Mention SwiftUI Preview and its known
limitations in the bilingual notes when cutting the next release.

The release job builds optimized Swift and Rust executables for the same target,
then validates the shell/helper versions, clean source stamp and all three
binary architectures. It reuses the existing Developer ID certificate and
`APPLE_ID`, `APPLE_APP_PASSWORD`, and `APPLE_TEAM_ID` secrets. The service, embedded
host and shell are signed inside out with hardened runtime; the app and DMG are
notarized and stapled before upload. Any signing, notarization or validation
failure stops preview upload. Notarization results are retained as Actions
artifacts. The final verification job requires both preview installers and
checksums alongside the stable updater manifest.

For a local optimized build (ad-hoc signed, not a distributable release):

```bash
bash scripts/build_native_macos.sh --release --target aarch64-apple-darwin
# Or --target x86_64-apple-darwin, with that Rust target installed.
```

The app is written to `target/native-macos-release/<target>/Wisp Science Preview.app`.
Omitting `--target` uses the host architecture and the parent output directory.
Normal debug and `--qa` outputs keep their existing paths. The native-preview CI
workflow uploads a separate, ad-hoc signed and **unnotarized** ZIP as an Actions
artifact; that ZIP is not the GitHub Release installer. Rebuilding an older tag
from the release workflow on `main` skips preview packaging when that tag lacks
the packaging script.

For the first published preview, verify each downloaded checksum, run
`codesign --verify --deep --strict` and `spctl --assess --type execute` on the
installed app, and launch it from Finder on Apple Silicon and Intel. Confirm the
embedded helper starts, projects and settings load, and the WebView app remains
launchable. Actual Developer ID notarization and downloaded-app Gatekeeper
behavior require this release CI/manual validation; local ad-hoc builds and
mocked packaging tests do not establish them.

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
