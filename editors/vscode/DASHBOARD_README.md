# Verum VS Code Extension - WebView Dashboard

## Overview

This document describes the WebView dashboard implementation for the Verum VS Code extension. The dashboard provides real-time performance profiling and actionable recommendations for Verum code.

## Implementation Status

**Status**: Complete ✅
**Files Modified/Created**: 5
**TypeScript Compilation**: Passing ✅

## Files

### 1. `/src/dashboardWebview.ts` (NEW)
Complete implementation of the dashboard webview provider with:
- WebView lifecycle management
- Message passing between extension and webview
- Profile data updates
- Quick fix application
- Navigation to hot spots
- Profile export functionality

**Key Classes**:
- `DashboardWebviewProvider`: Main webview provider implementing `vscode.WebviewViewProvider`
- Interfaces: `ProfileData`, `CompilationMetrics`, `RuntimeMetrics`, `HotSpot`, `Recommendation`

### 2. `/src/webview/dashboard.css` (NEW)
Complete CSS styling for the dashboard with:
- VS Code theme integration using CSS variables
- Responsive grid layout for metric cards
- Hot spots table styling
- Recommendations list styling
- Code change diff display
- Progress bars and badges
- Mobile-responsive breakpoints

**Theme Variables Used**:
- `--vscode-foreground`
- `--vscode-editor-background`
- `--vscode-button-background`
- `--vscode-panel-border`
- And 20+ other VS Code theme variables

### 3. `/src/extension.ts` (MODIFIED)
Added dashboard integration:
- Import and initialize `DashboardWebviewProvider`
- Register webview view provider
- New commands:
  - `verum.openProfile`: Opens dashboard with profiling
  - `verum.profileCurrentFile`: Profiles current file and shows dashboard
- Mock data generation function for testing

### 4. `/package.json` (MODIFIED)
Added dashboard contributions:
- New view: `verum.profileDashboard` in explorer sidebar
- New commands for opening and profiling
- Editor title menu item for quick profiling
- Command palette entries

### 5. `/src/webview/dashboard.html` (EMBEDDED)
HTML template embedded in TypeScript with:
- Summary metrics section
- Hot spots table with sorting
- Recommendations with code diffs
- Interactive buttons (refresh, export, apply fix, navigate)
- Empty state placeholder
- JavaScript for dynamic rendering

## Features

### 1. Summary Metrics
Displays key performance metrics:
- **Compilation Time**: Total time and breakdown (parsing, type checking, verification, codegen)
- **Runtime Performance**: Total time and CBGR overhead
- **Hot Spots Count**: Number of functions requiring optimization
- **Recommendations Count**: Number of actionable improvements

### 2. Hot Spots Table
Sortable table showing performance bottlenecks:
- Function name and location
- Type (Verification, CBGR, Codegen)
- Time spent
- Impact percentage with visual progress bar
- Navigate button to jump to code

### 3. Recommendations
Actionable improvement suggestions with:
- Priority badges (high, medium, low)
- Type icons (optimization, refactor, cbgr, verification, cache)
- Description and impact estimate
- Before/after code diffs
- "Apply Fix" button for auto-fixable recommendations

### 4. Interactive Features
- **Refresh**: Re-run profiling
- **Export**: Save profile data as JSON
- **Navigate**: Jump to hot spot in code
- **Apply Fix**: Automatically apply recommended code changes

## Usage

### Opening the Dashboard

1. **From Command Palette**:
   ```
   Ctrl/Cmd + Shift + P → "Verum: Open Profile Dashboard"
   ```

2. **From Editor Title**:
   - Open a `.vr` file
   - Click the profile icon in editor title bar

3. **Automatic View**:
   - Dashboard appears in Explorer sidebar as "Verum Profile"

### Profiling a File

1. Open a Verum file (`.vr`)
2. Run: `Verum: Profile Current File`
3. Dashboard updates with profiling data
4. Review hot spots and recommendations
5. Click "Apply Fix" on auto-fixable recommendations
6. Click "Navigate" to jump to hot spots

### Exporting Profile Data

1. Click "Export" button in dashboard
2. Choose save location
3. Profile saved as JSON file

## Data Flow

```
┌─────────────────┐
│  User Action    │
│  (Profile File) │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Extension     │
│  (extension.ts) │ ───► Run profiling command
└────────┬────────┘     (or LSP request)
         │
         ▼
┌─────────────────┐
│  Profile Data   │
│  (JSON)         │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ DashboardWebview│
│   Provider      │ ───► postMessage()
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Webview       │
│  (HTML/CSS/JS)  │ ───► Render dashboard
└─────────────────┘
```

## Mock Data (For Testing)

The current implementation includes mock profile data in `extension.ts`:
```typescript
function generateMockProfileData(uri: vscode.Uri): ProfileData
```

This generates realistic sample data for:
- Compilation metrics (parsing: 2.1s, verification: 28.3s, etc.)
- Runtime metrics (business logic: 2.18s, CBGR: 0.16s)
- 2 hot spots (complex_algorithm, process_matrix)
- 3 recommendations (split function, use &checked, enable cache)

**Production Integration**: Replace mock data with actual LSP requests:
```typescript
const result = await client.sendRequest('verum/getProfile', {
    textDocument: { uri: editor.document.uri.toString() }
});
dashboardProvider?.updateProfile(result);
```

## LSP Integration (Future)

The dashboard is designed to integrate with the Verum LSP server via custom protocol extension:

### New LSP Method: `verum/getProfile`

