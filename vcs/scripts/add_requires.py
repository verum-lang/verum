#!/usr/bin/env python3
"""
Add @requires directives to test files based on content patterns.
Also converts @skip directives to @requires where appropriate.
"""

import os
import re
from pathlib import Path

# Map skip reasons to required features
SKIP_REASON_TO_FEATURE = {
    'Type inference bug': 'type-inference-fix',
    'threading': 'threading',
    'Affine type checking': 'affine-checking',
    'Unsafe block checking': 'unsafe-checking',
    'turbofish syntax': 'turbofish',
    'Heap.new': 'heap',
    'CBGR tier': 'cbgr-runtime',
    'Meta-programming': 'meta-programming',
    'User-defined macro': 'meta-programming',
    'benchmark': 'benchmarks',
    'Stack overflow': 'stack-checking',
    'pattern matching not fully': 'pattern-matching-advanced',
    'feature not fully': 'partial-impl',
    'Weak type': 'weak-refs',
    'ThreadPool': 'threading',
    'channel': 'async-runtime',
    'recursive types': 'advanced-types',
    'reference tier': 'cbgr-runtime',
    'closure capture': 'closure-capture',
}

FEATURE_PATTERNS = {
    'gpu': [
        r'@device', r'kernel', r'GPU', r'CUDA', r'warp', r'cuda',
        r'Tensor(?:Int|Float|<)', r'shared_memory', r'cooperative_group',
        r'atomic_', r'synchronization', r'__global__', r'blockIdx',
        r'threadIdx', r'gridDim', r'blockDim',
    ],
    'ffi': [
        r'@extern', r'extern\s+"C"', r'FFI', r'libc', r'posix', r'windows',
        r'calling_convention', r'wasm_import', r'ffi_boundary',
    ],
    'verification': [
        r'theorem', r'@proof', r'lemma', r'@axiom', r'tactic',
        r'@smt', r'@verify', r'proof_search', r'induction_tactic',
        r'rewrite_tactic', r'simp_tactic', r'auto_tactic', r'proof_by',
        r'coinductive', r'inductive_types', r'proof_irrelevance',
        r'forall\s+\w+\s*:', r'exists\s+\w+\s*:',  # quantifiers
        r'refl\b', r'symmetry\b', r'transitivity\b',  # proof terms
    ],
    'advanced-types': [
        r'universe_polymorphism', r'type_level_computation',
        r'dependent_return', r'higher_order_dependent', r'pi_type',
        r'type_introspection', r'quantitative_types', r'HKT',
        r'type Apply<F<_>', r'type Compose<F<_>', r'type If<',
        r'Sigma<', r'Fin<', r'Vec<.*,\s*\d+>',  # indexed types
    ],
    'heap': [
        r'Heap\.new\(', r'Heap::new\(', r'drop\(', r'Heap<',
    ],
    'cbgr-runtime': [
        r'CBGR', r'generation_mismatch', r'epoch_violation',
        r'use.after.free', r'double.free', r'dangling',
    ],
    'async-runtime': [
        r'async\s+fn', r'await\b', r'spawn', r'channel',
        r'mutex', r'semaphore', r'async_', r'Future<',
    ],
    'autodiff': [
        r'@differentiable', r'gradient', r'backward', r'forward_diff',
        r'autograd', r'training_loop', r'backward_pass',
    ],
    'simd': [
        r'@simd', r'SIMD', r'neon', r'avx', r'sse', r'horizontal_ops',
        r'platform_intrinsics', r'simd_intrinsics',
    ],
    'incremental-parser': [
        r'green_tree', r'trivia_ownership', r'event_based',
        r'incremental', r'syntax_node', r'red_node', r'GreenNode',
        r'compile_time_parsing', r'ErrorNode',
    ],
    'overflowing-math': [
        r'overflowing_add', r'overflowing_sub', r'overflowing_mul',
        r'checked_add', r'checked_sub', r'saturating_',
    ],
    'collections-std': [
        r'Map\.new', r'Map\.with_capacity', r'List\.new',
        r'\.keys\(\)', r'\.values\(\)', r'\.into_iter\(\)',
        r'for\s+\([^)]+\)\s+in\s+\w+',  # tuple destructuring in for
    ],
    'derive-macros': [
        r'@derive\(', r'#\[derive\(',
    ],
    'pattern-matching-advanced': [
        r'where\s+\w+\s*:', r'if\s+let\s+',  # pattern guards
    ],
    'context-system': [
        r'provide\s+', r'using\s+\[', r'context\s+\w+',
    ],
    'meta-programming': [
        r'@const\s+fn', r'const\s+\{', r'meta\s+fn',
        r'quote!', r'splice!',
    ],
}


