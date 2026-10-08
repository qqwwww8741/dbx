import type { CompletionAssistantCandidate, DatabaseType, TreeNode } from "@/types/database";

const PACKAGE_MEMBER_GROUP_MARKER = ":members:";

export function packageMemberGroupOwnerId(node: TreeNode): string | null {
  if (node.parentType !== "package" || (node.type !== "group-procedures" && node.type !== "group-functions")) return null;
  const markerIndex = node.id.lastIndexOf(PACKAGE_MEMBER_GROUP_MARKER);
  if (markerIndex <= 0) return null;
  return node.id.slice(0, markerIndex);
}

export function markPackageNodesExpandable(nodes: TreeNode[]): TreeNode[] {
  return nodes.map((node) => (node.type === "package" ? { ...node, children: node.children ?? [] } : node));
}

function packageMemberNode(packageNode: TreeNode, kind: "procedure" | "function", name: string, signature: string): TreeNode {
  return {
    id: `${packageNode.id}:member:${kind}:${name}:${signature}`,
    label: signature ? `${name}(${signature})` : name,
    type: kind,
    objectName: name,
    signature: signature || undefined,
    parentName: packageNode.objectName || packageNode.label,
    parentSchema: packageNode.schema,
    parentType: "package",
    valid: packageNode.valid,
    connectionId: packageNode.connectionId,
    database: packageNode.database,
    schema: packageNode.schema,
    isExpanded: false,
    children: undefined,
  };
}

export function buildPackageMemberNodes(packageNode: TreeNode, candidates: readonly CompletionAssistantCandidate[], _databaseType?: DatabaseType): TreeNode[] {
  const seen = new Set<string>();
  const members: TreeNode[] = [];
  const procedures: TreeNode[] = [];
  const functions: TreeNode[] = [];

  for (const candidate of candidates) {
    if (candidate.kind !== "procedure" && candidate.kind !== "function") continue;
    const name = candidate.name.trim();
    if (!name) continue;
    const signature = candidate.signature?.trim() || "";
    const key = `${candidate.kind}\0${name}\0${signature}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const member = packageMemberNode(packageNode, candidate.kind, name, signature);
    members.push(member);
    if (candidate.kind === "procedure") procedures.push(member);
    else functions.push(member);
  }

  // Xugu presents package specifications and bodies as one logical package.
  // Keep the richer member folders scoped to Xugu so Oracle and other package
  // providers retain their existing flat member tree.
  {
    return members;
  }
}
