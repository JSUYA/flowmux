// SPDX-License-Identifier: GPL-3.0-or-later

import { readFile, stat } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const requiredFiles = [
  "THIRD_PARTY_NOTICES.md",
  "index.html",
  "main.js",
  "main.css",
  "editor.worker.js",
  "json.worker.js",
  "css.worker.js",
  "html.worker.js",
  "ts.worker.js",
];

for (const file of requiredFiles) {
  const details = await stat(resolve(root, "dist", file));
  if (!details.isFile() || details.size === 0) {
    throw new Error(`Editor asset is missing or empty: ${file}`);
  }
}

const html = await readFile(resolve(root, "dist", "index.html"), "utf8");
if (!html.includes("Content-Security-Policy") || !html.includes('src="./main.js"')) {
  throw new Error("Editor entry point is missing its CSP or main script");
}

console.log(`Verified ${requiredFiles.length} editor assets.`);
