import * as vscode from 'vscode';

export class DashboardWebviewProvider implements vscode.WebviewViewProvider {
    public static readonly viewType = 'verum.profileDashboard';

    private _view?: vscode.WebviewView;
    private _profileData: ProfileData | null = null;

    constructor(
        private readonly _extensionUri: vscode.Uri,
        private readonly _context: vscode.ExtensionContext
    ) {
        // Store context for potential future use
        void this._context;
    }

    public resolveWebviewView(
        webviewView: vscode.WebviewView,
        _context: vscode.WebviewViewResolveContext,
        _token: vscode.CancellationToken
    ) {
        this._view = webviewView;

        webviewView.webview.options = {
            enableScripts: true,
            localResourceRoots: [
                vscode.Uri.joinPath(this._extensionUri, 'src', 'webview')
            ]
        };

        webviewView.webview.html = this._getHtmlForWebview(webviewView.webview);

        // Handle messages from webview
        webviewView.webview.onDidReceiveMessage(async data => {
            switch (data.type) {
                case 'applyFix':
                    await this.applyQuickFix(data.fix);
                    break;
                case 'refresh':
                    await this.refresh();
                    break;
                case 'navigateToHotSpot':
                    await this.navigateToHotSpot(data.hotspot);
                    break;
                case 'exportProfile':
                    await this.exportProfile();
                    break;
            }
        });
    }

    public updateProfile(data: ProfileData) {
        this._profileData = data;
        if (this._view) {
            this._view.webview.postMessage({
                type: 'updateProfile',
                data: data
            });
        }
    }

