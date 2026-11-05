import { execFileSync } from "node:child_process";
import { createReadStream } from "node:fs";
import { mkdtemp, rm, stat, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { PACKAGES, RELEASES, assetName, assetNames, compareTags, contentType, downloadKey, validTag, validateManifest } from "../assets/download-catalog.mjs";

const repo = "xuzhougeng/wisp-science";

export function selectAssets(release, fallbackReleases = []) {
  if (!validTag(release.tagName) || release.isDraft || release.isPrerelease) throw new Error("Expected a stable published release");
  const candidates = [release, ...fallbackReleases.filter(item => validTag(item.tagName) &&
    !item.isDraft && !item.isPrerelease && compareTags(item.tagName, release.tagName) < 0)
    .sort((a, b) => compareTags(b.tagName, a.tagName))];
  return PACKAGES.flatMap(pkg => {
    const source = candidates.find(item => item.assets.some(asset => assetNames(item.tagName, pkg).includes(asset.name))) || release;
    // Prefer the formal name when an old tag has been rebuilt; otherwise retain
    // the last self-contained Preview package until the new installer is ready.
    const name = assetNames(source.tagName, pkg).find(name => source.assets.some(asset => asset.name === name)) ?? assetName(source.tagName, pkg);
    const matches = source.assets.filter(asset => asset.name === name);
    if (pkg.optional && matches.length === 0) return [];
    if (matches.length !== 1 || matches[0].size <= 0) throw new Error(`Missing or ambiguous installer: ${name}`);
    return [{ ...pkg, tag: source.tagName, name, size: matches[0].size }];
  });
}

// Publish the small manifest only after all content-addressed installers succeed.
// Dependencies are injectable so failure/retry behavior is tested without a network.
export async function syncRelease(release, { download, upload, currentTag, directory, fallbackReleases = [] }) {
  const assets = selectAssets(release, fallbackReleases);
  const manifest = { schema: 1, tag: release.tagName, packages: [] };
  for (const asset of assets) {
    const file = path.join(directory, asset.name);
    await download(asset.name, file, asset.tag);
    const { size } = await stat(file);
    if (size !== asset.size) throw new Error(`Size mismatch: ${asset.name}`);
    const hash = createHash("sha256");
    for await (const chunk of createReadStream(file)) hash.update(chunk);
    const sha256 = hash.digest("hex");
    const key = downloadKey(asset.tag, asset.name, sha256);
    await upload(key, file, contentType(asset.name), "public, max-age=31536000, immutable");
    manifest.packages.push({ id: asset.id, tag: asset.tag, name: asset.name, key, size, sha256 });
  }
  validateManifest(manifest);
  if (await currentTag() !== release.tagName) throw new Error("Latest release changed during upload; retry sync");
  const file = path.join(directory, "latest.json");
  await writeFile(file, JSON.stringify(manifest, null, 2) + "\n");
  await upload("latest.json", file, "application/json", "public, max-age=60");
  return manifest;
}

async function main() {
  const gh = args => execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
  const latest = () => JSON.parse(gh(["release", "view", "--repo", repo, "--json", "tagName,isDraft,isPrerelease,assets"]));
  const release = latest();
  const fallbackReleases = [];
  const missing = () => PACKAGES.some(pkg => ![release, ...fallbackReleases].some(item =>
    item.assets.some(asset => assetNames(item.tagName, pkg).includes(asset.name))));
  if (missing()) {
    const older = JSON.parse(gh(["release", "list", "--repo", repo, "--limit", "20", "--json", "tagName,isDraft,isPrerelease"]))
      .filter(item => validTag(item.tagName) && !item.isDraft && !item.isPrerelease && compareTags(item.tagName, release.tagName) < 0)
      .sort((a, b) => compareTags(b.tagName, a.tagName));
    for (const item of older) {
      fallbackReleases.push(JSON.parse(gh(["release", "view", item.tagName, "--repo", repo, "--json", "tagName,isDraft,isPrerelease,assets"])));
      if (!missing()) break;
    }
  }
  try { selectAssets(release, fallbackReleases); } catch (error) {
    if (process.env.SYNC_ALLOW_INCOMPLETE === "1") {
      console.log(`Waiting for all platforms: ${error.message}`);
      return;
    }
    throw error;
  }
  if (process.argv.includes("--dry-run")) {
    console.log(JSON.stringify({ tag: release.tagName, assets: selectAssets(release, fallbackReleases).map(a => a.name) }, null, 2));
    return;
  }
  const directory = await mkdtemp(path.join(tmpdir(), "wisp-downloads-"));
  const docs = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const wrangler = path.join(docs, "node_modules/wrangler/bin/wrangler.js");
  try {
    await syncRelease(release, {
      directory,
      fallbackReleases,
      download: (name, file, tag) => gh(["release", "download", tag, "--repo", repo, "--pattern", name, "--output", file]),
      currentTag: async () => latest().tagName,
      upload: async (key, file, contentType, cacheControl) => {
        console.log(`Uploading ${key}`);
        execFileSync(process.execPath, [wrangler, "r2", "object", "put", `wisp-science-downloads/${key}`,
          "--remote", "--file", file, "--content-type", contentType, "--cache-control", cacheControl, "--force"],
        { cwd: docs, stdio: "inherit" });
      },
    });
    console.log(`Published ${release.tagName}: ${RELEASES}/tag/${release.tagName}`);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
