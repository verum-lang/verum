import { sourceHash } from './source.ts';

export type ConversationAddress = { connectionId: string; threadId: string };
export type HandoffRequest = { from: ConversationAddress; to: ConversationAddress; seq: number };
export type Handoff = HandoffRequest & { id: string; turnId: string; itemId: string; text: string; sha256: string };

export function sameAddress(left: ConversationAddress, right: ConversationAddress): boolean {
  return left.connectionId === right.connectionId && left.threadId === right.threadId;
}

function isAddress(value: unknown): value is ConversationAddress {
  if (typeof value !== 'object' || value === null) return false;
  const address = value as Partial<ConversationAddress>;
  return typeof address.connectionId === 'string' && !!address.connectionId &&
    typeof address.threadId === 'string' && !!address.threadId;
}

export async function verifiedHandoff(value: unknown, expected?: HandoffRequest): Promise<Handoff | null> {
  if (typeof value !== 'object' || value === null) return null;
  const message = value as Partial<Handoff>;
  if (!isAddress(message.from) || !isAddress(message.to) || sameAddress(message.from, message.to) ||
      typeof message.seq !== 'number' || !Number.isSafeInteger(message.seq) || message.seq < 1 ||
      typeof message.id !== 'string' || !message.id || typeof message.itemId !== 'string' || !message.itemId ||
      typeof message.turnId !== 'string' || !message.turnId || typeof message.text !== 'string' ||
      typeof message.sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(message.sha256)) return null;
  if (expected && (expected.seq !== message.seq || !sameAddress(expected.from, message.from) ||
      !sameAddress(expected.to, message.to))) return null;
  return await sourceHash(message.text) === message.sha256 ? message as Handoff : null;
}

/** Keep every source line in the quote, including empty lines and literal markup. */
export function quotedHandoff(message: Handoff, heading: string): string {
  return heading + '\n' + JSON.stringify(message.from) + '\nSHA-256: ' + message.sha256 + '\n> ' + message.text.replace(/(\r\n|\r|\n)/g, '$1> ');
}
