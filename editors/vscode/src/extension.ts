import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    RequestType,
    Position as LSPPosition,
    Range as LSPRange,
    TextDocumentIdentifier,
    State,
} from 'vscode-languageclient/node';

import { RefinementValidator } from './refinementValidator';
import { VerumCodeActionProvider } from './codeActionProvider';
import { RefinementInlayHintsProvider } from './inlayHintsProvider';
import { DashboardWebviewProvider, ProfileData } from './dashboardWebview';
import { VerumDebugConfigurationProvider, VerumDebugAdapterDescriptorFactory } from './debugAdapter';

// ---------------------------------------------------------------------------
// LSP protocol types
// ---------------------------------------------------------------------------

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
    edits: TextEditLSP[];
    priority: number;
    impact: 'safe' | 'breaking' | 'unsafe';
    description?: string;
}

interface TextEditLSP {
    range: LSPRange;
    newText: string;
}

interface PromoteToCheckedParams {
    textDocument: TextDocumentIdentifier;
    range: LSPRange;
    includeProof?: boolean;
}

interface PromoteToCheckedResult {
    success: boolean;
    edits: TextEditLSP[];
    proofComment?: string;
}

interface InferRefinementParams {
    textDocument: TextDocumentIdentifier;
    symbol: string;
}

interface InferRefinementResult {
    inferredType: string;
    confidence: 'high' | 'medium' | 'low';
    usages: CodeLocation[];
    edits: TextEditLSP[];
}

interface CodeLocation {
    uri: string;
    range: LSPRange;
    context: string;
}

interface GetProfileParams {
    textDocument: TextDocumentIdentifier;
}

interface VerifyFunctionParams {
    textDocument: TextDocumentIdentifier;
    position: LSPPosition;
}

interface VerifyFunctionResult {
    verified: boolean;
    functionName: string;
    contracts: number;
    elapsed: number;
    errors?: string[];
}

// Custom request types
const validateRefinementRequest = new RequestType<ValidateRefinementParams, ValidateRefinementResult, void>('verum/validateRefinement');
const promoteToCheckedRequest = new RequestType<PromoteToCheckedParams, PromoteToCheckedResult, void>('verum/promoteToChecked');
const inferRefinementRequest = new RequestType<InferRefinementParams, InferRefinementResult, void>('verum/inferRefinement');
const getProfileRequest = new RequestType<GetProfileParams, ProfileData, void>('verum/getProfile');
const verifyFunctionRequest = new RequestType<VerifyFunctionParams, VerifyFunctionResult, void>('verum/verifyFunction');

// ---------------------------------------------------------------------------
// Module-level state
// ---------------------------------------------------------------------------

let client: LanguageClient | undefined;
let outputChannel: vscode.OutputChannel;
let statusBarItem: vscode.StatusBarItem;
let symbolCountItem: vscode.StatusBarItem;
let verificationStatusItem: vscode.StatusBarItem;
let dashboardProvider: DashboardWebviewProvider | undefined;
let crashCount = 0;

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    outputChannel = vscode.window.createOutputChannel('Verum Language Server');
    context.subscriptions.push(outputChannel);

    createStatusBarItems(context);

    const config = vscode.workspace.getConfiguration('verum.lsp');
    if (!config.get<boolean>('enable', true)) {
        statusBarItem.text = '$(circle-slash) Verum: Disabled';
        statusBarItem.tooltip = 'Verum Language Server is disabled. Enable with verum.lsp.enable.';
        return;
    }

    // Register providers and commands that work regardless of LSP state
    registerDashboard(context);
    registerCommands(context);
    registerTaskProvider(context);
    registerTerminalLinkProvider(context);
    registerFileDecorationProvider(context);
    registerDebugAdapter(context);

    // Configuration change handler
    context.subscriptions.push(
        vscode.workspace.onDidChangeConfiguration(e => {
            if (e.affectsConfiguration('verum')) {
                onConfigurationChanged();
            }
        })
    );

    // Track document symbols for status bar
    context.subscriptions.push(
        vscode.window.onDidChangeActiveTextEditor(editor => {
            updateSymbolCount(editor);
        })
    );
    context.subscriptions.push(
        vscode.workspace.onDidChangeTextDocument(e => {
            if (e.document === vscode.window.activeTextEditor?.document) {
                updateSymbolCount(vscode.window.activeTextEditor);
            }
        })
    );

    await startLanguageClient(context);
}

export async function deactivate(): Promise<void> {
    if (client) {
        await client.stop();
        client = undefined;
    }
}

// ---------------------------------------------------------------------------
// Language client lifecycle
// ---------------------------------------------------------------------------

