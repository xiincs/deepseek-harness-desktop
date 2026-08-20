// Installs the @deepseek-ai/dsh production dependency tree into
// src-tauri/resources/runtime so packaged builds can serve the harness
// without a network connection or a system npm. Part of the "bundled runtime"
// milestone.
//
// Usage: node scripts/prepare-runtime.mjs
// Env:   DSH_DESKTOP_DSH_VERSION  npm version spec (default 0.1.0-rc.8)
//        DSH_RUNTIME_SOURCE       directory containing node_modules/@deepseek-ai/dsh
//                                 (e.g. an existing npx cache root) — copies it
//                                 locally instead of hitting the npm registry.

import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const runtimeDir = join(root, "src-tauri", "resources", "runtime");
const version = process.env.DSH_DESKTOP_DSH_VERSION ?? "0.1.0-rc.8";
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
  execFileSync(
    `npm install --prefix "${runtimeDir}" "@deepseek-ai/dsh@${version}" --omit=dev --no-audit --no-fund --no-progress --prefer-offline --fetch-retries=5 --fetch-retry-mintimeout=2000`,
    { stdio: "inherit", shell: true },
  );
}

const bin = join(runtimeDir, "node_modules", "@deepseek-ai", "dsh", "lib", "bin.js");
if (!existsSync(bin)) {
  console.error("Runtime install failed: dsh bin.js not found");
  process.exit(1);
}
console.log("Runtime ready. Run `npm run fetch:node` too, then `npm run build`.");
