// SPDX-License-Identifier: GPL-3.0-or-later

import assert from "node:assert/strict";
import test from "node:test";
import { adjustedFontSize } from "../.test-build/editor_state.js";

test("font zoom stays inside a readable supported range", () => {
  assert.equal(adjustedFontSize(13, 1), 14);
  assert.equal(adjustedFontSize(32, 1), 32);
  assert.equal(adjustedFontSize(10, -1), 10);
});
