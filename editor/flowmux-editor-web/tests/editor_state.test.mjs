// SPDX-License-Identifier: GPL-3.0-or-later

import assert from "node:assert/strict";
import test from "node:test";
import {
  adjustedFontSize,
  conflictUiState,
  visibleDocumentState,
} from "../.test-build/editor_state.js";

test("font zoom stays inside a readable supported range", () => {
  assert.equal(adjustedFontSize(13, 1), 14);
  assert.equal(adjustedFontSize(32, 1), 32);
  assert.equal(adjustedFontSize(10, -1), 10);
});

test("conflict actions distinguish changed deleted and compare states", () => {
  assert.deepEqual(conflictUiState("modified", true, false), {
    hidden: false,
    message: "This file changed on disk while you were editing it.",
    compareDisabled: false,
    reloadDisabled: false,
    keepLabel: "Keep Mine",
    closeCompareHidden: true,
  });
  assert.deepEqual(conflictUiState("deleted", true, false), {
    hidden: false,
    message: "This file was deleted on disk.",
    compareDisabled: true,
    reloadDisabled: true,
    keepLabel: "Recreate on Save",
    closeCompareHidden: true,
  });
  assert.equal(conflictUiState("modified", true, true).compareDisabled, true);
  assert.equal(conflictUiState("unchanged", false, false).hidden, true);
});

test("document state distinguishes external changes and deletion", () => {
  assert.deepEqual(visibleDocumentState("modified", false, true), {
    text: "Changed on disk",
    kind: "conflict",
    hidden: false,
  });
  assert.deepEqual(visibleDocumentState("deleted", false, false), {
    text: "Deleted on disk",
    kind: "conflict",
    hidden: false,
  });
  assert.deepEqual(visibleDocumentState("unchanged", false, false), {
    text: "Saved",
    kind: "normal",
    hidden: true,
  });
});
