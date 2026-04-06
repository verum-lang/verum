import * as vscode from 'vscode';
import { LanguageClient, RequestType, Position as LSPPosition, Range as LSPRange, TextDocumentIdentifier } from 'vscode-languageclient/node';

interface ValidateRefinementParams {
    textDocument: TextDocumentIdentifier;
    position: LSPPosition;
    mode?: 'quick' | 'thorough' | 'complete';
}

interface ValidateRefinementResult {
    valid: boolean;
    diagnostics: RefinementDiagnostic[];
    performanceMs: number;
}

interface RefinementDiagnostic {
    range: LSPRange;
    severity: number;
    code?: string | number;
    source: string;
    message: string;
    counterexample?: CounterexampleData;
    quickFixes?: QuickFix[];
    validationTimeMs?: number;
    smtSolver?: string;
}

interface CounterexampleData {
    variable: string;
    value: string;
    type: string;
    constraint: string;
    violationReason: string;
    trace?: ExecutionTrace[];
}

interface ExecutionTrace {
    line: number;
    operation: string;
    value: string;
    explanation: string;
}

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

const validateRefinementRequest = new RequestType<ValidateRefinementParams, ValidateRefinementResult, void>('verum/validateRefinement');

export class RefinementValidator {
    private diagnosticCollection: vscode.DiagnosticCollection;
    private validationTimer: NodeJS.Timeout | undefined;
    private debounceDelay: number;

    constructor(
        private client: LanguageClient,
        private config: vscode.WorkspaceConfiguration
    ) {
        this.diagnosticCollection = vscode.languages.createDiagnosticCollection('verum-refinement');
        this.debounceDelay = config.get<number>('diagnosticDelay') || 200;
    }

    /**
     * Validate refinements on typing with debouncing
     * Called 200ms after typing stops to avoid overwhelming the SMT solver
     */
    public validateOnType(
        document: vscode.TextDocument,
        position?: vscode.Position
    ): void {
        // Clear previous timer
        if (this.validationTimer) {
            clearTimeout(this.validationTimer);
        }

        // Set new timer
        this.validationTimer = setTimeout(() => {
            this.performValidation(document, position);
        }, this.debounceDelay);
    }

    /**
     * Perform actual validation by sending request to LSP server
     */
    private async performValidation(
        document: vscode.TextDocument,
        position?: vscode.Position
    ): Promise<void> {
        if (!this.config.get<boolean>('enableRefinementValidation')) {
            return;
        }

        try {
            const validationMode = this.config.get<'quick' | 'thorough' | 'complete'>('validationMode') || 'quick';

            // If position is provided, validate at that position
            // Otherwise, validate entire document (will be handled by LSP server)
            const lspPosition: LSPPosition = position
                ? { line: position.line, character: position.character }
                : { line: 0, character: 0 };

            const result = await this.client.sendRequest(validateRefinementRequest, {
                textDocument: { uri: document.uri.toString() },
                position: lspPosition,
                mode: validationMode,
            });

            if (!result.valid && result.diagnostics.length > 0) {
                this.showDiagnostics(document, result.diagnostics);
            } else {
                // Clear diagnostics if validation passed
                this.diagnosticCollection.delete(document.uri);
            }
        } catch (error) {
            console.error('Refinement validation failed:', error);
            // Don't show error to user - validation failures should be silent
        }
    }

    /**
     * Convert LSP diagnostics to VS Code diagnostics and display them
     */
    private showDiagnostics(
        document: vscode.TextDocument,
        diagnostics: RefinementDiagnostic[]
    ): void {
        const vscodeDiagnostics = diagnostics.map(d => {
            const diagnostic = new vscode.Diagnostic(
                this.lspRangeToVSCodeRange(d.range),
                this.formatDiagnosticMessage(d),
                this.mapSeverity(d.severity)
            );

            diagnostic.code = d.code;
            diagnostic.source = d.source || 'verum';

            // Add related information for counterexamples
            if (d.counterexample && d.counterexample.trace) {
                diagnostic.relatedInformation = d.counterexample.trace.map(trace =>
                    new vscode.DiagnosticRelatedInformation(
                        new vscode.Location(
                            document.uri,
                            new vscode.Position(trace.line, 0)
                        ),
                        `${trace.operation}: ${trace.explanation}`
                    )
                );
            }

            return diagnostic;
        });

        this.diagnosticCollection.set(document.uri, vscodeDiagnostics);
    }

    /**
     * Format diagnostic message with counterexample information
     */
    private formatDiagnosticMessage(diagnostic: RefinementDiagnostic): string {
        let message = diagnostic.message;

        // Add counterexample details
        if (diagnostic.counterexample) {
            const ce = diagnostic.counterexample;
            message += `\n\nCounterexample: ${ce.variable} = ${ce.value}`;
            message += `\nReason: ${ce.violationReason}`;

            if (diagnostic.validationTimeMs) {
                message += `\n\nValidated in ${diagnostic.validationTimeMs}ms using ${diagnostic.smtSolver || 'SMT solver'}`;
            }
        }

        return message;
    }

    /**
     * Convert LSP range to VS Code range
     */
    private lspRangeToVSCodeRange(range: LSPRange): vscode.Range {
        return new vscode.Range(
            new vscode.Position(range.start.line, range.start.character),
            new vscode.Position(range.end.line, range.end.character)
        );
    }

    /**
     * Map LSP severity to VS Code severity
     */
    private mapSeverity(severity: number): vscode.DiagnosticSeverity {
        switch (severity) {
            case 1: return vscode.DiagnosticSeverity.Error;
            case 2: return vscode.DiagnosticSeverity.Warning;
            case 3: return vscode.DiagnosticSeverity.Information;
            case 4: return vscode.DiagnosticSeverity.Hint;
            default: return vscode.DiagnosticSeverity.Error;
        }
    }

    /**
     * Dispose resources
     */
    public dispose(): void {
        if (this.validationTimer) {
            clearTimeout(this.validationTimer);
        }
        this.diagnosticCollection.dispose();
    }
}
