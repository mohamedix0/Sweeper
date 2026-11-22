import type { ReactNode } from 'react';
import { formatBytes, normalizeDisplayPath } from '../utils/format';
import { CopyIcon, FolderOpenIcon } from './Icons';
import { copyText, openContainingFolder } from './cleanSharedCore';

function splitPathParts(args: { fullPath: string }) {
  const lastSep = Math.max(args.fullPath.lastIndexOf('\\'), args.fullPath.lastIndexOf('/'));
  if (lastSep <= 0) return { dirPart: '', leafPart: args.fullPath };
  return { dirPart: args.fullPath.slice(0, lastSep + 1), leafPart: args.fullPath.slice(lastSep + 1) };
}

function PathDirLine(props: { dirPart: string }) {
  if (!props.dirPart) return null;
  return <div className="path-well-dir path-well-line"><bdi>{props.dirPart}</bdi></div>;
}

export function PathWell(props: { path: string }) {
  const fullPath = normalizeDisplayPath({ p: props.path }) || 'C:\\';
  const parts = splitPathParts({ fullPath });
  let leaf = parts.leafPart;
  if (!leaf) leaf = fullPath;
  return (
    <div className="path-well path-well-compact" aria-label="Full path" title={fullPath}>
      <div className="path-well-lines mono">
        <PathDirLine dirPart={parts.dirPart} />
        <div className="path-well-file path-well-line"><bdi>{leaf}</bdi></div>
      </div>
    </div>
  );
}

export function PathActions(props: { path: string }) {
  const fullPath = normalizeDisplayPath({ p: props.path }) || 'C:\\';
  return (
    <div className="inspector-path-actions" aria-label="Path actions">
      <button type="button" className="btn small subtle-btn" onClick={() => void copyText({ text: fullPath })} aria-label="Copy path" title="Copy path"><CopyIcon /><span>Copy Path</span></button>
      <button type="button" className="btn small subtle-btn" onClick={() => openContainingFolder({ path: fullPath })} aria-label="Reveal in Explorer" title="Reveal in Explorer"><FolderOpenIcon /><span>Reveal in Explorer</span></button>
    </div>
  );
}

function metaSecondLabel(args: { modifiedAge?: string | null; extraLabel?: string }): string {
  if (args.modifiedAge != null) return 'Modified';
  if (args.extraLabel) return args.extraLabel;
  return 'Details';
}

function metaSecondValue(args: { modifiedAge?: string | null; extraValue?: ReactNode }): ReactNode {
  if (args.modifiedAge != null) return args.modifiedAge;
  if (args.extraValue != null) return args.extraValue;
  return '—';
}

export function InspectorMetaGrid(props: { size: string; modifiedAge?: string | null; safety?: string; eligibility: string; extraLabel?: string; extraValue?: ReactNode }) {
  const secondLabel = metaSecondLabel({ modifiedAge: props.modifiedAge, extraLabel: props.extraLabel });
  const secondValue = metaSecondValue({ modifiedAge: props.modifiedAge, extraValue: props.extraValue });
  return (
    <dl className="inspector-meta-grid inspector-meta-grid-v19" aria-label="Item metadata">
      <div><dt>Size</dt><dd className="num">{formatBytes({ bytes: props.size })}</dd></div>
      <div><dt>{secondLabel}</dt><dd>{secondValue}</dd></div>
      <div className="inspector-meta-eligibility inspector-meta-eligibility-full"><dt>Eligibility</dt><dd>{props.eligibility}</dd></div>
    </dl>
  );
}

function Subtitle(props: { subtitle?: string | null }) {
  if (!props.subtitle) return null;
  return <span className="inspector-subtitle muted">{props.subtitle}</span>;
}

export function InspectorTitleRow(props: { icon: ReactNode; title: string; subtitle?: string | null; badge: ReactNode }) {
  return (
    <div className="inspector-header-block">
      <div className="inspector-title-row">
        <span className="inspector-leading-icon-wrap" aria-hidden="true">{props.icon}</span>
        <div className="inspector-title-copy">
          <span className="inspector-headline-name" title={props.title}>{props.title}</span>
          <Subtitle subtitle={props.subtitle} />
        </div>
        {props.badge}
      </div>
    </div>
  );
}
