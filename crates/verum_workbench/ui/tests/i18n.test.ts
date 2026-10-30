import test from 'node:test';
import assert from 'node:assert/strict';
import { translations } from '../src/translations.ts';
import { resolveLocale, formatMessage, m } from '../src/i18n.ts';

test('every translation preserves exactly its named arguments', () => {
  const parameters = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort();
  for (const [key, translation] of Object.entries(translations)) {
    assert.ok(translation.trim(), key);
    assert.deepEqual(parameters(translation), parameters(key), key);
  }
});

test('saved language wins; browser region tags and unsupported preferences fall back predictably', () => {
  assert.equal(resolveLocale('ru', ['en-US']), 'ru');
  assert.equal(resolveLocale('en', ['ru-RU']), 'en');
  assert.equal(resolveLocale(null, ['de-DE', 'RU-ru', 'en']), 'ru');
  assert.equal(resolveLocale('invalid', ['en-GB', 'ru']), 'en');
  assert.equal(resolveLocale({}, ['ja']), 'en');
  assert.equal(resolveLocale(null, []), 'en');
});

test('nested UI messages translate without translating or re-interpolating source and evidence', () => {
  const message = m('{kind} · {path} · {status}', { kind: m('Execution'), path: 'Run/{status}/исходник.vr', status: m('succeeded') });
  assert.equal(formatMessage(message, 'ru'), 'Выполнение · Run/{status}/исходник.vr · успешно');
  assert.equal(formatMessage(message, 'en'), 'Execution · Run/{status}/исходник.vr · succeeded');
  assert.equal(formatMessage('Run', 'ru'), 'Run');
  assert.equal(formatMessage('print("{kind}"); <script>', 'ru'), 'print("{kind}"); <script>');
  assert.equal(formatMessage(m('Wall budget {seconds}s', { seconds: 1.5 }), 'ru'), 'Лимит полного времени: 1,5 с');
});
