/* commonmark.json: what the CommonMark reference parser (commonmark.js 0.31.2, the copy the
   Resources editor carries: app/web/resources/vendor) draws for the bodies the editor writes,
   and a few more. face-render's post page is held to it (tests.rs). Regenerate:

     node arc/lib/face-render/fixtures/commonmark.mjs */
import { readFileSync, writeFileSync } from 'node:fs';
import vm from 'node:vm';

const HERE = new URL('./', import.meta.url);
const VENDOR = new URL('../../../../app/web/resources/vendor/', HERE);
const window = {};
vm.runInContext(['pre.js', 'commonmark.min.js', 'post.js'].map((f) => readFileSync(new URL(f, VENDOR), 'utf8')).join(''), vm.createContext({ window }));
const cm = window.WallFlowersResources.commonmark;
const html = (t) => new cm.HtmlRenderer({ safe: true }).render(new cm.Parser().parse(t));

const TEXTS = [
  /* what the editor writes (app/web/resources/resources.test.mjs's round trip) */
  '# Title\n\nPara with **bold**, *em*, `code` and [a link](https://example.org/a%20b).',
  '## Sub\n\n> quoted **text**\n> and more\n\n- one\n- two\n  - nested\n- three',
  '1. first\n2. second\n\n- after',
  '3. starts at three\n4. four',
  '```js\nconst x = `y`;\n\n  indented\n```',
  '---',
  '![a cat, sitting](asset:0123456789abcdef)',
  'line one\\\nline two',
  '***both***',
  '**_x_ y**',
  'text with \\*escaped\\* stars, a\\_b\\_c and 5 \\* 3 \\< 4 & 1 > 0',
  '- a\n\n  b\n\n- c',
  '> - quoted list\n> - two',
  '- one\n- two\n\n* three\n* four',
  '`` a ` tick ``',
  '#### four\n\n###### six',
  'a [link *with* marks](mailto:a@b.org) here',
  '\\# not a heading\n\n1\\. not a list\n\n\\- not a list\n\n\\> not a quote\n\n\\<b\\>not html\\</b\\>',
  /* and what a person may type into a body by hand */
  'Head\n===\n\nand setext\n---',
  'a*b*c and __strong__ and _em_ and *a **b** c*',
  'two spaces  \nbreak',
  '~~~\ntilde fence\n~~~',
  '## closing hashes ##',
  '* * *\n\n___',
  '1) paren\n2) list',
  '> > nested\n> quote',
  '[titled](https://example.org "a title") and [paren](<https://example.org/a b>)',
  '![*alt* text](asset:00000000000000aa) inline',
  'soft\nbreak',
  '    indented code',
  '- tight\n- list\n\n\n- still one list',
];
/* Where the ICD's rule is not the reference's HtmlRenderer: raw HTML is text, not omitted; a
   link is http(s) or mailto or it is text; an image is the post's own asset or a link to it,
   never fetched. The page is held to these by its own tests, not to `html`. */
const ICD_RULES = [
  '<script>alert(1)</script>',
  '[js](javascript:alert(1)) and ![far](https://x.org/c.png) and [empty]()',
  'para <b>x</b>',
];
const out = TEXTS.map((text) => ({ text, html: html(text) })).concat(ICD_RULES.map((text) => ({ text, html: html(text), icd: true })));
writeFileSync(new URL('commonmark.json', HERE), JSON.stringify(out, null, 1) + '\n');
console.log('commonmark.json:', out.length, 'bodies');
