import ts from "typescript";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve("extension");
const manifestPath = resolve(root, "manifest.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const errors = [];

function requireValue(condition, message) {
  if (!condition) errors.push(message);
}

function requireFile(relativePath) {
  requireValue(existsSync(resolve(root, relativePath)), `Missing referenced file: ${relativePath}`);
}

requireValue(manifest.manifest_version === 3, "Extension must use Manifest V3.");
requireValue(
  manifest.background?.service_worker === "src/background.js",
  "Chromium service worker is missing.",
);
requireValue(
  manifest.background?.scripts?.includes("src/background.js"),
  "Firefox background event page is missing.",
);
requireValue(manifest.background?.type === "module", "Background script must load as an ES module.");
requireValue(
  manifest.browser_specific_settings?.gecko?.id,
  "Firefox extension ID is required for signing.",
);
requireValue(
  Number.parseInt(manifest.browser_specific_settings?.gecko?.strict_min_version, 10) >= 142,
  "Firefox 142+ is required for built-in data-collection consent.",
);

const declaredData =
  manifest.browser_specific_settings?.gecko?.data_collection_permissions?.required ?? [];
for (const type of ["authenticationInfo", "browsingActivity", "searchTerms", "websiteContent"]) {
  requireValue(declaredData.includes(type), `Firefox data declaration is missing ${type}.`);
}

requireFile(manifest.background.service_worker);
requireFile(manifest.options_page);
requireFile(manifest.action?.default_popup);
for (const script of manifest.content_scripts?.flatMap((entry) => entry.js ?? []) ?? []) {
  requireFile(script);
}
for (const icon of Object.values(manifest.icons ?? {})) requireFile(icon);

const scripts = [
  "src/background.js",
  "src/content.js",
  "src/options.js",
  "src/popup.js",
  "src/summarize.js",
];
for (const script of scripts) {
  requireFile(script);
  const fullPath = resolve(root, script);
  if (!existsSync(fullPath)) continue;
  const source = readFileSync(fullPath, "utf8");
  requireValue(
    !/\bchrome\./.test(source),
    `${script} uses a Chromium-only API instead of the shared browser alias.`,
  );
  const parsed = ts.createSourceFile(
    fullPath,
    source,
    ts.ScriptTarget.ESNext,
    false,
    ts.ScriptKind.JS,
  );
  const diagnostics = parsed.parseDiagnostics.map((diagnostic) =>
    ts.flattenDiagnosticMessageText(diagnostic.messageText, " "),
  );
  requireValue(
    diagnostics.length === 0,
    `${script} has invalid JavaScript: ${diagnostics.join("; ")}`,
  );
}

if (errors.length) {
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}

console.log("Tempo extension checks passed for current Chromium and Firefox manifests.");
