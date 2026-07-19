// SPDX-License-Identifier: GPL-3.0-or-later

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("the unfinished edit and diff mode controls stay hidden", async () => {
  const html = await readFile(new URL("../index.html", import.meta.url), "utf8");

  assert.doesNotMatch(html, /id="mode-(?:switch|edit|diff)"/);
  assert.match(html, /id="editor"/);
  assert.match(html, /id="diff-editor"/);
  assert.match(html, /id="conflict-compare"/);
});