async function startLanguageClient(context: vscode.ExtensionContext): Promise<void> {
    const config = vscode.workspace.getConfiguration('verum.lsp');
    const serverPath = config.get<string>('serverPath') || 'verum';

    statusBarItem.text = '$(loading~spin) Verum: Starting...';
    statusBarItem.tooltip = 'Starting Verum Language Server...';

    const serverOptions: ServerOptions = {
        command: serverPath,
        args: ['lsp', '--transport', 'stdio'],
    };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'verum' }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/*.vr'),
        },
        outputChannel,
        traceOutputChannel: outputChannel,
        revealOutputChannelOn: 4, // Never auto-reveal
        initializationOptions: {
            enableRefinementValidation: config.get('enableRefinementValidation'),
            validationMode: config.get('validationMode'),
            smtSolver: config.get('smtSolver'),
            smtTimeout: config.get('smtTimeout'),
            cacheValidationResults: config.get('cacheValidationResults'),
            showCounterexamples: config.get('showCounterexamples'),
            cacheTtlSeconds: config.get('cacheTtlSeconds'),
            cacheMaxEntries: config.get('cacheMaxEntries'),
            maxCounterexampleTraces: config.get('maxCounterexampleTraces'),
            cbgrEnableProfiling: vscode.workspace.getConfiguration('verum.cbgr').get('enableProfiling'),
            cbgrShowOptimizationHints: vscode.workspace.getConfiguration('verum.cbgr').get('showOptimizationHints'),
            verificationShowCostWarnings: vscode.workspace.getConfiguration('verum.verification').get('showCostWarnings'),
            verificationSlowThresholdMs: vscode.workspace.getConfiguration('verum.verification').get('slowThresholdMs'),
        },
        middleware: {
            // Forward workspace/didChangeConfiguration when settings change
            workspace: {
                didChangeConfiguration: (sections, next) => {
                    return next(sections);
                },
            },
        },
    };

    client = new LanguageClient(
        'verum-lsp',
        'Verum Language Server',
        serverOptions,
        clientOptions,
    );

    // Monitor state transitions
    client.onDidChangeState(event => {
        switch (event.newState) {
            case State.Running:
                crashCount = 0;
                statusBarItem.text = '$(check) Verum';
                statusBarItem.tooltip = 'Verum Language Server is running';
                statusBarItem.command = 'verum.showServerStatus';
                break;
            case State.Starting:
                statusBarItem.text = '$(loading~spin) Verum: Starting...';
                statusBarItem.tooltip = 'Verum Language Server is starting...';
                break;
            case State.Stopped:
                statusBarItem.text = '$(warning) Verum: Stopped';
                statusBarItem.tooltip = 'Verum Language Server stopped. Click to restart.';
                statusBarItem.command = 'verum.restartLanguageServer';
                onServerStopped();
                break;
        }
    });

    try {
        await client.start();
        registerLSPDependentProviders(context);
    } catch (error) {
        statusBarItem.text = '$(error) Verum: Error';
        statusBarItem.tooltip = `Failed to start Verum Language Server: ${error}`;
        statusBarItem.command = 'verum.restartLanguageServer';
        outputChannel.appendLine(`[ERROR] Failed to start language server: ${error}`);
        vscode.window.showErrorMessage(
            `Failed to start Verum Language Server: ${error}`,
            'Retry',
            'Show Output',
        ).then(action => {
            if (action === 'Retry') {
                vscode.commands.executeCommand('verum.restartLanguageServer');
            } else if (action === 'Show Output') {
                outputChannel.show();
            }
        });
    }
}

/**
 * Register providers that depend on an active LSP client.
 * Called after client.start() succeeds.
 */
function registerLSPDependentProviders(context: vscode.ExtensionContext): void {
    if (!client) { return; }

    const config = vscode.workspace.getConfiguration('verum.lsp');

    // Refinement validator (custom — adds on-type validation via custom LSP method)
    const validator = new RefinementValidator(client, config);
    context.subscriptions.push(validator);
    context.subscriptions.push(
        vscode.workspace.onDidChangeTextDocument(e => {
            if (e.document.languageId === 'verum' && e.contentChanges.length > 0) {
                validator.validateOnType(e.document, e.contentChanges[0]?.range?.start);
            }
        })
    );

    // Code action provider (custom — queries verum/getQuickFixes)
    context.subscriptions.push(
        vscode.languages.registerCodeActionsProvider(
            { language: 'verum', scheme: 'file' },
            new VerumCodeActionProvider(client),
            {
                providedCodeActionKinds: [
                    vscode.CodeActionKind.QuickFix,
                    vscode.CodeActionKind.Refactor,
                ],
            },
        )
    );

    // Refinement inlay hints (custom — queries verum/getInlayHints)
    if (config.get<boolean>('showInlayHints', true)) {
        context.subscriptions.push(
            vscode.languages.registerInlayHintsProvider(
                { language: 'verum', scheme: 'file' },
                new RefinementInlayHintsProvider(client),
            )
        );
    }

    // NOTE: We intentionally do NOT register custom hover or signature help
    // providers. The LanguageClient handles textDocument/hover and
    // textDocument/signatureHelp natively and will forward them to the LSP
    // server automatically. Registering custom providers would shadow the
    // LSP-provided ones.
}

