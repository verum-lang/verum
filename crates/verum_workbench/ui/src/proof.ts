import { parseArguments, type Receipt } from './model.ts';
import { sourceByteRange, sourceHash } from './source.ts';

/** The host validates the report; navigation also checks the exact visible text.
 * Callers must recheck the document revision after this asynchronous operation. */
export async function proofSourceRange(receipt: Receipt, span: unknown, path: string, hash: string | null, source: string) {
  const report = parseArguments(receipt.proof), binding = parseArguments(report.source), location = parseArguments(span);
  const outcomes: Record<string, Receipt['status']> = { verified: 'succeeded', rejected: 'failed', unsupported: 'unsupported' };
  if (receipt.kind !== 'proof' || typeof report.status !== 'string' || !Object.hasOwn(outcomes, report.status)
    || receipt.status !== outcomes[String(report.status)]
    || path !== receipt.path || hash !== receipt.source_hash || binding.sha256 !== hash
    || !receipt.declaration || binding.declaration !== receipt.declaration
    || !receipt.proof_profile || !['verum-source-pi-id/1', 'verum-source-finite/1'].includes(receipt.proof_profile) || report.profile !== receipt.proof_profile
    || typeof location.sha256 !== 'string') return null;
  const decoded = sourceByteRange(source, location.start, location.end);
  if (!decoded) return null;
  const [actualSource, actualSpan] = await Promise.all([sourceHash(source), sourceHash(decoded.text)]);
  return actualSource === hash && actualSpan === location.sha256 ? decoded.range : null;
}
