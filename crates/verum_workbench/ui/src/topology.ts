import type { Analysis, Binding, SourceLocation } from './language.ts';
import type { SourceMode } from './model.ts';

export type ProjectSource = {
  uri: string; module: string; source_hash: string; source_mode: SourceMode; version: number | null; parsed: boolean;
};
export type ImportFailure = { origin: ProjectSource; importer: string; source: string; location: SourceLocation; reason: string };
export type ProjectAnalysis = {
  project_revision: string;
  sources?: ProjectSource[];
  documents: Array<Analysis & { uri: string }>;
  issues?: string[];
  import_topology?: {
    scope: 'indexed-source-mounts'; project_revision: string;
    inspected_modules: number;
    uninspected_sources: Array<{ origin: ProjectSource; reason: 'parse_unavailable' | 'module_unavailable' }>;
    unresolved_mounts: number; failures?: ImportFailure[]; edges: Array<{
    origin: ProjectSource; importer: string; provider: string; location: SourceLocation; source: string; names: string[];
  }> };
};
export type ImportSite = { origin: ProjectSource; source: string; location: SourceLocation; names: Array<{ name: string; definition?: SourceLocation | null }> };
type ModuleLinks = Map<string, ImportSite[]>;
type ImportTopology = NonNullable<ProjectAnalysis['import_topology']>;
type ImportEdge = ImportTopology['edges'][number];
export type ImportDependent = { distance: number; edge: ImportEdge };

/** One shortest witness per reachable importer, not a claim of behavioral impact.
 * Keep a predecessor tree instead of duplicating paths: O(modules + mounts).
 * The server's resolver owns every edge, including aliases and reexports.
 */
function importDependents(topology: ImportTopology, owner: string): Map<string, ImportDependent> {
  const incoming = new Map<string, ImportEdge[]>();
  for (const edge of topology.edges) {
    const links = incoming.get(edge.provider) ?? [];
    links.push(edge); incoming.set(edge.provider, links);
  }
  const result = new Map<string, ImportDependent>();
  const queue = [owner];
  for (let head = 0; head < queue.length; head++) {
    const provider = queue[head];
    const distance = (result.get(provider)?.distance ?? 0) + 1;
    for (const edge of incoming.get(provider) ?? []) {
      if (edge.importer === owner || result.has(edge.importer)) continue;
      result.set(edge.importer, { distance, edge });
      queue.push(edge.importer);
    }
  }
  return result;
}

export function importDependencyPath(dependents: Map<string, ImportDependent>, importer: string): ImportEdge[] {
  const path: ImportEdge[] = [];
  let link = dependents.get(importer);
  while (link) {
    path.push(link.edge);
    link = dependents.get(link.edge.provider);
  }
  return path;
}

/** All relationships come from the resolver that populated this generation's binding table. */
export function moduleNeighborhood(project: ProjectAnalysis, uri: string, digest: string) {
  if (!project.project_revision) return null;
  const topology = project.import_topology;
  if (topology && (topology.project_revision !== project.project_revision || topology.scope !== 'indexed-source-mounts')) return null;
  const sources = new Map((project.sources ?? []).map(s => [s.uri, s]));
  const docs = project.documents.filter(d => d.project_revision === project.project_revision);
  const selected = docs.find(d => d.uri === uri && d.source_hash === digest);
  const subject = sources.get(uri);
  if (!selected || !subject || subject.source_hash !== digest) return null;
  const owner = subject.module;
  const incoming: ModuleLinks = new Map(), outgoing: ModuleLinks = new Map();
  const gaps: Array<{ site: ImportSite; reason?: string }> = [];
  for (const doc of docs) {
    const origin = sources.get(doc.uri);
    if (!origin || origin.source_hash !== doc.source_hash || (topology && doc !== selected)) continue;
    const from = origin.module;
    for (const mount of doc.mounts) {
      const site = { origin, source: mount.source, location: { uri: doc.uri, range: mount.range }, names: [] };
      const resolution = mount.resolution;
      if (resolution?.state !== 'resolved') {
        if (doc === selected) gaps.push({ site, reason: resolution?.reason });
        continue;
      }
      const names = doc.bindings ?? [];
      const { start, end } = resolution.bindings;
      const groups: Map<string, Binding[]> = new Map();
      for (const item of names.slice(start, end)) {
        if (!item.source_module) continue;
        const group = groups.get(item.source_module) ?? [];
        group.push(item); groups.set(item.source_module, group);
      }
      for (const [to, bindings] of groups) {
        if (doc !== selected && to !== owner) continue;
        const links = doc === selected ? outgoing : incoming;
        const key = doc === selected ? to : from;
        const sites = links.get(key) ?? [];
        sites.push({ ...site, names: bindings }); links.set(key, sites);
      }
    }
  }
  if (topology) {
    incoming.clear();
    for (const edge of topology.edges) {
      if (edge.provider !== owner) continue;
      const sites = incoming.get(edge.importer) ?? [];
      sites.push({ origin: edge.origin, source: edge.source, location: edge.location,
        names: edge.names.map(name => ({ name })) });
      incoming.set(edge.importer, sites);
    }
  }
  return { owner, incoming, outgoing, gaps, coverage: topology,
    dependents: topology ? importDependents(topology, owner) : null,
    analyzed: docs.length, indexed: sources.size };
}
