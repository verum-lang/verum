import * as vscode from 'vscode';
import { describe, it, expect, beforeAll } from './index';

describe('Verum Extension Test Suite', () => {
    beforeAll(() => {
        vscode.window.showInformationMessage('Start all tests.');
    });

    it('Extension should be present', () => {
        expect(vscode.extensions.getExtension('verum-lang.verum-language')).toBeTruthy();
    });

    it('Extension should activate', async () => {
        const ext = vscode.extensions.getExtension('verum-lang.verum-language');
        if (ext) {
            await ext.activate();
            expect(ext.isActive).toBe(true);
        }
    });

    it('Should register verum language', async () => {
        const languages = await vscode.languages.getLanguages();
        expect(languages).toContain('verum');
    });

    it('Should register all commands', async () => {
        const commands = await vscode.commands.getCommands(true);

        const verumCommands = [
            'verum.promoteToChecked',
            'verum.addRuntimeCheck',
            'verum.inferRefinement',
            'verum.validateRefinementAtCursor',
            'verum.showVerificationProfile',
            'verum.showCbgrProfile',
            'verum.restartLanguageServer',
            'verum.openProfile',
            'verum.profileCurrentFile'
        ];

        for (const cmd of verumCommands) {
            expect(commands).toContain(cmd);
        }
    });

    it('Configuration should have all required properties', () => {
        const config = vscode.workspace.getConfiguration('verum.lsp');

        const requiredProps = [
            'enable',
            'serverPath',
            'enableRefinementValidation',
            'validationMode',
            'showCounterexamples',
            'showInlayHints',
            'diagnosticDelay',
            'smtSolver',
            'smtTimeout',
            'cacheValidationResults'
        ];

        for (const prop of requiredProps) {
            const value = config.get(prop);
            expect(value).toBeDefined();
        }
    });

    it('Validation mode should have valid values', () => {
        const config = vscode.workspace.getConfiguration('verum.lsp');
        const mode = config.get<string>('validationMode');

        expect(mode === 'quick' || mode === 'thorough' || mode === 'complete').toBe(true);
    });

    it('SMT solver should have valid values', () => {
        const config = vscode.workspace.getConfiguration('verum.lsp');
        const solver = config.get<string>('smtSolver');

        expect(solver === 'z3' || solver === 'cvc5' || solver === 'auto').toBe(true);
    });

    it('Diagnostic delay should be within bounds', () => {
        const config = vscode.workspace.getConfiguration('verum.lsp');
        const delay = config.get<number>('diagnosticDelay');

        expect(delay).toBeDefined();
        expect(delay!).toBeGreaterThanOrEqual(0);
        expect(delay!).toBeLessThanOrEqual(2000);
    });

    it('SMT timeout should be within bounds', () => {
        const config = vscode.workspace.getConfiguration('verum.lsp');
        const timeout = config.get<number>('smtTimeout');

        expect(timeout).toBeDefined();
        expect(timeout!).toBeGreaterThanOrEqual(10);
        expect(timeout!).toBeLessThanOrEqual(1000);
    });
});