    private _getHtmlForWebview(webview: vscode.Webview): string {
        // Get URIs for CSS and HTML
        const styleUri = webview.asWebviewUri(
            vscode.Uri.joinPath(this._extensionUri, 'src', 'webview', 'dashboard.css')
        );

        const nonce = getNonce();

        return `<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}';">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link href="${styleUri}" rel="stylesheet">
    <title>Verum Profile Dashboard</title>
</head>
<body>
    <div class="dashboard">
        <div class="header">
            <h1>Verum Performance Profile</h1>
            <div class="actions">
                <button id="refreshBtn" class="btn btn-primary">
                    <span class="codicon codicon-refresh"></span> Refresh
                </button>
                <button id="exportBtn" class="btn btn-secondary">
                    <span class="codicon codicon-export"></span> Export
                </button>
            </div>
        </div>

        <div id="content" class="content">
            <div class="empty-state">
                <div class="empty-icon">📊</div>
                <h2>No profile data available</h2>
                <p>Run profiling on a file or project to see performance metrics</p>
                <button id="runProfileBtn" class="btn btn-primary">
                    <span class="codicon codicon-play"></span> Run Profile
                </button>
            </div>
        </div>
    </div>

    <script nonce="${nonce}">
        const vscode = acquireVsCodeApi();

        // Button event handlers
        document.getElementById('refreshBtn').addEventListener('click', () => {
            vscode.postMessage({ type: 'refresh' });
        });

        document.getElementById('exportBtn').addEventListener('click', () => {
            vscode.postMessage({ type: 'exportProfile' });
        });

        document.getElementById('runProfileBtn').addEventListener('click', () => {
            vscode.postMessage({ type: 'refresh' });
        });

        // Listen for messages from extension
        window.addEventListener('message', event => {
            const message = event.data;

            switch (message.type) {
                case 'updateProfile':
                    updateDashboard(message.data);
                    break;
            }
        });

        function updateDashboard(data) {
            const content = document.getElementById('content');

            content.innerHTML = \`
                <!-- Summary Section -->
                <section class="summary-section">
                    <h2>Summary</h2>
                    <div class="metric-cards">
                        <div class="metric-card">
                            <div class="metric-label">Compilation Time</div>
                            <div class="metric-value">\${formatDuration(data.compilationMetrics.total)}</div>
                            <div class="metric-breakdown">
                                <div class="breakdown-item">
                                    <span>Parsing:</span>
                                    <span>\${formatDuration(data.compilationMetrics.parsing)}</span>
                                </div>
                                <div class="breakdown-item">
                                    <span>Type Checking:</span>
                                    <span>\${formatDuration(data.compilationMetrics.typeChecking)}</span>
                                </div>
                                <div class="breakdown-item">
                                    <span>Verification:</span>
                                    <span>\${formatDuration(data.compilationMetrics.verification)}</span>
                                </div>
                                <div class="breakdown-item">
                                    <span>Codegen:</span>
                                    <span>\${formatDuration(data.compilationMetrics.codegen)}</span>
                                </div>
                            </div>
                        </div>

                        <div class="metric-card">
                            <div class="metric-label">Runtime Performance</div>
                            <div class="metric-value">\${formatDuration(data.runtimeMetrics.total)}</div>
                            <div class="metric-breakdown">
                                <div class="breakdown-item">
                                    <span>Business Logic:</span>
                                    <span>\${formatDuration(data.runtimeMetrics.businessLogic)}</span>
                                </div>
                                <div class="breakdown-item">
                                    <span>CBGR Overhead:</span>
                                    <span>\${formatDuration(data.runtimeMetrics.cbgrOverhead)} (\${formatPercent(data.runtimeMetrics.cbgrOverhead, data.runtimeMetrics.total)})</span>
                                </div>
                            </div>
                        </div>

                        <div class="metric-card">
                            <div class="metric-label">Hot Spots</div>
                            <div class="metric-value">\${data.hotSpots.length}</div>
                            <div class="metric-sublabel">Functions requiring optimization</div>
                        </div>

                        <div class="metric-card">
                            <div class="metric-label">Recommendations</div>
                            <div class="metric-value">\${data.recommendations.length}</div>
                            <div class="metric-sublabel">Actionable improvements</div>
                        </div>
                    </div>
                </section>

                <!-- Hot Spots Section -->
                <section class="hotspots-section">
                    <h2>Hot Spots</h2>
                    \${data.hotSpots.length > 0 ? renderHotSpots(data.hotSpots) : '<p class="empty-message">No hot spots detected</p>'}
                </section>

                <!-- Recommendations Section -->
                <section class="recommendations-section">
                    <h2>Recommendations</h2>
                    \${data.recommendations.length > 0 ? renderRecommendations(data.recommendations) : '<p class="empty-message">No recommendations available</p>'}
                </section>
            \`;

            // Attach event listeners
            attachEventListeners();
        }

        function renderHotSpots(hotSpots) {
            return \`
                <table class="hotspots-table">
                    <thead>
                        <tr>
                            <th>Function</th>
                            <th>File</th>
                            <th>Type</th>
                            <th>Time</th>
                            <th>Impact</th>
                            <th>Actions</th>
                        </tr>
                    </thead>
                    <tbody>
                        \${hotSpots.map(hs => \`
                            <tr class="hotspot-row" data-location="\${hs.location}">
                                <td class="function-name">\${escapeHtml(hs.functionName)}</td>
                                <td class="file-path">\${escapeHtml(hs.file)}:\${hs.line}</td>
                                <td>
                                    <span class="badge badge-\${hs.type.toLowerCase()}">\${hs.type}</span>
                                </td>
                                <td>\${formatDuration(hs.time)}</td>
                                <td>
                                    <div class="progress-bar">
                                        <div class="progress-fill" style="width: \${hs.impactPercent}%"></div>
                                        <span class="progress-label">\${hs.impactPercent.toFixed(1)}%</span>
                                    </div>
                                </td>
                                <td>
                                    <button class="btn btn-small navigate-btn" data-location="\${hs.location}">
                                        <span class="codicon codicon-go-to-file"></span> Navigate
                                    </button>
                                </td>
                            </tr>
                        \`).join('')}
                    </tbody>
                </table>
            \`;
        }

        function renderRecommendations(recommendations) {
            return \`
                <div class="recommendations-list">
                    \${recommendations.map((rec, idx) => \`
                        <div class="recommendation \${rec.priority === 'high' ? 'high-priority' : ''}">
                            <div class="recommendation-header">
                                <span class="recommendation-icon">\${getRecommendationIcon(rec.type)}</span>
                                <h3>\${escapeHtml(rec.title)}</h3>
                                <span class="badge badge-\${rec.priority}">\${rec.priority}</span>
                            </div>
                            <div class="recommendation-body">
                                <p>\${escapeHtml(rec.description)}</p>
                                \${rec.codeChange ? \`
                                    <div class="code-change">
                                        <div class="code-before">
                                            <div class="code-label">Before:</div>
                                            <pre><code>\${escapeHtml(rec.codeChange.before)}</code></pre>
                                        </div>
                                        <div class="code-after">
                                            <div class="code-label">After:</div>
                                            <pre><code>\${escapeHtml(rec.codeChange.after)}</code></pre>
                                        </div>
                                    </div>
                                \` : ''}
                                <div class="recommendation-footer">
                                    <span class="impact-text">Impact: \${rec.impact}</span>
                                    \${rec.autoFixable ? \`
                                        <button class="btn btn-primary apply-fix-btn" data-fix-index="\${idx}">
                                            <span class="codicon codicon-wand"></span> Apply Fix
                                        </button>
                                    \` : ''}
                                </div>
                            </div>
                        </div>
                    \`).join('')}
                </div>
            \`;
        }

        function attachEventListeners() {
            // Navigate buttons
            document.querySelectorAll('.navigate-btn').forEach(btn => {
                btn.addEventListener('click', (e) => {
                    const location = e.currentTarget.getAttribute('data-location');
                    vscode.postMessage({
                        type: 'navigateToHotSpot',
                        hotspot: { location }
                    });
                });
            });

            // Apply fix buttons
            document.querySelectorAll('.apply-fix-btn').forEach(btn => {
                btn.addEventListener('click', (e) => {
                    const fixIndex = parseInt(e.currentTarget.getAttribute('data-fix-index'));
                    vscode.postMessage({
                        type: 'applyFix',
                        fix: { index: fixIndex }
                    });
                });
            });
        }

        function formatDuration(ms) {
            if (ms < 1000) {
                return \`\${ms.toFixed(1)}ms\`;
            } else if (ms < 60000) {
                return \`\${(ms / 1000).toFixed(2)}s\`;
            } else {
                const minutes = Math.floor(ms / 60000);
                const seconds = ((ms % 60000) / 1000).toFixed(0);
                return \`\${minutes}m \${seconds}s\`;
            }
        }

        function formatPercent(value, total) {
            return ((value / total) * 100).toFixed(1) + '%';
        }

        function escapeHtml(unsafe) {
            return unsafe
                .replace(/&/g, "&amp;")
                .replace(/</g, "&lt;")
                .replace(/>/g, "&gt;")
                .replace(/"/g, "&quot;")
                .replace(/'/g, "&#039;");
        }

        function getRecommendationIcon(type) {
            const icons = {
                'optimization': '⚡',
                'refactor': '🔧',
                'cbgr': '🔒',
                'verification': '✓',
                'cache': '💾'
            };
            return icons[type] || '💡';
        }
    </script>
</body>
</html>`;
    }

