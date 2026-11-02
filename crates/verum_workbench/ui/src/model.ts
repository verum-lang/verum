import type { RuntimeReport } from './runtime';
export type RunPermissions = 'source' | 'deny-all' | 'allow-all';
export type ProofProfile = 'verum-source-pi-id/1' | 'verum-source-finite/1';
export type ExecutionPlan = {
  kind: 'run' | 'check' | 'proof'; path: string; source_hash: string;
  executable: string; timeout_ms: number; permissions: RunPermissions | null; runtime_diagnostics?: boolean | null; declaration: string | null;
  proof_profile?: ProofProfile | null;
};
export type Receipt = {
  id: string; kind: 'run' | 'check' | 'proof'; path: string; source_hash: string;
  status: 'running' | 'succeeded' | 'failed' | 'cancelled' | 'exhausted' | 'inconclusive' | 'unsupported';
  stdout: string; stderr: string; exit_code: number | null; elapsed_ms: number; proof?: unknown;
  timeout_ms?: number; evidence_path?: string; version?: number;
  started_unix_ms?: number; termination?: string;
  runtime_diagnostics?: boolean | null; runtime?: RuntimeReport | null; runtime_error?: string | null;
  permissions?: RunPermissions | null; declaration?: string | null;
  proof_profile?: ProofProfile | null;
};

export const STARTER = '#!/usr/bin/env verum\nlet answer = 6 * 7;\nprint(answer);\n';
export const IDENTITY = 'theorem identity(A: Type, x: A): A proof = x;\n';

export type DocumentRevision = { path: string; editVersion: number };
export function sameDocumentRevision(left: DocumentRevision, right: DocumentRevision): boolean {
  return left.path === right.path && left.editVersion === right.editVersion;
}

// Consent and fetched content belong to the editor revision that requested them.
// Keep typing enabled, but never replace a draft changed across an async boundary.
export async function replaceDocumentRevision<T>(revision: DocumentRevision,
  current: () => DocumentRevision, prepare: () => Promise<T | null>, apply: (value: T) => void,
): Promise<'applied' | 'cancelled' | 'stale'> {
  const expected = { ...revision };
  if (!sameDocumentRevision(expected, current())) return 'stale';
  const value = await prepare();
  if (value === null) return 'cancelled';
  if (!sameDocumentRevision(expected, current())) return 'stale';
  apply(value);
  return 'applied';
}

export function isStale(receipt: Receipt, path: string, hash: string | null): boolean {
  return receipt.path !== path || hash === null || receipt.source_hash !== hash;
}

export function mergeReceipts(existing: Receipt[], changes: Receipt[]): Receipt[] {
  const records = new Map(existing.map(receipt => [receipt.id, receipt]));
  for (const receipt of changes) {
    const previous = records.get(receipt.id);
    if (previous?.version !== undefined && receipt.version !== undefined && previous.version > receipt.version) continue;
    records.set(receipt.id, receipt);
  }
  return [...records.values()].sort((left, right) => (left.started_unix_ms ?? 0) - (right.started_unix_ms ?? 0)).slice(-128);
}

export function knownReceiptVersions(receipts: Receipt[]): Record<string, number> | null {
  if (!receipts.length || receipts.some(receipt => receipt.version === undefined)) return null;
  return Object.fromEntries(receipts.map(receipt => [receipt.id, receipt.version!]));
}

export function safeAuthUrl(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  try { const url = new URL(value); return url.protocol === 'https:' && !url.username && !url.password ? url.href : null; }
  catch { return null; }
}

export function relativeVerumPath(value: string): string | null {
  const path = value.trim();
  if (!path.endsWith('.vr') || path.startsWith('/') || path.includes('\\') || /[\u0000-\u001f]/.test(path)) return null;
  if (path.split('/').some(part => !part || part.startsWith('.'))) return null;
  return path;
}

export type ReviewKind = 'source' | 'execution' | 'read' | 'unsupported';
export function approvalKind(method: string, hostKind: unknown): ReviewKind {
  if (method === 'item/tool/call' && (hostKind === 'source' || hostKind === 'read' || hostKind === 'execution')) return hostKind;
  return 'unsupported';
}

export function approvalReply(method: string, approved: boolean): Record<string, unknown> {
  return method === 'item/tool/call' ? { approved } : { decision: approved ? 'accept' : 'decline' };
}

export function parseArguments(value: unknown): Record<string, unknown> {
  if (typeof value === 'string') { try { return parseArguments(JSON.parse(value)); } catch { return {}; } }
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
}
