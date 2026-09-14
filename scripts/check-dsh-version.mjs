// Checks the hardcoded @deepseek-ai/dsh default version against npm's
// "latest" dist-tag, and cross-checks the two source-of-truth locations
// (server.rs, prepare-runtime.mjs) against each other. Run this by hand
// before cutting a release; upstream is still in developer preview and
// publishes new RCs without notice. Deliberately checks the "latest" tag,
// not just the newest published version string — upstream sometimes ships
// a version under "next" that sits there for days without being promoted,
// and that isn't the same signal as "you should update to it".
//
// Usage: node scripts/check-dsh-version.mjs
// Exits non-zero if the pinned defaults disagree with each other, or if a
// newer version is available on npm than what's pinned.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));

function extract(path, pattern) {
  const text = readFileSync(join(root, path), "utf8");
  const match = text.match(pattern);
  if (!match) {
    console.error(`Could not find a version default in ${path}`);
    process.exit(1);
  }
  return match[1];
}

const serverRsVersion = extract(
  "src-tauri/src/server.rs",
  /const DSH_VERSION_DEFAULT: &str = "([^"]+)"/,
);
const prepareRuntimeVersion = extract(
  "scripts/prepare-runtime.mjs",
  /const version = process\.env\.DSH_DESKTOP_DSH_VERSION \?\? "([^"]+)"/,
);

console.log(`server.rs pins:            ${serverRsVersion}`);
console.log(`prepare-runtime.mjs pins:  ${prepareRuntimeVersion}`);

let failed = false;

if (serverRsVersion !== prepareRuntimeVersion) {
  console.error(
    `\nMismatch: server.rs and prepare-runtime.mjs pin different default versions.\n` +
      `Both must agree — a packaged build uses prepare-runtime.mjs's default, but the\n` +
      `managed per-user runtime install (no bundled resources) uses server.rs's default.`,
  );
  failed = true;
}

// Upstream `latest` versions this shell verifiably cannot run, and therefore
// does not adopt even though they are the recommended default. Every entry
// needs evidence, not a hunch — this map is the one deliberate exception to
// the "pin `latest`" rule above, so an entry here should read like a bug
// report that happens to end in "so we stay behind".
//
// Fail-closed by construction: a *newer* version upstream can't inherit an
// entry, so promoting past one of these fails this check again and forces a
// fresh look rather than silently extending the exception.
//
// Currently empty. The one entry it held — `0.1.5-rc.1` — is gone because the
// blocker was fixed rather than tolerated. 0.1.5 gates the whole UI behind
// browser auth that requires same-site: the ready line carries a required
// `?token=`, `GET /` without a cookie answers 401, the `dsh-auth-*` cookie is
// `SameSite=Strict` (so it only sticks when the harness is itself the top-level
// document), and `/api/*` is 401 without the cookie / 403 for
// `Sec-Fetch-Site: cross-site`. That was fatal while the harness was a
// cross-site `<iframe>` under `tauri.localhost`. It now renders as a top-level
// document in its own webview (see `HARNESS_WEBVIEW_LABEL` in
// src-tauri/src/lib.rs), so the cookie works and 0.1.5 is adoptable — hence the
// pin above.
//
// Re-verifying any of this? Note npm **republishes** these 0.x prereleases:
// same version string, different contents. A stale local copy whose build had
// the auth unwired (bare ready URL, `/` served unauthenticated) once produced a
// confidently wrong "0.1.5 has no auth" conclusion. Measure against what npm
// currently publishes.
const NOT_ADOPTABLE = new Map([]);

console.log("\nChecking npm dist-tags...");
let distTags;
try {
  // `npm` resolves inconsistently as a direct execFileSync target across
  // Windows npm installs (plain PATH npm vs. nvm-managed shims); `shell: true`
  // sidesteps that the same way a user's own shell would. The command has no
  // interpolated input, so the shell-escaping caveat that comes with
  // `shell: true` doesn't apply here.
  const raw = execFileSync("npm view @deepseek-ai/dsh dist-tags --json", {
    encoding: "utf8",
    shell: true,
  });
  distTags = JSON.parse(raw);
} catch (err) {
  console.error(`Could not query npm registry: ${err.message}`);
  process.exit(1);
}

const latest = distTags.latest;
console.log(`npm dist-tags: ${JSON.stringify(distTags)}`);

const notAdoptable = NOT_ADOPTABLE.get(latest);

if (latest !== serverRsVersion && !notAdoptable) {
  console.error(
    `\nPinned default (${serverRsVersion}) is not npm's "latest" dist-tag (${latest}).\n` +
      `Upstream is in developer preview and iterates fast — review the changelog before bumping,\n` +
      `then update DSH_VERSION_DEFAULT in src-tauri/src/server.rs and the default in\n` +
      `scripts/prepare-runtime.mjs (and the docs in README.md) together. If the newer version only\n` +
      `shows up under a tag other than "latest" (e.g. "next"), that's upstream's own signal that\n` +
      `it isn't the recommended default yet — don't bump to chase it.`,
  );
  failed = true;
}

if (notAdoptable) {
  console.log(
    `\nPinned default (${serverRsVersion}) is behind npm's "latest" (${latest}) on purpose:\n` +
      `  ${notAdoptable}`,
  );
}

if (failed) {
  process.exit(1);
}
console.log(
  notAdoptable
    ? `\nOK: pinned defaults agree, and ${latest} is a documented exception.`
    : "\nOK: pinned defaults agree and match npm's latest.",
);
