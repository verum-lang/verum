export type Receipt = {
  id: string; kind: 'run' | 'check' | 'proof'; path: string; source_hash: string;
  status: 'running' | 'succeeded' | 'failed' | 'cancelled' | 'exhausted' | 'inconclusive';
  stdout: string; stderr: string; exit_code: number | null; elapsed_ms: number; proof?: unknown;
  timeout_ms?: number; evidence_path?: string; version?: number;
  started_unix_ms?: number; termination?: string;
};

export const STARTER = 'let answer = 6 * 7;\nprint(answer);\n';
export const IDENTITY = 'theorem identity(A: Type, x: A): A proof = x;\n';

export type DocumentRevision = { path: string; editVersion: number };
export function sameDocumentRevision(left: DocumentRevision, right: DocumentRevision): boolean {
  return left.path === right.path && left.editVersion === right.editVersion;
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

export function approvalKind(method: string, params: Record<string, unknown>): 'source' | 'execution' | 'read' | 'unsupported' {
  if (method === 'item/tool/call') {
    if (params.tool === 'verum_propose' || params.tool === 'propose_source') return 'source';
    if (params.tool === 'verum_read' || params.tool === 'verum_analyze' || params.tool === 'verum_project') return 'read';
    if (['verum_run', 'verum_check', 'verum_proof'].includes(String(params.tool))) return 'execution';
    return 'unsupported';
  }
  // This workbench permits Verum tools, never built-in shell or filesystem bypasses.
  return 'unsupported';
}

export function approvalReply(method: string, approved: boolean): Record<string, unknown> {
  return method === 'item/tool/call' ? { approved } : { decision: approved ? 'accept' : 'decline' };
}

export function parseArguments(value: unknown): Record<string, unknown> {
  if (typeof value === 'string') { try { return parseArguments(JSON.parse(value)); } catch { return {}; } }
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
}