    private async applyQuickFix(fix: { index: number }) {
        if (!this._profileData || !this._profileData.recommendations[fix.index]) {
            vscode.window.showErrorMessage('Fix not found');
            return;
        }

        const recommendation = this._profileData.recommendations[fix.index];

        if (!recommendation.autoFixable || !recommendation.edits) {
            vscode.window.showWarningMessage('This fix cannot be applied automatically');
            return;
        }

        const edit = new vscode.WorkspaceEdit();

        for (const textEdit of recommendation.edits) {
            const uri = vscode.Uri.file(textEdit.file);
            const range = new vscode.Range(
                new vscode.Position(textEdit.range.start.line, textEdit.range.start.character),
                new vscode.Position(textEdit.range.end.line, textEdit.range.end.character)
            );

            edit.replace(uri, range, textEdit.newText);
        }

        const applied = await vscode.workspace.applyEdit(edit);

        if (applied) {
            vscode.window.showInformationMessage(`Applied: ${recommendation.title}`);
        } else {
            vscode.window.showErrorMessage('Failed to apply fix');
        }
    }

    private async refresh() {
        const editor = vscode.window.activeTextEditor;

        if (!editor) {
            vscode.window.showWarningMessage('No active editor');
            return;
        }

        // Show progress
        await vscode.window.withProgress({
            location: vscode.ProgressLocation.Notification,
            title: 'Running Verum profiler...',
            cancellable: false
        }, async (_progress) => {
            try {
                // Create terminal and run profiling command
                const terminal = vscode.window.createTerminal({
                    name: 'Verum Profile',
                    hideFromUser: true
                });

                const uri = editor.document.uri;
                terminal.sendText(`verum profile --all --export=json ${uri.fsPath}`);

                // In production, use LSP request instead:
                // const result = await client.sendRequest('verum/getProfile', {
                //     textDocument: { uri: uri.toString() }
                // });
                // this.updateProfile(result);

                // For now, show informational message
                vscode.window.showInformationMessage(
                    'Profile command sent. Results will appear when profiling completes.'
                );
            } catch (error) {
                vscode.window.showErrorMessage(`Profiling failed: ${error}`);
            }
        });
    }