/**
 * Handle unexpected server stop — offer restart with exponential backoff.
 */
function onServerStopped(): void {
    crashCount++;
    if (crashCount <= 3) {
        const delay = Math.min(1000 * Math.pow(2, crashCount - 1), 8000);
        vscode.window.showWarningMessage(
            `Verum Language Server stopped unexpectedly (attempt ${crashCount}/3). Restarting in ${delay / 1000}s...`,
            'Restart Now',
            'Show Output',
        ).then(action => {
            if (action === 'Restart Now') {
                vscode.commands.executeCommand('verum.restartLanguageServer');
            } else if (action === 'Show Output') {
                outputChannel.show();
            }
        });
        setTimeout(() => {
            if (client && crashCount <= 3) {
                client.start().catch(err => {
                    outputChannel.appendLine(`[ERROR] Auto-restart failed: ${err}`);
                });
            }
        }, delay);
    } else {
        vscode.window.showErrorMessage(
            'Verum Language Server crashed repeatedly. Click to restart manually.',
            'Restart',
            'Show Output',
        ).then(action => {
            if (action === 'Restart') {
                crashCount = 0;
                vscode.commands.executeCommand('verum.restartLanguageServer');
            } else if (action === 'Show Output') {
                outputChannel.show();
            }
        });
    }
}

/**
 * Forward configuration changes to the LSP server.
 */
function onConfigurationChanged(): void {
    if (!client) { return; }
    const config = vscode.workspace.getConfiguration('verum.lsp');
    client.sendNotification('workspace/didChangeConfiguration', {
        settings: {
            verum: {
                lsp: {
                    enableRefinementValidation: config.get('enableRefinementValidation'),
                    validationMode: config.get('validationMode'),
                    smtSolver: config.get('smtSolver'),
                    smtTimeout: config.get('smtTimeout'),
                    cacheValidationResults: config.get('cacheValidationResults'),
                    showCounterexamples: config.get('showCounterexamples'),
                },
                cbgr: {
                    enableProfiling: vscode.workspace.getConfiguration('verum.cbgr').get('enableProfiling'),
                    showOptimizationHints: vscode.workspace.getConfiguration('verum.cbgr').get('showOptimizationHints'),
                },
                verification: {
                    showCostWarnings: vscode.workspace.getConfiguration('verum.verification').get('showCostWarnings'),
                    slowThresholdMs: vscode.workspace.getConfiguration('verum.verification').get('slowThresholdMs'),
                },
            },
        },
    });
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

function createStatusBarItems(context: vscode.ExtensionContext): void {
    // Main status item (right-most)
    statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    statusBarItem.text = '$(loading~spin) Verum';
    statusBarItem.command = 'verum.showServerStatus';
    statusBarItem.show();
    context.subscriptions.push(statusBarItem);

    // Document symbol count
    symbolCountItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 99);
    symbolCountItem.tooltip = 'Verum: Document symbols';
    context.subscriptions.push(symbolCountItem);

    // Verification status
    verificationStatusItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 98);
    verificationStatusItem.tooltip = 'Verum: Verification status';
    context.subscriptions.push(verificationStatusItem);
}

let symbolCountDebounce: NodeJS.Timeout | undefined;

function updateSymbolCount(editor: vscode.TextEditor | undefined): void {
    if (!editor || editor.document.languageId !== 'verum') {
        symbolCountItem.hide();
        verificationStatusItem.hide();
        return;
    }

    if (symbolCountDebounce) { clearTimeout(symbolCountDebounce); }
    symbolCountDebounce = setTimeout(async () => {
        try {
            const symbols = await vscode.commands.executeCommand<vscode.DocumentSymbol[]>(
                'vscode.executeDocumentSymbolProvider',
                editor.document.uri,
            );
            if (symbols && symbols.length > 0) {
                const fnCount = countSymbolsOfKind(symbols, vscode.SymbolKind.Function);
                const typeCount = countSymbolsOfKind(symbols, vscode.SymbolKind.Struct)
                    + countSymbolsOfKind(symbols, vscode.SymbolKind.Enum)
                    + countSymbolsOfKind(symbols, vscode.SymbolKind.Interface)
                    + countSymbolsOfKind(symbols, vscode.SymbolKind.Class);
                symbolCountItem.text = `$(symbol-function) ${fnCount}  $(symbol-class) ${typeCount}`;
                symbolCountItem.show();
            } else {
                symbolCountItem.hide();
            }
        } catch {
            symbolCountItem.hide();
        }

        // Update verification status from diagnostics
        const diagnostics = vscode.languages.getDiagnostics(editor.document.uri);
        const errors = diagnostics.filter(d => d.severity === vscode.DiagnosticSeverity.Error);
        const warnings = diagnostics.filter(d => d.severity === vscode.DiagnosticSeverity.Warning);
        if (errors.length > 0) {
            verificationStatusItem.text = `$(error) ${errors.length}  $(warning) ${warnings.length}`;
            verificationStatusItem.backgroundColor = new vscode.ThemeColor('statusBarItem.errorBackground');
            verificationStatusItem.show();
        } else if (warnings.length > 0) {
            verificationStatusItem.text = `$(check) 0  $(warning) ${warnings.length}`;
            verificationStatusItem.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
            verificationStatusItem.show();
        } else if (diagnostics.length === 0) {
            verificationStatusItem.text = '$(verified) Verified';
            verificationStatusItem.backgroundColor = undefined;
            verificationStatusItem.show();
        } else {
            verificationStatusItem.hide();
        }
    }, 500);
}

