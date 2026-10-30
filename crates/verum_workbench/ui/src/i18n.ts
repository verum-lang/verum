import { translations, type Message } from './translations.ts';

export const locales = ['en', 'ru'] as const;
export type Locale = typeof locales[number];
const numbers = Object.fromEntries(locales.map(locale => [locale, new Intl.NumberFormat(locale, { maximumFractionDigits: 2 })])) as Record<Locale, Intl.NumberFormat>;
type Placeholders<S extends string> = S extends `${string}{${infer Key}}${infer Rest}` ? Key | Placeholders<Rest> : never;
export type Localized = { key: Message; values: Record<string, string | number | Localized>; whitespace?: [string, string] };
export type Content = string | Localized;
type Arguments<K extends Message> = [Placeholders<K>] extends [never] ? [] : [values: Record<Placeholders<K>, string | number | Localized>];
export function m<K extends Message>(key: K, ...args: Arguments<K>): Localized { return { key, values: args[0] || {} }; }
export function resolveLocale(preference: unknown, languages: readonly string[]): Locale {
  if (preference === 'en' || preference === 'ru') return preference;
  for (const language of languages) {
    const primary = language.toLowerCase().split('-')[0];
    if (primary === 'en' || primary === 'ru') return primary;
  }
  return 'en';
}
export function formatMessage(message: Content, locale: Locale): string {
  if (typeof message === 'string') return message;
  const template = locale === 'ru' ? translations[message.key] ?? message.key : message.key;
  const text = template.replace(/\{(\w+)\}/g, (_, key: string) => {
    const value = message.values[key];
    if (value === undefined) throw new Error(`Missing translation argument: ${message.key} / ${key}`);
    return typeof value === 'number' ? numbers[locale].format(value) : formatMessage(value, locale);
  });
  return (message.whitespace?.[0] || '') + text + (message.whitespace?.[1] || '');
}
let locale: Locale = 'en';
const textBindings = new WeakMap<Node, Localized>();
const attributeBindings = new WeakMap<Element, Map<string, Localized>>();
export const currentLocale = () => locale;
export const t = (message: Content) => formatMessage(message, locale);
export function setText(target: Node, content: Content) {
  // Bind the text leaf, never its container: labels may acquire inputs or buttons later.
  const leaf = target.nodeType === Node.TEXT_NODE ? target : document.createTextNode('');
  if (leaf !== target) { target.textContent = ''; target.appendChild(leaf); }
  if (typeof content === 'string') textBindings.delete(leaf); else textBindings.set(leaf, content);
  leaf.textContent = t(content);
}
export function setAttribute(target: Element, name: string, content: Content) {
  let bindings = attributeBindings.get(target);
  if (!bindings) { bindings = new Map(); attributeBindings.set(target, bindings); }
  if (typeof content === 'string') bindings.delete(name); else bindings.set(name, content);
  target.setAttribute(name, t(content));
}
export function setLocale(next: Locale) {
  locale = next;
  document.documentElement.lang = next;
  try { localStorage.setItem('verum.studio.language', next); } catch { /* Session-only preference. */ }
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
  // Collect before replacing leaf text: mutation must not invalidate the traversal.
  const nodes: Node[] = [];
  while (walker.nextNode()) nodes.push(walker.currentNode);
  for (const node of nodes) {
    const message = textBindings.get(node);
    if (message) node.textContent = t(message);
    if (node instanceof Element) for (const [name, value] of attributeBindings.get(node) || []) node.setAttribute(name, t(value));
  }
  document.dispatchEvent(new CustomEvent('studio:language', { detail: next }));
}

/** Capture only the bundled shell, before source, evidence or provider text enters the DOM. */
export function initializeLanguage() {
  let preference: unknown;
  try { preference = localStorage.getItem('verum.studio.language'); } catch { /* Use browser languages. */ }
  locale = resolveLocale(preference, navigator.languages);
  const known = (value: string): value is Message => Object.hasOwn(translations, value);
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode, key = node.textContent?.trim() || '';
    if (known(key) && !node.parentElement?.closest('pre,code,textarea,script,style,#language')) {
      const value = node.textContent || '';
      textBindings.set(node, { key, values: {}, whitespace: [value.match(/^\s*/)?.[0] || '', value.match(/\s*$/)?.[0] || ''] });
    }
  }
  for (const element of document.querySelectorAll('*')) for (const attribute of ['title', 'aria-label', 'placeholder']) {
    const key = element.getAttribute(attribute);
    if (key && known(key)) setAttribute(element, attribute, { key, values: {} });
  }
  setLocale(locale);
  return locale;
}
