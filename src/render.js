import { marked } from 'marked';
import DOMPurify from 'dompurify';

// Double tabs belong to the portable output. Remove them for Markdown display,
// where eight spaces would otherwise turn a section into a code block.
export function markdownForDisplay(text) {
  let fence = false;
  return text.normalize('NFC').replace(/\r\n?/g, '\n').split('\n').map(line => {
    if (/^\s*(`{3,}|~{3,})/.test(line)) { fence = !fence; return line.trimStart(); }
    if (fence) return line;
    line = line.replace(/^\t+/, '');
    return line.replace(/^(\s*)(?:[-*+•]\s+){2,}/, '$1- ');
  }).join('\n');
}
export function renderMarkdown(text) {
  return DOMPurify.sanitize(marked.parse(markdownForDisplay(text), { gfm: true, breaks: true }), {
    ALLOWED_TAGS: ['p', 'br', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'ul', 'ol', 'li', 'strong', 'em', 'del', 'code', 'pre', 'blockquote', 'hr', 'table', 'thead', 'tbody', 'tr', 'th', 'td', 'a'],
    ALLOWED_ATTR: ['title']
  });
}
