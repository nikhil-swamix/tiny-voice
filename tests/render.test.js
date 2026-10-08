import test from 'node:test';
import assert from 'node:assert/strict';
import { marked } from 'marked';
import { markdownForDisplay } from '../src/render.js';

test('portable double tabs render as Markdown headings and lists instead of code blocks', () => {
  const text = '## Launch\n\t\t- Ship **Monday**\n\n\t\t### Checks\n\t\t\t\t- Test\n\n## Hints\n\t\t- [hint: Check the finished work.]';
  const html = marked.parse(markdownForDisplay(text));
  assert.match(html, /<h2>Launch<\/h2>/); assert.match(html, /<h3>Checks<\/h3>/);
  assert.match(html, /<strong>Monday<\/strong>/); assert.equal((html.match(/<li>/g) || []).length, 3);
  assert.ok(!html.includes('<pre>'));
});
test('duplicate list symbols are removed without changing Markdown emphasis or fenced code', () => {
  const text = '## Notes\r\n\t\t- • - Keep **this**\n\n```js\n\t\tconst x = 1;\n```';
  const normalized = markdownForDisplay(text);
  assert.match(normalized, /^- Keep \*\*this\*\*$/m); assert.match(normalized, /\n\t\tconst x = 1;/);
});
test('new two-space output is standard Markdown without a custom rendering pass', () => {
  const text = '  ## Launch\n\n  - Publish **Monday**.\n\n  ### Checks\n\n  - Count words.\n\n  ## Hints\n\n  - Verify the result before delivery.';
  const html = marked.parse(text);
  assert.match(html, /<h2>Launch<\/h2>/); assert.match(html, /<h3>Checks<\/h3>/);
  assert.equal((html.match(/<li>/g) || []).length, 3); assert.ok(!html.includes('<pre>'));
});
