/* members.js — W-98 Members. Ralph, 30 Sep: "New Members page accessed by clicking the
   'MEMBERS — x' control on the right rail. Ability to post a developed, site scoped bio and
   info. Site owner/admin can define free questions and multiselect polls." After
   node.cyp3.xyz's Node Grove, whose members answer SEEK, OFFER (multiselect tags) and IMAGINE.

   Drawn by two guarded hooks in webapp.js: the right rail's "Members — n" is a button while
   this file is loaded, and drawPane() hands S.view 'members' to draw(f, site, ctx), with ctx
   {door, el, icon, iconUrl, role (ROLE), head, titleOf, composeFor, busy, toast, model (M),
   redraw}. Without this file the page is as it was.

   READ from the common route, GET /v2/members/<site id>, through the webapp's own Door session
   (the contract of 1 Oct, ICD 2.3.1 draft, facets `about` and `questions`): the members in the
   route's order, `me`, `can_define`, the Site's questions. Drawn as the route answers: a name
   or "New member", never a key; ROLE's word; intentions as labels.
   WRITTEN through POST /v2/apply on the Site: base.publishAbout (the whole record, every
   time), base.answerQuestion (the whole answer), base.defineQuestion and base.retireQuestion.
   Links, options and choices are JSON array text; a question is named by the author and gen
   of its id; WallFlowers adds the gen, so none is sent. Caps are counted before sending, in
   bytes of UTF-8 where the ICD says bytes. After a 200 the route is read again; a refusal is
   the Door's sentence as it stands. What a member wrote is text, never HTML; only an https
   link is a link. Pages carry labels only (BUILD, 1 Oct): UX's two notes
   (pdr/w98-mockup.html), each one MEMBER_WORD entry drawn once at the head of what it covers,
   say who sees what I write: the about editor, and my answers. Nothing labels what is only
   read (ASSURANCE's rule, in UX's words). No connect or withdraw control: the LINKS routes
   are not there, and D-59 waits on Ralph.

   KEPT UNDER A MEMBER TYPING. drawPane() runs on every change the Door reports; the page is
   built once for each answer of the route and put back as it is, and while any editor is
   open it is not rebuilt at all: a newer answer waits until the last editor closes. */
