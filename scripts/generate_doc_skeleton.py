#!/usr/bin/env python3
"""
Documentation Skeleton Generator for Verum Platform

Parses Rust source files and generates documentation templates
for undocumented public items.
"""

import re
import sys
from pathlib import Path
from typing import List, Dict, Optional, Tuple
from dataclasses import dataclass
from enum import Enum


class ItemKind(Enum):
    STRUCT = "struct"
    ENUM = "enum"
    TRAIT = "trait"
    FUNCTION = "fn"
    TYPE = "type"
    CONST = "const"
    STATIC = "static"
    MOD = "mod"
    IMPL = "impl"


@dataclass
class RustItem:
    """Represents a Rust item that needs documentation."""
    kind: ItemKind
    name: str
    signature: str
    is_unsafe: bool
    is_async: bool
    returns_result: bool
    file_path: Path
    line_number: int
    existing_doc: Optional[str]
    params: List[Tuple[str, str]]  # (name, type)
    generics: List[str]


class DocTemplate:
    """Generates documentation templates for different item types."""

    @staticmethod
    def generate_summary(item: RustItem) -> str:
        """Generate a one-line summary for the item."""
        summaries = {
            ItemKind.STRUCT: f"Represents a {item.name.lower().replace('_', ' ')}.",
            ItemKind.ENUM: f"Enumerates the different types of {item.name.lower().replace('_', ' ')}.",
            ItemKind.TRAIT: f"Defines behavior for {item.name.lower().replace('_', ' ')}.",
            ItemKind.FUNCTION: f"Performs {item.name.lower().replace('_', ' ')} operation.",
            ItemKind.TYPE: f"Type alias for {item.name.lower().replace('_', ' ')}.",
            ItemKind.CONST: f"Constant value for {item.name.lower().replace('_', ' ')}.",
        }
        return summaries.get(item.kind, f"{item.kind.value.capitalize()} {item.name}.")

    @staticmethod
    def generate_description(item: RustItem) -> str:
        """Generate a detailed description template."""
        if item.kind == ItemKind.STRUCT:
            return """
/// This structure provides:
/// - TODO: List key features
/// - TODO: Explain main use cases
/// - TODO: Describe important invariants
///
/// # Memory Layout
///
/// TODO: Describe memory layout if relevant
///
/// # Thread Safety
///
/// TODO: Document thread safety guarantees"""

        elif item.kind == ItemKind.FUNCTION:
            return """
/// This function:
/// - TODO: Explain what it does
/// - TODO: When to use it
/// - TODO: How it works (algorithm if complex)"""

        elif item.kind == ItemKind.TRAIT:
            return """
/// Implementors of this trait must:
/// - TODO: List requirements
/// - TODO: Describe guarantees
///
/// # Implementation Notes
///
/// TODO: Guidance for implementors"""

        elif item.kind == ItemKind.ENUM:
            return """
/// This enumeration covers:
/// - TODO: List variants and their meanings
/// - TODO: Explain when each variant is used"""

        return "\n/// TODO: Add detailed description"

    @staticmethod
    def generate_examples(item: RustItem) -> str:
        """Generate example code templates."""
        if item.kind == ItemKind.STRUCT:
            return f"""
/// # Examples
///
/// Basic usage:
/// ```
/// use verum_{{crate}}::{item.name};
///
/// let instance = {item.name}::new();
/// // TODO: Add usage example
/// ```
///
/// Advanced usage:
/// ```
/// use verum_{{crate}}::{item.name};
///
/// // TODO: Add advanced example
/// ```"""

        elif item.kind == ItemKind.FUNCTION:
            params_str = ", ".join([f"/* {name}: {typ} */" for name, typ in item.params])
            return f"""
/// # Examples
///
/// ```
/// use verum_{{crate}}::{item.name};
///
/// let result = {item.name}({params_str});
/// // TODO: Add assertions/checks
/// ```"""

        elif item.kind == ItemKind.TRAIT:
            return f"""
/// # Examples
///
/// Implementing this trait:
/// ```
/// use verum_{{crate}}::{item.name};
///
/// struct MyType;
///
/// impl {item.name} for MyType {{
///     // TODO: Add implementation
/// }}
/// ```"""

        return ""

    @staticmethod
    def generate_safety_section(item: RustItem) -> str:
        """Generate safety documentation for unsafe items."""
        if not item.is_unsafe:
            return ""

        return """
/// # Safety
///
/// This function is unsafe because:
/// - TODO: Explain what could go wrong
///
/// The caller must ensure:
/// - TODO: List preconditions
/// - TODO: List invariants that must hold
/// - TODO: List memory safety requirements"""

    @staticmethod
    def generate_panics_section(item: RustItem) -> str:
        """Generate panics section."""
        if item.kind != ItemKind.FUNCTION:
            return ""

        return """
/// # Panics
///
/// Panics if:
/// - TODO: List panic conditions (or remove if doesn't panic)"""

    @staticmethod
    def generate_errors_section(item: RustItem) -> str:
        """Generate errors section for Result-returning functions."""
        if not item.returns_result:
            return ""

        return """
/// # Errors
///
/// Returns `Err` if:
/// - TODO: List error conditions
/// - TODO: Document error types"""

    @staticmethod
    def generate_performance_section(item: RustItem) -> str:
        """Generate performance notes for critical items."""
        if item.kind not in [ItemKind.STRUCT, ItemKind.FUNCTION]:
            return ""

        # Only add performance section for certain patterns
        perf_keywords = ["alloc", "deref", "check", "verify", "infer", "compile"]
        if any(kw in item.name.lower() for kw in perf_keywords):
            return """
/// # Performance
///
/// - Time complexity: TODO
/// - Space complexity: TODO
/// - Typical overhead: TODO"""

        return ""

    @staticmethod
    def generate_full_doc(item: RustItem) -> str:
        """Generate complete documentation for an item."""
        parts = [
            f"/// {DocTemplate.generate_summary(item)}",
            "///",
            DocTemplate.generate_description(item),
        ]

        if examples := DocTemplate.generate_examples(item):
            parts.append(examples)

        if panics := DocTemplate.generate_panics_section(item):
            parts.append(panics)

        if errors := DocTemplate.generate_errors_section(item):
            parts.append(errors)

        if safety := DocTemplate.generate_safety_section(item):
            parts.append(safety)

        if perf := DocTemplate.generate_performance_section(item):
            parts.append(perf)

        # See also section
        parts.append("""
/// # See Also
///
/// - TODO: Link to related items""")

        return "\n".join(parts)


