#!/usr/bin/env bun
/**
 * Verum Specification Merger
 *
 * This script merges all specification files from docs/detailed/ into a single
 * comprehensive specification document at docs/full-spec.md.
 *
 * Usage:
 *   bun run scripts/merge_spec.ts
 */

import { readdir, readFile, writeFile } from 'fs/promises';
import { join, resolve } from 'path';

// Configuration
const DOCS_DIR = resolve(import.meta.dir, '../docs/detailed');
const OUTPUT_FILE = resolve(import.meta.dir, '../docs/full-spec.md');

// Specification file ordering (by number prefix)
const SPEC_FILES = [
  '01-philosophy.md',
  '02-core-semantics.md',
  '03-type-system.md',
  '04-memory-model.md',
  '05-syntax-grammar.md',
  '06-compilation-pipeline.md',
  '07-runtime-system.md',
  '08-standard-library.md',
  '09-verification-system.md',
  '10-concurrency-model.md',
  '11-gpu-computing.md',
  '12-dependent-types.md',
  '13-formal-proofs.md',
  '14-module-system.md',
  '15-package-management.md',
  '15-package-distribution-architecture.md',
  '16-context-system.md',
  '17-meta-system.md',
  '18-advanced-protocols.md',
  '19-optimization-hints.md',
  '20-error-handling.md',
  '21-interop.md',
  '22-simd.md',
  '23-memory-safety-models.md',
  '24-cbgr-implementation.md',
  '25-developer-tooling.md',
  '26-unified-execution-architecture.md',
  '27-examples.md',
  '28-implementation-roadmap.md',
  '29-module-distribution.md'
];

interface Section {
  filename: string;
  title: string;
  content: string;
}

/**
 * Extract title from markdown content (first # header)
 */
function extractTitle(content: string): string {
  const match = content.match(/^#\s+(.+)$/m);
  return match ? match[1].trim() : 'Untitled';
}

/**
 * Process markdown content to adjust heading levels
 * First-level headers become second-level, etc.
 */
function adjustHeadingLevels(content: string): string {
  const lines = content.split('\n');
  const adjusted = lines.map(line => {
    // Skip the first header (it will be the section title)
    if (line.match(/^#\s+/)) {
      return line.replace(/^#/, '##');
    } else if (line.match(/^##/)) {
      return line.replace(/^##/, '###');
    } else if (line.match(/^###/)) {
      return line.replace(/^###/, '####');
    } else if (line.match(/^####/)) {
      return line.replace(/^####/, '#####');
    } else if (line.match(/^#####/)) {
      return line.replace(/^#####/, '######');
    }
    return line;
  });

  return adjusted.join('\n');
}

/**
 * Remove the first header from content (will be replaced with section header)
 */
function removeFirstHeader(content: string): string {
  const lines = content.split('\n');
  let headerFound = false;
  const filtered = lines.filter(line => {
    if (!headerFound && line.match(/^#\s+/)) {
      headerFound = true;
      return false;
    }
    return true;
  });

  return filtered.join('\n').trim();
}

/**
 * Read and process a specification file
 */
async function readSpecFile(filename: string): Promise<Section> {
  const filepath = join(DOCS_DIR, filename);
  const content = await readFile(filepath, 'utf-8');
  const title = extractTitle(content);

  // Process content: remove first header and adjust heading levels
  let processedContent = removeFirstHeader(content);
  processedContent = adjustHeadingLevels(processedContent);

  return {
    filename,
    title,
    content: processedContent.trim()
  };
}

/**
 * Generate table of contents
 */
function generateTOC(sections: Section[]): string {
  const lines = ['## Table of Contents\n'];

  sections.forEach((section, index) => {
    const num = index + 1;
    const anchor = section.title
      .toLowerCase()
      .replace(/[^\w\s-]/g, '')
      .replace(/\s+/g, '-');
    lines.push(`${num}. [${section.title}](#${num}-${anchor})`);
  });

  return lines.join('\n');
}

/**
 * Generate header for the merged document
 */
function generateHeader(): string {
  const now = new Date();
  const dateStr = now.toISOString().split('T')[0];

  return `# Verum Language Specification
**Complete Specification Document**

*Generated: ${dateStr}*

---

This document is a comprehensive compilation of the entire Verum language specification,
combining all detailed specification files into a single reference document.

For the modular specification structure, see \`docs/detailed/00-index.md\`.

---
`;
}

/**
 * Main merge function
 */
async function mergeSpecification(): Promise<void> {
  console.log('🔍 Reading specification files...');

  // Read all specification files in order
  const sections: Section[] = [];

  for (const filename of SPEC_FILES) {
    try {
      console.log(`  📄 Processing ${filename}...`);
      const section = await readSpecFile(filename);
      sections.push(section);
    } catch (error) {
      console.error(`  ⚠️  Warning: Could not read ${filename}:`, error);
    }
  }

  console.log(`\n✅ Read ${sections.length} specification files\n`);

  // Generate merged document
  console.log('📝 Generating merged specification...');

  const parts = [
    generateHeader(),
    generateTOC(sections),
    '\n---\n'
  ];

  // Add each section
  sections.forEach((section, index) => {
    const num = index + 1;
    parts.push(`\n# ${num}. ${section.title}\n`);
    parts.push(section.content);
    parts.push('\n\n---\n');
  });

  // Add footer
  parts.push(`\n---\n\n*End of Verum Language Specification*\n`);

  const mergedContent = parts.join('\n');

  // Write output file
  console.log(`💾 Writing to ${OUTPUT_FILE}...`);
  await writeFile(OUTPUT_FILE, mergedContent, 'utf-8');

  // Statistics
  const totalLines = mergedContent.split('\n').length;
  const totalChars = mergedContent.length;
  const totalWords = mergedContent.split(/\s+/).length;

  console.log('\n✨ Specification merge complete!\n');
  console.log(`📊 Statistics:`);
  console.log(`   - Sections:    ${sections.length}`);
  console.log(`   - Total lines: ${totalLines.toLocaleString()}`);
  console.log(`   - Total words: ${totalWords.toLocaleString()}`);
  console.log(`   - Total chars: ${totalChars.toLocaleString()}`);
  console.log(`   - Output file: ${OUTPUT_FILE}\n`);
}

// Run the merge
try {
  await mergeSpecification();
} catch (error) {
  console.error('❌ Error merging specification:', error);
  process.exit(1);
}
