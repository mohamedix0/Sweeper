import type { AnalyzerNodeDto } from '../../contract/bindings';

export type FlatAnalyzerRow = { node: AnalyzerNodeDto; depth: number; siblingMax: bigint };
export type AnalyzerStatusFilter = 'all' | 'safe' | 'caution' | 'unknown' | 'locked';
export type Pill = { label: string; className: string } | null;
