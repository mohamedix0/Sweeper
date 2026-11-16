import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { InfoIcon } from './Icons';

function addTipListeners(args: { onDocPointerDown: (e: MouseEvent | PointerEvent) => void; onKey: (e: KeyboardEvent) => void }) {
  document.addEventListener('pointerdown', args.onDocPointerDown);
  document.addEventListener('keydown', args.onKey, true);
  window.addEventListener('keydown', args.onKey);
  return () => {
    document.removeEventListener('pointerdown', args.onDocPointerDown);
    document.removeEventListener('keydown', args.onKey, true);
    window.removeEventListener('keydown', args.onKey);
  };
}

function useInfoTipDismiss(args: { open: boolean; rootRef: React.RefObject<HTMLSpanElement>; close: () => void }) {
  useEffect(() => {
    if (!args.open) return;
    function onDocPointerDown(e: MouseEvent | PointerEvent) {
      const root = args.rootRef.current;
      if (!root) return;
      let inside = false;
      if (e.target instanceof Node) inside = root.contains(e.target);
      if (!inside) args.close();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      args.close();
    }
    return addTipListeners({ onDocPointerDown, onKey });
  }, [args]);
}

function infoTipOpen(args: { pinned: boolean; hovered: boolean; focused: boolean }): boolean {
  if (args.pinned) return true;
  if (args.hovered) return true;
  return args.focused;
}

interface InfoTipTriggerProps { label: string; open: boolean; tooltipId: string; onClick: () => void; onFocus: () => void; onBlur: (e: React.FocusEvent<HTMLButtonElement>) => void }

function describedById(args: { open: boolean; tooltipId: string }): string | undefined {
  if (args.open) return args.tooltipId;
  return undefined;
}

function InfoTipTrigger(props: InfoTipTriggerProps) {
  const describedBy = describedById({ open: props.open, tooltipId: props.tooltipId });
  return <button type="button" className="info-tip-btn" aria-label={props.label} aria-expanded={props.open} aria-describedby={describedBy} onClick={props.onClick} onFocus={props.onFocus} onBlur={props.onBlur}><InfoIcon /></button>;
}

interface InfoTipState { pinned: boolean; hovered: boolean; focused: boolean }

function tipAlignClass(args: { align: 'left' | 'right' }): string {
  if (args.align === 'right') return 'info-tip right';
  return 'info-tip';
}

function blurKeepsFocus(args: { e: React.FocusEvent<HTMLButtonElement>; root: HTMLSpanElement | null }): boolean {
  if (!(args.e.relatedTarget instanceof Node)) return false;
  if (!args.root) return false;
  return args.root.contains(args.e.relatedTarget);
}

function useInfoTipState() {
  const [state, setState] = useState<InfoTipState>({ pinned: false, hovered: false, focused: false });
  const close = () => setState({ pinned: false, hovered: false, focused: false });
  const setHovered = (hovered: boolean) => setState((s) => ({ ...s, hovered }));
  const setFocused = (focused: boolean) => setState((s) => ({ ...s, focused }));
  const togglePinned = () => setState((s) => ({ pinned: !s.pinned, hovered: false, focused: false }));
  return { state, close, setHovered, setFocused, togglePinned };
}

function InfoTipPopover(props: { open: boolean; id: string; children: ReactNode }) {
  if (!props.open) return null;
  return <span id={props.id} className="info-tip-popover" role="tooltip">{props.children}</span>;
}

function InfoTipRoot(props: { align: 'left' | 'right'; children: ReactNode; onEnter: () => void; onLeave: () => void; onFocus: () => void; onBlur: (e: React.FocusEvent<HTMLButtonElement>) => void; onKeyDown: (e: React.KeyboardEvent<HTMLSpanElement>) => void; rootRef: React.RefObject<HTMLSpanElement> }) {
  return (
    <span ref={props.rootRef} className={tipAlignClass({ align: props.align })} onMouseEnter={props.onEnter} onMouseLeave={props.onLeave} onFocus={props.onFocus} onBlur={props.onBlur} onKeyDown={props.onKeyDown}>
      {props.children}
    </span>
  );
}

export function InfoTip(props: { label: string; children: ReactNode; align?: 'left' | 'right' }) {
  const { state, close, setHovered, setFocused, togglePinned } = useInfoTipState();
  const open = infoTipOpen({ pinned: state.pinned, hovered: state.hovered, focused: state.focused });
  const tooltipId = useId();
  const rootRef = useRef<HTMLSpanElement | null>(null);
  useInfoTipDismiss({ open, rootRef, close });
  const onBlur = (e: React.FocusEvent<HTMLButtonElement>) => {
    if (blurKeepsFocus({ e, root: rootRef.current })) return;
    setFocused(false);
  };
  const onKeyDown = (e: React.KeyboardEvent<HTMLSpanElement>) => {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    close();
  };
  let align: 'left' | 'right' = 'left';
  if (props.align === 'right') align = 'right';
  return (
    <InfoTipRoot align={align} rootRef={rootRef} onEnter={() => setHovered(true)} onLeave={() => setHovered(false)} onFocus={() => setFocused(true)} onBlur={onBlur} onKeyDown={onKeyDown}>
      <InfoTipTrigger label={props.label} open={open} tooltipId={tooltipId} onClick={togglePinned} onFocus={() => setFocused(true)} onBlur={onBlur} />
      <InfoTipPopover open={open} id={tooltipId}>{props.children}</InfoTipPopover>
    </InfoTipRoot>
  );
}
