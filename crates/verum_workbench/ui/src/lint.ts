import type { Position } from './language.ts';

export type LintFinding = { rule: string; severity: number; message: string; position: Position | null; edits: unknown[] };
export type LintFile = { path: string; source_hash: string; parsed: boolean; findings: LintFinding[] };
export type LintReport = { schema_version: number; files: LintFile[] };

/** Unparsed files stay visible even when text rules produced no findings. */
export function lintPage(report: LintReport, query: string, limit: number) {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const groups: { file: LintFile; findings: (LintFinding | null)[] }[] = [];
  let matched = 0, total = 0, parsed = 0;
  for (const file of report.files) {
    total += file.findings.length;
    if (file.parsed) parsed++;
    const findings: (LintFinding | null)[] = [];
    const needsParseNotice = !file.parsed && !file.findings.some(finding => finding.rule === 'parse-error');
    for (const finding of needsParseNotice ? [null, ...file.findings] : file.findings) {
      const match = [file.path, finding?.rule, finding?.message].join(' ').toLowerCase();
      if (!terms.every(term => match.includes(term))) continue;
      if (++matched <= limit) findings.push(finding);
    }
    if (findings.length) groups.push({ file, findings });
  }
  return { groups, total, matched, parsed };
}
