import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { InfoIcon } from './Icons';

function blurKeepsFocus(input: { e: React.FocusEvent<HTMLButtonElement>; root: HTMLSpanElement | null }): boolean {
  if (!(input.e.relatedTarget instanceof Node)) return false;
  if (!input.root) return false;
  return input.root.contains(input.e.relatedTarget);
}

function infoTipOpen(input: { pinned: boolean; hovered: boolean; focused: boolean }): boolean {
  if (input.pinned) return true;
  if (input.hovered) return true;
  return input.focused;
}

function tipAlignClass(input: { align: 'left' | 'right' }): string {
  if (input.align === 'right') return 'info-tip right';
  return 'info-tip';
}

function useDismissOnOutside(input: { open: boolean; rootRef: React.RefObject<HTMLSpanElement>; close: () => void }) {
  useEffect(() => {
    if (!input.open) return;
    function onDocPointerDown(e: MouseEvent | PointerEvent) {
      const root = input.rootRef.current;
      if (!root) return;
      if (e.target instanceof Node) {
        if (root.contains(e.target)) return;
      }
      input.close();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      input.close();
    }
    document.addEventListener('pointerdown', onDocPointerDown);
    document.addEventListener('keydown', onKey, true);
    window.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onDocPointerDown);
      document.removeEventListener('keydown', onKey, true);
      window.removeEventListener('keydown', onKey);
    };
  }, [input]);
}

export function InfoTip(props: { label: string; children: ReactNode; align?: 'left' | 'right' }) {
  const [state, setState] = useState({ pinned: false, hovered: false, focused: false });
  const open = infoTipOpen(state);
  const tooltipId = useId();
  const rootRef = useRef<HTMLSpanElement | null>(null);
  const close = () => setState({ pinned: false, hovered: false, focused: false });
  useDismissOnOutside({ open, rootRef, close });
  const onFocus = () => setState((s) => ({ ...s, focused: true }));
  const onBlur = (e: React.FocusEvent<HTMLButtonElement>) => {
    if (blurKeepsFocus({ e, root: rootRef.current })) return;
    setState((s) => ({ ...s, focused: false }));
  };
  const onKeyDown = (e: React.KeyboardEvent<HTMLSpanElement>) => {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    close();
  };
  const align: 'left' | 'right' = props.align === 'right' ? 'right' : 'left';
  const describedBy = open ? tooltipId : undefined;
  return (
    <span ref={rootRef} className={tipAlignClass({ align })} onMouseEnter={() => setState((s) => ({ ...s, hovered: true }))} onMouseLeave={() => setState((s) => ({ ...s, hovered: false }))} onFocus={onFocus} onBlur={onBlur} onKeyDown={onKeyDown}>
      <button type="button" className="info-tip-btn" aria-label={props.label} aria-expanded={open} aria-describedby={describedBy} onClick={() => setState((s) => ({ pinned: !s.pinned, hovered: false, focused: false }))} onFocus={onFocus} onBlur={onBlur}><InfoIcon /></button>
      {open ? <span id={tooltipId} className="info-tip-popover" role="tooltip">{props.children}</span> : null}
    </span>
  );
}