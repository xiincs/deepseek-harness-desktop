// Installs the @deepseek-ai/dsh production dependency tree into
// src-tauri/resources/runtime so packaged builds can serve the harness
// without a network connection or a system npm. Part of the "bundled runtime"
// milestone.
//
// Usage: node scripts/prepare-runtime.mjs
// Env:   DSH_DESKTOP_DSH_VERSION  npm version spec (default 0.1.1-rc.2)
//        DSH_RUNTIME_SOURCE       directory containing node_modules/@deepseek-ai/dsh
//                                 (e.g. an existing npx cache root) — copies it
//                                 locally instead of hitting the npm registry.

import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const runtimeDir = join(root, "src-tauri", "resources", "runtime");
const version = process.env.DSH_DESKTOP_DSH_VERSION ?? "0.1.1-rc.2";
mkdirSync(runtimeDir, { recursive: true });

const source = process.env.DSH_RUNTIME_SOURCE;
const sourceBin = source
  ? join(source, "node_modules", "@deepseek-ai", "dsh", "lib", "bin.js")
  : null;
if (sourceBin && existsSync(sourceBin)) {
  console.log(`Copying runtime from DSH_RUNTIME_SOURCE: ${source}`);
  cpSync(join(source, "node_modules"), join(runtimeDir, "node_modules"), {
    recursive: true,
    force: true,
  });
} else {
  console.log(`Installing @deepseek-ai/dsh@${version} → ${runtimeDir}`);
  // `npm` resolves inconsistently as a direct execFileSync target across
  // Windows npm installs (plain PATH npm vs. nvm-managed shims, and the
  // .cmd shim GitHub Actions' windows-latest runner uses); `shell: true`
  // sidesteps that the same way a user's own shell would. runtimeDir and
  // version are ours (env var / hardcoded default), not attacker input, so
  // the shell-escaping caveat that comes with `shell: true` doesn't apply.
  // No --prefer-offline: this registry republishes 0.x prerelease packages
  // (including transitive deps, within their existing dist-tag ranges)
  // multiple times a day, so a long-lived local npm cache can go stale
  // within hours and resolve to a hard ETARGET for a version that exists
  // on the real registry right now.
  execFileSync(
    `npm install --prefix "${runtimeDir}" "@deepseek-ai/dsh@${version}" --omit=dev --no-audit --no-fund --no-progress --fetch-retries=5 --fetch-retry-mintimeout=2000`,
    {
      stdio: "inherit",
      shell: true,
      // macos-latest CI runners are 7GB total; V8's auto-sized heap ceiling
      // (~2GB there) is too tight to resolve this dependency tree with no
      // lockfile to shortcut against, and reliably crashes with "JavaScript
      // heap out of memory" instead of completing. 4096 leaves real headroom
      // under the runner's actual ceiling — the commonly-suggested 8192
      // would exceed this runner's total physical memory outright.
      env: { ...process.env, NODE_OPTIONS: `${process.env.NODE_OPTIONS ?? ""} --max-old-space-size=4096`.trim() },
    },
  );
}

const bin = join(runtimeDir, "node_modules", "@deepseek-ai", "dsh", "lib", "bin.js");
if (!existsSync(bin)) {
  console.error("Runtime install failed: dsh bin.js not found");
  process.exit(1);
}
console.log("Runtime ready. Run `npm run fetch:node` too, then `npm run build`.");
