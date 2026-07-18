// SPDX-License-Identifier: GPL-3.0-or-later

export function adjustedFontSize(current: number, delta: number): number {
  return Math.min(32, Math.max(10, current + delta));
}
