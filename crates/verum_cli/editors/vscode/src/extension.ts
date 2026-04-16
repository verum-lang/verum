import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;
let statusBar: vscode.StatusBarItem;

export function activate(context: vscode.ExtensionContext) {
  // Status bar
  statusBar = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Left,
    100
  );
  statusBar.text = "$(loading~spin) Verum";
  statusBar.tooltip = "Verum Language Server starting...";
  statusBar.show();
  context.subscriptions.push(statusBar);

  // Find the verum binary
  const config = vscode.workspace.getConfiguration("verum");
  const lspEnabled = config.get<boolean>("lsp.enable", true);

  if (!lspEnabled) {
    statusBar.text = "$(circle-slash) Verum (LSP disabled)";
    statusBar.tooltip = "Set verum.lsp.enable = true to activate";
    return;
  }

  startLanguageServer(context);

  // Commands
  context.subscriptions.push(
    vscode.commands.registerCommand("verum.restartServer", () => {
      deactivate().then(() => startLanguageServer(context));
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("verum.verify", () => {
      const editor = vscode.window.activeTextEditor;
      if (editor && editor.document.languageId === "verum") {
        const terminal = vscode.window.createTerminal("Verum Verify");
        terminal.sendText(`verum verify "${editor.document.fileName}"`);
        terminal.show();
      }
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("verum.showCosts", () => {
      const editor = vscode.window.activeTextEditor;
      if (editor && editor.document.languageId === "verum") {
        const terminal = vscode.window.createTerminal("Verum Profile");
        terminal.sendText(
          `verum profile "${editor.document.fileName}" --suggest`
        );
        terminal.show();
      }
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("verum.profile", () => {
      const editor = vscode.window.activeTextEditor;
      if (editor && editor.document.languageId === "verum") {
        const terminal = vscode.window.createTerminal("Verum Profile");
        terminal.sendText(
          `verum profile "${editor.document.fileName}" --memory --suggest`
        );
        terminal.show();
      }
    })
  );
}

function startLanguageServer(context: vscode.ExtensionContext) {
  // Try to find verum in PATH, then in known locations
  const serverCommand = findVerumBinary();

  const serverOptions: ServerOptions = {
    run: {
      command: serverCommand,
      args: ["lsp", "--transport", "stdio"],
      transport: TransportKind.stdio,
    },
    debug: {
      command: serverCommand,
      args: ["lsp", "--transport", "stdio"],
      transport: TransportKind.stdio,
    },
  };

  const config = vscode.workspace.getConfiguration("verum");
  const trace = config.get<string>("trace.server", "off");

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "verum" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.vr"),
    },
    outputChannelName: "Verum Language Server",
    traceOutputChannel:
      trace !== "off"
        ? vscode.window.createOutputChannel("Verum LSP Trace")
        : undefined,
  };

  client = new LanguageClient(
    "verum",
    "Verum Language Server",
    serverOptions,
    clientOptions
  );

  client.start().then(
    () => {
      statusBar.text = "$(check) Verum";
      statusBar.tooltip = "Verum Language Server running";
    },
    (err: Error) => {
      statusBar.text = "$(error) Verum";
      statusBar.tooltip = `Failed to start: ${err.message}`;
      vscode.window.showErrorMessage(
        `Verum LSP failed to start: ${err.message}. ` +
          `Make sure 'verum' is in your PATH.`
      );
    }
  );

  context.subscriptions.push(client);
}

function findVerumBinary(): string {
  const config = vscode.workspace.getConfiguration("verum");
  const customPath = config.get<string>("lsp.path");
  if (customPath) return customPath;

  // Default: assume verum is in PATH
  return "verum";
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}
