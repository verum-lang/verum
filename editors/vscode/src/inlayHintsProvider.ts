import * as vscode from 'vscode';
import { LanguageClient, RequestType, Position as LSPPosition, Range as LSPRange, TextDocumentIdentifier } from 'vscode-languageclient/node';

interface InlayHint {
    position: LSPPosition;
    label: string;
    kind: 'type' | 'parameter';
    tooltip?: string;
}

interface GetInlayHintsParams {
    textDocument: TextDocumentIdentifier;
    range: LSPRange;
}

const getInlayHintsRequest = new RequestType<GetInlayHintsParams, InlayHint[], void>('verum/getInlayHints');

export class RefinementInlayHintsProvider implements vscode.InlayHintsProvider {
    constructor(private client: LanguageClient) {}

    public async provideInlayHints(
        document: vscode.TextDocument,
        range: vscode.Range,
        _token: vscode.CancellationToken
    ): Promise<vscode.InlayHint[]> {
        try {
            const hints = await this.client.sendRequest(getInlayHintsRequest, {
                textDocument: { uri: document.uri.toString() },
                range: {
                    start: { line: range.start.line, character: range.start.character },
                    end: { line: range.end.line, character: range.end.character },
                },
            });

            return hints.map(h => this.createInlayHint(h));
        } catch (error) {
            console.error('Failed to get inlay hints:', error);
            return [];
        }
    }

    /**
     * Create VS Code inlay hint from LSP inlay hint
     */
    private createInlayHint(hint: InlayHint): vscode.InlayHint {
        const position = new vscode.Position(hint.position.line, hint.position.character);
        const kind = hint.kind === 'type'
            ? vscode.InlayHintKind.Type
            : vscode.InlayHintKind.Parameter;

        const inlayHint = new vscode.InlayHint(
            position,
            hint.label,
            kind
        );

        if (hint.tooltip) {
            inlayHint.tooltip = hint.tooltip;
        }

        // Style type hints differently
        if (kind === vscode.InlayHintKind.Type) {
            inlayHint.paddingLeft = true;
        }

        return inlayHint;
    }
}