function countSymbolsOfKind(symbols: vscode.DocumentSymbol[], kind: vscode.SymbolKind): number {
    let count = 0;
    for (const sym of symbols) {
        if (sym.kind === kind) { count++; }
        if (sym.children) { count += countSymbolsOfKind(sym.children, kind); }
    }
    return count;
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

function registerDashboard(context: vscode.ExtensionContext): void {
    dashboardProvider = new DashboardWebviewProvider(context.extensionUri, context);
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider(
            DashboardWebviewProvider.viewType,
            dashboardProvider,
        )
    );
}

// ---------------------------------------------------------------------------
// Task provider
// ---------------------------------------------------------------------------

class VerumTaskProvider implements vscode.TaskProvider {
    static readonly type = 'verum';

    provideTasks(): vscode.Task[] {
        const tasks: vscode.Task[] = [];
        const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
        if (!workspaceFolder) { return tasks; }

        const config = vscode.workspace.getConfiguration('verum.lsp');
        const verum = config.get<string>('serverPath') || 'verum';

        const buildDef = new vscode.Task(
            { type: VerumTaskProvider.type, task: 'build' },
            workspaceFolder,
            'build',
            'verum',
            new vscode.ShellExecution(`${verum} build .`),
            '$verum',
        );
        buildDef.group = vscode.TaskGroup.Build;
        buildDef.presentationOptions = { reveal: vscode.TaskRevealKind.Silent, panel: vscode.TaskPanelKind.Shared };
        tasks.push(buildDef);

        const runDef = new vscode.Task(
            { type: VerumTaskProvider.type, task: 'run' },
            workspaceFolder,
            'run',
            'verum',
            new vscode.ShellExecution(`${verum} run .`),
            '$verum',
        );
        runDef.presentationOptions = { reveal: vscode.TaskRevealKind.Always, panel: vscode.TaskPanelKind.Dedicated };
        tasks.push(runDef);

        const testDef = new vscode.Task(
            { type: VerumTaskProvider.type, task: 'test' },
            workspaceFolder,
            'test',
            'verum',
            new vscode.ShellExecution(`${verum} test .`),
            '$verum',
        );
        testDef.group = vscode.TaskGroup.Test;
        testDef.presentationOptions = { reveal: vscode.TaskRevealKind.Always, panel: vscode.TaskPanelKind.Shared };
        tasks.push(testDef);

        const checkDef = new vscode.Task(
            { type: VerumTaskProvider.type, task: 'check' },
            workspaceFolder,
            'check',
            'verum',
            new vscode.ShellExecution(`${verum} check .`),
            '$verum',
        );
        checkDef.group = vscode.TaskGroup.Build;
        checkDef.presentationOptions = { reveal: vscode.TaskRevealKind.Silent, panel: vscode.TaskPanelKind.Shared };
        tasks.push(checkDef);

        return tasks;
    }

    resolveTask(task: vscode.Task): vscode.Task | undefined {
        const definition = task.definition;
        if (definition.type !== VerumTaskProvider.type) { return undefined; }

        const config = vscode.workspace.getConfiguration('verum.lsp');
        const verum = config.get<string>('serverPath') || 'verum';
        const taskName = definition.task as string;

        return new vscode.Task(
            definition,
            task.scope ?? vscode.TaskScope.Workspace,
            task.name,
            'verum',
            new vscode.ShellExecution(`${verum} ${taskName} .`),
            '$verum',
        );
    }
}

function registerTaskProvider(context: vscode.ExtensionContext): void {
    context.subscriptions.push(
        vscode.tasks.registerTaskProvider(VerumTaskProvider.type, new VerumTaskProvider())
    );

    // Register a problem matcher via output channel pattern matching
    // The actual problem matcher pattern is defined in package.json
}

