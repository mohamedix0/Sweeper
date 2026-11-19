/** Format a count with thousands separators and the correct plural noun. */
export function localizedCount(args: { count: number; singular: string; plural: string }): string {
  const noun = args.count === 1 ? args.singular : args.plural;
  return `${args.count.toLocaleString('en-US')} ${noun}`;
}

/** Explicit, prominent label for the single permanent-delete action. */
export function deleteActionLabel(args: { count: number; size: string; cleaning: boolean; singular: string; plural: string }): string {
  if (args.cleaning) return 'Cleaning…';
  const count = localizedCount({ count: args.count, singular: args.singular, plural: args.plural });
  return `Delete ${count} permanently · ${args.size}`;
}
