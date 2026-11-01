// Bundles the same page and icons the binary relay embeds with include_str!/
// include_bytes!, so the two editions cannot drift (rules in wrangler.jsonc).
import html from "../src/remote.html";
import script from "../src/remote.js";
import icon192 from "../../../src-tauri/icons/android/mipmap-xxxhdpi/ic_launcher.png";
import icon512 from "../../../src-tauri/icons/icon.png";
import touchIcon from "../../../src-tauri/icons/ios/AppIcon-60x60@3x.png";
import { handle, RemoteSid } from "./relay.mjs";

export { RemoteSid };

const assets = { html, script, icon192, icon512, touchIcon };

export default {
  fetch: (request, env) => handle(request, env, assets),
};
