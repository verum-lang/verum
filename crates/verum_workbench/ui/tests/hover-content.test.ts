import test from 'node:test';
import assert from 'node:assert/strict';
import { parseHTML } from 'linkedom';
import { renderHoverContent } from '../src/hover-content.ts';

function render(contents: unknown) {
  const { document } = parseHTML('<html><body></body></html>');
  return renderHoverContent(contents, document);
}

test('LSP Markdown renders the Verum signature and diagnostic as separate blocks', () => {
  const tooltip = render({ kind: 'markdown', value: '```verum\ntype Source\n```\n\n- type not found: Source' });
  assert.ok(tooltip);
  assert.equal(tooltip.querySelector('pre code')?.textContent, 'type Source\n');
  assert.equal(tooltip.querySelector('code')?.getAttribute('data-language'), 'verum');
  assert.equal(tooltip.querySelector('ul li')?.textContent, 'type not found: Source');
  assert.ok(!tooltip.textContent?.includes('```'));
});

test('plaintext remains literal and legacy MarkedString arrays retain code languages', () => {
  const literal = '```verum\ntype Source\n```\n**literal** <b>text</b>';
  const plain = render({ kind: 'plaintext', value: literal });
  assert.equal(plain?.textContent, literal);
  assert.equal(plain?.querySelector('code, strong, b'), null);
  const marked = render([{ language: 'verum', value: 'fn read<T>(value: T) -> T' }, '**Generic** `T`']);
  assert.equal(marked?.querySelector('pre code')?.textContent, 'fn read<T>(value: T) -> T');
  assert.equal(marked?.querySelector('strong')?.textContent, 'Generic');
  assert.equal(marked?.querySelector('p code')?.textContent, 'T');
});

test('source documentation cannot execute HTML, command URLs or remote images', () => {
  const tooltip = render('<img src="https://example.org/track" onerror="alert(1)">\n\n'
    + '[command](command:workbench.action.closeWindow) [script](javascript:alert(1)) '
    + '![remote](https://example.org/track) [docs](https://verum-lang.org "Read docs")');
  assert.ok(tooltip);
  assert.equal(tooltip.querySelector('img, script, iframe, [onerror], style'), null);
  const links = [...tooltip.querySelectorAll('a')];
  assert.equal(links.length, 1);
  assert.equal(links[0].getAttribute('href'), 'https://verum-lang.org/');
  assert.equal(links[0].getAttribute('rel'), 'noopener noreferrer');
  assert.ok(tooltip.textContent?.includes('remote'));
});

test('nested documentation, entities and multiline code preserve their meaning', () => {
  const tooltip = render('## API\n\n> **Read** &lt;T&gt;\n\n3. first\n   - nested\n\n'
    + '| Input | Result |\n| --- | --- |\n| `T` | *T* |\n\n```verum\nfn read() {\n    7\n}\n```');
  assert.equal(tooltip?.querySelector('h2')?.textContent, 'API');
  assert.equal(tooltip?.querySelector('blockquote')?.textContent, 'Read <T>');
  assert.equal(tooltip?.querySelector('ol')?.getAttribute('start'), '3');
  assert.equal(tooltip?.querySelector('ol ul li')?.textContent, 'nested');
  assert.equal(tooltip?.querySelectorAll('th').length, 2);
  assert.equal(tooltip?.querySelector('pre code')?.textContent, 'fn read() {\n    7\n}\n');
});

test('empty or malformed hover contents do not create an empty tooltip', () => {
  for (const contents of [null, undefined, '', [], {}, { value: {} }, { kind: 'unknown', value: 'ignored' }]) {
    assert.equal(render(contents), null);
  }
});
