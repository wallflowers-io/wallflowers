/* THE RESOURCES EDITOR (W-98; RESOURCES-A). A post, written as Ghost writes one, and
   published, not created: a draft is a post the Site does not yet name, and Publish is the
   Site's `created` from both ends (group.setAffiliation and the post's own base.setBacklink).
   One component, served by the Door at /v2/resources.js and pinned by its SRI; a public site
   gives it the session element.js made, and the webapp its own.

     WallFlowersResources.mount(el, {fetch, site, post, icd, onSaved, onClose}) → {close, dirty}
       fetch(path, init) → a Response: session.fetch, or the webapp's
       site   the Site that publishes (session.site)
       post   a post to edit; none writes a new one
       icd    the model, if the page has it; else /v2/icd from this script's Door
       onClose  given, the bar has Close (Discard, while there are unsaved changes)

   Every op, arg, vocabulary and cap it writes is read from the ICD (md.* is the body's). */
(function (root) {
  'use strict';
  var NS = root.WallFlowersResources = root.WallFlowersResources || {};
  var md = NS.markdown;
  var SCRIPT = typeof document !== 'undefined' ? document.currentScript : null;
  var DOOR = SCRIPT && SCRIPT.src ? new URL(SCRIPT.src).origin : '';
  var CREATED = 'created';

  /* ── the model ────────────────────────────────────────────────────────── */

  function model(icd) {
    var ops = {};
    Object.keys(icd.kinds || {}).forEach(function (k) {
      Object.keys(icd.kinds[k].ops || {}).forEach(function (n) { ops[n] = icd.kinds[k].ops[n]; });
    });
    Object.keys(icd.facets || {}).forEach(function (f) {
      Object.keys(icd.facets[f].ops || {}).forEach(function (n) { ops[n] = icd.facets[f].ops[n]; });
    });
    function arg(op, a) { return ((ops[op] || {}).args || {})[a] || null; }
    function cap(op, a) { var x = arg(op, a); return (x && x.maxLength) || null; }
    return {
      has: function (op) { return !!ops[op]; },
      arg: arg,
      cap: cap,
      /* The bytes an inline base64 arg holds, decoded: a file's ceiling. */
      bytes: function (op, a) { var c = cap(op, a); return c ? Math.floor(c / 4) * 3 : null; },
      /* How many of a set may be live at once (post.addAsset's `maxLive`). */
      max: function (op) { return (ops[op] || {}).maxLive || null; },
      /* A cover's room in its Delta: the banner's cap, and the carriage ceiling one media item
         is held to (post.addAsset's asset), less the icon post.setMedia carries with it. */
      cover: function (icon) {
        var c = [cap('post.setMedia', 'banner'), cap('post.addAsset', 'asset')].filter(Boolean);
        return c.length ? Math.min.apply(null, c) - String(icon || '').length : null;
      },
      /* Refused before it is sent, in the model's terms: an op or arg it does not declare, a
         required arg missing, a value outside a vocabulary or its pattern, or over a cap, counted
         in characters as the fold counts them. */
      check: function (step) {
        var o = ops[step.op];
        if (!o) return step.op + ' is not in the model';
        var args = step.args || {}, decl = o.args || {};
        for (var k in args) {
          var d = decl[k], v = args[k];
          if (!d) return step.op + ' has no arg ' + k;
          if (typeof v === 'string' && d.vocabulary && v !== '' && !(v in d.vocabulary)) return k + ' is not one of ' + Object.keys(d.vocabulary).join(', ');
          if (typeof v === 'string' && d.pattern && !new RegExp(d.pattern).test(v)) return k + ' is not ' + d.pattern;
          if (typeof v === 'string' && d.maxLength && v.length > d.maxLength && Array.from(v).length > d.maxLength) return k + ' is over ' + d.maxLength + ' characters';
        }
        for (var r in decl) if (decl[r].required && !(r in args)) return step.op + ' needs ' + r;
        return null;
      }
    };
  }

  /* ── a post, as the view folds it and as the editor holds it ──────────── */

  function pick(v, camel, snake) { return v[camel] !== undefined ? v[camel] : v[snake]; }

  function fromView(v) {
    v = v || {};
    var assets = {};
    (v.assets || []).forEach(function (a) {
      assets[a.id] = { data: a.data || '', mime: a.mime || '', width: a.width || 0, height: a.height || 0, alt: a.alt || '', at: a.at || 0 };
    });
    var d = v.document;
    return {
      title: v.title || '',
      body: v.body || '',
      bodyFormat: pick(v, 'bodyFormat', 'body_format') || 'plain',
      excerpt: v.excerpt || '',
      form: v.form || 'article',
      link: v.link || '',
      icon: v.icon || '', iconMime: pick(v, 'iconMime', 'icon_mime') || '',
      banner: v.banner || '', bannerMime: pick(v, 'bannerMime', 'banner_mime') || '', bannerAlt: pick(v, 'bannerAlt', 'banner_alt') || '',
      document: d && d.data ? { data: d.data, mime: d.mime || 'application/pdf', name: d.name || '' } : null,
      assets: assets,
      retracted: !!v.retracted
    };
  }

  /* Whether the Site names the post, from both ends, and when it named it. */
  function standing(site, post, id) {
    var aff = ((site && site.view && site.view.affiliations) || []).filter(function (a) { return a.peer === id && a.rel === CREATED; })[0];
    var back = ((post && post.view && post.view.backlinks) || []).some(function (b) { return b.object === (site && site.id) && b.rel === CREATED; });
    return { published: !!(aff && back), at: aff ? aff.at : 0, half: !!aff !== back };
  }

  /* ── what a save writes ───────────────────────────────────────────────── */

  /* The steps, in order, for the Door. The post is `POST` until it is minted. Content first,
     the Site's naming last, so the Face never hydrates a post whose words are still on their way. */
  function plan(m, o) {
    var was = o.was, now = o.now, steps = [], P = 'POST';
    function apply(object, op, args) { steps.push({ do: 'apply', object: object, op: op, args: args }); }
    /* A held document makes a pdf; a post that points elsewhere (link, image, a linked pdf)
       keeps its form and its link; an article has no link (the fold refuses one). */
    var form = now.document ? 'pdf' : now.link && now.form && now.form !== 'article' ? now.form : 'article';
    var link = form === 'article' ? '' : now.link || '';

    if (!o.post) steps.push({ do: 'mint', kind: 'post', draft: { name: now.title } });
    var profile = { title: now.title, body: now.body, bodyFormat: 'markdown', form: form };
    if (now.excerpt) profile.excerpt = now.excerpt;
    if (link) profile.link = link;
    if (!was || was.title !== now.title || was.body !== now.body || was.bodyFormat !== 'markdown' || was.excerpt !== now.excerpt || was.form !== form || (was.link || '') !== link) apply(P, 'post.setProfile', profile);

    if (!was ? now.banner : was.banner !== now.banner || was.bannerAlt !== now.bannerAlt) {
      var media = {};
      if (was && was.icon) { media.icon = was.icon; media.iconMime = was.iconMime; }
      media.banner = now.banner;
      if (now.banner) { media.bannerMime = now.bannerMime; if (now.bannerAlt) media.bannerAlt = now.bannerAlt; }
      apply(P, 'post.setMedia', media);
    }

    var wasDoc = was && was.document, nowDoc = now.document;
    if ((wasDoc && wasDoc.data) !== (nowDoc && nowDoc.data) || (nowDoc && wasDoc && wasDoc.name !== nowDoc.name)) {
      apply(P, 'post.setDocument', nowDoc
        ? { document: nowDoc.data, documentMime: nowDoc.mime, documentVia: 'inline', documentKind: 'document', name: nowDoc.name }
        : { document: '', documentMime: '' });
    }

    var used = md.assetsOf(now.body), had = (was && was.assets) || {};
    used.forEach(function (id) {
      var a = now.assets[id];
      if (!a || had[id]) return;
      var args = { id: id, asset: a.data, assetMime: a.mime, assetVia: 'inline', assetKind: 'still', at: a.at };
      if (a.width) args.assetW = a.width;
      if (a.height) args.assetH = a.height;
      if (a.alt) args.alt = a.alt;
      apply(P, 'post.addAsset', args);
    });
    Object.keys(had).forEach(function (id) { if (used.indexOf(id) < 0) apply(P, 'post.removeAsset', { id: id }); });

    var at = o.at || Date.now();
    if (o.publish && !o.published) {
      apply(o.site, 'group.setAffiliation', { peer: P, rel: CREATED, name: now.title, at: at });
      apply(P, 'base.setBacklink', { object: o.site, rel: CREATED, at: at });
    } else if (o.published && was && was.title !== now.title) {
      apply(o.site, 'group.setAffiliation', { peer: P, rel: CREATED, name: now.title, at: o.publishedAt || at });
    }
    if (o.unpublish && o.published) {
      apply(o.site, 'group.clearAffiliation', { peer: P });
      apply(P, 'base.clearBacklink', { object: o.site, rel: CREATED });
    }
    return steps;
  }

  /* What the model refuses of the post as a whole, before a step is planned: more live
     pictures than post.addAsset's maxLive. */
  function limits(m, now) {
    var max = m.max('post.addAsset'), n = md.assetsOf(now.body).length;
    return max && n > max ? 'at most ' + max + ' pictures' : null;
  }

  /* The steps as requests: at most `per` steps, and about `bytes` of body, a batch; the first
     carries the mint, so `POST` there is {$step: 0}, and the post's id after it. */
  function batches(steps, per, bytes) {
    var out = [], cur = [], size = 0;
    steps.forEach(function (s) {
      var n = JSON.stringify(s).length;
      if (cur.length && (cur.length >= per || size + n > bytes)) { out.push(cur); cur = []; size = 0; }
      cur.push(s); size += n;
    });
    if (cur.length) out.push(cur);
    return out;
  }
  function named(step, post, minted) {
    var ref = minted ? { $step: 0 } : post;
    function sub(v) { return v === 'POST' ? ref : v; }
    var s = { do: step.do };
    if (step.do === 'mint') { s.kind = step.kind; s.draft = step.draft; return s; }
    s.object = sub(step.object);
    s.op = step.op;
    s.args = {};
    Object.keys(step.args).forEach(function (k) { s.args[k] = sub(step.args[k]); });
    return s;
  }

  /* Save: the steps, checked against the model, sent in order. Answers the post's id; a
     refusal part-way says what stands, in the Door's words. */
  function save(m, send, steps, post) {
    for (var i = 0; i < steps.length; i++) {
      var why = steps[i].do === 'apply' && m.check(named(steps[i], '0'.repeat(64), false));
      if (why) return Promise.reject(new Error(why));
    }
    var groups = batches(steps, 16, 900000), id = post;
    return groups.reduce(function (p, group, gi) {
      return p.then(function () {
        var minted = gi === 0 && group[0].do === 'mint';
        var body = group.map(function (s) { return named(s, id, minted); });
        return send('/v2/batch', { method: 'POST', body: JSON.stringify({ steps: body }) }).then(function (r) {
          return r.text().then(function (t) {
            var o = null;
            try { o = JSON.parse(t); } catch (e) { o = null; }
            if (!o || !Array.isArray(o.made)) throw new Error(t || String(r.status));
            if (minted && o.made.length) id = o.made[0];
            if (o.refused) throw new Error(o.refused.why || 'refused');
          });
        });
      });
    }, Promise.resolve()).then(function () { return id; }, function (e) {
      /* What was made stands: the post a refusal came after is the one the next save writes. */
      if (id) e.post = id;
      throw e;
    });
  }

  /* ── pictures: a still that fits its Delta ────────────────────────────── */

  /* JPEG, the longest side at most `side`, smaller and softer until its base64 fits `cap`. */
  function still(file, cap, side) {
    return new Promise(function (ok, no) {
      var url = URL.createObjectURL(file), img = new Image();
      img.onload = function () {
        URL.revokeObjectURL(url);
        var w = img.naturalWidth, h = img.naturalHeight, s = Math.min(1, side / Math.max(w, h)), q = 0.86;
        for (var tries = 0; tries < 12; tries++) {
          var c = document.createElement('canvas');
          c.width = Math.max(1, Math.round(w * s));
          c.height = Math.max(1, Math.round(h * s));
          var g = c.getContext('2d');
          g.fillStyle = '#fff';
          g.fillRect(0, 0, c.width, c.height);
          g.drawImage(img, 0, 0, c.width, c.height);
          var data = c.toDataURL('image/jpeg', q).split(',')[1];
          if (!cap || data.length <= cap) return ok({ data: data, mime: 'image/jpeg', width: c.width, height: c.height });
          if (q > 0.6) q -= 0.1; else s *= 0.8;
        }
        no(new Error('too large to fit'));
      };
      img.onerror = function () { URL.revokeObjectURL(url); no(new Error('not a picture')); };
      img.src = url;
    });
  }
  function hex16() {
    var b = crypto.getRandomValues(new Uint8Array(8)), s = '';
    for (var i = 0; i < b.length; i++) s += (b[i] < 16 ? '0' : '') + b[i].toString(16);
    return s;
  }
  function b64(bytes) {
    var s = '';
    for (var i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    return btoa(s);
  }
  function unb64(s) {
    var bin = atob(s), out = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  /* ── the page ─────────────────────────────────────────────────────────── */

  var CSS = [
    '.wf-res{--ink:#1c1b19;--soft:#6b6760;--line:#e6e2da;--paper:#fff;--wash:#f6f4ef;--go:#1c1b19;--bad:#a2261b;',
    'color:var(--ink);background:var(--paper);font:17px/1.6 -apple-system,BlinkMacSystemFont,"Segoe UI",Helvetica,Arial,sans-serif;',
    'max-width:760px;margin:0 auto;padding:0 16px 96px;box-sizing:border-box;-webkit-text-size-adjust:100%}',
    '.wf-res *{box-sizing:border-box}',
    '.wf-res .bar{position:sticky;top:0;z-index:3;display:flex;flex-wrap:wrap;align-items:center;gap:6px 8px;padding:12px 0;background:var(--paper);border-bottom:1px solid var(--line)}',
    '.wf-res .state{font-size:13px;color:var(--soft);margin-right:auto;white-space:nowrap}',
    '.wf-res button{font:inherit;font-size:14px;line-height:1;border-radius:6px;border:1px solid var(--line);background:var(--paper);color:var(--ink);padding:9px 12px;cursor:pointer;white-space:nowrap}',
    '.wf-res button:disabled{opacity:.45;cursor:default}',
    '.wf-res button.go{background:var(--go);border-color:var(--go);color:#fff;font-weight:600}',
    '.wf-res button.quiet{border-color:transparent;color:var(--soft)}',
    '.wf-res .why{color:var(--bad);font-size:14px;margin:8px 0 0;min-height:0}',
    '.wf-res .why:empty{display:none}',
    '.wf-res .cover{margin:20px 0 0}',
    '.wf-res .cover img{display:block;width:100%;max-height:420px;object-fit:cover;border-radius:6px}',
    '.wf-res .cover .row{display:flex;gap:8px;align-items:center;margin-top:8px}',
    '.wf-res .cover input{flex:1;min-width:0;font:inherit;font-size:14px;border:0;border-bottom:1px solid var(--line);padding:6px 0;color:var(--ink);background:transparent}',
    '.wf-res textarea{display:block;width:100%;border:0;resize:none;overflow:hidden;background:transparent;color:var(--ink);padding:0;margin:0;font:inherit}',
    '.wf-res textarea:focus,.wf-res .body:focus,.wf-res input:focus{outline:none}',
    '.wf-res .title{font-size:40px;line-height:1.15;font-weight:700;letter-spacing:-.01em;margin-top:28px}',
    '.wf-res .excerpt{font-size:20px;line-height:1.45;color:var(--soft);margin-top:12px}',
    '.wf-res .doc{margin:20px 0 0;border:1px dashed var(--line);border-radius:6px;padding:14px;font-size:14px;color:var(--soft);cursor:pointer}',
    '.wf-res .doc.has{border-style:solid;cursor:default;color:var(--ink)}',
    '.wf-res .doc.over{border-color:var(--ink);background:var(--wash);color:var(--ink)}',
    '.wf-res .doc .row{display:flex;gap:8px;align-items:center;justify-content:space-between}',
    '.wf-res .tools{position:sticky;top:var(--wf-bar,57px);z-index:2;display:flex;gap:4px;padding:8px 0;margin-top:24px;background:var(--paper);overflow-x:auto;scrollbar-width:none}',
    '.wf-res .tools::-webkit-scrollbar{display:none}',
    '.wf-res .tools button{min-width:36px;padding:8px 10px;border-color:transparent;background:var(--wash)}',
    '.wf-res .body{min-height:40vh;margin-top:12px;outline:none;word-wrap:break-word}',
    '.wf-res .body:empty:before,.wf-res .body>p:only-child:empty:before{content:attr(data-placeholder);color:#b3aea5}',
    '.wf-res [placeholder]::placeholder{color:#b3aea5}',
    '.wf-res .body p,.wf-res .read p{margin:0 0 1em}',
    '.wf-res h1,.wf-res h2,.wf-res h3,.wf-res h4{line-height:1.25;margin:1.4em 0 .5em;font-weight:700}',
    '.wf-res .body h1,.wf-res .read h1,.wf-res .body h2,.wf-res .read h2{font-size:28px}',
    '.wf-res .body h3,.wf-res .read h3{font-size:22px}',
    '.wf-res blockquote{margin:0 0 1em;padding:0 0 0 18px;border-left:3px solid var(--ink);font-style:italic}',
    '.wf-res ul,.wf-res ol{margin:0 0 1em;padding-left:1.4em}',
    '.wf-res pre{background:var(--wash);border-radius:6px;padding:14px;overflow-x:auto;font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace;margin:0 0 1em;white-space:pre}',
    '.wf-res code{font:.88em ui-monospace,SFMono-Regular,Menlo,monospace;background:var(--wash);padding:.1em .3em;border-radius:3px}',
    '.wf-res pre code{background:none;padding:0;font-size:inherit}',
    '.wf-res hr{border:0;text-align:center;margin:2em 0}',
    '.wf-res hr:before{content:"\\2022\\2003\\2022\\2003\\2022";color:var(--soft)}',
    '.wf-res figure{margin:0 0 1.2em}',
    '.wf-res figure img,.wf-res .read img{display:block;max-width:100%;height:auto;margin:0 auto;border-radius:4px}',
    '.wf-res .body figure.on img{outline:2px solid var(--ink);outline-offset:2px}',
    '.wf-res a{color:inherit;text-decoration:underline;text-underline-offset:2px}',
    '.wf-res .read .excerpt{margin-top:12px}',
    '.wf-res .read .body{min-height:0}',
    '.wf-res [hidden]{display:none!important}',
    '@media (max-width:480px){.wf-res{font-size:16px}.wf-res .title{font-size:30px}.wf-res .excerpt{font-size:18px}.wf-res .bar button{padding:8px 10px}',
    '.wf-res .state{order:9;flex-basis:100%;margin:0}.wf-res .bar .close{margin-right:auto}}'
  ].join('');

  function style(doc) {
    if (doc.getElementById('wf-res-style')) return;
    var s = doc.createElement('style');
    s.id = 'wf-res-style';
    s.textContent = CSS;
    doc.head.appendChild(s);
  }

  function mount(host, o) {
    o = o || {};
    var doc = host.ownerDocument, send = o.fetch;
    style(doc);
    function el(tag, attrs, kids) {
      var e = doc.createElement(tag);
      Object.keys(attrs || {}).forEach(function (k) {
        if (k === 'text') e.textContent = attrs[k];
        else if (k === 'on') Object.keys(attrs.on).forEach(function (t) { e.addEventListener(t, attrs.on[t]); });
        else if (attrs[k] !== null && attrs[k] !== undefined && attrs[k] !== false) e.setAttribute(k, attrs[k] === true ? '' : attrs[k]);
      });
      (kids || []).forEach(function (c) { if (c) e.appendChild(c); });
      return e;
    }

    var ed = { post: o.post || null, was: null, now: null, published: false, at: 0, dirty: false, m: null, docView: null };
    var box = el('div', { class: 'wf-res' });
    host.textContent = '';
    host.appendChild(box);

    /* the bar */
    var state = el('span', { class: 'state' });
    var bPreview = el('button', { type: 'button', text: 'Preview' });
    var bDraft = el('button', { type: 'button', text: 'Save draft' });
    var bPublish = el('button', { type: 'button', class: 'go', text: 'Publish' });
    var bUnpublish = el('button', { type: 'button', class: 'quiet', text: 'Unpublish', hidden: true });
    var bClose = o.onClose ? el('button', { type: 'button', class: 'quiet close', text: 'Close' }) : null;
    var why = el('p', { class: 'why', role: 'alert' });
    var bar = el('div', { class: 'bar' }, [bClose, state, bUnpublish, bPreview, bDraft, bPublish]);
    box.appendChild(bar);
    /* The toolbar sits under the bar, however many rows the bar takes. */
    var barRO = root.ResizeObserver ? new root.ResizeObserver(function () { box.style.setProperty('--wf-bar', bar.offsetHeight + 'px'); }) : null;
    if (barRO) barRO.observe(bar);
    var discard = false;
    if (bClose) bClose.addEventListener('click', function () {
      if (ed.dirty && !discard) { discard = true; bClose.textContent = 'Discard'; return; }
      o.onClose();
    });
    box.appendChild(why);

    /* the writing surface */
    var edit = el('div', { class: 'edit' });
    var coverBox = el('div', { class: 'cover' });
    var title = el('textarea', { class: 'title', rows: '1', placeholder: 'Title', 'aria-label': 'Title' });
    var excerpt = el('textarea', { class: 'excerpt', rows: '1', placeholder: 'Excerpt', 'aria-label': 'Excerpt' });
    var docBox = el('div', { class: 'doc' });
    var tools = el('div', { class: 'tools', role: 'toolbar' });
    var body = el('div', { class: 'body', contenteditable: 'true', role: 'textbox', 'aria-multiline': 'true', 'aria-label': 'Body', 'data-placeholder': 'Write…' });
    [coverBox, title, excerpt, docBox, tools, body].forEach(function (c) { edit.appendChild(c); });
    var read = el('div', { class: 'read', hidden: true });
    box.appendChild(edit);
    box.appendChild(read);
    var pick = el('input', { type: 'file', accept: 'image/*', hidden: true });
    box.appendChild(pick);

    function say(t) { why.textContent = t ? String(t.message || t) : ''; }
    function touch() { ed.dirty = true; discard = false; paintBar(); }
    function grow(t) { t.style.height = 'auto'; t.style.height = t.scrollHeight + 'px'; }
    [title, excerpt].forEach(function (t) {
      t.addEventListener('input', function () { grow(t); touch(); });
      t.addEventListener('keydown', function (ev) { if (ev.key === 'Enter') { ev.preventDefault(); (t === title ? excerpt : body).focus(); } });
    });

    function paintBar() {
      if (bClose && !discard) bClose.textContent = 'Close';
      state.textContent = (ed.published ? 'Published' : ed.post ? 'Draft' : 'New') + (ed.dirty ? ' · edited' : '');
      bPublish.textContent = ed.published ? 'Update' : 'Publish';
      bDraft.hidden = ed.published;
      bUnpublish.hidden = !ed.published;
      bPublish.disabled = bDraft.disabled = !title.value.trim() || !ed.m;
      bUnpublish.disabled = !ed.m;
    }

    /* ── the cover ── */
    function paintCover() {
      coverBox.textContent = '';
      var now = ed.now;
      if (!now.banner) {
        coverBox.appendChild(el('button', { type: 'button', class: 'quiet', text: '+ Cover', on: { click: function () { choose('cover'); } } }));
        return;
      }
      coverBox.appendChild(el('img', { src: 'data:' + now.bannerMime + ';base64,' + now.banner, alt: now.bannerAlt }));
      var alt = el('input', { type: 'text', placeholder: 'Alt text', value: now.bannerAlt, maxlength: capOf('post.setMedia', 'bannerAlt') });
      alt.addEventListener('input', function () { now.bannerAlt = alt.value; touch(); });
      coverBox.appendChild(el('div', { class: 'row' }, [alt, el('button', { type: 'button', class: 'quiet', text: 'Remove', on: { click: function () { now.banner = now.bannerMime = now.bannerAlt = ''; touch(); paintCover(); } } })]));
    }
    function capOf(op, a) { var c = ed.m && ed.m.cap(op, a); return c ? String(c) : null; }

    /* ── the document (RESOURCES-B's module) ── */
    var zone = null, shown = null;
    function paintDoc() {
      if (zone) { zone.destroy(); zone = null; }
      if (shown) { shown.destroy(); shown = null; }
      docBox.textContent = '';
      var d = ed.now.document, max = ed.m && ed.m.bytes('post.setDocument', 'document');
      docBox.className = 'doc' + (d ? ' has' : '');
      if (!NS.pdf) { docBox.hidden = true; return; }
      if (!d) {
        docBox.appendChild(el('span', { text: '+ PDF' }));
        zone = NS.pdf.zone(docBox, {
          maxBytes: max || 0,
          onFile: function (f) { ed.now.document = { data: b64(f.bytes), mime: f.mime, name: f.name }; touch(); paintDoc(); },
          onRefuse: function (w) { say(w); }
        });
        return;
      }
      var view = el('div', {});
      docBox.appendChild(view);
      shown = NS.pdf.preview(view, { name: d.name, bytes: unb64(d.data) });
      docBox.appendChild(el('div', { class: 'row' }, [el('span', {}), el('button', { type: 'button', class: 'quiet', text: 'Remove', on: { click: function () { ed.now.document = null; touch(); paintDoc(); } } })]));
    }

    /* ── pictures ── */
    var want = null;
    function choose(what) { want = what; pick.value = ''; pick.click(); }
    pick.addEventListener('change', function () { var f = pick.files[0]; if (f) take(f, want); });
    function take(f, what) {
      say('');
      var cap = what === 'cover' ? ed.m.cover(ed.was && ed.was.icon) : ed.m.cap('post.addAsset', 'asset');
      return still(f, cap, 1600).then(function (s) {
        if (what === 'cover') {
          ed.now.banner = s.data; ed.now.bannerMime = s.mime;
          touch(); paintCover();
          return;
        }
        var max = ed.m.max('post.addAsset'), live = md.assetsOf(md.write(body)).length;
        if (max && live >= max) throw new Error('at most ' + max + ' pictures');
        var id = hex16();
        ed.now.assets[id] = { data: s.data, mime: s.mime, width: s.width, height: s.height, alt: '', at: Date.now() };
        insertFigure(id);
        touch();
      }).then(null, say);
    }
    function figureFor(id) {
      var a = ed.now.assets[id];
      return el('figure', { contenteditable: 'false' }, [el('img', { 'data-asset': id, alt: a.alt || '', src: 'data:' + a.mime + ';base64,' + a.data, width: a.width || null, height: a.height || null })]);
    }
    function insertFigure(id) {
      var at = blockAtCaret() || body.lastChild, fig = figureFor(id), after = el('p', {}, [el('br')]);
      if (at && at.parentNode === body) { body.insertBefore(fig, at.nextSibling); body.insertBefore(after, fig.nextSibling); }
      else { body.appendChild(fig); body.appendChild(after); }
      caretInto(after);
    }
    body.addEventListener('click', function (ev) {
      var f = ev.target.closest && ev.target.closest('figure');
      Array.prototype.forEach.call(body.querySelectorAll('figure.on'), function (x) { if (x !== f) x.classList.remove('on'); });
      if (!f) return;
      f.classList.add('on');
      var img = f.querySelector('img'), alt = root.prompt ? root.prompt('Alt text', img.getAttribute('alt') || '') : null;
      if (alt === null) return;
      img.setAttribute('alt', alt);
      var a = ed.now.assets[img.getAttribute('data-asset')];
      if (a) a.alt = alt;
      touch();
    });

    /* ── the body: its blocks kept as CommonMark can say them ── */
    function blockAtCaret() {
      var sel = doc.getSelection && doc.getSelection();
      var n = sel && sel.rangeCount ? sel.getRangeAt(0).startContainer : null;
      while (n && n.parentNode !== body) n = n.parentNode;
      return n && n.parentNode === body ? n : null;
    }
    function caretInto(n, end) {
      var r = doc.createRange(), sel = doc.getSelection();
      r.selectNodeContents(n);
      r.collapse(!end);
      sel.removeAllRanges();
      sel.addRange(r);
    }
    function cmd(name, value) { body.focus(); doc.execCommand(name, false, value); normalise(); touch(); }
    function blockAs(tagName) {
      var b = blockAtCaret(), cur = b && b.tagName;
      cmd('formatBlock', cur === tagName.toUpperCase() ? 'P' : tagName.toUpperCase());
    }
    function link() {
      var href = root.prompt ? root.prompt('Link', 'https://') : null;
      if (!href) return;
      if (!/^(https?:|mailto:)/i.test(href)) return say('a link is https, http or mailto');
      cmd('createLink', href);
    }
    var TOOLS = [
      ['B', 'Bold', function () { cmd('bold'); }],
      ['I', 'Italic', function () { cmd('italic'); }],
      ['H', 'Heading', function () { blockAs('h2'); }],
      ['h', 'Subheading', function () { blockAs('h3'); }],
      ['❝', 'Quote', function () { blockAs('blockquote'); }],
      ['•', 'List', function () { cmd('insertUnorderedList'); }],
      ['1.', 'Numbered list', function () { cmd('insertOrderedList'); }],
      ['↗', 'Link', link],
      ['<>', 'Code', function () { blockAs('pre'); }],
      ['—', 'Divider', function () { divider(); }],
      ['▣', 'Picture', function () { choose('asset'); }]
    ];
    TOOLS.forEach(function (t) {
      var b = el('button', { type: 'button', title: t[1], 'aria-label': t[1], text: t[0] });
      b.addEventListener('mousedown', function (ev) { ev.preventDefault(); });
      b.addEventListener('click', t[2]);
      if (t[0] === 'B') b.style.fontWeight = '700';
      if (t[0] === 'I') b.style.fontStyle = 'italic';
      tools.appendChild(b);
    });
    function divider() {
      var at = blockAtCaret(), hr = el('hr'), after = el('p', {}, [el('br')]);
      if (at) { body.insertBefore(hr, at.nextSibling); body.insertBefore(after, hr.nextSibling); }
      else { body.appendChild(hr); body.appendChild(after); }
      caretInto(after);
      touch();
    }

    /* Markdown as it is typed at a block's start: #, ##, ###, >, -, *, 1., ``` and ---. */
    var SHORT = [
      [/^#{1,2}\s$/, 'H2'], [/^###\s$/, 'H3'], [/^>\s$/, 'BLOCKQUOTE'], [/^```$/, 'PRE'],
      [/^[-*+]\s$/, 'UL'], [/^1[.)]\s$/, 'OL']
    ];
    body.addEventListener('input', function () {
      var b = blockAtCaret();
      if (b && (b.tagName === 'P' || b.tagName === 'DIV')) {
        var t = b.textContent.replace(/ /g, ' ');
        for (var i = 0; i < SHORT.length; i++) {
          if (!SHORT[i][0].test(t)) continue;
          b.textContent = '';
          b.appendChild(el('br'));
          caretInto(b);
          var to = SHORT[i][1];
          if (to === 'UL') doc.execCommand('insertUnorderedList');
          else if (to === 'OL') doc.execCommand('insertOrderedList');
          else doc.execCommand('formatBlock', false, to);
          break;
        }
      }
      normalise();
      touch();
    });
    body.addEventListener('keydown', function (ev) {
      var k = ev.key, mod = ev.metaKey || ev.ctrlKey;
      if (mod && (k === 'b' || k === 'i')) { ev.preventDefault(); cmd(k === 'b' ? 'bold' : 'italic'); return; }
      if (mod && k === 'k') { ev.preventDefault(); link(); return; }
      if (k !== 'Enter' || ev.shiftKey) return;
      var b = blockAtCaret();
      if (b && (b.tagName === 'P' || b.tagName === 'DIV') && /^(-{3,}|\*{3,}|_{3,})$/.test(b.textContent.trim())) {
        ev.preventDefault();
        var hr = el('hr'), after = el('p', {}, [el('br')]);
        body.replaceChild(hr, b);
        body.insertBefore(after, hr.nextSibling);
        caretInto(after);
        touch();
        return;
      }
      /* Enter at the end of a heading, quote or code block goes on in a paragraph. */
      if (b && /^(H[1-6]|BLOCKQUOTE)$/.test(b.tagName)) {
        var sel = doc.getSelection(), r = sel.rangeCount && sel.getRangeAt(0).cloneRange();
        if (r) { r.selectNodeContents(b); r.setStart(sel.getRangeAt(0).endContainer, sel.getRangeAt(0).endOffset); }
        if (r && !r.toString().length) {
          ev.preventDefault();
          var p = el('p', {}, [el('br')]);
          body.insertBefore(p, b.nextSibling);
          caretInto(p);
          touch();
        }
      }
    });
    /* Paste is text, read as CommonMark: another page's markup never enters. */
    body.addEventListener('paste', function (ev) {
      var t = ev.clipboardData && ev.clipboardData.getData('text/plain');
      if (t === undefined || t === null) return;
      ev.preventDefault();
      if (!/\n/.test(t)) { doc.execCommand('insertText', false, t); return; }
      var frag = md.render(t, doc, { assets: {} }), at = blockAtCaret(), last = frag.lastChild;
      if (at && at.nextSibling) body.insertBefore(frag, at.nextSibling); else body.appendChild(frag);
      if (at && !at.textContent.trim() && !at.querySelector('img')) body.removeChild(at);
      if (last) caretInto(last, true);
      normalise();
      touch();
    });
    body.addEventListener('drop', function (ev) {
      var f = ev.dataTransfer && ev.dataTransfer.files[0];
      if (!f || !/^image\//.test(f.type)) return;
      ev.preventDefault();
      take(f, 'asset');
    });

    /* What a browser leaves in a contenteditable, as the blocks CommonMark has: a <div> or a
       stray line is a paragraph, <b>/<i> are strong/em, a style or a <span> is nothing. The
       text nodes are moved, never remade, so the caret is put back where it was: in the
       middle of a word being typed, a bold run's <b> becomes <strong> under it. */
    function normalise() {
      var sel = doc.getSelection && doc.getSelection(), r = sel && sel.rangeCount ? sel.getRangeAt(0) : null;
      var at = r && body.contains(r.startContainer) ? [r.startContainer, r.startOffset, r.endContainer, r.endOffset] : null;
      tidy();
      if (at && body.contains(at[0]) && body.contains(at[2])) {
        try { var x = doc.createRange(); x.setStart(at[0], at[1]); x.setEnd(at[2], at[3]); sel.removeAllRanges(); sel.addRange(x); } catch (e) { /* the caret's node is gone: the browser's place stands */ }
      }
    }
    function tidy() {
      Array.prototype.slice.call(body.childNodes).forEach(function (n) {
        if (n.nodeType === 3) {
          if (!n.nodeValue.trim()) { if (body.childNodes.length > 1) body.removeChild(n); return; }
          var p = el('p');
          body.replaceChild(p, n);
          p.appendChild(n);
          return;
        }
        if (n.nodeType !== 1) { body.removeChild(n); return; }
        if (n.tagName === 'DIV') {
          var q = el('p');
          while (n.firstChild) q.appendChild(n.firstChild);
          body.replaceChild(q, n);
          n = q;
        }
        if (n.tagName === 'BR') { body.removeChild(n); return; }
        if (n.tagName === 'SPAN' || n.tagName === 'FONT') {
          var s = el('p');
          while (n.firstChild) s.appendChild(n.firstChild);
          body.replaceChild(s, n);
          n = s;
        }
        clean(n);
      });
      if (!body.firstChild) body.appendChild(el('p', {}, [el('br')]));
    }
    function clean(n) {
      if (n.nodeType !== 1) return;
      n.removeAttribute('style');
      if (n.tagName !== 'IMG' && n.tagName !== 'FIGURE') n.removeAttribute('class');
      Array.prototype.slice.call(n.childNodes).forEach(function (c) {
        if (c.nodeType !== 1) return;
        var t = c.tagName;
        if (t === 'B' || t === 'I') {
          var e = el(t === 'B' ? 'strong' : 'em');
          while (c.firstChild) e.appendChild(c.firstChild);
          n.replaceChild(e, c);
          c = e;
        } else if ((t === 'SPAN' || t === 'FONT' || t === 'U') && n.tagName !== 'FIGURE') {
          while (c.firstChild) n.insertBefore(c.firstChild, c);
          n.removeChild(c);
          return;
        }
        clean(c);
      });
    }

    /* ── reading it back ── */
    function assetsForRead() {
      var out = {};
      Object.keys(ed.now.assets).forEach(function (id) {
        var a = ed.now.assets[id];
        out[id] = { src: 'data:' + a.mime + ';base64,' + a.data, width: a.width, height: a.height };
      });
      return out;
    }
    function fill() {
      var now = ed.now;
      title.value = now.title;
      excerpt.value = now.excerpt;
      body.textContent = '';
      if (now.bodyFormat === 'markdown') body.appendChild(md.render(now.body, doc, { assets: assetsForRead() }));
      else now.body.split(/\n{2,}/).forEach(function (para) { if (para.trim()) body.appendChild(el('p', { text: para.replace(/\n/g, ' ') })); });
      normalise();
      paintCover();
      paintDoc();
      setTimeout(function () { grow(title); grow(excerpt); }, 0);
      paintBar();
    }
    var reading = false, readDoc = null;
    function preview(on) {
      reading = on;
      edit.hidden = on;
      read.hidden = !on;
      bPreview.textContent = on ? 'Edit' : 'Preview';
      if (readDoc) { readDoc.destroy(); readDoc = null; }
      read.textContent = '';
      if (!on) return;
      var now = collect();
      if (now.banner) read.appendChild(el('div', { class: 'cover' }, [el('img', { src: 'data:' + now.bannerMime + ';base64,' + now.banner, alt: now.bannerAlt })]));
      read.appendChild(el('h1', { class: 'title', text: now.title }));
      if (now.excerpt) read.appendChild(el('p', { class: 'excerpt', text: now.excerpt }));
      if (now.document && NS.pdf) { var d = el('div', { class: 'doc has' }); read.appendChild(d); readDoc = NS.pdf.preview(d, { name: now.document.name, bytes: unb64(now.document.data) }); }
      var b = el('div', { class: 'body' });
      b.appendChild(md.render(now.body, doc, { assets: assetsForRead(), read: true }));
      read.appendChild(b);
    }
    bPreview.addEventListener('click', function () { preview(!reading); });

    function collect() {
      var now = ed.now;
      now.title = title.value.trim();
      now.excerpt = excerpt.value.trim();
      now.body = md.write(body);
      now.bodyFormat = 'markdown';
      return now;
    }

    /* ── saving ── */
    function run(what) {
      say('');
      var now = collect();
      if (!now.title) return say('a title');
      var over = limits(ed.m, now);
      if (over) return say(over);
      var steps = plan(ed.m, { was: ed.was, now: now, post: ed.post, site: o.site, publish: what === 'publish', unpublish: what === 'unpublish', published: ed.published, publishedAt: ed.at });
      if (!steps.length) { ed.dirty = false; paintBar(); return; }
      [bPublish, bDraft, bUnpublish].forEach(function (b) { b.disabled = true; });
      return save(ed.m, send, steps, ed.post).then(function (id) {
        ed.post = id;
        return load();
      }).then(function () {
        if (o.onSaved) o.onSaved({ post: ed.post, published: ed.published });
      }, function (e) {
        if (e && e.post && !ed.post) {
          /* Minted, then refused: the post stands, and what it holds is read back. */
          ed.post = e.post;
          var kept = ed.now;
          return load().then(function () { ed.now = kept; ed.dirty = true; fill(); say(e); if (o.onSaved) o.onSaved({ post: ed.post, published: ed.published }); }, function () { say(e); paintBar(); });
        }
        say(e); paintBar();
      });
    }
    bDraft.addEventListener('click', function () { run('draft'); });
    bPublish.addEventListener('click', function () { run('publish'); });
    bUnpublish.addEventListener('click', function () { run('unpublish'); });

    /* The post and its Site as the Door folds them, or a new one. */
    function load() {
      if (!ed.post) { ed.was = null; ed.now = fromView({}); ed.published = false; fill(); return Promise.resolve(); }
      return send('/v2/graph', {}).then(function (r) { return r.ok ? r.json() : r.text().then(function (t) { throw new Error(t || r.status); }); }).then(function (g) {
        var objs = g.objects || [], post = objs.filter(function (x) { return x.id === ed.post; })[0], site = objs.filter(function (x) { return x.id === o.site; })[0];
        if (!post) throw new Error('this post is not here');
        ed.was = fromView(post.view);
        ed.now = fromView(post.view);
        var st = standing(site, post, ed.post);
        ed.published = st.published;
        ed.at = st.at;
        ed.dirty = false;
        fill();
      });
    }

    var icd = o.icd ? Promise.resolve(o.icd) : fetch(DOOR + '/v2/icd').then(function (r) { return r.json(); });
    ed.now = fromView({});
    paintBar();
    icd.then(function (m) { ed.m = model(m); return load(); }).then(null, say);

    return {
      close: function () { if (barRO) barRO.disconnect(); if (zone) zone.destroy(); if (shown) shown.destroy(); if (readDoc) readDoc.destroy(); host.textContent = ''; },
      get dirty() { return ed.dirty; }
    };
  }

  NS.mount = mount;
  /* On a page that has WallFlowers (the webapp's modules; a site's sign-in), it is Resources there. */
  if (root.WallFlowers && typeof root.WallFlowers === 'object') root.WallFlowers.Resources = NS;
  NS.model = model;
  NS.plan = plan;
  NS.save = save;
  NS.limits = limits;
  NS.fromView = fromView;
  NS.standing = standing;
})(typeof window !== 'undefined' ? window : globalThis);
