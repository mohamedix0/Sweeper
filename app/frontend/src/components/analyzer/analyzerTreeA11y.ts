export function treeItemDomId(nodeId: string): string {
  return `analyzer-tree-item-${nodeId}`;
}

export function showAllButtonDomId(parentId: string): string {
  return `analyzer-show-all-${parentId}`;
}