// ---------------------------------------------------------------------------
// Terminal link provider
// ---------------------------------------------------------------------------

class VerumTerminalLinkProvider implements vscode.TerminalLinkProvider {
    // Match patterns like:
    //   error[E123]: message at file.vr:42:10
    //   warning[W456]: message at file.vr:10:5
    //   --> file.vr:42:10
    //   file.vr:42:10: error: message
    private static readonly patterns = [
        // "at file.vr:42:10"
        /at\s+(\S+\.vr):(\d+):(\d+)/g,
        // "--> file.vr:42:10"
        /-->\s+(\S+\.vr):(\d+):(\d+)/g,
        // "file.vr:42:10: error"
        /(\S+\.vr):(\d+):(\d+):/g,
        // "file.vr:42"
        /(\S+\.vr):(\d+)\b/g,
    ];

    provideTerminalLinks(context: vscode.TerminalLinkContext): vscode.TerminalLink[] {
        const links: vscode.TerminalLink[] = [];
        const line = context.line;

        for (const pattern of VerumTerminalLinkProvider.patterns) {
            pattern.lastIndex = 0;
            let match: RegExpExecArray | null;
            while ((match = pattern.exec(line)) !== null) {
                const startIndex = match.index;
                const length = match[0].length;
                const file = match[1];
                const lineNum = parseInt(match[2], 10);
                const col = match[3] ? parseInt(match[3], 10) : 1;

                const link = new VerumTerminalLink(startIndex, length, file, lineNum, col);
                links.push(link);
            }
        }

        return links;
    }

    handleTerminalLink(link: VerumTerminalLink): void {
        const uri = resolveVerumFilePath(link.file);
        const position = new vscode.Position(Math.max(0, link.line - 1), Math.max(0, link.col - 1));
        vscode.window.showTextDocument(uri, {
            selection: new vscode.Range(position, position),
            preview: true,
        });
    }
}

class VerumTerminalLink extends vscode.TerminalLink {
    constructor(
        startIndex: number,
        length: number,
        public readonly file: string,
        public readonly line: number,
        public readonly col: number,
    ) {
        super(startIndex, length, `Open ${file}:${line}:${col}`);
    }
}

function resolveVerumFilePath(file: string): vscode.Uri {
    // If absolute, use directly
    if (file.startsWith('/') || /^[a-zA-Z]:/.test(file)) {
        return vscode.Uri.file(file);
    }
    // Otherwise resolve relative to workspace
    const wsFolder = vscode.workspace.workspaceFolders?.[0];
    if (wsFolder) {
        return vscode.Uri.joinPath(wsFolder.uri, file);
    }
    return vscode.Uri.file(file);
}

function registerTerminalLinkProvider(context: vscode.ExtensionContext): void {
    context.subscriptions.push(
        vscode.window.registerTerminalLinkProvider(new VerumTerminalLinkProvider())
    );
}

// ---------------------------------------------------------------------------
// File decoration provider (verified files)
// ---------------------------------------------------------------------------

class VerumFileDecorationProvider implements vscode.FileDecorationProvider {
    private readonly _onDidChangeFileDecorations = new vscode.EventEmitter<vscode.Uri | vscode.Uri[] | undefined>();
    readonly onDidChangeFileDecorations = this._onDidChangeFileDecorations.event;

    private verifiedFiles = new Set<string>();

    provideFileDecoration(uri: vscode.Uri): vscode.FileDecoration | undefined {
        if (!uri.fsPath.endsWith('.vr')) { return undefined; }
        if (this.verifiedFiles.has(uri.toString())) {
            return new vscode.FileDecoration(
                '\u2713', // checkmark
                'Verified',
                new vscode.ThemeColor('charts.green'),
            );
        }
        return undefined;
    }

    markVerified(uri: vscode.Uri): void {
        this.verifiedFiles.add(uri.toString());
        this._onDidChangeFileDecorations.fire(uri);
    }

    markUnverified(uri: vscode.Uri): void {
        if (this.verifiedFiles.delete(uri.toString())) {
            this._onDidChangeFileDecorations.fire(uri);
        }
    }

    dispose(): void {
        this._onDidChangeFileDecorations.dispose();
    }
}

let fileDecorationProvider: VerumFileDecorationProvider;

function registerFileDecorationProvider(context: vscode.ExtensionContext): void {
    fileDecorationProvider = new VerumFileDecorationProvider();
    context.subscriptions.push(
        vscode.window.registerFileDecorationProvider(fileDecorationProvider)
    );

    // Update decorations when diagnostics change
    context.subscriptions.push(
        vscode.languages.onDidChangeDiagnostics(e => {
            for (const uri of e.uris) {
                if (!uri.fsPath.endsWith('.vr')) { continue; }
                const diagnostics = vscode.languages.getDiagnostics(uri);
                const hasErrors = diagnostics.some(d => d.severity === vscode.DiagnosticSeverity.Error);
                if (hasErrors) {
                    fileDecorationProvider.markUnverified(uri);
                } else if (diagnostics.length === 0) {
                    fileDecorationProvider.markVerified(uri);
                }
            }
        })
    );
}