class RustParser:
    """Parses Rust source files to find undocumented public items."""

    # Regex patterns for different item types
    PATTERNS = {
        ItemKind.STRUCT: re.compile(
            r'^(\s*)((?:///.*\n)*)\s*pub\s+(struct)\s+(\w+)(<[^>]+>)?\s*(\{|;|\()',
            re.MULTILINE
        ),
        ItemKind.ENUM: re.compile(
            r'^(\s*)((?:///.*\n)*)\s*pub\s+(enum)\s+(\w+)(<[^>]+>)?\s*\{',
            re.MULTILINE
        ),
        ItemKind.TRAIT: re.compile(
            r'^(\s*)((?:///.*\n)*)\s*pub\s+(trait)\s+(\w+)(<[^>]+>)?\s*\{',
            re.MULTILINE
        ),
        ItemKind.FUNCTION: re.compile(
            r'^(\s*)((?:///.*\n)*)\s*pub\s+((?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn)\s+(\w+)(<[^>]+>)?\s*\(',
            re.MULTILINE
        ),
        ItemKind.TYPE: re.compile(
            r'^(\s*)((?:///.*\n)*)\s*pub\s+type\s+(\w+)(<[^>]+>)?\s*=',
            re.MULTILINE
        ),
        ItemKind.CONST: re.compile(
            r'^(\s*)((?:///.*\n)*)\s*pub\s+const\s+(\w+)\s*:',
            re.MULTILINE
        ),
    }

    @staticmethod
    def parse_file(file_path: Path) -> List[RustItem]:
        """Parse a Rust file and extract all public items."""
        try:
            content = file_path.read_text()
        except Exception as e:
            print(f"Error reading {file_path}: {e}", file=sys.stderr)
            return []

        items = []

        for kind, pattern in RustParser.PATTERNS.items():
            for match in pattern.finditer(content):
                indent = match.group(1)
                existing_doc = match.group(2).strip() if len(match.groups()) > 2 else ""

                # Skip if already documented
                if existing_doc and "///" in existing_doc:
                    continue

                # Extract item name (position varies by pattern)
                if kind in [ItemKind.STRUCT, ItemKind.ENUM, ItemKind.TRAIT]:
                    name = match.group(4)
                    signature = match.group(0).strip()
                    generics = match.group(5) or ""
                elif kind == ItemKind.FUNCTION:
                    name = match.group(4)
                    signature = match.group(0).strip()
                    generics = match.group(5) or ""
                else:
                    name = match.group(3)
                    signature = match.group(0).strip()
                    generics = ""

                # Calculate line number
                line_number = content[:match.start()].count('\n') + 1

                # Determine properties
                is_unsafe = "unsafe" in signature
                is_async = "async" in signature
                returns_result = "Result<" in signature

                # Try to extract parameters for functions
                params = []
                if kind == ItemKind.FUNCTION:
                    params = RustParser.extract_params(signature)

                item = RustItem(
                    kind=kind,
                    name=name,
                    signature=signature,
                    is_unsafe=is_unsafe,
                    is_async=is_async,
                    returns_result=returns_result,
                    file_path=file_path,
                    line_number=line_number,
                    existing_doc=existing_doc if existing_doc else None,
                    params=params,
                    generics=[g.strip() for g in generics.strip("<>").split(",") if g.strip()]
                )

                items.append(item)

        return items

    @staticmethod
    def extract_params(signature: str) -> List[Tuple[str, str]]:
        """Extract parameter names and types from function signature."""
        # Simplified parameter extraction
        params = []
        param_match = re.search(r'\((.*?)\)', signature)
        if param_match:
            param_str = param_match.group(1)
            for param in param_str.split(','):
                param = param.strip()
                if ':' in param:
                    parts = param.split(':', 1)
                    name = parts[0].strip()
                    typ = parts[1].strip()
                    # Skip 'self' parameters
                    if name not in ['self', '&self', '&mut self']:
                        params.append((name, typ))
        return params


