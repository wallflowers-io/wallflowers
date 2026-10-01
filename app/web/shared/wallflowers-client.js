/* ═══════════════════════════════════════════════════════════════════════════
   THE SOCIAL CLIENT — what a site imports to ask WallFlowers something.

   Served beside pacific.js. One file, three ways to load it: a <script> tag
   (it sets self.WallflowersClient), a side-effect `import`, or require() under
   Node. It replaces each consumer's own copy (egregore's lib/social/client.ts,
   thedoor's wallflowers/client.js), which is what was agreed on 22 Sep: the
   producer publishes the client, and a consumer writes only its compose layer.

   FOUR METHODS, over the keyholder's port:
     capabilities()             → {writes, ops: {op: {reachable, why, needs, on}}}
     fold(group?)               → the fold, read through the producer's own
                                  normalise (wallflowers-fold.js), with the
                                  recovery records beside it
     mint(kind, draft)          → {ok: true, group}
     author(group, op, args)    → {ok: true, deltaId, group}
   Every failure is a refusal that says whose it is: {ok: false, op, by, why}.

   THE CONNECTION IS PASSED IN. socialApi(connect) takes a function that opens
   the port, e.g. () => Pacific.connect({keyholder, site}), because each site
   opens its own with its own site handle. This file performs no cryptography,
   keeps no model and holds no key. A write is a request; its answer is a delta
   id or a refusal.

   WHOSE REFUSAL IT IS, by the rules pinned in refusal-cases.json:
     R1 the keyholder's structured refusal is the only source of core, binding
        and keyholder;
     R2 otherwise the envelope's words, without the "<method>: " prefix;
     R3 an unknown method is the producer's gap;
     R4 a failure to ask at all is transport. This client opens the connection,
        so it tags that STRUCTURALLY: anything `connect` throws is transport,
        whatever its words. The text match stays only for failures pacific.js
        raises after connecting (a port that stopped answering);
     R5 anything else is the keyholder's own words.
   `site` is never taken from a producer answer. It is for a refusal a site makes
   before asking.

   A FIXTURE SAYS SO. sim(answer) is a connection that answers object.fold from a
   producer fold answer and refuses every write as `site` (nothing was asked of
   WallFlowers, so no producer `by` would be true), and the model it produces
   carries `fixture: true`. It exists so a site can draw before a keyholder is
   up without hand-building fold output. The producer serves the standard one
   beside pacific.js as wallflowers-sim.json: the core's own fold of a committed
   archive, regenerated at every stage (shared/gen-sim.mjs). So:
     fetch('/wallflowers-sim.json').then(r => r.json())
       .then(a => WallflowersClient.socialApi(WallflowersClient.sim(a)))
   or, from another origin, <script src=".../wallflowers-sim.js"> and
   sim(self.WallflowersSimAnswer). wallflowers-sim-site.{json,js} is a Site as a
   site draws one: offices, an affiliation, a forum with a room, a notebook and
   an event with a ticket link.
   ═══════════════════════════════════════════════════════════════════════════ */
