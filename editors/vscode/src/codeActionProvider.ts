import * as vscode from 'vscode';
import { LanguageClient, RequestType, Range as LSPRange, TextDocumentIdentifier } from 'vscode-languageclient/node';

interface QuickFix {
    title: string;
    kind: string;
    edits: TextEdit[];
    priority: number;
    impact: 'safe' | 'breaking' | 'unsafe';
    description?: string;
}

interface TextEdit {
    range: LSPRange;
    newText: string;
}

interface GetQuickFixesParams {
    textDocument: TextDocumentIdentifier;
    range: LSPRange;
}

const getQuickFixesRequest = new RequestType<GetQuickFixesParams, QuickFix[], void>('verum/getQuickFixes');

export class VerumCodeActionProvider implements vscode.CodeActionProvider {
    constructor(private client: LanguageClient) {}

    public async provideCodeActions(
        document: vscode.TextDocument,
        range: vscode.Range,
        context: vscode.CodeActionContext,
        _token: vscode.CancellationToken
    ): Promise<vscode.CodeAction[]> {
        const actions: vscode.CodeAction[] = [];

        // Only provide actions for Verum diagnostics
        const verumDiagnostics = context.diagnostics.filter(d => d.source === 'verum');
        if (verumDiagnostics.length === 0) {
            return actions;
        }

        try {
            for (const diagnostic of verumDiagnostics) {
                const quickFixes = await this.client.sendRequest(getQuickFixesRequest, {
                    textDocument: { uri: document.uri.toString() },
                    range: this.vscodeRangeToLSPRange(diagnostic.range),
                });

                for (const fix of quickFixes) {
                    const action = this.createCodeAction(document, diagnostic, fix);
                    actions.push(action);
                }
            }
        } catch (error) {
            console.error('Failed to get quick fixes:', error);
        }

        return actions;
    }

    /**
     * Create a VS Code code action from a quick fix
     */
    private createCodeAction(
        document: vscode.TextDocument,
        diagnostic: vscode.Diagnostic,
        fix: QuickFix
    ): vscode.CodeAction {
        const action = new vscode.CodeAction(
            fix.title,
            this.mapQuickFixKind(fix.kind)
        );

        // Create workspace edit
        const edit = new vscode.WorkspaceEdit();
        for (const textEdit of fix.edits) {
            const range = new vscode.Range(
                new vscode.Position(textEdit.range.start.line, textEdit.range.start.character),
                new vscode.Position(textEdit.range.end.line, textEdit.range.end.character)
            );
            edit.replace(document.uri, range, textEdit.newText);
        }

        action.edit = edit;
        action.diagnostics = [diagnostic];
        action.isPreferred = fix.priority === 1;

        // Add tooltip with description and impact
        if (fix.description) {
            action.command = {
                command: 'verum.showQuickFixInfo',
                title: 'Show Quick Fix Info',
                tooltip: `${fix.description}\n\nImpact: ${this.formatImpact(fix.impact)}`,
            };
        }

        return action;
    }

    /**
     * Map quick fix kind to VS Code code action kind
     */
    private mapQuickFixKind(kind: string): vscode.CodeActionKind {
        switch (kind) {
            case 'runtime_check':
            case 'inline_refinement':
            case 'assertion':
            case 'weaken_refinement':
                return vscode.CodeActionKind.QuickFix;

            case 'sigma_type':
            case 'promote_to_checked':
                return vscode.CodeActionKind.RefactorRewrite;

            default:
                return vscode.CodeActionKind.QuickFix;
        }
    }

    /**
     * Format impact message for display
     */
    private formatImpact(impact: 'safe' | 'breaking' | 'unsafe'): string {
        switch (impact) {
            case 'safe':
                return 'Safe (no breaking changes)';
            case 'breaking':
                return 'Breaking (may require caller changes)';
            case 'unsafe':
                return 'Unsafe (requires manual verification)';
        }
    }

    /**
     * Convert VS Code range to LSP range
     */
    private vscodeRangeToLSPRange(range: vscode.Range): LSPRange {
        return {
            start: { line: range.start.line, character: range.start.character },
            end: { line: range.end.line, character: range.end.character },
        };
    }
}