def find_rust_files(crate_path: Path) -> List[Path]:
    """Find all Rust source files in a crate."""
    rust_files = []

    # Check src directory
    src_dir = crate_path / "src"
    if src_dir.exists():
        rust_files.extend(src_dir.rglob("*.rs"))

    return rust_files


def generate_skeleton_for_crate(crate_name: str, crate_path: Path, output_dir: Path):
    """Generate documentation skeletons for all undocumented items in a crate."""
    print(f"\n{'='*70}")
    print(f"Processing crate: {crate_name}")
    print(f"{'='*70}")

    rust_files = find_rust_files(crate_path)

    if not rust_files:
        print(f"  No Rust files found in {crate_path}")
        return

    all_items = []
    for rust_file in rust_files:
        items = RustParser.parse_file(rust_file)
        all_items.extend(items)

    if not all_items:
        print(f"  ✓ All public items are documented!")
        return

    print(f"  Found {len(all_items)} undocumented public items")

    # Group items by file
    items_by_file: Dict[Path, List[RustItem]] = {}
    for item in all_items:
        if item.file_path not in items_by_file:
            items_by_file[item.file_path] = []
        items_by_file[item.file_path].append(item)

    # Generate documentation for each file
    output_crate_dir = output_dir / crate_name
    output_crate_dir.mkdir(parents=True, exist_ok=True)

    for file_path, items in items_by_file.items():
        relative_path = file_path.relative_to(crate_path)
        output_file = output_crate_dir / f"{relative_path.stem}_docs.md"
        output_file.parent.mkdir(parents=True, exist_ok=True)

        with output_file.open('w') as f:
            f.write(f"# Documentation for {relative_path}\n\n")
            f.write(f"File: `{file_path}`\n\n")
            f.write(f"Found {len(items)} undocumented items\n\n")
            f.write("---\n\n")

            for item in sorted(items, key=lambda x: x.line_number):
                f.write(f"## {item.kind.value} `{item.name}` (line {item.line_number})\n\n")
                f.write("### Generated Documentation Template:\n\n")
                f.write("```rust\n")
                f.write(DocTemplate.generate_full_doc(item).replace("{crate}", crate_name))
                f.write(f"\n{item.signature}\n")
                f.write("```\n\n")
                f.write("---\n\n")

        print(f"    Generated: {output_file}")


def main():
    """Main entry point."""
    if len(sys.argv) < 2:
        print("Usage: generate_doc_skeleton.py <project_root>")
        sys.exit(1)

    project_root = Path(sys.argv[1])
    crates_dir = project_root / "crates"
    output_dir = project_root / "target" / "doc_skeletons"

    if not crates_dir.exists():
        print(f"Error: Crates directory not found: {crates_dir}")
        sys.exit(1)

    output_dir.mkdir(parents=True, exist_ok=True)

    # Core crates to process (in priority order)
    core_crates = [
        "verum_cbgr",
        "verum_types",
        "verum_std",
        "verum_runtime",
        "verum_compiler",
        "verum_codegen",
        "verum_context",
        "verum_verification",
        "verum_smt",
    ]

    print("=" * 70)
    print("Verum Platform - Documentation Skeleton Generator")
    print("=" * 70)

    for crate_name in core_crates:
        crate_path = crates_dir / crate_name
        if crate_path.exists():
            generate_skeleton_for_crate(crate_name, crate_path, output_dir)
        else:
            print(f"\n  ⚠  Crate not found: {crate_name}")

    print(f"\n{'='*70}")
    print(f"Documentation skeletons generated in: {output_dir}")
    print(f"{'='*70}\n")


if __name__ == "__main__":
    main()
