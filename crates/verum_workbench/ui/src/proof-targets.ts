import { sameRevision, type Analysis, type Declaration, type DocumentRevision } from './language.ts';

export type ProofProfile = 'verum-source-pi-id/1' | 'verum-source-finite/1';
export type ProofSelection = { profile: ProofProfile; declaration: string };

/** Discovery is syntactic, not proof admission. The source worker checks the
 * selected profile, complete declaration and actual proof independently. */
export function declarationProfile(declaration: Declaration): ProofProfile | null {
  if (['theorem', 'lemma', 'corollary'].includes(declaration.kind)) return 'verum-source-pi-id/1';
  if (declaration.kind === 'function' && declaration.postconditions?.length) return 'verum-source-finite/1';
  return null;
}

export class ProofTargets {
  private choices = new Map<string, ProofSelection>();
  private revision: DocumentRevision | null = null;
  private declarations: Declaration[] | null = null;

  begin(revision: DocumentRevision) {
    this.revision = { ...revision };
    this.declarations = null;
  }

  selection(path: string): ProofSelection {
    return { ...(this.choices.get(path) ?? { profile: 'verum-source-pi-id/1', declaration: '' }) };
  }

  accept(revision: DocumentRevision, analysis: Analysis): boolean {
    if (!this.revision || !sameRevision(revision, this.revision)) return false;
    this.declarations = analysis.parsed ? analysis.declarations : [];
    this.select(revision, this.selection(revision.path));
    return true;
  }

  options(revision: DocumentRevision, profile: ProofProfile): Declaration[] | null {
    if (!this.revision || !sameRevision(revision, this.revision) || !this.declarations) return null;
    const counts = new Map<string, number>();
    for (const item of this.declarations) counts.set(item.name, (counts.get(item.name) ?? 0) + 1);
    return this.declarations.filter(item => declarationProfile(item) === profile && counts.get(item.name) === 1);
  }

  select(revision: DocumentRevision, selection: ProofSelection) {
    const options = this.options(revision, selection.profile);
    // A first, unambiguous choice is convenient. A removed or renamed explicit
    // target must never be silently replaced by a different declaration.
    const declaration = !selection.declaration && options?.length === 1 ? options[0].name : selection.declaration;
    this.choices.set(revision.path, { ...selection, declaration });
  }

  request(revision: DocumentRevision): ProofSelection | null {
    const selection = this.selection(revision.path);
    return this.options(revision, selection.profile)?.some(item => item.name === selection.declaration) ? selection : null;
  }

  clear() { this.choices.clear(); this.revision = null; this.declarations = null; }
}
