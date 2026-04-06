#!/usr/bin/env python3
"""
Deep Specification Compliance Verification
Checks EVERY section of EVERY specification document
"""

import os
import re
import subprocess
from pathlib import Path
from typing import Dict, List, Set, Tuple
from dataclasses import dataclass
import json

@dataclass
class SpecSection:
    file: str
    section_num: str
    title: str
    level: int
    line_num: int

@dataclass
class ComplianceCheck:
    spec_file: str
    section: str
    title: str
    keywords: List[str]
    found_in: List[str]
    status: str
    confidence: str

class DeepComplianceVerifier:
    def __init__(self, root_dir: str):
        self.root = Path(root_dir)
        self.docs = self.root / "docs" / "detailed"
        self.crates = self.root / "crates"
        self.checks: List[ComplianceCheck] = []

    def extract_all_sections(self, spec_file: Path) -> List[SpecSection]:
        """Extract all section headers from a spec file"""
        sections = []
        try:
            with open(spec_file, 'r', encoding='utf-8') as f:
                for line_num, line in enumerate(f, 1):
                    match = re.match(r'^(#{2,6})\s+(.+)$', line)
                    if match:
                        hashes = match.group(1)
                        title = match.group(2).strip()
                        level = len(hashes) - 1  # ## = level 1, ### = level 2, etc.

                        # Extract section number if present
                        sec_match = re.match(r'^([\d.]+)\s+(.+)$', title)
                        if sec_match:
                            section_num = sec_match.group(1)
                            title = sec_match.group(2)
                        else:
                            section_num = ""

                        sections.append(SpecSection(
                            file=spec_file.name,
                            section_num=section_num,
                            title=title,
                            level=level,
                            line_num=line_num
                        ))
        except Exception as e:
            print(f"Error reading {spec_file}: {e}")

        return sections

    def extract_keywords_from_section(self, spec_file: Path, section: SpecSection) -> List[str]:
        """Extract key terms/keywords from a section for searching"""
        keywords = []

        # Extract from section title
        title_words = re.findall(r'\w+', section.title)
        keywords.extend([w for w in title_words if len(w) > 3])

        # Common patterns to look for
        patterns = [
            r'`([^`]+)`',  # Code snippets
            r'\b(type|fn|let|struct|enum|trait|impl)\s+(\w+)',  # Type definitions
        ]

        try:
            with open(spec_file, 'r', encoding='utf-8') as f:
                lines = f.readlines()

            # Read ~20 lines from the section
            start = section.line_num
            end = min(start + 20, len(lines))

            section_text = ''.join(lines[start:end])

            for pattern in patterns:
                matches = re.findall(pattern, section_text)
                keywords.extend([m if isinstance(m, str) else m[0] for m in matches])

        except Exception:
            pass

        return list(set(keywords))[:10]  # Limit to 10 most unique keywords

    def search_implementation(self, keywords: List[str], spec_ref: str) -> List[str]:
        """Search codebase for implementation of keywords"""
        found_files = set()

        for keyword in keywords:
            if len(keyword) < 3:
                continue

            try:
                # Search for keyword in Rust files
                result = subprocess.run(
                    ['grep', '-r', '-l', '--include=*.rs', keyword, str(self.crates)],
                    capture_output=True,
                    text=True,
                    timeout=5
                )

                if result.returncode == 0:
                    files = result.stdout.strip().split('\n')
                    found_files.update([f for f in files if f])

            except Exception:
                pass

        # Also search for spec references
        try:
            spec_name = Path(spec_ref).stem
            result = subprocess.run(
                ['grep', '-r', '-l', '--include=*.rs', f'Spec:.*{spec_name}', str(self.crates)],
                capture_output=True,
                text=True,
                timeout=5
            )

            if result.returncode == 0:
                files = result.stdout.strip().split('\n')
                found_files.update([f for f in files if f])

        except Exception:
            pass

        # Relative paths
        relative_files = []
        for f in found_files:
            try:
                rel = Path(f).relative_to(self.root)
                relative_files.append(str(rel))
            except Exception:
                pass

        return sorted(relative_files)[:5]  # Top 5 most relevant

    def determine_status(self, found_files: List[str], keywords: List[str]) -> Tuple[str, str]:
        """Determine implementation status and confidence"""
        if not found_files:
            return "NOT_STARTED", "HIGH"

        if len(found_files) >= 3:
            return "COMPLETE", "HIGH"
        elif len(found_files) >= 1:
            return "PARTIAL", "MEDIUM"
        else:
            return "MISSING", "LOW"

    def verify_spec(self, spec_file: Path) -> List[ComplianceCheck]:
        """Verify all sections of a specification"""
        print(f"  📄 {spec_file.name}")

        sections = self.extract_all_sections(spec_file)
        checks = []

        for section in sections:
            if section.level <= 3:  # Only check top 3 levels
                keywords = self.extract_keywords_from_section(spec_file, section)
                found_files = self.search_implementation(keywords, spec_file.name)
                status, confidence = self.determine_status(found_files, keywords)

                check = ComplianceCheck(
                    spec_file=spec_file.name,
                    section=section.section_num or f"L{section.level}",
                    title=section.title,
                    keywords=keywords[:5],
                    found_in=found_files,
                    status=status,
                    confidence=confidence
                )
                checks.append(check)

        return checks

    def verify_all_specs(self):
        """Verify all specification documents"""
        print("\n🔬 Deep Compliance Verification\n")
        print("=" * 80)

        # Priority specs
        priority_specs = [
            "02-core-semantics.md",
            "03-type-system.md",
            "04-memory-model.md",
            "05-syntax-grammar.md",
            "06-compilation-pipeline.md",
            "07-runtime-system.md",
            "08-standard-library.md",
            "09-verification-system.md",
            "10-concurrency-model.md",
            "16-context-system.md",
            "20-error-handling.md",
            "24-cbgr-implementation.md",
        ]

        for spec_name in priority_specs:
            spec_path = self.docs / spec_name
            if spec_path.exists():
                checks = self.verify_spec(spec_path)
                self.checks.extend(checks)

        # Check remaining specs
        for spec_path in sorted(self.docs.glob("*.md")):
            if spec_path.name not in priority_specs and spec_path.name not in ["README.md", "00-index.md"]:
                checks = self.verify_spec(spec_path)
                self.checks.extend(checks)

    def generate_report(self) -> Dict:
        """Generate comprehensive compliance report"""
        total = len(self.checks)
        complete = sum(1 for c in self.checks if c.status == "COMPLETE")
        partial = sum(1 for c in self.checks if c.status == "PARTIAL")
        missing = sum(1 for c in self.checks if c.status == "MISSING")
        not_started = sum(1 for c in self.checks if c.status == "NOT_STARTED")

        compliance_pct = (complete / total * 100) if total > 0 else 0

        print("\n" + "=" * 80)
        print("📊 Deep Compliance Summary\n")
        print(f"   Total Sections Analyzed:  {total}")
        print(f"   ✅ Complete:              {complete} ({complete/total*100:.1f}%)")
        print(f"   ⚠️  Partial:               {partial} ({partial/total*100:.1f}%)")
        print(f"   ❌ Missing:               {missing} ({missing/total*100:.1f}%)")
        print(f"   ⭕ Not Started:           {not_started} ({not_started/total*100:.1f}%)")
        print(f"\n   📈 Overall Compliance: {compliance_pct:.2f}%")
        print(f"   🎯 Target: 100.00%")
        print(f"   📉 Gap: {100 - compliance_pct:.2f}%")
        print("=" * 80)

        return {
            "total": total,
            "complete": complete,
            "partial": partial,
            "missing": missing,
            "not_started": not_started,
            "compliance_percentage": round(compliance_pct, 2),
            "checks": [
                {
                    "spec": c.spec_file,
                    "section": c.section,
                    "title": c.title,
                    "status": c.status,
                    "confidence": c.confidence,
                    "keywords": c.keywords,
                    "implementations": c.found_in
                }
                for c in self.checks
            ]
        }

    def print_gaps(self):
        """Print detailed gap analysis"""
        print("\n\n🔍 Gap Analysis - Items Needing Attention:\n")
        print("=" * 80)

        gaps = [c for c in self.checks if c.status in ["MISSING", "NOT_STARTED", "PARTIAL"]]

        by_spec = {}
        for gap in gaps:
            if gap.spec_file not in by_spec:
                by_spec[gap.spec_file] = []
            by_spec[gap.spec_file].append(gap)

        for spec_file in sorted(by_spec.keys()):
            items = by_spec[spec_file]
            print(f"\n📄 {spec_file} - {len(items)} gaps")
            print("-" * 80)

            for item in items[:10]:  # Show top 10 per spec
                emoji = {"PARTIAL": "⚠️", "MISSING": "❌", "NOT_STARTED": "⭕"}[item.status]
                print(f"  {emoji} §{item.section} {item.title}")
                print(f"     Status: {item.status} (confidence: {item.confidence})")
                if item.found_in:
                    print(f"     Found in: {', '.join(item.found_in[:2])}")
                else:
                    print(f"     Keywords: {', '.join(item.keywords[:3])}")

def main():
    root_dir = Path(__file__).parent.parent
    verifier = DeepComplianceVerifier(str(root_dir))

    verifier.verify_all_specs()
    report = verifier.generate_report()
    verifier.print_gaps()

    # Save detailed report
    output_file = root_dir / "deep_compliance_report.json"
    with open(output_file, 'w') as f:
        json.dump(report, f, indent=2)

    print(f"\n💾 Detailed report saved to: {output_file}")

    if report["compliance_percentage"] < 100:
        print(f"\n❌ COMPLIANCE: {report['compliance_percentage']:.2f}% (target: 100%)")
        return 1
    else:
        print(f"\n✅ COMPLIANCE: 100%")
        return 0

if __name__ == "__main__":
    exit(main())
