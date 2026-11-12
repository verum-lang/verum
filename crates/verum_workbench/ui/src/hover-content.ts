import MarkdownIt, { type Token } from 'markdown-it';

const markdown = new MarkdownIt({ html: false, linkify: false, typographer: false, maxNesting: 20 });
const tags = new Set('p blockquote ul ol li strong em s h1 h2 h3 h4 h5 h6 table thead tbody tr th td'.split(' '));

function codeBlock(parent: HTMLElement, source: string, language = '') {
  const pre = parent.ownerDocument.createElement('pre');
  const code = parent.ownerDocument.createElement('code');
  code.textContent = source;
  if (/^[\w+-]{1,64}$/.test(language)) code.dataset.language = language;
  pre.append(code); parent.append(pre);
}

function safeLink(href: string | number | null): string | null {
  if (typeof href !== 'string' || !href) return null;
  try {
    const url = new URL(href);
    return ['https:', 'http:', 'mailto:'].includes(url.protocol) ? url.href : null;
  } catch { return null; }
}

// Parse Markdown with CommonMark rules, then create only allowed DOM nodes.
// Source comments cannot introduce HTML, execute commands or load remote images.
function appendMarkdown(root: HTMLElement, source: string) {
  const parents = [root];
  const pending: { tokens: Token[]; index: number }[] = [{ tokens: markdown.parse(source, {}), index: 0 }];
  while (pending.length) {
    const frame = pending[pending.length - 1];
    if (frame.index === frame.tokens.length) { pending.pop(); continue; }
    const token = frame.tokens[frame.index++];
    const parent = parents[parents.length - 1];
    const document = root.ownerDocument;
    if (token.type === 'inline') {
      if (token.children) pending.push({ tokens: token.children, index: 0 });
    } else if (token.nesting === -1) {
      if (parents.length > 1) parents.pop();
    } else if (token.nesting === 1) {
      const href = token.tag === 'a' ? safeLink(token.attrGet('href')) : null;
      const element = document.createElement(href ? 'a' : tags.has(token.tag) ? token.tag : 'span');
      if (href) {
        element.setAttribute('href', href);
        element.setAttribute('target', '_blank');
        element.setAttribute('rel', 'noopener noreferrer');
        const title = token.attrGet('title');
        if (typeof title === 'string') element.title = title;
      }
      if (token.tag === 'ol') {
        const start = token.attrGet('start');
        if (start !== null && /^\d+$/.test(String(start))) element.setAttribute('start', String(start));
      }
      parent.append(element); parents.push(element);
    } else if (token.type === 'fence' || token.type === 'code_block') {
      codeBlock(parent, token.content, token.info.trim().split(/\s+/)[0]);
    } else if (token.type === 'code_inline') {
      const code = document.createElement('code'); code.textContent = token.content; parent.append(code);
    } else if (token.type === 'hardbreak' || token.type === 'hr') {
      parent.append(document.createElement(token.type === 'hr' ? 'hr' : 'br'));
    } else if (token.type === 'softbreak') {
      parent.append(document.createTextNode('\n'));
    } else {
      parent.append(document.createTextNode(token.content));
    }
  }
}

/** LSP HoverContents: MarkupContent, MarkedString, or a MarkedString array. */
export function renderHoverContent(contents: unknown, document: Document): HTMLElement | null {
  const root = document.createElement('div');
  root.className = 'language-hover';
  for (const content of Array.isArray(contents) ? contents : [contents]) {
    if (typeof content === 'string') {
      appendMarkdown(root, content);
    } else if (content && typeof content === 'object' && 'value' in content && typeof content.value === 'string') {
      if ('kind' in content) {
        if (content.kind === 'markdown') appendMarkdown(root, content.value);
        else if (content.kind === 'plaintext') {
          const text = document.createElement('div');
          text.className = 'hover-plaintext'; text.textContent = content.value; root.append(text);
        }
      } else if ('language' in content && typeof content.language === 'string') {
        codeBlock(root, content.value, content.language);
      }
    }
  }
  return root.textContent?.trim() ? root : null;
}