(function (root) {
  'use strict';

  /* THE PAGE'S WORDS: short labels, all of them here, as webapp.js keeps ROLE's and KIND_WORD's.
     A member's role is ROLE's own word. */
  var MEMBER_WORD = {
    title: 'Members',
    everyone: 'Everyone',
    anon: 'New member',
    you: 'You',
    loading: 'Loading…',
    seeAnswer: 'Members see your answer.',
    nothing: 'Nothing yet',
    notAnswered: 'Not answered yet',
    about: 'Your about',
    bio: 'Bio',
    links: 'Links, https only',
    link: 'Link',
    answers: 'Your answers',
    edit: 'Edit',
    answer: 'Answer',
    yourAnswer: 'Your answer',
    save: 'Save',
    cancel: 'Cancel',
    saved: 'Saved',
    questions: 'Questions',
    newQuestion: 'New question',
    question: 'Question',
    hint: 'Hint (optional)',
    options: 'Options, one per line; none for a free question',
    multi: 'Several choices',
    max: 'At most (optional)',
    free: 'Free text as well',
    textMax: 'Text limit in bytes (optional)',
    define: 'Add question',
    retire: 'Retire',
    retireAsk: 'Retire it? It takes no new answers.',
    keep: 'Keep',
    retired: 'Retired',
    freeText: 'Free text',
    one: 'One choice',
    several: 'Several choices',
    plusText: ' + text',
    upTo: 'Up to ',
    tooLong: 'Too long: ',
    https: 'Links: https only',
    linkLong: 'A link: 2048 characters at most',
    noText: 'The question is empty',
    hintLong: 'Hint: 200 bytes at most',
    optCount: 'Options: 2 to 24',
    optSame: 'Options: each different',
    optLong: 'Options: 80 characters each',
    maxRange: 'At most: 2 to the number of options',
    textMaxRange: 'Text limit: 1 to 4096',
    intent: { resources: 'Resources', skills: 'Skills', financial: 'Financial Support' }
  };
  var W = MEMBER_WORD;
  /* The ICD 2.3.1 draft's caps: bytes of UTF-8 for text, characters for a link and an option. */
  var CAP = { bio: 4096, links: 3, link: 2048, text: 1024, hint: 200, option: 80, optionsMin: 2, optionsMax: 24, answer: 4096 };

  var MEM = null;   // the page, for one Site: {site, data, why, ver, built, node, stamp, asked, sel, open}
  var C = null;     // webapp.js's hands, as the last draw gave them

  function own(o, k) { return !!o && Object.prototype.hasOwnProperty.call(o, k); }
  function bytes(s) {
    s = String(s);
    return typeof TextEncoder === 'function' ? new TextEncoder().encode(s).length : unescape(encodeURIComponent(s)).length;
  }
  function chars(s) { return Array.from(String(s)).length; }
  function https(u) { try { var x = new URL(String(u)); return x.protocol === 'https:' ? x.href : null; } catch (e) { return null; } }
  function said(e) { return String(e && e.message || e); }
  function whole(v) { v = String(v == null ? '' : v).trim(); return /^\d+$/.test(v) ? Number(v) : v ? NaN : null; }
  /* A question's id is `<author>:<gen>`, its base.defineQuestion's identity: the args that name it. */
  function ref(id) {
    var m = /^([0-9a-f]{64}):(\d+)$/.exec(String(id || ''));
    return m ? { target_author: m[1], target_gen: Number(m[2]) } : null;
  }

  /* ── the route's answer, read defensively ─────────────────────────────── */
  function membersIn(d) { return (d && Array.isArray(d.members) ? d.members : []).filter(function (m) { return m && typeof m === 'object' && typeof m.key === 'string'; }); }
  function questionsIn(d) { return (d && Array.isArray(d.questions) ? d.questions : []).filter(function (q) { return q && typeof q === 'object' && ref(q.id); }); }
  function meIn(d) {
    var me = d && typeof d.me === 'string' ? d.me : '';
    return me ? membersIn(d).filter(function (m) { return m.key === me; })[0] || null : null;
  }
  function nameOf(m) { return (typeof m.name === 'string' && m.name.trim()) || W.anon; }
  function optionsOf(q) { return Array.isArray(q.options) ? q.options.map(function (o) { return String(o); }) : []; }
  function freeOf(q) { return !optionsOf(q).length || q.free === true; }
  function textMaxOf(q) { return typeof q.textMax === 'number' && q.textMax % 1 === 0 && q.textMax >= 1 && q.textMax <= CAP.answer ? q.textMax : CAP.answer; }
  function capOf(q) {
    var n = optionsOf(q).length;
    if (q.multi !== true) return 1;
    return typeof q.max === 'number' && q.max % 1 === 0 && q.max >= 1 && q.max < n ? q.max : n;
  }
  function choicesIn(x, n) {
    return (x && Array.isArray(x.choices) ? x.choices : []).filter(function (i, k, all) {
      return typeof i === 'number' && i % 1 === 0 && i >= 0 && i < n && all.indexOf(i) === k;
    });
  }
  function answerOf(m, q) { var a = m && m.answers && typeof m.answers === 'object' && own(m.answers, q.id) ? m.answers[q.id] : null; return a && typeof a === 'object' ? a : null; }
  function intentsOf(m) {
    return (Array.isArray(m.intentions) ? m.intentions : []).filter(function (x, k, all) { return typeof x === 'string' && own(W.intent, x) && all.indexOf(x) === k; });
  }

  /* ── the page ─────────────────────────────────────────────────────────── */
  function draw(f, site, ctx) {
    C = ctx;
    ctx.composeFor(null);
    ctx.head(W.title, ctx.titleOf(site));
    var shown = !!(MEM && MEM.node && f.firstChild === MEM.node);
    if (!MEM || MEM.site !== site.id) {
      MEM = { site: site.id, data: null, why: null, ver: 0, built: -1, node: null, stamp: ctx.model, asked: 0, sel: null, open: {} };
      read();
    } else if (!shown || MEM.stamp !== ctx.model) {
      /* Opened again, or the Door reported a change: read the route again. */
      MEM.stamp = ctx.model;
      read();
    }
    if (MEM.node && (MEM.built === MEM.ver || editing())) {
      if (!shown) { f.textContent = ''; f.appendChild(MEM.node); }
      return;
    }
    MEM.open = {};
    MEM.node = build(MEM);
    MEM.built = MEM.ver;
    f.textContent = '';
    f.appendChild(MEM.node);
  }
  function read() {
    var mine = MEM, n = ++mine.asked;
    return C.door('/v2/members/' + encodeURIComponent(mine.site)).then(function (d) {
      if (MEM !== mine || n !== mine.asked) return;
      mine.data = d && typeof d === 'object' ? d : {};
      mine.why = null;
      mine.ver++;
      C.redraw();
    }, function (e) {
      if (MEM !== mine || n !== mine.asked) return;
      mine.why = said(e);
      mine.ver++;
      C.redraw();
    });
  }
  function editing() { return !!MEM && Object.keys(MEM.open).length > 0; }
  /* An editor opens or closes. The last to close lets a newer answer of the route be drawn. */
  function hold(name, on) {
    if (!MEM) return;
    if (on) MEM.open[name] = true; else delete MEM.open[name];
    if (!on && !editing() && MEM.built !== MEM.ver) C.redraw();
  }
  /* One write on the Site. After a 200, `done` (the editor closes on what was sent), then the
     route again; a refusal is the Door's sentence, and the editor stays as it is. */
  function write(btn, why, op, args, done) {
    var mine = MEM;
    C.busy(btn, true);
    why.textContent = '';
    return C.door('/v2/apply', { method: 'POST', body: { object: mine.site, op: op, args: args } }).then(function () {
      if (MEM !== mine) return;
      C.toast(W.saved);
      done();
      return read();
    }, function (e) { why.textContent = said(e); }).then(function () { C.busy(btn, false); });
  }

  function el(tag, attrs, kids) { return C.el(tag, attrs, kids); }
  /* UX's label by an editable field: who sees what is written there. */
  function note(text) { return el('p', { class: 'note', text: text }); }
  function button(go, text, onclick, cls) { return el('button', { class: cls || 'mem-alt', type: 'button', 'data-go': go, text: text, onclick: onclick }); }
  function counter(input, max) {
    var ct = el('span', { class: 'ct' });
    function count() { var n = bytes(input.value.trim()); ct.textContent = n + ' / ' + max; ct.className = 'ct' + (n > max ? ' over' : ''); }
    input.addEventListener('input', count);
    count();
    return ct;
  }

  function build(mem) {
    var d = mem.data, box = el('div', { class: 'col members' });
    if (mem.why) box.appendChild(el('p', { class: 'why', text: mem.why }));
    if (!d) { if (!mem.why) box.appendChild(el('div', { class: 'none', text: W.loading })); return box; }
    var list = membersIn(d), qs = questionsIn(d), live = qs.filter(function (q) { return q.retired !== true; }), me = meIn(d);
    /* Everyone first: the page is who is here. Then mine, then the questions, the definer's. */
    box.appendChild(roster(list, live, me));
    if (me) {
      box.appendChild(aboutSection());
      if (live.length) box.appendChild(answersSection(live));
    }
    if (d.can_define === true) box.appendChild(questionsSection(qs));
    return box;
  }

  /* ── everyone ─────────────────────────────────────────────────────────── */
  function face(m) {
    var pic = C.iconUrl(m.icon), a = el('span', { class: 'av neutral' + (pic ? '' : ' anon'), 'aria-hidden': 'true' });
    if (pic) a.appendChild(el('img', { src: pic, alt: '' })); else a.innerHTML = C.icon('person');
    return a;
  }
  function intents(m) {
    var xs = intentsOf(m);
    return xs.length ? el('span', { class: 'intents' }, xs.map(function (x) { return el('span', { class: 'intent', 'data-i': x, text: W.intent[x] }); })) : null;
  }
  function roster(list, live, me) {
    var box = el('section', { class: 'mem-roster', 'data-sec': 'roster' }), shows = [];
    box.appendChild(el('h3', { text: W.everyone }));
    list.forEach(function (m) {
      var r = typeof m.role === 'string' && own(C.role, m.role) ? m.role : 'member', R = C.role[r], det = detail(m, live);
      var b = el('button', { class: 'mem', type: 'button', 'aria-expanded': 'false' }, [face(m),
        el('span', { class: 'grow' }, [el('span', { class: 't', text: nameOf(m) }), intents(m)]),
        m === me ? el('span', { class: 's', text: W.you }) : null,
        el('span', { class: 'mrole', style: '--r:' + R.hue, text: R.say })]);
      function show(on) { det.hidden = !on; b.setAttribute('aria-expanded', on ? 'true' : 'false'); }
      shows.push(show);
      b.addEventListener('click', function () {
        var on = det.hidden;
        shows.forEach(function (s) { s(false); });
        show(on);
        MEM.sel = on ? m.key : null;
      });
      show(MEM.sel != null && MEM.sel === m.key);
      box.appendChild(b);
      box.appendChild(det);
    });
    if (!list.length) box.appendChild(el('div', { class: 'none', text: W.nothing }));
    return box;
  }
  /* What a member wrote: the bio as text, and each link, a link only where it is https. */
  function aboutView(a) {
    var out = [];
    a = a && typeof a === 'object' ? a : {};
    if (typeof a.bio === 'string' && a.bio) out.push(el('p', { class: 'mem-text', text: a.bio }));
    var links = (Array.isArray(a.links) ? a.links : []).filter(function (u) { return typeof u === 'string' && u; });
    if (links.length) out.push(el('ul', { class: 'mem-links' }, links.map(function (u) {
      var h = https(u);
      return el('li', {}, [h ? el('a', { href: h, target: '_blank', rel: 'noopener noreferrer', text: u }) : el('span', { text: u })]);
    })));
    return out;
  }
  function answerView(q, x) {
    var opts = optionsOf(q), picked = choicesIn(x, opts.length), text = x && typeof x.text === 'string' ? x.text : '';
    if (!text && !picked.length) return null;
    return el('div', { class: 'mem-a' }, [text ? el('p', { class: 'mem-text', text: text }) : null,
      picked.length ? el('span', { class: 'chips' }, picked.map(function (i) { return el('span', { class: 'chip on', text: opts[i] }); })) : null]);
  }
  /* A member, selected: their about, their answers to the open questions, their intentions. */
  function detail(m, live) {
    var box = el('div', { class: 'mem-detail' }), said = 0;
    box.hidden = true;
    aboutView(m.about).forEach(function (n) { box.appendChild(n); said++; });
    var dl = el('dl', { class: 'mem-ans' }), n = 0;
    live.forEach(function (q) {
      var v = answerView(q, answerOf(m, q));
      if (!v) return;
      n++;
      dl.appendChild(el('dt', { text: String(q.text || '') }));
      dl.appendChild(el('dd', {}, [v]));
    });
    if (n) { box.appendChild(dl); said++; }
    var i = intents(m);
    if (i) { box.appendChild(i); said++; }
    if (!said) box.appendChild(el('p', { class: 'none', text: W.nothing }));
    return box;
  }

  /* ── mine ─────────────────────────────────────────────────────────────── */
  /* My about: base.publishAbout {bio, links}, the whole record every time. */
  function aboutSection() {
    var box = el('section', { class: 'mem-sec', 'data-sec': 'about' }), body = el('div');
    box.appendChild(el('h3', { text: W.about }));
    box.appendChild(body);
    function closed() {
      body.textContent = '';
      var me = meIn(MEM.data), shown = aboutView(me && me.about);
      if (!shown.length) shown.push(el('p', { class: 'none', text: W.nothing }));
      shown.forEach(function (n) { body.appendChild(n); });
      body.appendChild(button('edit', W.edit, opened));
    }
    function opened() {
      hold('about', true);
      body.textContent = '';
      var me = meIn(MEM.data), a = me && me.about && typeof me.about === 'object' ? me.about : {};
      var bio = el('textarea', { rows: '6', 'data-f': 'bio', 'aria-label': W.bio });
      bio.value = typeof a.bio === 'string' ? a.bio : '';
      var had = (Array.isArray(a.links) ? a.links : []).filter(function (u) { return typeof u === 'string'; });
      var links = [0, 1, 2].map(function (i) {
        var x = el('input', { type: 'url', 'data-f': 'link', 'aria-label': W.link + ' ' + (i + 1), placeholder: 'https://', autocapitalize: 'off', autocorrect: 'off', spellcheck: 'false' });
        x.value = had[i] || '';
        return x;
      });
      var why = el('p', { class: 'why', 'aria-live': 'polite' });
      var save = button('save', W.save, function () {
        var text = bio.value.trim(), out = [], bad = null;
        links.forEach(function (x) {
          var u = x.value.trim();
          if (!u || bad) return;
          if (!https(u)) bad = W.https;
          else if (chars(u) > CAP.link) bad = W.linkLong;
          else out.push(u);
        });
        if (bad) { why.textContent = bad; return; }
        if (bytes(text) > CAP.bio) { why.textContent = W.tooLong + bytes(text) + ' / ' + CAP.bio; return; }
        write(save, why, 'base.publishAbout', { bio: text, links: JSON.stringify(out) }, function () {
          var m = meIn(MEM.data);
          if (m) m.about = { bio: text, links: out };
          closed();
          hold('about', false);
        });
      }, 'mem-go');
      body.appendChild(el('label', { class: 'fld' }, [el('span', { text: W.bio }), bio, counter(bio, CAP.bio)]));
      body.appendChild(el('div', { class: 'fld' }, [el('span', { text: W.links })].concat(links)));
      body.appendChild(why);
      body.appendChild(el('div', { class: 'mem-row' }, [save, button('cancel', W.cancel, function () { closed(); hold('about', false); })]));
    }
    closed();
    return box;
  }

  /* My answers to the open questions, one at a time: base.answerQuestion, the whole answer
     every time. Text where the question takes it; choices where it has options, one unless
     multi, and no more than its max. An empty answer clears it. */
  function answersSection(live) {
    var box = el('section', { class: 'mem-sec', 'data-sec': 'answers' });
    box.appendChild(el('h3', { text: W.answers }));
    box.appendChild(note(W.seeAnswer));   // UX: once, at the head of my answers, above the first question
    live.forEach(function (q) { box.appendChild(answerRow(q)); });
    return box;
  }
  function questionHead(row, q) {
    row.appendChild(el('p', { class: 'qt', text: String(q.text || '') }));
    if (typeof q.hint === 'string' && q.hint) row.appendChild(el('p', { class: 'qh', text: q.hint }));
  }
  function answerRow(q) {
    var row = el('div', { class: 'mem-q' }), name = 'answer ' + q.id;
    function closed() {
      row.textContent = '';
      questionHead(row, q);
      var x = answerOf(meIn(MEM.data), q), v = answerView(q, x);
      row.appendChild(v || el('p', { class: 'none', text: W.notAnswered }));
      row.appendChild(button('edit', v ? W.edit : W.answer, opened));
    }
    function opened() {
      hold(name, true);
      row.textContent = '';
      questionHead(row, q);
      var opts = optionsOf(q), free = freeOf(q), cap = capOf(q), had = answerOf(meIn(MEM.data), q) || {};
      var picked = choicesIn(had, opts.length).slice(0, cap);
      var why = el('p', { class: 'why', 'aria-live': 'polite' });
      if (opts.length) row.appendChild(el('p', { class: 'qmeta', text: cap === 1 ? W.one : W.upTo + cap }));
      var chips = opts.map(function (o, i) {
        return el('button', { class: 'chip', type: 'button', text: o, onclick: function () {
          var at = picked.indexOf(i);
          why.textContent = '';
          if (at >= 0) picked.splice(at, 1);
          else if (cap === 1) picked = [i];
          else if (picked.length >= cap) { why.textContent = W.upTo + cap; return; }
          else picked.push(i);
          paint();
        } });
      });
      function paint() { chips.forEach(function (c, i) { c.setAttribute('aria-pressed', picked.indexOf(i) >= 0 ? 'true' : 'false'); }); }
      paint();
      if (chips.length) row.appendChild(el('div', { class: 'chips' }, chips));
      var text = null, max = textMaxOf(q);
      if (free) {
        text = el('textarea', { rows: '3', 'data-f': 'text', 'aria-label': W.yourAnswer, placeholder: W.yourAnswer });
        text.value = typeof had.text === 'string' ? had.text : '';
        row.appendChild(el('label', { class: 'fld' }, [text, counter(text, max)]));
      }
      var save = button('save', W.save, function () {
        var args = ref(q.id), t = free ? text.value.trim() : null;
        if (free && bytes(t) > max) { why.textContent = W.tooLong + bytes(t) + ' / ' + max; return; }
        if (free) args.text = t;
        var chosen = picked.slice().sort(function (a, b) { return a - b; });
        if (opts.length) args.choices = JSON.stringify(chosen);
        write(save, why, 'base.answerQuestion', args, function () {
          var m = meIn(MEM.data);
          if (m) { if (!m.answers || typeof m.answers !== 'object') m.answers = {}; m.answers[q.id] = { text: free ? t : null, choices: chosen }; }
          closed();
          hold(name, false);
        });
      }, 'mem-go');
      row.appendChild(why);
      row.appendChild(el('div', { class: 'mem-row' }, [save, button('cancel', W.cancel, function () { closed(); hold(name, false); })]));
    }
    closed();
    return row;
  }

  /* ── the owner's and the admins' (can_define) ─────────────────────────── */
  function questionsSection(qs) {
    var box = el('section', { class: 'mem-sec', 'data-sec': 'questions' });
    box.appendChild(el('h3', { text: W.questions }));
    qs.forEach(function (q) { box.appendChild(questionRow(q)); });
    box.appendChild(defineForm());
    return box;
  }
  /* A question as its definer sees it: its form, its tally per option, and Retire, asked once. */
  function questionRow(q) {
    var row = el('div', { class: 'mem-q' }), name = 'retire ' + q.id;
    function closed() {
      row.textContent = '';
      row.className = 'mem-q' + (q.retired === true ? ' retired' : '');
      questionHead(row, q);
      var opts = optionsOf(q), tally = Array.isArray(q.tally) ? q.tally : [], cap = capOf(q);
      var form = !opts.length ? W.freeText : (cap === 1 ? W.one : q.max != null && cap < opts.length ? W.upTo + cap : W.several) + (q.free === true ? W.plusText : '');
      row.appendChild(el('p', { class: 'qmeta' }, [form, q.retired === true ? el('span', { class: 'pill', text: W.retired }) : null]));
      if (opts.length) row.appendChild(el('div', { class: 'chips' }, opts.map(function (o, i) {
        return el('span', { class: 'tally' }, [o, el('b', { text: String(typeof tally[i] === 'number' ? tally[i] : 0) })]);
      })));
      if (q.retired !== true) row.appendChild(button('retire', W.retire, asked));
    }
    function asked() {
      hold(name, true);
      row.textContent = '';
      questionHead(row, q);
      var why = el('p', { class: 'why', 'aria-live': 'polite' });
      var yes = button('retire-yes', W.retire, function () {
        write(yes, why, 'base.retireQuestion', ref(q.id), function () { q.retired = true; closed(); hold(name, false); });
      }, 'mem-go');
      row.appendChild(el('p', { class: 'qmeta', text: W.retireAsk }));
      row.appendChild(why);
      row.appendChild(el('div', { class: 'mem-row' }, [yes, button('retire-no', W.keep, function () { closed(); hold(name, false); })]));
    }
    closed();
    return row;
  }
  /* A new question: its words, a hint, options one per line (none: a free question), multi and
     its max, free text as well, and the text's limit. Checked here as the ICD caps them. */
  function defineForm() {
    var box = el('div', { class: 'mem-new' });
    function closed() {
      box.textContent = '';
      box.removeAttribute('data-edit');
      box.appendChild(button('new', W.newQuestion, opened));
    }
    function opened() {
      hold('define', true);
      box.textContent = '';
      box.setAttribute('data-edit', 'define');
      var text = el('input', { type: 'text', 'data-f': 'qtext', 'aria-label': W.question });
      var hint = el('input', { type: 'text', 'data-f': 'hint', 'aria-label': W.hint });
      var options = el('textarea', { rows: '4', 'data-f': 'options', 'aria-label': W.options });
      var multi = el('input', { type: 'checkbox', 'data-f': 'multi' });
      var max = el('input', { type: 'number', min: '2', max: String(CAP.optionsMax), inputmode: 'numeric', 'data-f': 'max', 'aria-label': W.max });
      var free = el('input', { type: 'checkbox', 'data-f': 'free' });
      var textMax = el('input', { type: 'number', min: '1', max: String(CAP.answer), inputmode: 'numeric', 'data-f': 'textMax', 'aria-label': W.textMax });
      var why = el('p', { class: 'why', 'aria-live': 'polite' });
      function lines() { return options.value.split('\n').map(function (o) { return o.trim(); }).filter(Boolean); }
      /* What applies, as the options stand: multi, its max and free only with options. */
      function sync() {
        var some = lines().length > 0;
        multi.disabled = free.disabled = !some;
        max.disabled = !some || !multi.checked;
        textMax.disabled = some && !free.checked;
      }
      options.addEventListener('input', sync);
      multi.addEventListener('change', sync);
      free.addEventListener('change', sync);
      sync();
      function no(w) { why.textContent = w; }
      var add = button('define', W.define, function () {
        var t = text.value.trim(), h = hint.value.trim(), opts = lines(), args = { text: t };
        why.textContent = '';
        if (!t) return no(W.noText);
        if (bytes(t) > CAP.text) return no(W.tooLong + bytes(t) + ' / ' + CAP.text);
        if (bytes(h) > CAP.hint) return no(W.hintLong);
        if (h) args.hint = h;
        var tm = whole(textMax.value);
        if (!opts.length) {
          args.free = 1;
        } else {
          if (opts.length < CAP.optionsMin || opts.length > CAP.optionsMax) return no(W.optCount);
          if (opts.some(function (o, k) { return opts.indexOf(o) !== k; })) return no(W.optSame);
          if (opts.some(function (o) { return chars(o) > CAP.option; })) return no(W.optLong);
          args.options = JSON.stringify(opts);
          args.multi = multi.checked ? 1 : 0;
          var mx = whole(max.value);
          if (multi.checked && mx !== null) {
            if (!(mx >= 2 && mx <= opts.length)) return no(W.maxRange);
            args.max = mx;
          }
          args.free = free.checked ? 1 : 0;
        }
        if (args.free === 1 && tm !== null) {
          if (!(tm >= 1 && tm <= CAP.answer)) return no(W.textMaxRange);
          args.textMax = tm;
        }
        write(add, why, 'base.defineQuestion', args, function () { closed(); hold('define', false); });
      }, 'mem-go');
      box.appendChild(el('label', { class: 'fld' }, [el('span', { text: W.question }), text, counter(text, CAP.text)]));
      box.appendChild(el('label', { class: 'fld' }, [el('span', { text: W.hint }), hint]));
      box.appendChild(el('label', { class: 'fld' }, [el('span', { text: W.options }), options]));
      box.appendChild(el('div', { class: 'mem-chk' }, [el('label', {}, [multi, el('span', { text: W.multi })]),
        el('label', {}, [free, el('span', { text: W.free })])]));
      box.appendChild(el('div', { class: 'mem-nums' }, [el('label', { class: 'fld' }, [el('span', { text: W.max }), max]),
        el('label', { class: 'fld' }, [el('span', { text: W.textMax }), textMax])]));
      box.appendChild(why);
      box.appendChild(el('div', { class: 'mem-row' }, [add, button('cancel', W.cancel, function () { closed(); hold('define', false); })]));
    }
    closed();
    return box;
  }

  root.WallFlowers = root.WallFlowers || {};
  root.WallFlowers.Members = { draw: draw, WORD: MEMBER_WORD };
})(typeof window !== 'undefined' ? window : this);