**Request**:
```typescript
interface GetProfileParams {
    textDocument: TextDocumentIdentifier;
    options?: {
        includeCompilation?: boolean;
        includeRuntime?: boolean;
        includeCBGR?: boolean;
    };
}
```

**Response**:
```typescript
interface GetProfileResult {
    compilationMetrics: CompilationMetrics;
    runtimeMetrics: RuntimeMetrics;
    hotSpots: HotSpot[];
    recommendations: Recommendation[];
}
```

## Architecture

### Component Hierarchy

```
DashboardWebviewProvider
├── Webview (HTML)
│   ├── Header (title, actions)
│   ├── Summary Section
│   │   └── Metric Cards (4x grid)
│   ├── Hot Spots Section
│   │   └── Table (sortable)
│   └── Recommendations Section
│       └── Recommendation Cards
└── Message Handlers
    ├── applyFix
    ├── refresh
    ├── navigateToHotSpot
    └── exportProfile
```

### Message Protocol

**Extension → Webview**:
```typescript
{
    type: 'updateProfile',
    data: ProfileData
}
```

**Webview → Extension**:
```typescript
// Apply fix
{ type: 'applyFix', fix: { index: number } }

// Refresh profile
{ type: 'refresh' }

// Navigate to hot spot
{ type: 'navigateToHotSpot', hotspot: { location: string } }

// Export profile
{ type: 'exportProfile' }
```

## Styling

The dashboard uses VS Code's theme system for consistent appearance:

### Color Variables
- Background: `--vscode-editor-background`
- Foreground: `--vscode-foreground`
- Borders: `--vscode-panel-border`
- Buttons: `--vscode-button-background`
- Errors: `--vscode-errorForeground`
- Warnings: `--vscode-editorWarning-foreground`

### Layout
- Grid system for responsive metric cards
- Flexbox for recommendation layout
- CSS Grid for before/after code diffs
- Table layout for hot spots

### Responsive Design
- Desktop: 4-column metric grid
- Tablet: 2-column metric grid
- Mobile: 1-column metric grid
- Table: Condensed padding on small screens

## Security

The webview implements security best practices:

1. **Content Security Policy (CSP)**:
   ```html
   <meta http-equiv="Content-Security-Policy"
         content="default-src 'none';
                  style-src ${webview.cspSource} 'unsafe-inline';
                  script-src 'nonce-${nonce}';">
   ```

2. **Nonce-based Script Execution**: All inline scripts use nonces
3. **Local Resource Roots**: Restricted to extension directory
4. **HTML Escaping**: All user data escaped before rendering

## Testing

### Manual Testing Checklist

- [ ] Dashboard appears in Explorer sidebar
- [ ] "Profile Current File" command works
- [ ] Summary metrics display correctly
- [ ] Hot spots table renders
- [ ] Recommendations display with badges
- [ ] "Navigate" button jumps to correct location
- [ ] "Apply Fix" applies code changes
- [ ] "Refresh" re-runs profiling
- [ ] "Export" saves JSON file
- [ ] Dark theme styling works
- [ ] Light theme styling works
- [ ] Responsive layout on narrow panels

### TypeScript Compilation

```bash
cd editors/vscode
npm run compile
```

**Expected Output**: No errors ✅

### Linting

```bash
cd editors/vscode
npm run lint
```

## Known Limitations

1. **Mock Data**: Currently using mock data instead of real profiling
2. **LSP Integration**: Requires LSP server implementation of `verum/getProfile`
3. **Auto-Fix Validation**: No validation of fix safety before applying
4. **Caching**: No caching of profile data between sessions
5. **Incremental Updates**: Full profile refresh on each run

## Future Enhancements

1. **Real-time Updates**: Stream profiling data as it's collected
2. **Historical Trends**: Track performance over time
3. **Comparison Mode**: Compare profiles before/after changes
4. **Custom Filters**: Filter hot spots by type, threshold, etc.
5. **Export Formats**: Support CSV, HTML report exports
6. **Interactive Charts**: Add Chart.js for visual trends
7. **Quick Fix Preview**: Show diff preview before applying
8. **Batch Fixes**: Apply multiple recommendations at once

## Reference

**Specification**: `/docs/detailed/25-developer-tooling.md` Section 3.6

**Related Files**:
- `/crates/verum_compiler/src/unified_dashboard.rs` (Rust implementation)
- `/crates/verum_compiler/src/verification_config.rs` (Config)
- `/crates/verum_compiler/src/profile_cmd.rs` (Profiling command)

## Troubleshooting

### Dashboard Not Appearing
- Check Explorer sidebar for "Verum Profile" view
- Run: `Developer: Reload Window`

### Compilation Errors
- Run: `npm install` to ensure dependencies are installed
- Check TypeScript version: `npx tsc --version` (should be 5.0+)

### Webview Not Updating
- Check browser console in webview DevTools
- Verify message passing in extension host logs
- Run: `Developer: Open Webview Developer Tools`

### CSP Errors
- Check nonce generation
- Verify CSP meta tag
- Review browser console for CSP violations

## Contributing

When modifying the dashboard:

1. Update TypeScript interfaces if data structure changes
2. Maintain theme variable usage for styling
3. Test in both light and dark themes
4. Update this README if features change
5. Run `npm run compile` to verify TypeScript
6. Test message passing between extension and webview

## Version History

- **v1.0.0** (2025-12-20): Initial implementation
  - Basic dashboard with metrics, hot spots, recommendations
  - Mock data support
  - Interactive features (navigate, apply fix, export)
  - Complete VS Code theme integration
