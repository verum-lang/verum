import { m, t } from './i18n';
import { StudioSession, SessionAccessError, SessionChangedError, type SessionState } from './session';

const fragment = new URLSearchParams(location.hash.slice(1));
const raw = location.hash.slice(1);
const supplied = fragment.get('token') || (/^[\w-]{24,}$/.test(raw) ? raw : null);
function savedToken() { try { return sessionStorage.getItem('verum.workbench.token') || ''; } catch { return ''; } }
function rememberToken(value: string) {
  try { if (value) sessionStorage.setItem('verum.workbench.token', value); else sessionStorage.removeItem('verum.workbench.token'); }
  catch { /* An authenticated session can still work with tab storage disabled. */ }
}
export const initialToken = supplied || savedToken();
if (supplied) {
  history.replaceState(null, '', location.pathname + location.search);
}

const listeners = new Set<(state: SessionState) => void>();
const session = new StudioSession(send, state => {
  if (state === 'locked') rememberToken('');
  for (const listener of listeners) listener(state);
});
export function onSessionChange(listener: (state: SessionState) => void) {
  listeners.add(listener); listener(session.state);
  return () => listeners.delete(listener);
}
export function isConnected() { return session.state === 'connected'; }
function localized(error: unknown): never {
  if (error instanceof SessionAccessError) throw new SessionAccessError(t(m('Access token required or rejected. Open the current launch URL or enter its token.')));
  if (error instanceof SessionChangedError) throw new SessionChangedError(t(m('The workspace session changed. Retry after connecting.')));
  throw error;
}
export async function connectSession(value: string) {
  try { await session.connect(value); rememberToken(value.trim()); }
  catch (error) { localized(error); }
}

export async function api<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
  try { return await session.call<T>(method, params); }
  catch (error) { localized(error); }
}

async function send(token: string, method: string, params: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
  const response = await fetch('/api', {
    method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
    body: JSON.stringify({ method, params }), cache: 'no-store', credentials: 'omit', redirect: 'error', signal,
  });
  if (response.status === 401 || response.status === 403) throw new SessionAccessError();
  let body: { result?: unknown; error?: string };
  try { body = await response.json(); } catch { throw new Error(t(m('Server returned HTTP {status} without a JSON response.', { status: response.status }))); }
  if (!response.ok || body.error !== undefined) throw new Error(body.error || t(m('Server returned HTTP {status}.', { status: response.status })));
  if (!('result' in body)) throw new Error(t(m('Server returned an incomplete response.')));
  return body.result;
}