// ---------------------------------------------------------------------------
// Debug adapter
// ---------------------------------------------------------------------------

function registerDebugAdapter(context: vscode.ExtensionContext): void {
    const configProvider = new VerumDebugConfigurationProvider();
    context.subscriptions.push(
        vscode.debug.registerDebugConfigurationProvider('verum', configProvider)
    );

    const descriptorFactory = new VerumDebugAdapterDescriptorFactory();
    context.subscriptions.push(
        vscode.debug.registerDebugAdapterDescriptorFactory('verum', descriptorFactory)
    );
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

function registerCommands(context: vscode.ExtensionContext): void {
    const register = (id: string, handler: (...args: unknown[]) => unknown) => {
        context.subscriptions.push(vscode.commands.registerCommand(id, handler));
    };

    // --- Server management ---

    register('verum.restartLanguageServer', async () => {
        if (client) {
            crashCount = 0;
            statusBarItem.text = '$(loading~spin) Verum: Restarting...';
            try {
                await client.restart();
                vscode.window.showInformationMessage('Verum Language Server restarted.');
            } catch (error) {
                outputChannel.appendLine(`[ERROR] Restart failed: ${error}`);
                vscode.window.showErrorMessage(`Failed to restart: ${error}`);
            }
        } else {
            await startLanguageClient(context);
        }
    });

    register('verum.showServerStatus', () => {
        const running = client && client.state === State.Running;
        const items: string[] = [
            `Status: ${running ? 'Running' : 'Stopped'}`,
            `Crash count: ${crashCount}`,
        ];
        outputChannel.appendLine(`[INFO] Server status: ${items.join(', ')}`);
        if (running) {
            vscode.window.showInformationMessage(`Verum Language Server: Running`);
        } else {
            vscode.window.showWarningMessage(
                `Verum Language Server: Stopped`,
                'Restart',
            ).then(action => {
                if (action === 'Restart') {
                    vscode.commands.executeCommand('verum.restartLanguageServer');
                }
            });
        }
    });

    // --- Run / Test ---

    register('verum.runFile', async () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || editor.document.languageId !== 'verum') {
            vscode.window.showWarningMessage('Open a .vr file first.');
            return;
        }
        await editor.document.save();
        const config = vscode.workspace.getConfiguration('verum.lsp');
        const verum = config.get<string>('serverPath') || 'verum';
        const terminal = getOrCreateTerminal('Verum Run');
        terminal.show();
        terminal.sendText(`${verum} run "${editor.document.uri.fsPath}"`);
    });

    register('verum.runTest', async () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || editor.document.languageId !== 'verum') {
            vscode.window.showWarningMessage('Open a .vr file first.');
            return;
        }
        await editor.document.save();
        const config = vscode.workspace.getConfiguration('verum.lsp');
        const verum = config.get<string>('serverPath') || 'verum';

        // Try to find test function name at cursor
        const position = editor.selection.active;
        const line = editor.document.lineAt(position.line).text;
        const testMatch = /fn\s+(\w+)/.exec(line);
        const testName = testMatch ? testMatch[1] : undefined;

        const terminal = getOrCreateTerminal('Verum Test');
        terminal.show();
        if (testName) {
            terminal.sendText(`${verum} test "${editor.document.uri.fsPath}" --filter "${testName}"`);
        } else {
            terminal.sendText(`${verum} test "${editor.document.uri.fsPath}"`);
        }
    });

    // --- Verification ---

    register('verum.verifyFunction', async () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || !client) {
            vscode.window.showWarningMessage('No active editor or language server not running.');
            return;
        }

        const position = editor.selection.active;
        await vscode.window.withProgress({
            location: vscode.ProgressLocation.Notification,
            title: 'Verifying function contracts...',
            cancellable: false,
        }, async () => {
            try {
                const result = await client!.sendRequest(verifyFunctionRequest, {
                    textDocument: { uri: editor.document.uri.toString() },
                    position: { line: position.line, character: position.character },
                });
                if (result.verified) {
                    vscode.window.showInformationMessage(
                        `${result.functionName}: ${result.contracts} contract(s) verified in ${result.elapsed}ms`
                    );
                    fileDecorationProvider?.markVerified(editor.document.uri);
                } else {
                    const errMsg = result.errors?.join('\n') || 'Verification failed.';
                    vscode.window.showErrorMessage(`${result.functionName}: ${errMsg}`);
                }
            } catch (error) {
                vscode.window.showErrorMessage(`Verification failed: ${error}`);
            }
        });
    });

    // --- Navigation delegates ---

    register('verum.findReferences', async () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor) { return; }
        await vscode.commands.executeCommand(
            'editor.action.referenceSearch.trigger',
        );
    });

    register('verum.showImplementations', async () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor) { return; }
        await vscode.commands.executeCommand(
            'editor.action.goToImplementation',
        );
    });

    register('verum.showTypeHierarchy', async () => {
        await vscode.commands.executeCommand(
            'editor.showTypeHierarchy',
        );
    });

    register('verum.formatDocument', async () => {
        await vscode.commands.executeCommand(
            'editor.action.formatDocument',
        );
    });

    // --- Refinement / CBGR commands (preserved from original) ---

    register('verum.promoteToChecked', async () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || !client) { return; }

        const selection = editor.selection;
        const range: LSPRange = {
            start: { line: selection.start.line, character: selection.start.character },
            end: { line: selection.end.line, character: selection.end.character },
        };

        try {
            const result = await client.sendRequest(promoteToCheckedRequest, {
                textDocument: { uri: editor.document.uri.toString() },
                range,
                includeProof: true,
            });

            if (result.success) {
                const edit = new vscode.WorkspaceEdit();
                for (const e of result.edits) {
                    edit.replace(
                        editor.document.uri,
                        lspRangeToVSCode(e.range),
                        e.newText,
                    );
                }
                if (result.proofComment) {
                    const pos = new vscode.Position(selection.start.line, 0);
                    edit.insert(editor.document.uri, pos, result.proofComment + '\n');
                }
                await vscode.workspace.applyEdit(edit);
                vscode.window.showInformationMessage('Promoted to &checked reference.');
            } else {
                vscode.window.showWarningMessage('Cannot promote: escape analysis could not prove safety.');
            }
        } catch (error) {
            vscode.window.showErrorMessage(`Failed to promote to &checked: ${error}`);
        }
    });

    register('verum.addRuntimeCheck', async () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || !client) {
            vscode.window.showWarningMessage('No active Verum editor or language server not running.');
            return;
        }

        const selection = editor.selection;
        const range: LSPRange = {
            start: { line: selection.start.line, character: selection.start.character },
            end: { line: selection.end.line, character: selection.end.character },
        };

        try {
            const actions = await client.sendRequest('textDocument/codeAction', {
                textDocument: { uri: editor.document.uri.toString() },
                range,
                context: { diagnostics: [] },
            }) as Array<{ title: string; kind: string; edit?: { changes: Record<string, TextEditLSP[]> } }>;

            const runtimeCheckAction = actions?.find(a =>
                a.title.toLowerCase().includes('runtime check') ||
                a.kind === 'quickfix.runtime_check'
            );

            if (runtimeCheckAction?.edit) {
                const edit = new vscode.WorkspaceEdit();
                for (const [uri, edits] of Object.entries(runtimeCheckAction.edit.changes)) {
                    for (const e of edits) {
                        edit.replace(vscode.Uri.parse(uri), lspRangeToVSCode(e.range), e.newText);
                    }
                }
                await vscode.workspace.applyEdit(edit);
                vscode.window.showInformationMessage('Runtime check added.');
            } else {
                await vscode.commands.executeCommand('editor.action.quickFix');
            }
        } catch (error) {
            vscode.window.showErrorMessage(`Failed to add runtime check: ${error}`);
        }
    });

    register('verum.inferRefinement', async () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || !client) { return; }

        const position = editor.selection.active;
        const wordRange = editor.document.getWordRangeAtPosition(position);
        if (!wordRange) { return; }

        const symbol = editor.document.getText(wordRange);

        try {
            const result = await client.sendRequest(inferRefinementRequest, {
                textDocument: { uri: editor.document.uri.toString() },
                symbol,
            });

            const message = `Inferred type: ${result.inferredType} (${result.confidence} confidence)`;
            const action = await vscode.window.showInformationMessage(message, 'Apply', 'Show Usages');

            if (action === 'Apply') {
                const edit = new vscode.WorkspaceEdit();
                for (const e of result.edits) {
                    edit.replace(editor.document.uri, lspRangeToVSCode(e.range), e.newText);
                }
                await vscode.workspace.applyEdit(edit);
            } else if (action === 'Show Usages') {
                const locations = result.usages.map(u =>
                    new vscode.Location(
                        vscode.Uri.parse(u.uri),
                        lspRangeToVSCode(u.range),
                    )
                );
                await vscode.commands.executeCommand(
                    'editor.action.showReferences',
                    editor.document.uri,
                    position,
                    locations,
                );
            }
        } catch (error) {
            vscode.window.showErrorMessage(`Failed to infer refinement: ${error}`);
        }
    });

    register('verum.validateRefinementAtCursor', async () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || !client) { return; }

        const position = editor.selection.active;
        const lspPosition: LSPPosition = { line: position.line, character: position.character };

        try {
            const result = await client.sendRequest(validateRefinementRequest, {
                textDocument: { uri: editor.document.uri.toString() },
                position: lspPosition,
                mode: 'thorough',
            });

            if (result.valid) {
                vscode.window.showInformationMessage(
                    `Refinement is valid (verified in ${result.performanceMs}ms).`
                );
            } else {
                const diagnostic = result.diagnostics[0];
                if (diagnostic?.counterexample) {
                    const ce = diagnostic.counterexample;
                    vscode.window.showErrorMessage(
                        `Counterexample: ${ce.variable} = ${ce.value}\n${ce.violationReason}`
                    );
                } else {
                    vscode.window.showWarningMessage('Refinement validation failed.');
                }
            }
        } catch (error) {
            vscode.window.showErrorMessage(`Validation failed: ${error}`);
        }
    });

    // --- Profiling commands ---

    register('verum.showVerificationProfile', async () => {
        await runProfileCommand('verification');
    });

    register('verum.showCbgrProfile', async () => {
        await runProfileCommand('cbgr');
    });

    register('verum.openProfile', async () => {
        await vscode.commands.executeCommand('verum.profileDashboard.focus');
        await runProfileCommand('all');
    });

    register('verum.profileCurrentFile', async () => {
        await runProfileCommand('all');
    });
}