    private async navigateToHotSpot(hotspot: { location: string }) {
        // Parse location string (format: "file:line:column")
        const parts = hotspot.location.split(':');
        if (parts.length < 2) {
            vscode.window.showErrorMessage('Invalid location format');
            return;
        }

        const file = parts[0];
        const line = parseInt(parts[1]) - 1;
        const column = parts.length > 2 ? parseInt(parts[2]) - 1 : 0;

        try {
            const uri = vscode.Uri.file(file);
            const document = await vscode.workspace.openTextDocument(uri);
            const editor = await vscode.window.showTextDocument(document);

            const position = new vscode.Position(line, column);
            editor.selection = new vscode.Selection(position, position);
            editor.revealRange(
                new vscode.Range(position, position),
                vscode.TextEditorRevealType.InCenter
            );
        } catch (error) {
            vscode.window.showErrorMessage(`Failed to navigate: ${error}`);
        }
    }

    private async exportProfile() {
        if (!this._profileData) {
            vscode.window.showWarningMessage('No profile data to export');
            return;
        }

        const uri = await vscode.window.showSaveDialog({
            defaultUri: vscode.Uri.file('verum-profile.json'),
            filters: {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                'JSON': ['json']
            }
        });

        if (uri) {
            const json = JSON.stringify(this._profileData, null, 2);
            await vscode.workspace.fs.writeFile(uri, Buffer.from(json, 'utf8'));
            vscode.window.showInformationMessage(`Profile exported to ${uri.fsPath}`);
        }
    }
}

// Interfaces
export interface ProfileData {
    compilationMetrics: CompilationMetrics;
    runtimeMetrics: RuntimeMetrics;
    hotSpots: HotSpot[];
    recommendations: Recommendation[];
}

export interface CompilationMetrics {
    total: number;
    parsing: number;
    typeChecking: number;
    verification: number;
    codegen: number;
}

export interface RuntimeMetrics {
    total: number;
    businessLogic: number;
    cbgrOverhead: number;
}

export interface HotSpot {
    functionName: string;
    file: string;
    line: number;
    location: string;
    type: 'Verification' | 'CBGR' | 'Codegen';
    time: number;
    impactPercent: number;
}

export interface Recommendation {
    title: string;
    description: string;
    type: 'optimization' | 'refactor' | 'cbgr' | 'verification' | 'cache';
    priority: 'high' | 'medium' | 'low';
    impact: string;
    autoFixable: boolean;
    codeChange?: {
        before: string;
        after: string;
    };
    edits?: TextEdit[];
}

export interface TextEdit {
    file: string;
    range: {
        start: { line: number; character: number };
        end: { line: number; character: number };
    };
    newText: string;
}

function getNonce() {
    let text = '';
    const possible = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
    for (let i = 0; i < 32; i++) {
        text += possible.charAt(Math.floor(Math.random() * possible.length));
    }
    return text;
}
