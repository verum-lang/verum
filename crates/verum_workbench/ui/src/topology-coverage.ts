import { m, t, setAttribute, setText, type Content } from './i18n.ts';
import type { SourceLocation } from './language.ts';
import type { ProjectAnalysis, ProjectSource } from './topology.ts';

type ImportCoverage = NonNullable<ProjectAnalysis['import_topology']>;
export type CoverageTarget = { origin: ProjectSource; location?: SourceLocation };
type CoverageOptions = {
  indexed: number;
  analyzed: number;
  chevron: Node;
  sourcePath: (origin: ProjectSource) => string | null;
  navigate: (target: CoverageTarget) => unknown | Promise<unknown>;
  failure: (error: unknown) => void;
};
const pageSize = 24;

function element<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: Content) {
  const result = document.createElement(tag); result.className = className;
  if (text !== undefined) setText(result, text);
  return result;
}

/** This is the boundary of a source-mount projection, not a completeness score. */
export function topologyCoverage(coverage: ImportCoverage | undefined, options: CoverageOptions) {
  if (!coverage) {
    const boundary = element('div', 'topology-boundary');
    boundary.append(element('p', 'topology-coverage', m('Import coverage: {analyzed} analyzed documents · {indexed} indexed sources', options)),
      element('p', 'muted', m('Project import scan unavailable. Links come from analyzed documents.')));
    return boundary;
  }
  const boundary = element('details', 'topology-boundary');
  const summary = element('summary', '');
  const heading = element('span', '');
  heading.append(element('span', 'topology-boundary-label', m('Indexed source imports')),
    element('span', 'topology-coverage', m('Sources inspected: {inspected} / {indexed}',
      { inspected: coverage.inspected_modules, indexed: options.indexed })));
  const counts = element('span', 'topology-boundary-counts');
  counts.append(element('span', coverage.unresolved_mounts ? 'topology-unknown' : 'muted',
    m('{count} unresolved imports', { count: coverage.unresolved_mounts })),
  element('span', coverage.uninspected_sources.length ? 'topology-unknown' : 'muted',
    m('{count} sources not inspected', { count: coverage.uninspected_sources.length })));
  summary.append(options.chevron, heading, counts); boundary.append(summary);
  let initialized = false;
  boundary.addEventListener('toggle', () => {
    if (!boundary.open || initialized) return;
    initialized = true;
    boundary.append(element('p', 'topology-coverage', m('This view follows resolved source mounts. Calls, context provision and runtime flows are outside its scope.')));
    const failures = coverage.failures ?? [];
    const unavailable = Math.max(0, coverage.unresolved_mounts - failures.length);
    if (unavailable) boundary.append(element('p', 'topology-unknown',
      m('Source details unavailable for {count} unresolved imports.', { count: unavailable })));
    const rows = [
      ...coverage.uninspected_sources.map(({ origin, reason }) => ({
        origin, location: undefined as SourceLocation | undefined, source: '',
        status: m('Source not inspected'),
        reason: reason === 'parse_unavailable' ? m('Source could not be parsed') : m('Module identity unavailable'),
      })),
      ...failures.map(failure => ({ ...failure, status: m('Unresolved import'), reason: failure.reason as Content })),
    ];
    if (!rows.length) {
      if (!unavailable) boundary.append(element('p', 'muted', m('No gaps reported within this source-mount scan.')));
      return;
    }
    const search = element('input', 'topology-gap-filter'); search.type = 'search';
    setAttribute(search, 'aria-label', m('Filter coverage gaps'));
    setAttribute(search, 'placeholder', m('Filter by module, path or reason'));
    const list = element('div', 'topology-gap-list');
    const count = element('p', 'topology-coverage'); count.setAttribute('role', 'status'); count.setAttribute('aria-live', 'polite');
    const more = element('button', 'quiet-button', m('Show more coverage gaps')); more.type = 'button';
    boundary.append(search, count, list, more);
    let limit = pageSize;
    const prepared = rows.map(row => {
      const path = options.sourcePath(row.origin);
      return { ...row, path, search: [row.origin.module, path ?? row.origin.uri, row.source].join('\n').toLocaleLowerCase() };
    });
    function draw() {
      const query = search.value.trim().toLocaleLowerCase();
      const matches = prepared.filter(row => `${row.search}\n${t(row.reason).toLocaleLowerCase()}`.includes(query));
      const shown = matches.slice(0, limit);
      list.replaceChildren();
      for (const row of shown) {
        const item = element('article', 'topology-gap');
        item.append(element('span', 'topology-gap-state', row.status));
        const label = row.source || row.path || row.origin.uri;
        const source = row.path === null
          ? element('code', 'topology-site', label)
          : element('button', 'topology-site', label);
        if (row.path !== null) {
          (source as HTMLButtonElement).type = 'button';
          source.addEventListener('click', () => {
            Promise.resolve().then(() => options.navigate({ origin: row.origin, location: row.location })).catch(options.failure);
          });
        }
        item.append(source);
        if (row.source) item.append(element('code', 'topology-gap-path', row.path ?? row.origin.uri));
        item.append(element('p', 'muted', row.reason)); list.append(item);
      }
      setText(count, m('Showing {shown} of {matching} matching gaps · {total} reported',
        { shown: shown.length, matching: matches.length, total: prepared.length }));
      more.hidden = shown.length === matches.length;
    }
    search.addEventListener('input', () => { limit = pageSize; draw(); });
    more.addEventListener('click', () => { limit += pageSize; draw(); });
    draw();
  });
  return boundary;
}
