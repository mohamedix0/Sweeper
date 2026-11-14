import type { SVGProps } from 'react';

const base = {
  width: 16,
  height: 16,
  viewBox: '0 0 16 16',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.75,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
  'aria-hidden': true,
  focusable: 'false' as const,
};

const navBase = {
  ...base,
  width: 16,
  height: 16,
  strokeWidth: 1.5,
};

export function ChevronRightIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M6 4l4 4-4 4" />
    </svg>
  );
}

export function ChevronDownIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M4 6l4 4 4-4" />
    </svg>
  );
}

export function DotIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <circle cx="8" cy="8" r="1.75" fill="currentColor" stroke="none" />
    </svg>
  );
}

export function ScanIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...navBase} {...props}>
      <circle cx="7" cy="7" r="4.25" />
      <path d="M10.1 10.1L14 14" />
      <path d="M5 7h4M7 5v4" />
    </svg>
  );
}

export function AnalyzerIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...navBase} {...props}>
      <path d="M3 3h4v4H3zM9 3h4v4H9zM3 9h4v4H3z" />
      <path d="M9.5 11h3M11 9.5v3" />
    </svg>
  );
}

export function SettingsIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...navBase} {...props}>
      <path d="M6.5 2.5h3l.5 1.5 1.6.4 1.3-.9 2.1 2.1-.9 1.3.4 1.6 1.5.5v3l-1.5.5-.4 1.6.9 1.3-2.1 2.1-1.3-.9-1.6.4-.5 1.5h-3l-.5-1.5-1.6-.4-1.3.9-2.1-2.1.9-1.3-.4-1.6L1 9.5v-3L2.5 6l.4-1.6-.9-1.3 2.1-2.1 1.3.9 1.6-.4z" />
      <circle cx="8" cy="8" r="2" />
    </svg>
  );
}

export function LogsIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...navBase} {...props}>
      <rect x="3" y="2.5" width="10" height="11" rx="1.5" />
      <path d="M5.5 5.5h5M5.5 8h5M5.5 10.5h3" />
    </svg>
  );
}

export function InfoIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={14} height={14} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <circle cx="8" cy="8" r="6" />
      <path d="M8 7v4" />
      <circle cx="8" cy="4.75" r="0.75" fill="currentColor" stroke="none" />
    </svg>
  );
}

export function LockIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={14} height={14} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <rect x="3.5" y="7" width="9" height="6.5" rx="1.5" />
      <path d="M5.5 7V5.5a2.5 2.5 0 0 1 5 0V7" />
    </svg>
  );
}

export function ShieldIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={14} height={14} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <path d="M8 2.25 3.5 4v3.5c0 2.8 1.9 4.9 4.5 6.25 2.6-1.35 4.5-3.45 4.5-6.25V4L8 2.25Z" />
    </svg>
  );
}

export function FolderIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={16} height={16} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <path d="M2.5 4.5a1 1 0 0 1 1-1H6l1.2 1.5H12.5a1 1 0 0 1 1 1v5.5a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1v-7Z" />
    </svg>
  );
}

export function FileIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={16} height={16} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <path d="M5 2.5h3.8L12 5.7v7.8a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-10a1 1 0 0 1 1-1Z" />
      <path d="M8.5 2.5v3.5H12" />
    </svg>
  );
}

export function DriveIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={16} height={16} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <rect x="2.5" y="4" width="11" height="7.5" rx="1.5" />
      <path d="M4.5 9.5h1.5M10 9.5h1.5" />
    </svg>
  );
}

export function QuestionIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={14} height={14} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <circle cx="8" cy="8" r="6" />
      <path d="M6.3 6.2a1.8 1.8 0 1 1 2.5 1.6c-.6.3-.8.6-.8 1.2V9.2" />
      <circle cx="8" cy="11.4" r="0.75" fill="currentColor" stroke="none" />
    </svg>
  );
}

export function RefreshIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={14} height={14} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <path d="M13 8a5 5 0 1 1-1.3-3.4" />
      <path d="M13 3v3h-3" />
    </svg>
  );
}

export function BrowseIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={14} height={14} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <path d="M2.5 4.5a1 1 0 0 1 1-1H6l1.2 1.5h5.3a1 1 0 0 1 1 1v.8" />
      <rect x="3.2" y="6.2" width="9.6" height="5.8" rx="1" />
    </svg>
  );
}

export function CopyIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={14} height={14} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <rect x="5.5" y="5.5" width="7" height="8" rx="1.2" />
      <path d="M10.5 5.5V3.7a1.2 1.2 0 0 0-1.2-1.2H4.7a1.2 1.2 0 0 0-1.2 1.2v6.6a1.2 1.2 0 0 0 1.2 1.2h.8" />
    </svg>
  );
}

export function ExportIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={14} height={14} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <path d="M8 2.5v7" />
      <path d="M5.5 5 8 2.5 10.5 5" />
      <path d="M3.5 9.5v2.5a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1V9.5" />
    </svg>
  );
}

export function SunIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={14} height={14} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <circle cx="8" cy="8" r="3" />
      <path d="M8 1.5v1.5M8 13v1.5M14.5 8H13M3 8H1.5M12.6 3.4l-1.1 1.1M4.5 11.5l-1.1 1.1M12.6 12.6l-1.1-1.1M4.5 4.5L3.4 3.4" />
    </svg>
  );
}

export function MoonIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} width={14} height={14} viewBox="0 0 16 16" strokeWidth={1.5} {...props}>
      <path d="M13 9.5A5.5 5.5 0 1 1 6.5 3a4.5 4.5 0 0 0 6.5 6.5Z" />
    </svg>
  );
}
