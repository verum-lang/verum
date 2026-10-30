import { m, t } from './i18n';

const fragment = new URLSearchParams(location.hash.slice(1));
const raw = location.hash.slice(1);
const supplied = fragment.get('token') || (/^[\w-]{24,}$/.test(raw) ? raw : null);
let token = supplied || sessionStorage.getItem('verum.workbench.token') || '';
if (supplied) {
  sessionStorage.setItem('verum.workbench.token', supplied);
  history.replaceState(null, '', location.pathname + location.search);
}

export function installToken(value: string) {
  token = value.trim();
  sessionStorage.setItem('verum.workbench.token', token);
}
export function hasToken() { return token.length > 0; }

export async function api<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
  const response = await fetch('/api', {
    method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
    body: JSON.stringify({ method, params }), cache: 'no-store', credentials: 'omit', redirect: 'error',
  });
  let body: { result?: T; error?: string };
  try { body = await response.json(); } catch { throw new Error(t(m('Local service returned HTTP {status} without a JSON response.', { status: response.status }))); }
  if (!response.ok || body.error !== undefined) throw new Error(body.error || t(m('Local service returned HTTP {status}.', { status: response.status })));
  if (!('result' in body)) throw new Error(t(m('Local service returned an incomplete response.')));
  return body.result as T;
}