def get_skip_reason(content):
    """Extract @skip reason from content."""
    match = re.search(r'// @skip:\s*(.+?)(?:\n|$)', content)
    return match.group(1).strip() if match else None


def feature_from_skip_reason(reason):
    """Map skip reason to a feature."""
    if not reason:
        return None
    for pattern, feature in SKIP_REASON_TO_FEATURE.items():
        if pattern.lower() in reason.lower():
            return feature
    return None


def get_required_features_from_content(content):
    """Determine which features a test requires based on content patterns."""
    required = set()
    for feature, patterns in FEATURE_PATTERNS.items():
        for pattern in patterns:
            if re.search(pattern, content, re.IGNORECASE):
                required.add(feature)
                break
    return required


def convert_skip_to_requires(path):
    """Convert @skip directive to @requires if possible."""
    with open(path, 'r') as f:
        content = f.read()

    # Check if has @skip but no @requires
    if '@requires:' in content:
        return False, "already has @requires"

    skip_reason = get_skip_reason(content)
    if not skip_reason:
        return False, "no @skip"

    # Get feature from skip reason
    feature = feature_from_skip_reason(skip_reason)
    if not feature:
        # Try to detect from content
        features = get_required_features_from_content(content)
        if features:
            feature = sorted(features)[0]  # Take first feature
        else:
            return False, f"unknown skip reason: {skip_reason[:50]}"

    # Replace @skip with @requires
    new_content = re.sub(
        r'// @skip:.*\n',
        f'// @requires: {feature}\n',
        content,
        count=1
    )

    if new_content == content:
        return False, "could not replace"

    with open(path, 'w') as f:
        f.write(new_content)

    return True, f"converted to @requires: {feature}"


def add_requires_to_file(path):
    """Add @requires directive to a test file if needed."""
    with open(path, 'r') as f:
        content = f.read()

    if '@skip' in content or '@requires' in content:
        return False

    features = get_required_features_from_content(content)
    if not features:
        return False

    # Find where to insert @requires (after @test line)
    lines = content.split('\n')
    insert_idx = None
    for i, line in enumerate(lines):
        if line.strip().startswith('// @test:'):
            insert_idx = i + 1
            break

    if insert_idx is None:
        return False

    # Add @requires
    requires_line = f"// @requires: {', '.join(sorted(features))}"
    lines.insert(insert_idx, requires_line)

    with open(path, 'w') as f:
        f.write('\n'.join(lines))

    return True


def main():
    specs_dir = Path(__file__).parent.parent / 'specs'
    converted = 0
    added = 0
    failed = []

    # First pass: convert @skip to @requires
    print("Converting @skip to @requires...")
    for path in specs_dir.rglob('*.vr'):
        success, msg = convert_skip_to_requires(path)
        if success:
            print(f"  [CONVERTED] {path.relative_to(specs_dir)}: {msg}")
            converted += 1
        elif "unknown skip reason" in msg:
            failed.append((path.relative_to(specs_dir), msg))

    # Second pass: add @requires based on content patterns
    print("\nAdding @requires based on content...")
    for path in specs_dir.rglob('*.vr'):
        if add_requires_to_file(path):
            print(f"  [ADDED] {path.relative_to(specs_dir)}")
            added += 1

    print(f"\n{'='*60}")
    print(f"Converted @skip -> @requires: {converted} files")
    print(f"Added @requires from content: {added} files")
    if failed:
        print(f"\nFailed conversions ({len(failed)}):")
        for path, msg in failed[:10]:
            print(f"  {path}: {msg}")
        if len(failed) > 10:
            print(f"  ... and {len(failed) - 10} more")
    print(f"{'='*60}")


if __name__ == '__main__':
    main()
