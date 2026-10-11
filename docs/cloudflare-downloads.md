# Website and installer downloads on Cloudflare

The official website is `https://wispscience.com`, deployed by GitHub Pages.
The homepage contains the download picker at `/#download`: the primary navigation
and hero download buttons scroll to that section without leaving the homepage.
Users choose their operating system, processor and format in Chinese or English.
The old `download.html` URL redirects to the homepage section, preserving language.

The homepage also displays cumulative community milestones since the first v0.1
release: 19K downloads, 50K+ reads of related tutorials and articles, and 1,000+
GitHub Stars. These are maintainer-provided milestones, not live API counters.
Update the values in `docs/index.html` when new milestones are confirmed; the
Chinese and English labels live in `docs/assets/i18n.js`. The Stars label links
to the repository's stargazers. The three metrics remain visible on mobile and
without JavaScript.

Installer downloads and the release manifest use the Cloudflare Worker at
`https://wisp-science.sfl.bio` and its private `wisp-science-downloads` R2 bucket.
GitHub Releases remains the fallback and the home for older releases.

| System | Processor | Formats |
| --- | --- | --- |
| Windows | Intel / AMD x64 | EXE (standard setup), MSI |
| macOS | Apple Silicon / Intel | DMG |
| Linux | Intel / AMD x64 / ARM64 | DEB, AppImage |

The browser suggests an operating system only. Mac and Linux users explicitly
choose their processor. Mobile visitors choose the target desktop system. There
is no native Windows ARM64 package. The page shows the mirrored version,
filename, size and SHA-256 checksum, along with installation guidance.

## Local deployment

Run from the repository root. Use the local Cloudflare login; never copy OAuth
refresh tokens into source code or GitHub Actions secrets.

```bash
npm ci --prefix docs
cd docs
npx wrangler login
npx wrangler whoami
npx wrangler r2 bucket list
# Create once, only if the bucket is not already present:
npx wrangler r2 bucket create wisp-science-downloads
cd ..
python -m pip install -r docs/requirements-pages.txt
python docs/build_tutorials.py
python docs/build_skills.py
npm test --prefix docs
npm run build --prefix docs
cd docs
npx wrangler deploy --dry-run
# Requires gh access to the public repository and Cloudflare R2 write access:
npm run sync-downloads -- --dry-run
npm run sync-downloads
npm run deploy
```

`docs/wrangler.jsonc` binds `wisp-science.sfl.bio`. The logged-in Cloudflare account
must own the `sfl.bio` zone. If several accounts are available, set
`CLOUDFLARE_ACCOUNT_ID` to that account. Wrangler manages the domain binding and
certificate. The build copies only public HTML, assets, tutorials and the skills
catalog; Worker source, configuration, dependencies and credentials stay private.

## Synchronization behavior

The sync script resolves the latest stable GitHub release at execution time. It
accepts the exact eight installer names for that version; missing/duplicate files
fail validation before any upload. Source archives, signatures and `.app.tar.gz`
updater bundles are excluded.

Each installer is checked against the GitHub asset size, hashed with SHA-256,
and uploaded to `releases/<tag>/<sha256>/<original-filename>`. The script checks
that the latest release has not changed and publishes `latest.json` **last**.
An upload failure leaves the previous manifest available. Content-addressed
paths prevent a rebuild of the same tag from replacing an installer referenced
by a cached manifest. Repeated syncs are safe; failures may leave unreferenced
objects. Temporary local files are removed when synchronization ends.

The Worker streams installers and supports HEAD, ETags and single byte ranges.
Installer URLs are immutable; the manifest has a 60-second cache. CORS allows
GitHub Pages to read the manifest. A missing/invalid manifest leaves the GitHub
fallback available.

This mirror is for manual installation. The desktop's signed updater continues
using the existing GitHub endpoint. The website manifest is a separate schema
and must not be configured as a Tauri updater endpoint.

## GitHub Actions

- `Deploy website to Cloudflare`: deploys on website/skills changes to `main`, or
  manual dispatch. GitHub Pages continues deploying the same public build.
- `Sync installers to Cloudflare R2`: runs after a successful Windows, macOS or
  Linux release workflow, daily as a recovery check, or manually. A platform
  completion with missing installers waits for the other platforms; manual and
  scheduled runs fail visibly if packages are missing. Runs are serialized and
  read the latest stable release using trusted `main` code.

For unattended Cloudflare CI, set repository variable
`CLOUDFLARE_AUTOMATION_ENABLED=true` after configuring repository secrets `CLOUDFLARE_ACCOUNT_ID` and
`CLOUDFLARE_API_TOKEN`. Use a dedicated token with permissions to deploy Workers
and static assets, bind the domain in `sfl.bio`, and write the R2 bucket. Local
Wrangler OAuth login does **not** configure CI. The existing platform workflows
still own release creation and signing. These workflows never edit release
titles, notes, tags, or updater manifests.

## Validation and smoke checks

```bash
npm test --prefix docs
python -m unittest discover -s docs -p 'test_build_*.py'
npm run build --prefix docs
cd ui-tests
npm ci
npx playwright test --config playwright.website.config.ts
```

Tests use fake releases and R2 objects, temporary files, and a mocked browser
manifest. They require no Cloudflare account or real upload. Browser tests use
a local Python HTTP server on port 1433.

CI runs the same four steps in `.github/workflows/website.yml`, only when
`docs/`, `skills/`, a README or the website tests change. The browser tests
live in `ui-tests/website-tests/` and do not need Trunk or the app UI.

After deployment, open `https://wispscience.com/#download` and click the homepage
download action to verify it stays on the homepage. Select
every system/processor/format, switch languages and check a narrow viewport.
Confirm the displayed release, check HEAD, request `Range: bytes=0-15` (expect
206 and 16 bytes), and compare a full download's SHA-256 with the manifest.
Simulate manifest failure to check GitHub fallback. Deployment is complete only
after public endpoints work.

Old R2 versions are retained so existing links and cached manifests keep working.
Storage pruning is a separate maintenance task; do not delete keys referenced
by a live manifest. New platforms or asset names require updating the shared
package catalog and its tests.

References: [Workers static assets](https://developers.cloudflare.com/workers/static-assets/binding/),
[R2 Workers API](https://developers.cloudflare.com/r2/api/workers/workers-api-reference/),
[R2 uploads](https://developers.cloudflare.com/r2/objects/upload-objects/).
