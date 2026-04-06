import * as vscode from 'vscode';

/**
 * Resolves the path to the verum CLI executable used for the DAP server.
 * Checks verum.debug.dapServerPath first, then verum.lsp.serverPath, then
 * falls back to "verum" (assumed on $PATH).
 */
function getVerumPath(): string {
    const debugConfig = vscode.workspace.getConfiguration('verum.debug');
    const dapPath = debugConfig.get<string>('dapServerPath');
    if (dapPath && dapPath.trim().length > 0) {
        return dapPath.trim();
    }
    const lspConfig = vscode.workspace.getConfiguration('verum.lsp');
    return lspConfig.get<string>('serverPath') || 'verum';
}

/**
 * Provides initial debug configurations when no launch.json exists.
 */
export class VerumDebugConfigurationProvider implements vscode.DebugConfigurationProvider {

    /**
     * Called when the user presses F5 with no launch.json.
     * Provides a default configuration.
     */
    resolveDebugConfiguration(
        _folder: vscode.WorkspaceFolder | undefined,
        config: vscode.DebugConfiguration,
        _token?: vscode.CancellationToken,
    ): vscode.ProviderResult<vscode.DebugConfiguration> {
        // If launch.json is missing or empty, provide a default config
        if (!config.type && !config.request && !config.name) {
            const editor = vscode.window.activeTextEditor;
            if (editor && editor.document.languageId === 'verum') {
                config.type = 'verum';
                config.name = 'Debug Verum Program';
                config.request = 'launch';
                config.program = '${file}';
                config.stopOnEntry = false;
            }
        }

        if (!config.program) {
            return vscode.window.showInformationMessage(
                'Cannot start debugging: no program specified in launch configuration.'
            ).then(_ => undefined);
        }

        // Apply defaults
        if (config.stopOnEntry === undefined) {
            config.stopOnEntry = false;
        }
        if (!config.args) {
            config.args = [];
        }
        if (!config.tier) {
            const debugConfig = vscode.workspace.getConfiguration('verum.debug');
            config.tier = debugConfig.get<string>('defaultTier') || 'interpreter';
        }
        if (!config.cwd) {
            config.cwd = '${workspaceFolder}';
        }

        return config;
    }
}

/**
 * Factory that creates the debug adapter process.
 *
 * The DAP server is launched as `verum dap --transport stdio`, which speaks
 * the Debug Adapter Protocol over stdin/stdout.
 */
export class VerumDebugAdapterDescriptorFactory implements vscode.DebugAdapterDescriptorFactory {

    createDebugAdapterDescriptor(
        _session: vscode.DebugSession,
        _executable: vscode.DebugAdapterExecutable | undefined,
    ): vscode.ProviderResult<vscode.DebugAdapterDescriptor> {
        const verumPath = getVerumPath();
        const args = ['dap', '--transport', 'stdio'];

        return new vscode.DebugAdapterExecutable(verumPath, args);
    }
}
