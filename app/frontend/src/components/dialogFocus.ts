import type { KeyboardEvent } from 'react';

// Cycle focus within an alertdialog on Tab / Shift+Tab. Every Tab press is
// handled (not just the wrap-around) so focus can never escape, and the
// behaviour is identical in real browsers and in jsdom-based tests.
export function trapTab(event: KeyboardEvent<HTMLDivElement>, focusables: HTMLElement[]): void {
  if (focusables.length === 0) return;
  event.preventDefault();
  const active = document.activeElement as HTMLElement | null;
  const index = focusables.indexOf(active as HTMLElement);
  if (index === -1) {
    focusables[event.shiftKey ? focusables.length - 1 : 0].focus();
    return;
  }
  const step = event.shiftKey ? -1 : 1;
  const next = (index + step + focusables.length) % focusables.length;
  focusables[next].focus();
}
