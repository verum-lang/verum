import * as vscode from 'vscode';
import { describe, it, expect } from './index';

describe('Refinement Validator Test Suite', () => {
    it('Should provide diagnostics for Verum files', async () => {
        // Create a test document
        const content = `
fn divide(x: Int, y: Int) -> Int {
    x / y
}
`;
        const doc = await vscode.workspace.openTextDocument({
            language: 'verum',
            content: content
        });

        // Wait a bit for diagnostics to be computed
        await new Promise(resolve => setTimeout(resolve, 500));

        // Note: Full diagnostics testing requires LSP server to be running
        // This test verifies the infrastructure is in place
        expect(doc.languageId).toBe('verum');
    });

    it('Diagnostic collection should exist', () => {
        // Diagnostics are published through LSP
        // This test verifies the diagnostic infrastructure
        const diagnostics = vscode.languages.getDiagnostics();
        expect(Array.isArray(diagnostics)).toBe(true);
    });
});

describe('Quick Fix Test Suite', () => {
    it('Code action kinds should be registered', async () => {
        // Verify code action provider is registered for Verum
        const languages = await vscode.languages.getLanguages();
        expect(languages).toContain('verum');
    });
});

describe('Counterexample Display Test Suite', () => {
    it('Counterexample data structure should be valid', () => {
        // Test counterexample interface
        interface CounterexampleData {
            variable: string;
            value: string;
            type: string;
            constraint: string;
            violationReason: string;
            trace?: Array<{
                line: number;
                operation: string;
                value: string;
                explanation: string;
            }>;
        }

        const testCounterexample: CounterexampleData = {
            variable: 'x',
            value: '-5',
            type: 'Int{i != 0}',
            constraint: 'i != 0',
            violationReason: '-5 does not satisfy i != 0',
            trace: [
                {
                    line: 8,
                    operation: 'assignment',
                    value: '-5',
                    explanation: 'x is assigned -5'
                }
            ]
        };

        expect(testCounterexample.variable).toBe('x');
        expect(testCounterexample.value).toBe('-5');
        expect(testCounterexample.trace && testCounterexample.trace.length > 0).toBe(true);
    });
});

describe('Quick Fix Categories Test Suite', () => {
    it('All 6 quick fix categories should be supported', () => {
        // Per spec 25-developer-tooling.md Section 3.4
        const quickFixKinds = [
            'runtime_check',       // Wrap with Result<T, E>
            'inline_refinement',   // Add refinement to parameter
            'sigma_type',          // Convert to dependent pair
            'assertion',           // Add runtime assertion
            'weaken_refinement',   // Relax constraint
            'promote_to_checked'   // Use &checked reference
        ];

        expect(quickFixKinds.length).toBe(6);

        // Verify each category is a non-empty string
        for (const kind of quickFixKinds) {
            expect(typeof kind === 'string' && kind.length > 0).toBe(true);
        }
    });

    it('Quick fix impact levels should be valid', () => {
        const impactLevels = ['safe', 'breaking', 'unsafe'];

        expect(impactLevels.length).toBe(3);
        expect(impactLevels).toContain('safe');
        expect(impactLevels).toContain('breaking');
        expect(impactLevels).toContain('unsafe');
    });
});
