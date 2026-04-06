import * as vscode from 'vscode';
import { LanguageClient } from 'vscode-languageclient/node';

/**
 * Enhanced hover provider for Verum language.
 * Provides additional context for CBGR costs and refinement types.
 *
 * Per spec 25-developer-tooling.md Section 3.6:
 * - Shows type signatures with CBGR costs
 * - Displays refinement constraints
 * - Shows counterexample information when available
 */
export class VerumHoverProvider implements vscode.HoverProvider {
    constructor(private client: LanguageClient) {}

    async provideHover(
        document: vscode.TextDocument,
        position: vscode.Position,
        _token: vscode.CancellationToken
    ): Promise<vscode.Hover | undefined> {
        if (!this.client) {
            return undefined;
        }

        try {
            // Use the LSP hover request through the client
            const hover = await this.client.sendRequest('textDocument/hover', {
                textDocument: { uri: document.uri.toString() },
                position: { line: position.line, character: position.character }
            }) as { contents: string | { kind: string; value: string } | Array<string | { kind: string; value: string }>; range?: { start: { line: number; character: number }; end: { line: number; character: number } } } | null;

            if (!hover || !hover.contents) {
                return undefined;
            }

            // Convert LSP hover to VS Code hover
            const contents = this.convertContents(hover.contents);

            let range: vscode.Range | undefined;
            if (hover.range) {
                range = new vscode.Range(
                    new vscode.Position(hover.range.start.line, hover.range.start.character),
                    new vscode.Position(hover.range.end.line, hover.range.end.character)
                );
            }

            return new vscode.Hover(contents, range);
        } catch (_error) {
            // Silently fail - LSP will provide hover through its own mechanism
            return undefined;
        }
    }

    private convertContents(
        contents: string | { kind: string; value: string } | Array<string | { kind: string; value: string }>
    ): vscode.MarkdownString[] {
        const result: vscode.MarkdownString[] = [];

        if (typeof contents === 'string') {
            result.push(new vscode.MarkdownString(contents));
        } else if (Array.isArray(contents)) {
            for (const content of contents) {
                if (typeof content === 'string') {
                    result.push(new vscode.MarkdownString(content));
                } else if (content.kind === 'markdown') {
                    const md = new vscode.MarkdownString(content.value);
                    md.isTrusted = true;
                    result.push(md);
                } else {
                    result.push(new vscode.MarkdownString(content.value));
                }
            }
        } else if (contents.kind === 'markdown') {
            const md = new vscode.MarkdownString(contents.value);
            md.isTrusted = true;
            result.push(md);
        } else {
            result.push(new vscode.MarkdownString(contents.value));
        }

        return result;
    }
}

/**
 * Signature help provider for Verum functions.
 * Shows function parameters with refinement type information.
 *
 * Per spec 25-developer-tooling.md Section 3.5:
 * - Function parameter hints
 * - Refinement constraints on parameters
 * - CBGR tier information for reference parameters
 */
export class VerumSignatureHelpProvider implements vscode.SignatureHelpProvider {
    constructor(private client: LanguageClient) {}

    async provideSignatureHelp(
        document: vscode.TextDocument,
        position: vscode.Position,
        _token: vscode.CancellationToken,
        _context: vscode.SignatureHelpContext
    ): Promise<vscode.SignatureHelp | undefined> {
        if (!this.client) {
            return undefined;
        }

        try {
            const result = await this.client.sendRequest('textDocument/signatureHelp', {
                textDocument: { uri: document.uri.toString() },
                position: { line: position.line, character: position.character }
            }) as {
                signatures: Array<{
                    label: string;
                    documentation?: string | { kind: string; value: string };
                    parameters?: Array<{
                        label: string | [number, number];
                        documentation?: string | { kind: string; value: string };
                    }>;
                }>;
                activeSignature?: number;
                activeParameter?: number;
            } | null;

            if (!result || !result.signatures || result.signatures.length === 0) {
                return undefined;
            }

            const help = new vscode.SignatureHelp();

            help.signatures = result.signatures.map(sig => {
                const signature = new vscode.SignatureInformation(sig.label);

                if (sig.documentation) {
                    if (typeof sig.documentation === 'string') {
                        signature.documentation = sig.documentation;
                    } else {
                        const md = new vscode.MarkdownString(sig.documentation.value);
                        md.isTrusted = true;
                        signature.documentation = md;
                    }
                }

                if (sig.parameters) {
                    signature.parameters = sig.parameters.map(param => {
                        const label = typeof param.label === 'string'
                            ? param.label
                            : sig.label.substring(param.label[0], param.label[1]);

                        const paramInfo = new vscode.ParameterInformation(label);

                        if (param.documentation) {
                            if (typeof param.documentation === 'string') {
                                paramInfo.documentation = param.documentation;
                            } else {
                                const md = new vscode.MarkdownString(param.documentation.value);
                                md.isTrusted = true;
                                paramInfo.documentation = md;
                            }
                        }

                        return paramInfo;
                    });
                }

                return signature;
            });

            help.activeSignature = result.activeSignature ?? 0;
            help.activeParameter = result.activeParameter ?? 0;

            return help;
        } catch (_error) {
            // Silently fail - LSP will provide signature help through its own mechanism
            return undefined;
        }
    }
}
