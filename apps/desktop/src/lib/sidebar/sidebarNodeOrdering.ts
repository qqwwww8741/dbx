import type { DatabaseType, TreeNode } from "@/types/database";

const sidebarTreeCollator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

function sortByLabel(nodes: readonly TreeNode[]): TreeNode[] {
  return [...nodes].sort((left, right) => sidebarTreeCollator.compare(left.label, right.label));
}

function sortRecursive(node: TreeNode, databaseType?: DatabaseType): TreeNode {
  const children = node.children ? sortSidebarTreeChildrenForParent(node, node.children, databaseType) : node.children;
  const hiddenChildren = node.hiddenChildren ? sortSidebarTreeChildrenForParent(node, node.hiddenChildren, databaseType) : node.hiddenChildren;
  if (children === node.children && hiddenChildren === node.hiddenChildren) return node;
  return {
    ...node,
    children,
    hiddenChildren,
  };
}

/**
 * Virtual table groups are a display-level projection (see
 * `applyTableVGroupsToChildren`) whose order comes from the stored layout, not
 * from the alphabet. Every ordering pass therefore has to leave them ahead of
 * the flat rows: a name-sorted metadata merge would otherwise drop a group back
 * to its alphabetical slot, and the row would visibly jump from the top of the
 * list to the bottom until the next projection ran (issues #9644 / #9653).
 */
function hoistTableVGroupChildren(nodes: TreeNode[]): TreeNode[] {
  if (!nodes.some((node) => node.type === "table-vgroup")) return nodes;
  return [...nodes.filter((node) => node.type === "table-vgroup"), ...nodes.filter((node) => node.type !== "table-vgroup")];
}

export function sortSidebarTreeChildrenForParent(parent: Pick<TreeNode, "type">, children: readonly TreeNode[], databaseType?: DatabaseType): TreeNode[] {
  return hoistTableVGroupChildren(orderSidebarTreeChildrenForParent(parent, children, databaseType));
}

/**
 * Orders a container whose rows are merged from paged metadata: only the flat
 * rows are sorted by label, while projected virtual table groups keep the
 * position (and relative order) the layout projection gave them. Sorting the
 * groups by name would move them out of the top slot they were projected into
 * (issues #9644 / #9653).
 */
export function sortSidebarTreeChildrenByNameKeepingTableVGroups(parent: Pick<TreeNode, "type">, children: readonly TreeNode[], databaseType?: DatabaseType): TreeNode[] {
  const groups = children.filter((node) => node.type === "table-vgroup");
  const flat = children.filter((node) => node.type !== "table-vgroup");
  const orderedFlat = [...flat].sort((left, right) => sidebarTreeCollator.compare(left.label, right.label));
  return sortSidebarTreeChildrenForParent(parent, [...groups, ...orderedFlat], databaseType);
}

function isBottomConnectionUtilityNode(child: TreeNode): boolean {
  return child.type === "user-admin";
}

function orderSidebarTreeChildrenForParent(parent: Pick<TreeNode, "type">, children: readonly TreeNode[], databaseType?: DatabaseType): TreeNode[] {
  const normalized = children.map((child) => sortRecursive(child, databaseType));

  {}

  {}

  {}

  if (parent.type === "connection") {
    const savedSqlNodes = normalized.filter((child) => child.type === "saved-sql-root");
    const bottomUtilityNodes = normalized.filter(isBottomConnectionUtilityNode);
    const regularChildren = normalized.filter((child) => child.type !== "saved-sql-root" && !isBottomConnectionUtilityNode(child));
    const withConnectionUtilityOrder = (children: TreeNode[]) => [...savedSqlNodes, ...children, ...bottomUtilityNodes];

    {
    }

    {
    }

    if (regularChildren.every((child) => child.type === "database")) {
      return withConnectionUtilityOrder(sortByLabel(regularChildren));
    }

    if (regularChildren.every((child) => child.type === "schema")) {
      return withConnectionUtilityOrder(sortByLabel(regularChildren));
    }

    return withConnectionUtilityOrder(regularChildren);
  }

  if (parent.type === "database") {
    {
    }

    if (normalized.every((child) => child.type === "schema")) {
      return sortByLabel(normalized);
    }
  }

  return normalized;
}
