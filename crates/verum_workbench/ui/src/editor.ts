import { EditorState, Compartment } from '@codemirror/state';
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter, drawSelection, rectangularSelection } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search';
import { autocompletion, completionKeymap } from '@codemirror/autocomplete';
import { StreamLanguage, syntaxHighlighting, HighlightStyle, bracketMatching, indentOnInput } from '@codemirror/language';
import { tags } from '@lezer/highlight';

const keywords = new Set('let fn is type implement mount module cog pub public return if else match for while loop break continue async await spawn select using provide with mut const pure unsafe where theorem lemma axiom proof by forall exists invariant requires ensures decreases'.split(' '));
// Lexical coloring only. Compiler diagnostics and semantic claims come from Verum.
const verum = StreamLanguage.define({
  startState: () => ({ commentDepth: 0 }),
  token(stream, state) {
    if (state.commentDepth) {
      while (!stream.eol()) {
        if (stream.match('/*')) state.commentDepth++;
        else if (stream.match('*/')) { if (--state.commentDepth === 0) break; }
        else stream.next();
      }
      return 'comment';
    }
    if (stream.eatSpace()) return null;
    if (stream.match('//')) { stream.skipToEnd(); return 'comment'; }
    if (stream.match('/*')) { state.commentDepth = 1; return 'comment'; }
    if (stream.match(/(?:f|b)?"/)) {
      let escaped = false;
      while (!stream.eol()) {
        const char = stream.next();
        if (char === '"' && !escaped) break;
        escaped = char === '\\' && !escaped;
      }
      return 'string';
    }
    if (stream.match(/(?:0[xX][\da-fA-F_]+|0[bB][01_]+|\d[\d_]*(?:\.[\d_]+)?(?:[eE][+-]?\d+)?)/)) return 'number';
    if (stream.match(/@[\p{L}_][\p{L}\p{N}_]*/u)) return 'meta';
    const word = stream.match(/[\p{L}_][\p{L}\p{N}_]*/u);
    if (word && typeof word !== 'boolean') {
      if (keywords.has(word[0])) return 'keyword';
      if (/^(true|false)$/.test(word[0])) return 'bool';
      return /^\p{Lu}/u.test(word[0]) ? 'typeName' : 'variableName';
    }
    if (stream.match(/[+*/%=!<>:&|?~^-]+/)) return 'operator';
    stream.next(); return null;
  },
  languageData: { commentTokens: { line: '//', block: { open: '/*', close: '*/' } } },
});

const theme = new Compartment();
function editorTheme(dark: boolean) {
  return [EditorView.theme({
    '&': { color: 'var(--text)', backgroundColor: 'var(--editor)' },
    '.cm-cursor': { borderLeftColor: 'var(--accent)' },
    '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: dark ? '#ffffff04' : '#37654b04' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': { backgroundColor: dark ? '#49735180' : '#bcd8b885' },
    '.cm-tooltip': { backgroundColor: 'var(--surface)', borderColor: 'var(--border)', color: 'var(--text)' },
    '.cm-panels': { backgroundColor: 'var(--surface)', color: 'var(--text)' },
    '.cm-searchMatch': { backgroundColor: dark ? '#8a722d88' : '#f7d77777' },
  }, { dark }), syntaxHighlighting(HighlightStyle.define([
    { tag: tags.keyword, color: dark ? '#d1b1e8' : '#78538f' },
    { tag: [tags.string, tags.special(tags.string)], color: dark ? '#c2d897' : '#687a2c' },
    { tag: tags.comment, color: dark ? '#809384' : '#89968c', fontStyle: 'italic' },
    { tag: [tags.number, tags.bool], color: dark ? '#e6ba86' : '#a46330' },
    { tag: tags.typeName, color: dark ? '#9ccac4' : '#387575' },
    { tag: [tags.meta, tags.operator], color: dark ? '#b2c5b2' : '#536c58' },
  ]))];
}

export function makeEditor(parent: HTMLElement, source: string, onChange: (source: string) => void, onSelection: (line: number, column: number) => void) {
  const stateFor = (source: string) => EditorState.create({ doc: source, extensions: [
      lineNumbers(), highlightActiveLineGutter(), history(), drawSelection(), rectangularSelection(),
      highlightActiveLine(), highlightSelectionMatches(), bracketMatching(), indentOnInput(), verum,
      autocompletion({ override: [context => {
        const word = context.matchBefore(/[\p{L}_][\p{L}\p{N}_]*/u);
        if (!word || (!context.explicit && word.from === word.to)) return null;
        return { from: word.from, options: [...keywords].map(label => ({ label, type: 'keyword', detail: 'Verum keyword' })) };
      }] }),
      keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, ...completionKeymap]),
      EditorState.tabSize.of(4),
      EditorView.contentAttributes.of({ 'aria-label': 'Verum source code', spellcheck: 'false' }),
      theme.of(editorTheme(document.documentElement.dataset.theme === 'dark')),
      EditorView.updateListener.of(update => {
        if (update.docChanged) onChange(update.state.doc.toString());
        if (update.selectionSet || update.docChanged) {
          const offset = update.state.selection.main.head;
          const line = update.state.doc.lineAt(offset);
          onSelection(line.number, offset - line.from + 1);
        }
      }),
    ] });
  const view = new EditorView({ parent, state: stateFor(source) });
  return {
    view,
    source: () => view.state.doc.toString(),
    replace(source: string) {
      // Each opened file owns its undo history; an undo must never restore another file.
      view.setState(stateFor(source)); onSelection(1, 1);
    },
    theme(dark: boolean) { view.dispatch({ effects: theme.reconfigure(editorTheme(dark)) }); },
  };
}
