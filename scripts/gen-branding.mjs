// Propagates branding.json (the single white-label source of truth) into:
//   - src/branding.ts        (frontend constants)
//   - src-tauri/tauri.conf.json  (productName / identifier / window title)
//   - index.html             (<title>)
// The Rust side reads branding.json separately via src-tauri/build.rs.
//
// Run via `npm run brand` after editing branding.json. It also runs
// automatically before `dev`/`build` (predev/prebuild) to keep src/branding.ts
// in sync. NOTE: Tauri reads tauri.conf.json at startup, before beforeBuildCommand,
// so after changing branding.json you must run this once (or `npm run brand`)
// and commit the result before the next `tauri build` picks up the new
// productName/identifier.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (p) => readFileSync(join(root, p), "utf8");
const write = (p, s) => writeFileSync(join(root, p), s);

// Which branding file to apply. Defaults to the generic `branding.json`;
// override for a company-specific build, e.g.
//   BRANDING_FILE=branding.acme.json npm run brand
const brandingFile = process.env.BRANDING_FILE || "branding.json";
const b = JSON.parse(read(brandingFile));

// ---- Validate the fields the build genuinely depends on. ----
for (const key of ["appName", "appId"]) {
  if (typeof b[key] !== "string" || b[key].trim() === "") {
    throw new Error(`branding.json: "${key}" must be a non-empty string`);
  }
}
if (!/^[a-zA-Z0-9-]+(\.[a-zA-Z0-9-]+)+$/.test(b.appId)) {
  throw new Error(`branding.json: "appId" must be reverse-DNS, e.g. com.example.app (got "${b.appId}")`);
}

const defaults = b.defaults ?? {};
const branding = {
  appName: b.appName,
  idpLabel: b.idpLabel ?? "single sign-on",
  companyName: b.companyName ?? "",
  supportUrl: b.supportUrl ?? "",
  defaults: {
    sessionName: defaults.sessionName ?? "",
    startUrl: defaults.startUrl ?? "",
    ssoRegion: defaults.ssoRegion ?? "",
    regionPresets:
      Array.isArray(defaults.regionPresets) && defaults.regionPresets.length > 0
        ? defaults.regionPresets
        : ["eu-central-1", "eu-west-1", "us-east-1"],
  },
};

// ---- src/branding.ts ----
write(
  "src/branding.ts",
  `// AUTO-GENERATED from branding.json by scripts/gen-branding.mjs — do not edit.\n` +
    `// To re-brand this app, edit branding.json and run \`npm run brand\`.\n\n` +
    `export const branding = ${JSON.stringify(branding, null, 2)} as const;\n`,
);

// ---- src-tauri/tauri.conf.json (patch the three branded fields only) ----
const confPath = "src-tauri/tauri.conf.json";
const conf = JSON.parse(read(confPath));
conf.productName = b.appName;
conf.identifier = b.appId;
conf.app = conf.app ?? {};
conf.app.windows = conf.app.windows ?? [{}];
conf.app.windows[0].title = b.appName;
write(confPath, JSON.stringify(conf, null, 2) + "\n");

// ---- index.html <title> ----
const htmlPath = "index.html";
const html = read(htmlPath).replace(
  /<title>.*?<\/title>/,
  `<title>${b.appName}</title>`,
);
write(htmlPath, html);

console.log(`branding: applied "${b.appName}" (${b.appId}) from ${brandingFile}`);
