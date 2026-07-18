// SPDX-License-Identifier: GPL-3.0-or-later

import type { DocumentDiskStatus } from "./protocol";

export function adjustedFontSize(current: number, delta: number): number {
  return Math.min(32, Math.max(10, current + delta));
}

export interface VisibleDocumentState {
  text: string;
  kind: "normal" | "dirty" | "conflict";
  hidden: boolean;
}

export function visibleDocumentState(
  diskStatus: DocumentDiskStatus,
  readOnly: boolean,
  dirty: boolean,
): VisibleDocumentState {
  if (diskStatus === "deleted") {
    return { text: "Deleted on disk", kind: "conflict", hidden: false };
  }
  if (diskStatus === "modified") {
    return { text: "Changed on disk", kind: "conflict", hidden: false };
  }
  if (readOnly) {
    return { text: "Read only", kind: "normal", hidden: false };
  }
  if (dirty) {
    return { text: "Unsaved", kind: "dirty", hidden: false };
  }
  return { text: "Saved", kind: "normal", hidden: true };
}
