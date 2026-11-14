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