// ---------------------------------------------------------------------------
// Profiling helpers
// ---------------------------------------------------------------------------

async function runProfileCommand(kind: 'verification' | 'cbgr' | 'all'): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'verum') {
        vscode.window.showWarningMessage('Open a .vr file first.');
        return;
    }

    await vscode.window.withProgress({
        location: vscode.ProgressLocation.Notification,
        title: `Running ${kind} profiler...`,
        cancellable: false,
    }, async () => {
        if (!client) {
            const config = vscode.workspace.getConfiguration('verum.lsp');
            const verum = config.get<string>('serverPath') || 'verum';
            const terminal = getOrCreateTerminal('Verum Profile');
            terminal.show();
            const flag = kind === 'cbgr' ? '--memory' : kind === 'verification' ? '--verify' : '--all';
            terminal.sendText(`${verum} profile ${flag} "${editor.document.uri.fsPath}"`);
            return;
        }

        try {
            const profileData = await client.sendRequest(getProfileRequest, {
                textDocument: { uri: editor.document.uri.toString() },
            });
            dashboardProvider?.updateProfile(profileData);
            await vscode.commands.executeCommand('verum.profileDashboard.focus');

            if (kind === 'verification') {
                const time = profileData.compilationMetrics?.verification || 0;
                vscode.window.showInformationMessage(`Verification time: ${(time / 1000).toFixed(1)}s`);
            } else if (kind === 'cbgr') {
                const overhead = profileData.runtimeMetrics?.cbgrOverhead || 0;
                const total = profileData.runtimeMetrics?.total || 1;
                const pct = ((overhead / total) * 100).toFixed(1);
                vscode.window.showInformationMessage(`CBGR overhead: ${overhead}ms (${pct}% of runtime)`);
            } else {
                vscode.window.showInformationMessage('Profile data loaded.');
            }
        } catch (error) {
            outputChannel.appendLine(`[WARN] LSP profile request failed: ${error}`);
            const config = vscode.workspace.getConfiguration('verum.lsp');
            const verum = config.get<string>('serverPath') || 'verum';
            const action = await vscode.window.showWarningMessage(
                'LSP profiling not available.',
                'Open Terminal',
            );
            if (action === 'Open Terminal') {
                const terminal = getOrCreateTerminal('Verum Profile');
                terminal.show();
                const flag = kind === 'cbgr' ? '--memory' : kind === 'verification' ? '--verify' : '--all';
                terminal.sendText(`${verum} profile ${flag} "${editor.document.uri.fsPath}"`);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

function lspRangeToVSCode(range: LSPRange): vscode.Range {
    return new vscode.Range(
        new vscode.Position(range.start.line, range.start.character),
        new vscode.Position(range.end.line, range.end.character),
    );
}

const terminalCache = new Map<string, vscode.Terminal>();

function getOrCreateTerminal(name: string): vscode.Terminal {
    // Clean dead terminals
    for (const [key, term] of terminalCache) {
        if (term.exitStatus !== undefined) {
            terminalCache.delete(key);
        }
    }
    let terminal = terminalCache.get(name);
    if (!terminal || terminal.exitStatus !== undefined) {
        terminal = vscode.window.createTerminal({ name });
        terminalCache.set(name, terminal);
    }
    return terminal;
}