(function (root, factory) {
  var api = factory(root);
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) root.WallflowersClient = api;
})(typeof self !== 'undefined' ? self : (typeof globalThis !== 'undefined' ? globalThis : null), function (root) {
  'use strict';

  var METHOD = {
    capabilities: 'ops.capabilities',
    fold: 'object.fold',
    mint: 'object.mint',
    author: 'object.author'
  };

  var PRODUCER_BY = { core: true, binding: true, keyholder: true };
  var TRANSPORT = /refused this origin|keyholder origin is required|did not load|did not answer|failed to load|loaded without connect/;

  function refusal(op, by, why) { return { ok: false, op: op, by: by, why: why }; }

  /* A rejected call, read by R1–R5. `atConnect` is set by this client when the
     failure came from opening the connection, so R4 does not depend on words. */
  function refusalOf(op, method, err, atConnect) {
    var r = err && err.refusal;
    if (r && typeof r.why === 'string' && PRODUCER_BY[r.by] === true) {
      return refusal(typeof r.op === 'string' ? r.op : op, r.by, r.why);
    }
    var said = String(err && err.message !== undefined ? err.message : err);
    var why = said.indexOf(method + ': ') === 0 ? said.slice(method.length + 2) : said;
    if (atConnect) return refusal(op, 'transport', why);
    if (why.indexOf('no such method: ') === 0) {
      return refusal(op, 'keyholder', 'The producer does not serve ' + method + ' yet.');
    }
    return refusal(op, TRANSPORT.test(why) ? 'transport' : 'keyholder', why);
  }

  function isRefused(v) {
    return typeof v === 'object' && v !== null && v.ok === false && typeof v.why === 'string';
  }

  /* A refusal made on the site's side, before anything was asked. */
  function siteRefusal(op, why) { return refusal(op, 'site', why); }

  /* The recovery records the fold carries (recovery-contract.json). Every value
     is the core's; this only refuses to read a malformed record as a good one.
     A record it cannot read is left out, and left out is drawn as "cannot tell". */
  function objectRecovery(r) {
    if (!r || typeof r.named !== 'boolean' || typeof r.speakable !== 'boolean') return null;
    var nums = function (a) { return Array.isArray(a) ? a.filter(function (e) { return typeof e === 'number'; }) : []; };
    return {
      named: r.named,
      speakable: r.speakable,
      first_epoch: typeof r.first_epoch === 'number' ? r.first_epoch : null,
      current_epoch: typeof r.current_epoch === 'number' ? r.current_epoch : null,
      unreadable_epochs: nums(r.unreadable_epochs)
    };
  }
  var TAILS = { intact: true, missing: true, forked: true, unknown: true };
  function accountRecovery(r) {
    if (!r || typeof r !== 'object') return null;
    return {
      /* "A missing or malformed record, or a tail value not in the list, reads as
         unknown, never as intact." */
      tail: TAILS[r.tail] === true ? r.tail : 'unknown',
      holes: Array.isArray(r.holes) ? r.holes.filter(function (i) { return typeof i === 'number'; }) : [],
      extra: typeof r.extra === 'number' ? r.extra : 0,
      found: typeof r.found === 'number' ? r.found : null
    };
  }

  /* The producer's normalise: the one the caller hands in, or the one loaded
     beside this file. Never a copy of its own. */
  function findNormalise(given) {
    if (typeof given === 'function') return given;
    var F = root && root.WallflowersFold;
    return F && typeof F.normalise === 'function' ? F.normalise : null;
  }

  /**
   * @param connect  () => Promise<{call(method, args)}>
   * @param options  {normalise?} — defaults to WallflowersFold.normalise
   */
  function socialApi(connect, options) {
    var normalise = findNormalise(options && options.normalise);
    var port = null;
    var link = function () {
      if (!port) {
        port = Promise.resolve().then(connect).then(null, function (e) {
          port = null;
          var tagged = e instanceof Error ? e : new Error(String(e));
          tagged.atConnect = true;
          throw tagged;
        });
      }
      return port;
    };

    /* One call: a value as {v}, or a refusal as itself. */
    function ask(method, op, args) {
      return link().then(function (p) { return p.call(method, args); }).then(
        function (v) { return { v: v }; },
        function (e) {
          /* A fixture's refusal is the SITE's: nothing was asked of WallFlowers, so
             neither `keyholder` nor any producer `by` would be true. */
          if (e && e.fixture === true) return siteRefusal(op, e.why);
          return refusalOf(op, method, e, !!(e && e.atConnect));
        });
    }
    function unread(op, why) { return refusal(op, 'keyholder', why); }

    return {
      /* {writes, ops}. `writes` is whether THIS origin may write at all; anything
         but a plain true is not taken as permission. */
      capabilities: function () {
        return ask(METHOD.capabilities, METHOD.capabilities, {}).then(function (a) {
          if (isRefused(a)) return a;
          var table = a.v && a.v.ops;
          if (!table || typeof table !== 'object' || Array.isArray(table)) {
            return unread(METHOD.capabilities, 'The producer answered without its op table.');
          }
          var ops = {};
          Object.keys(table).forEach(function (op) {
            var s = table[op];
            if (!s || typeof s.reachable !== 'boolean') return;
            ops[op] = { reachable: s.reachable, why: typeof s.why === 'string' ? s.why : null,
                        needs: typeof s.needs === 'string' ? s.needs : null,
                        /* The kinds the door will write this op onto. Check it before
                           asking: a note op is not written onto a Site's group. */
                        on: Array.isArray(s.on) ? s.on.filter(function (k) { return typeof k === 'string'; }) : [] };
          });
          return { writes: a.v.writes === true, ops: ops };
        });
      },

      fold: function (group) {
        if (!normalise) {
          return Promise.resolve(siteRefusal(METHOD.fold,
            "The producer's fold (wallflowers-fold.js) did not load here, so an answer could not be read."));
        }
        return ask(METHOD.fold, METHOD.fold, group ? { group: group } : {}).then(function (a) {
          if (isRefused(a)) return a;
          var raw = a.v;
          if (!raw || !Array.isArray(raw.groups) || !Array.isArray(raw.rejected)) {
            return unread(METHOD.fold, 'The producer answered in a shape this site does not read, so nothing is drawn from it.');
          }
          var model = normalise(raw);
          /* normalise keeps the groups in order; the per-object records ride beside them. */
          model.groups.forEach(function (g, i) { g.recovery = objectRecovery((raw.groups[i] || {}).recovery); });
          model.recovery = accountRecovery(raw.recovery);
          model.fixture = raw.fixture === true;
          return model;
        });
      },

      mint: function (kind, draft) {
        var op = kind + '.mint';
        return ask(METHOD.mint, op, { kind: kind, draft: draft }).then(function (a) {
          if (isRefused(a)) return a;
          var group = a.v && a.v.group_id;
          if (typeof group !== 'string') {
            return unread(op, METHOD.mint + ' answered without a group id, so nothing is shown as made.');
          }
          return { ok: true, group: group };
        });
      },

      author: function (group, op, args) {
        return ask(METHOD.author, op, { group: group, op: op, args: args }).then(function (a) {
          if (isRefused(a)) return a;
          var deltaId = a.v && a.v.deltaId;
          if (typeof deltaId !== 'string') {
            return unread(op, METHOD.author + ' answered without a delta id, so nothing is shown as written.');
          }
          return { ok: true, deltaId: deltaId, group: group };
        });
      }
    };
  }

  /* A connection that answers object.fold from a producer fold `answer` and
     refuses every write in words. The model it yields says `fixture: true`. */
  function sim(answer) {
    var copy = JSON.parse(JSON.stringify(answer || { groups: [], rejected: [] }));
    copy.fixture = true;
    var WHY = 'this is a fixture — nothing is written, and nothing here is a member';
    var no = function (m) {
      var e = new Error(m + ': ' + WHY);
      e.fixture = true;
      e.why = WHY;
      return Promise.reject(e);
    };
    return function () {
      return Promise.resolve({
        call: function (method) {
          if (method === METHOD.fold) return Promise.resolve(copy);
          if (method === METHOD.capabilities) {
            /* Every op unreachable, because nothing is written, but each with the
               kinds the core would write it onto (`ops_on`, carried in the served
               sim answer), so a site's own kind checks run on a fixture page as
               they do on a real one. */
            var ops = {};
            var why = 'this is a fixture — nothing is written';
            if (copy.ops_on && typeof copy.ops_on === 'object') {
              Object.keys(copy.ops_on).forEach(function (k) {
                ops[k] = { reachable: false, why: why, on: copy.ops_on[k] };
              });
            } else {
              var O = root && root.WallflowersOps;
              if (O && O.OPS) Object.keys(O.OPS).forEach(function (k) { ops[k] = { reachable: false, why: why }; });
            }
            return Promise.resolve({ writes: false, ops: ops });
          }
          return no(method);
        }
      });
    };
  }

  return {
    METHOD: METHOD,
    socialApi: socialApi,
    refusalOf: refusalOf,
    isRefused: isRefused,
    siteRefusal: siteRefusal,
    sim: sim
  };
});
