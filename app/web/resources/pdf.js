/* THE PDF MODULE (W-98 Resources, part B): a post's document, taken and shown. The document is
   a PDF the post holds (post.setDocument); how many bytes one Delta holds is the model's, and
   the editor says it (maxBytes).

     WallFlowersResources.pdf.zone(el, {maxBytes, onFile, onRefuse}) → {destroy}
       `el` takes a dropped file, and a tap chooses one. onFile({name, mime, bytes}) for one
       PDF (its header, not its name) within maxBytes, bytes a Uint8Array; onRefuse(why) else.
     WallFlowersResources.pdf.preview(el, {name, bytes, lib}) → {destroy}
       The document in `el`: its first page, its name, pages and size, and Open. The page is
       PDF.js's, from this script's Door (the webapp's vendor/pdfjs), where the page may load
       it; `lib` stands in for that loader. Without it, the rest. */
(function (root) {
  'use strict';
  var NS = root.WallFlowersResources = root.WallFlowersResources || {};
  var PDF = 'application/pdf';
  var SCRIPT = typeof document !== 'undefined' ? document.currentScript : null;
  var DOOR = SCRIPT && SCRIPT.src ? new URL(SCRIPT.src).origin : '';

  /* The header, `%PDF-`, within the first 1024 bytes, where readers look for it. */
  function isPdf(b) {
    var n = Math.min(b.length - 4, 1024);
    for (var i = 0; i < n; i++) if (b[i] === 37 && b[i + 1] === 80 && b[i + 2] === 68 && b[i + 3] === 70 && b[i + 4] === 45) return true;
    return false;
  }
  function size(n) {
    return n >= 1048576 ? (n / 1048576).toFixed(1) + ' MB' : n >= 1024 ? Math.round(n / 1024) + ' KB' : n + ' bytes';
  }
  function pages(n) { return n === 1 ? '1 page' : n + ' pages'; }
  /* Its page objects, counted in the bytes: what an uncompressed PDF says without PDF.js.
     Object streams hide them, and then it says nothing. */
  function counted(b) {
    var s = '';
    for (var i = 0; i < b.length; i += 0x8000) s += String.fromCharCode.apply(null, b.subarray(i, i + 0x8000));
    var m = s.match(/\/Type\s*\/Page(?![A-Za-z])/g);
    return m ? m.length : 0;
  }

  var CSS = [
    '.wf-pdf{display:flex;gap:14px;align-items:center;min-width:0}',
    '.wf-pdf-pg{flex:none;width:84px;aspect-ratio:612/792;background:#f6f4ef;border:1px solid #e6e2da;border-radius:3px;overflow:hidden;display:grid;place-items:center;color:#a2261b;font:700 12px/1 -apple-system,BlinkMacSystemFont,"Segoe UI",Helvetica,Arial,sans-serif;letter-spacing:.04em}',
    '.wf-pdf-pg canvas{display:block;width:100%;height:auto}',
    '.wf-pdf-nm{min-width:0;flex:1}',
    '.wf-pdf-nm b{display:block;font-weight:600;color:#1c1b19;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}',
    '.wf-pdf-nm span{display:block;font-size:13px;color:#6b6760;margin-top:2px}',
    '.wf-pdf a{flex:none;font-size:14px;color:#1c1b19;border:1px solid #e6e2da;border-radius:6px;padding:8px 12px;text-decoration:none}'
  ].join('');
  function style(doc) {
    if (!doc.head || (doc.getElementById && doc.getElementById('wf-pdf-style'))) return;
    var s = doc.createElement('style');
    s.id = 'wf-pdf-style';
    s.textContent = CSS;
    doc.head.appendChild(s);
  }

  function zone(el, o) {
    var doc = el.ownerDocument, input = doc.createElement('input');
    input.type = 'file';
    input.accept = PDF;
    input.hidden = true;
    el.appendChild(input);
    function take(f) {
      if (!f) return;
      if (o.maxBytes && f.size > o.maxBytes) return o.onRefuse(size(f.size) + ' is over ' + size(o.maxBytes));
      f.arrayBuffer().then(function (buf) {
        var b = new Uint8Array(buf);
        if (!isPdf(b)) return o.onRefuse('not a PDF');
        o.onFile({ name: f.name, mime: PDF, bytes: b });
      }, function (e) { o.onRefuse(String(e && e.message || e)); });
    }
    function over(ev) { ev.preventDefault(); el.classList.add('over'); }
    function leave(ev) { if (!ev.relatedTarget || !el.contains || !el.contains(ev.relatedTarget)) el.classList.remove('over'); }
    function drop(ev) { ev.preventDefault(); el.classList.remove('over'); take(ev.dataTransfer && ev.dataTransfer.files && ev.dataTransfer.files[0]); }
    function tap(ev) { if (ev.target !== input) input.click(); }
    function chosen() { take(input.files && input.files[0]); input.value = ''; }
    el.addEventListener('dragover', over);
    el.addEventListener('dragleave', leave);
    el.addEventListener('drop', drop);
    el.addEventListener('click', tap);
    input.addEventListener('change', chosen);
    return {
      destroy: function () {
        el.removeEventListener('dragover', over);
        el.removeEventListener('dragleave', leave);
        el.removeEventListener('drop', drop);
        el.removeEventListener('click', tap);
        el.classList.remove('over');
        input.remove();
      }
    };
  }

  /* PDF.js, as the webapp serves it (vendor/pdfjs), from this script's Door; its worker too. */
  var loading = null;
  function pdfjs() {
    if (!loading) loading = import(DOOR + '/vendor/pdfjs/pdf.min.mjs').then(function (lib) {
      lib.GlobalWorkerOptions.workerSrc = DOOR + '/vendor/pdfjs/pdf.worker.min.mjs';
      return lib;
    }, function (e) { loading = null; throw e; });
    return loading;
  }

  function preview(el, o) {
    var doc = el.ownerDocument;
    style(doc);
    function mk(tag, text) { var e = doc.createElement(tag); if (text !== undefined) e.textContent = text; return e; }
    var box = mk('div'), pg = mk('div', 'PDF'), nm = mk('div'), sub = mk('span'), open = mk('a', 'Open');
    box.className = 'wf-pdf';
    pg.className = 'wf-pdf-pg';
    nm.className = 'wf-pdf-nm';
    nm.appendChild(mk('b', o.name || 'PDF'));
    nm.appendChild(sub);
    var n = counted(o.bytes);
    function say() { sub.textContent = (n ? pages(n) + ' · ' : '') + size(o.bytes.length); }
    say();
    var url = URL.createObjectURL(new Blob([o.bytes], { type: PDF }));
    open.setAttribute('href', url);
    open.setAttribute('target', '_blank');
    open.setAttribute('rel', 'noopener');
    if (o.name) open.setAttribute('download', o.name);
    box.appendChild(pg);
    box.appendChild(nm);
    box.appendChild(open);
    el.appendChild(box);

    var gone = false, task = null;
    (o.lib || pdfjs)().then(function (lib) {
      if (gone) return;
      /* PDF.js takes its bytes to its worker: a copy, so the post keeps its own. */
      task = lib.getDocument({ data: o.bytes.slice(), isEvalSupported: false });
      return task.promise.then(function (d) {
        if (gone) return;
        n = d.numPages;
        say();
        return d.getPage(1).then(function (p) {
          if (gone) return;
          var dpr = Math.min(root.devicePixelRatio || 1, 2), base = p.getViewport({ scale: 1 });
          var vp = p.getViewport({ scale: 84 * dpr / base.width }), c = doc.createElement('canvas');
          c.width = Math.floor(vp.width);
          c.height = Math.floor(vp.height);
          return p.render({ canvasContext: c.getContext('2d'), viewport: vp }).promise.then(function () {
            if (gone) return;
            pg.textContent = '';
            pg.appendChild(c);
          });
        });
      });
    }).then(null, function () { /* no PDF.js here, or a PDF it will not draw: the rest stands */ });

    return {
      destroy: function () {
        gone = true;
        var d = task && task.destroy && task.destroy();
        if (d && d.then) d.then(null, function () {});
        URL.revokeObjectURL(url);
        box.remove();
      }
    };
  }

  NS.pdf = { zone: zone, preview: preview };
})(typeof window !== 'undefined' ? window : globalThis);
