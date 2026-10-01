/* A post's `markdown` body, read and written (W-98 Resources). post.setProfile's bodyFormat
   `markdown` is "CommonMark 0.31.2; raw HTML escaped; images only `asset:<id>` of this post"
   (the ICD). Reading is the reference parser's (vendor/). Its tree becomes elements, never HTML
   text, so a member's words cannot become markup. The editor's elements are written back as
   CommonMark that the same parser reads as the same tree (resources.test.mjs).

     md.parse(text)                       → the parser's tree
     md.render(text, doc, {assets, read}) → a DocumentFragment of block elements
     md.write(root)                       → CommonMark, from the block elements under root
     md.assetsOf(text)                    → the asset ids the body shows, in order */
(function (root) {
  'use strict';
  var NS = root.WallFlowersResources = root.WallFlowersResources || {};
  var cm = NS.commonmark;

  var ASSET = /^asset:([0-9a-f]{16})$/;
  var SAFE = /^(https?:|mailto:)/i;

  function parse(text) { return new cm.Parser().parse(String(text || '')); }

  function kids(node) {
    var out = [];
    for (var c = node.firstChild; c; c = c.next) out.push(c);
    return out;
  }
  function plainOf(node) {
    var s = '';
    var w = node.walker(), e;
    while ((e = w.next())) {
      if (!e.entering) continue;
      var n = e.node;
      if (n.type === 'text' || n.type === 'code' || n.type === 'html_inline') s += n.literal;
      else if (n.type === 'softbreak' || n.type === 'linebreak') s += ' ';
    }
    return s;
  }

  /* ── the tree, as elements ──────────────────────────────────────────── */

  function render(text, doc, o) {
    o = o || {};
    var frag = doc.createDocumentFragment();
    kids(parse(text)).forEach(function (b) { var e = block(b, doc, o); if (e) frag.appendChild(e); });
    return frag;
  }

  function soleImage(p) {
    var ks = kids(p).filter(function (k) { return !(k.type === 'text' && !k.literal.trim()) && k.type !== 'softbreak'; });
    return ks.length === 1 && ks[0].type === 'image' && ASSET.test(ks[0].destination) ? ks[0] : null;
  }

  function block(n, doc, o) {
    var e;
    switch (n.type) {
      case 'paragraph':
        var img = soleImage(n);
        if (img) {
          e = doc.createElement('figure');
          e.appendChild(image(img, doc, o));
          if (!o.read) e.setAttribute('contenteditable', 'false');
          return e;
        }
        return inline(doc.createElement('p'), n, doc, o);
      case 'heading':
        return inline(doc.createElement('h' + n.level), n, doc, o);
      case 'block_quote':
        e = doc.createElement('blockquote');
        kids(n).forEach(function (k) { var b = block(k, doc, o); if (b) e.appendChild(b); });
        return e;
      case 'list':
        e = doc.createElement(n.listType === 'ordered' ? 'ol' : 'ul');
        if (n.listType === 'ordered' && n.listStart !== 1) e.setAttribute('start', String(n.listStart));
        kids(n).forEach(function (item) {
          var li = doc.createElement('li');
          kids(item).forEach(function (k) {
            if (n.listTight && k.type === 'paragraph') inline(li, k, doc, o);
            else { var b = block(k, doc, o); if (b) li.appendChild(b); }
          });
          e.appendChild(li);
        });
        return e;
      case 'code_block':
        e = doc.createElement('pre');
        var code = doc.createElement('code');
        if (n.info) code.setAttribute('data-info', n.info);
        code.textContent = n.literal.replace(/\n$/, '');
        e.appendChild(code);
        return e;
      case 'thematic_break':
        e = doc.createElement('hr');
        return e;
      case 'html_block':
        e = doc.createElement('p');
        e.textContent = n.literal.replace(/\n+$/, '');
        return e;
      default:
        return null;
    }
  }

  function inline(into, n, doc, o) {
    kids(n).forEach(function (c) {
      var e;
      switch (c.type) {
        case 'text': case 'html_inline':
          into.appendChild(doc.createTextNode(c.literal));
          return;
        case 'softbreak':
          into.appendChild(doc.createTextNode(' '));
          return;
        case 'linebreak':
          into.appendChild(doc.createElement('br'));
          return;
        case 'emph': case 'strong':
          into.appendChild(inline(doc.createElement(c.type === 'emph' ? 'em' : 'strong'), c, doc, o));
          return;
        case 'code':
          e = doc.createElement('code');
          e.textContent = c.literal;
          into.appendChild(e);
          return;
        case 'link':
          if (!SAFE.test(c.destination)) { inline(into, c, doc, o); return; }
          e = doc.createElement('a');
          e.setAttribute('href', c.destination);
          if (o.read) { e.setAttribute('target', '_blank'); e.setAttribute('rel', 'noopener noreferrer'); }
          into.appendChild(inline(e, c, doc, o));
          return;
        case 'image':
          if (ASSET.test(c.destination)) { into.appendChild(image(c, doc, o)); return; }
          /* An image from elsewhere is a link to it, never fetched (the ICD: a reader's device
             fetching a member's URL is a beacon). */
          if (!SAFE.test(c.destination)) { into.appendChild(doc.createTextNode(plainOf(c))); return; }
          e = doc.createElement('a');
          e.setAttribute('href', c.destination);
          e.textContent = plainOf(c) || c.destination;
          into.appendChild(e);
          return;
      }
    });
    return into;
  }

  function image(n, doc, o) {
    var id = ASSET.exec(n.destination)[1], a = (o.assets || {})[id], e = doc.createElement('img');
    e.setAttribute('data-asset', id);
    e.setAttribute('alt', plainOf(n));
    if (a && a.src) e.setAttribute('src', a.src);
    if (a && a.width) e.setAttribute('width', String(a.width));
    if (a && a.height) e.setAttribute('height', String(a.height));
    return e;
  }

  function assetsOf(text) {
    var out = [], w = parse(text).walker(), e;
    while ((e = w.next())) {
      var m = e.entering && e.node.type === 'image' && ASSET.exec(e.node.destination);
      if (m && out.indexOf(m[1]) < 0) out.push(m[1]);
    }
    return out;
  }

  /* ── the elements, as CommonMark ────────────────────────────────────── */

  var BLOCK = /^(P|DIV|H[1-6]|BLOCKQUOTE|UL|OL|PRE|HR|FIGURE)$/;
  function tag(n) { return n.nodeType === 1 ? String(n.tagName || n.nodeName).toUpperCase() : ''; }
  function childrenOf(n) { return Array.prototype.slice.call(n.childNodes || []); }

  /* Text as itself: every character the parser would read as syntax, escaped. */
  function escText(s) {
    return s.replace(/[\\`*_\[\]<]/g, '\\$&').replace(/&(?=#?[A-Za-z0-9]+;)/g, '\\&');
  }
  /* A line's start, where a heading, a quote, a list, a fence or an underline would begin. */
  function escStart(line) {
    return line.replace(/^\s+/, '')
      .replace(/^(#{1,6})(?=\s|$)/, '\\$1')
      .replace(/^([>+~=-])/, '\\$1')
      .replace(/^(\d{1,9})([.)])(?=\s|$)/, '$1\\$2');
  }
  function lines(s) { return s.split('\n').map(escStart).join('\n').replace(/\s+$/, ''); }

  function wrap(d, inner) {
    var m = /^(\s*)([\s\S]*?)(\s*)$/.exec(inner);
    return m[2] ? m[1] + d + m[2] + d + m[3] : inner;
  }
  function codeSpan(s) {
    s = s.replace(/\n/g, ' ');
    var run = 0;
    (s.match(/`+/g) || []).forEach(function (r) { run = Math.max(run, r.length); });
    var f = new Array(run + 2).join('`'), pad = /^`|`$/.test(s) || (/^ .* $/.test(s) && s.trim()) ? ' ' : '';
    return f + pad + s + pad + f;
  }
  function dest(href) {
    return href.replace(/[ ()<>\\]/g, function (c) { return '%' + c.charCodeAt(0).toString(16).toUpperCase(); });
  }
  function imageMd(img) {
    return '![' + escText(img.getAttribute('alt') || '') + '](asset:' + img.getAttribute('data-asset') + ')';
  }

  function inlineMd(n, inside) {
    var out = '';
    childrenOf(n).forEach(function (c, i, all) {
      if (c.nodeType === 3) { out += escText(String(c.nodeValue).replace(/[\s\u00a0]+/g, ' ')); return; }
      if (c.nodeType !== 1) return;
      var t = tag(c), inner;
      switch (t) {
        case 'STRONG': case 'B':
          inner = inlineMd(c, 'STRONG');
          out += inside === 'STRONG' ? inner : wrap('**', inner);
          return;
        case 'EM': case 'I':
          inner = inlineMd(c, 'EM');
          /* At a strong's edge `*` would run into its `**`: `_` keeps them apart. */
          out += inside === 'EM' ? inner : wrap(inside === 'STRONG' && (i === 0 || i === all.length - 1) ? '_' : '*', inner);
          return;
        case 'CODE':
          out += codeSpan(c.textContent);
          return;
        case 'A':
          inner = inlineMd(c, inside);
          var href = c.getAttribute('href') || '';
          out += SAFE.test(href) && inner.trim() ? '[' + inner + '](' + dest(href) + ')' : inner;
          return;
        case 'BR':
          out += inside === 'HEADING' ? ' ' : '\\\n';
          return;
        case 'IMG':
          if (c.getAttribute('data-asset')) out += imageMd(c);
          return;
        default:
          if (!BLOCK.test(t)) out += inlineMd(c, inside);
      }
    });
    return out;
  }

  function write(rootEl) {
    var out = blocksMd(rootEl, { bullet: '' });
    return out.length ? out.join('\n\n') + '\n' : '';
  }

  function blocksMd(parent, st) {
    var out = [], loose = [];
    function flush() {
      if (!loose.length) return;
      var t = lines(inlineMd({ childNodes: loose }));
      if (t.trim()) { out.push(t); st.bullet = ''; }
      loose = [];
    }
    childrenOf(parent).forEach(function (c) {
      var t = tag(c);
      if (c.nodeType === 3 || (c.nodeType === 1 && !BLOCK.test(t))) { if (t !== 'BR' || loose.length) loose.push(c); return; }
      if (c.nodeType !== 1) return;
      flush();
      var s = blockMd(c, t, st);
      if (s !== null && s.trim()) out.push(s);
      if (t !== 'UL' && t !== 'OL') st.bullet = '';
    });
    flush();
    return out;
  }

  function blockMd(c, t, st) {
    var h = /^H([1-6])$/.exec(t);
    if (h) {
      var text = lines(inlineMd(c, 'HEADING').replace(/\n/g, ' ')).replace(/#(?=\s*$)/, '\\#');
      return text.trim() ? new Array(+h[1] + 1).join('#') + ' ' + text : null;
    }
    switch (t) {
      case 'P': case 'DIV':
        if (childrenOf(c).some(function (k) { return BLOCK.test(tag(k)); })) return blocksMd(c, st).join('\n\n');
        return lines(inlineMd(c));
      case 'BLOCKQUOTE':
        var inner = childrenOf(c).some(function (k) { return BLOCK.test(tag(k)); }) ? blocksMd(c, { bullet: '' }).join('\n\n') : lines(inlineMd(c));
        return inner.trim() ? inner.split('\n').map(function (l) { return l ? '> ' + l : '>'; }).join('\n') : null;
      case 'UL': case 'OL':
        return list(c, t === 'OL', st);
      case 'PRE':
        var code = c.textContent.replace(/\n$/, ''), run = 0;
        (code.match(/^`{3,}/gm) || []).forEach(function (r) { run = Math.max(run, r.length); });
        var fence = new Array(Math.max(3, run + 1) + 1).join('`'), el = childrenOf(c).filter(function (k) { return tag(k) === 'CODE'; })[0];
        var info = ((el && el.getAttribute('data-info')) || '').replace(/[`\n]/g, '');
        return fence + info + '\n' + code + '\n' + fence;
      case 'HR':
        return '---';
      case 'FIGURE':
        var img = childrenOf(c).filter(function (k) { return tag(k) === 'IMG' && k.getAttribute('data-asset'); })[0];
        return img ? imageMd(img) : null;
    }
    return null;
  }

  /* Two lists in a row would read as one: the second takes the other marker. */
  function list(el, ordered, st) {
    var mark = ordered ? (st.bullet === '.' ? ')' : '.') : (st.bullet === '-' ? '*' : '-');
    st.bullet = mark;
    var start = ordered ? parseInt(el.getAttribute('start'), 10) || 1 : 0;
    var items = childrenOf(el).filter(function (k) { return tag(k) === 'LI'; });
    var blocky = /^(P|DIV|H[1-6]|BLOCKQUOTE|PRE|HR|FIGURE)$/;
    var loose = items.some(function (li) { return childrenOf(li).some(function (k) { return blocky.test(tag(k)); }); });
    return items.map(function (li, i) {
      var marker = ordered ? (start + i) + mark + ' ' : mark + ' ', pad = new Array(marker.length + 1).join(' ');
      var body;
      if (loose) body = blocksMd(li, { bullet: '' }).join('\n\n');
      else {
        var parts = [], inl = [];
        childrenOf(li).forEach(function (k) {
          if (tag(k) !== 'UL' && tag(k) !== 'OL') { inl.push(k); return; }
          if (inl.length) { parts.push(lines(inlineMd({ childNodes: inl }))); inl = []; }
          parts.push(list(k, tag(k) === 'OL', { bullet: '' }));
        });
        if (inl.length) parts.push(lines(inlineMd({ childNodes: inl })));
        body = parts.join('\n');
      }
      return (marker + body.split('\n').map(function (l, j) { return j && l ? pad + l : l; }).join('\n')).replace(/\s+$/, '');
    }).join(loose ? '\n\n' : '\n');
  }

  NS.markdown = { parse: parse, render: render, write: write, assetsOf: assetsOf };
})(typeof window !== 'undefined' ? window : globalThis);
