export type ProjectEntry = { path: string; kind: 'directory' | 'file'; size: number };
export type ProjectListing = { root: string; entries: ProjectEntry[]; truncated: boolean };
export type GitChange = { path: string; index: string; worktree: string; original_path: string | null };
export type GitSnapshot = { state: 'ready' | 'not_repository' | 'unavailable'; branch: string | null; entries: GitChange[]; error: string | null };
export type TreeRow = ProjectEntry & { depth: number; position: number; siblings: number };
export const parentPath = (path: string) => path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
export const fileName = (path: string) => path.slice(path.lastIndexOf('/') + 1);
export const sortEntries = (entries: ProjectEntry[]) => entries.sort((a,b) => Number(a.kind !== 'directory') - Number(b.kind !== 'directory') || a.path.localeCompare(b.path));

/** A filtered projection retains ancestors without loading unrelated directories. */
export function projectedTree(entries: ProjectEntry[]): Map<string, ProjectEntry[]> {
  const unique = new Map(entries.map(entry => [entry.path, entry]));
  for (const entry of entries) for (let parent = parentPath(entry.path); parent; parent = parentPath(parent)) {
    if (!unique.has(parent)) unique.set(parent, { path: parent, kind: 'directory', size: 0 });
  }
  const directories = new Map<string, ProjectEntry[]>();
  for (const entry of unique.values()) {
    const parent = parentPath(entry.path);
    if (!directories.has(parent)) directories.set(parent, []);
    directories.get(parent)!.push(entry);
  }
  for (const children of directories.values()) sortEntries(children);
  return directories;
}
export function visibleTree(directories: Map<string, ProjectEntry[]>, expanded: Set<string>, revealAll = false): TreeRow[] {
  const rows: TreeRow[] = [];
  function visit(parent: string, depth: number) {
    const children = directories.get(parent) ?? [];
    children.forEach((entry, index) => {
      rows.push({ ...entry, depth, position: index + 1, siblings: children.length });
      if (entry.kind === 'directory' && (revealAll || expanded.has(entry.path))) visit(entry.path, depth + 1);
    });
  }
  visit('', 1); return rows;
}
export function changedDirectories(changes: GitChange[]): Map<string, number> {
  const result = new Map<string, number>();
  for (const change of changes) for (let parent = parentPath(change.path); parent; parent = parentPath(parent)) result.set(parent, (result.get(parent) ?? 0) + 1);
  return result;
}
