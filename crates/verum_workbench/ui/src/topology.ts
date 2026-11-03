import type { Analysis, Binding, SourceLocation } from './language.ts';

export type ProjectSource = { uri: string; module?: string };
export type ProjectAnalysis = {
  project_revision: string;
  sources?: ProjectSource[];
  documents: Array<Analysis & { uri: string }>;
  issues?: string[];
  import_topology?: { inspected_modules: number; unresolved_mounts: number; edges: Array<{
    importer: string; provider: string; location: SourceLocation; source: string; names: string[];
  }> };
};
export type ImportSite = { source: string; location: SourceLocation; names: Array<{ name: string; definition?: SourceLocation | null }> };
type ModuleLinks = Map<string, ImportSite[]>;

/** All relationships come from the resolver that populated this generation's binding table. */
export function moduleNeighborhood(project: ProjectAnalysis, uri: string, digest: string) {
  if (!project.project_revision) return null;
  const sources = new Map((project.sources ?? []).map(s => [s.uri, s.module]));
  const docs = project.documents.filter(d => d.project_revision === project.project_revision);
  const selected = docs.find(d => d.uri === uri && d.source_hash === digest);
  const owner = sources.get(uri);
  if (!selected || !owner) return null;
  const incoming: ModuleLinks = new Map(), outgoing: ModuleLinks = new Map();
  const gaps: Array<{ site: ImportSite; reason?: string }> = [];
  for (const doc of docs) {
    const from = sources.get(doc.uri);
    if (!from || (project.import_topology && doc !== selected)) continue;
    for (const mount of doc.mounts) {
      const site = { source: mount.source, location: { uri: doc.uri, range: mount.range }, names: [] };
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
  const topology = project.import_topology;
  if (topology) {
    incoming.clear();
    for (const edge of topology.edges) {
      if (edge.provider !== owner) continue;
      const sites = incoming.get(edge.importer) ?? [];
      sites.push({ source: edge.source, location: edge.location,
        names: edge.names.map(name => ({ name })) });
      incoming.set(edge.importer, sites);
    }
  }
  return { owner, incoming, outgoing, gaps, coverage: topology, analyzed: docs.length, indexed: sources.size };
}
