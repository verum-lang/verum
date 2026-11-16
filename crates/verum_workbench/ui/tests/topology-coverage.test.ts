import assert from 'node:assert/strict';
import test from 'node:test';
import { parseHTML } from 'linkedom';
import { topologyCoverage, type CoverageTarget } from '../src/topology-coverage.ts';
import type { ProjectAnalysis, ProjectSource } from '../src/topology.ts';

type Coverage = NonNullable<ProjectAnalysis['import_topology']>;
const range = { start: { line: 7, character: 0 }, end: { line: 7, character: 20 } };
function origin(name: string): ProjectSource {
  return { uri: `file:///workspace/${name}.vr`, module: name, source_hash: `digest-${name}`,
    source_mode: 'source', version: null, parsed: false };
}
function coverage(values: Partial<Coverage> = {}): Coverage {
  return { scope: 'indexed-source-mounts', project_revision: 'revision', inspected_modules: 2,
    unresolved_mounts: 0, uninspected_sources: [], edges: [], ...values };
}
function render(data: Coverage | undefined, indexed = 3) {
  const { document, window } = parseHTML('<html><body></body></html>');
  Object.assign(globalThis, { document, Node: window.Node });
  const navigated: CoverageTarget[] = [], errors: unknown[] = [];
  const view = topologyCoverage(data, { indexed, analyzed: 1, chevron: document.createElement('span'),
    sourcePath: source => source.uri.startsWith('file:///workspace/') ? source.uri.slice('file:///workspace/'.length) : null,
    navigate: target => { navigated.push(target); }, failure: error => errors.push(error),
  });
  document.body.append(view);
  function expand() {
    (view as HTMLDetailsElement).open = true;
    view.dispatchEvent(new window.Event('toggle'));
  }
  function filter(query: string) {
    const field = view.querySelector('input')!; field.value = query;
    field.dispatchEvent(new window.Event('input'));
  }
  return { view, navigated, errors, expand, filter };
}

test('the visible boundary separates unresolved imports and uninspected sources before disclosure', () => {
  const skipped = origin('broken');
  const ui = render(coverage({ unresolved_mounts: 1,
    uninspected_sources: [{ origin: skipped, reason: 'parse_unavailable' }],
    failures: [{ origin: origin('client'), importer: 'client', source: 'mount absent;',
      location: { uri: 'file:///workspace/client.vr', range }, reason: 'No provider' }],
  }));
  assert.match(ui.view.querySelector('summary')!.textContent!, /Sources inspected: 2 \/ 3/);
  assert.match(ui.view.querySelector('summary')!.textContent!, /1 unresolved imports/);
  assert.match(ui.view.querySelector('summary')!.textContent!, /1 sources not inspected/);
  assert.equal(ui.view.querySelectorAll('.topology-gap').length, 0);
  ui.expand(); ui.expand();
  assert.deepEqual([...ui.view.querySelectorAll('.topology-gap-state')].map(row => row.textContent),
    ['Source not inspected', 'Unresolved import']);
  assert.match(ui.view.textContent!, /Source could not be parsed/);
  assert.match(ui.view.textContent!, /Calls, context provision and runtime flows are outside its scope/);
  assert.equal(ui.view.querySelectorAll('input').length, 1);
});

test('navigation retains the exact source identity and only an actual mount receives a range', async () => {
  const skipped = origin('unknown'), imported = { ...origin('client'), parsed: true, version: 17, source_mode: 'script' as const };
  const location = { uri: imported.uri, range };
  const ui = render(coverage({ unresolved_mounts: 1,
    uninspected_sources: [{ origin: skipped, reason: 'module_unavailable' }],
    failures: [{ origin: imported, importer: 'client', source: 'mount absent;', location, reason: 'Absent provider' }],
  }));
  ui.expand();
  for (const button of ui.view.querySelectorAll<HTMLButtonElement>('.topology-site')) button.click();
  await Promise.resolve();
  assert.deepEqual(ui.navigated, [{ origin: skipped, location: undefined }, { origin: imported, location }]);
  assert.equal(ui.navigated[0].origin, skipped);
  assert.equal(ui.navigated[1].origin, imported);
  assert.equal(ui.navigated[1].location, location);
  assert.deepEqual(ui.errors, []);
  assert.match(ui.view.textContent!, /Module identity unavailable/);
});

test('large gap sets have bounded disclosure and the filter searches the whole scan', () => {
  const ui = render(coverage({ uninspected_sources: Array.from({ length: 12000 }, (_, i) => ({
    origin: origin(`module${i}`), reason: 'parse_unavailable' as const,
  })) }), 12002);
  ui.expand();
  assert.equal(ui.view.querySelectorAll('.topology-gap').length, 24);
  assert.match(ui.view.querySelector('[role=status]')!.textContent!, /24 of 12,000 matching gaps/);
  ui.view.querySelector<HTMLButtonElement>('.quiet-button')!.click();
  assert.equal(ui.view.querySelectorAll('.topology-gap').length, 48);
  ui.filter('module11999');
  assert.equal(ui.view.querySelectorAll('.topology-gap').length, 1);
  assert.equal(ui.view.querySelector('.topology-site')!.textContent, 'module11999.vr');
  assert.equal(ui.view.querySelector<HTMLButtonElement>('.quiet-button')!.hidden, true);
  ui.filter('could not be parsed');
  assert.equal(ui.view.querySelectorAll('.topology-gap').length, 24);
  ui.filter('no such source');
  assert.equal(ui.view.querySelectorAll('.topology-gap').length, 0);
  assert.match(ui.view.querySelector('[role=status]')!.textContent!, /0 of 0 matching gaps · 12,000 reported/);
});

test('missing failure details remain unknown and do not acquire invented source links', () => {
  const ui = render(coverage({ unresolved_mounts: 3 })); ui.expand();
  assert.match(ui.view.textContent!, /Source details unavailable for 3 unresolved imports/);
  assert.equal(ui.view.querySelector('.topology-site'), null);
  assert.doesNotMatch(ui.view.textContent!, /No gaps reported/);
});

test('external origins are readable and source payloads stay literal', () => {
  const external = { ...origin('external'), uri: 'file:///elsewhere/external.vr' };
  const payload = '<img src=x onerror=alert(1)>', ui = render(coverage({ unresolved_mounts: 1,
    failures: [{ origin: external, importer: 'external', source: payload,
      location: { uri: external.uri, range }, reason: '<script>unresolved</script>' }],
  }));
  ui.expand();
  assert.equal(ui.view.querySelector('.topology-site')!.tagName, 'CODE');
  assert.equal(ui.view.querySelector('.topology-site')!.textContent, payload);
  assert.equal(ui.view.querySelector('img,script,[onerror]'), null);
});

test('empty and unavailable scans make different bounded claims', () => {
  const empty = render(coverage()); empty.expand();
  assert.match(empty.view.textContent!, /No gaps reported within this source-mount scan/);
  assert.doesNotMatch(empty.view.textContent!, /complete|isolated|proven/i);
  const unavailable = render(undefined);
  assert.match(unavailable.view.textContent!, /1 analyzed documents · 3 indexed sources/);
  assert.match(unavailable.view.textContent!, /Project import scan unavailable/);
  assert.equal(unavailable.view.querySelector('summary'), null);
});
