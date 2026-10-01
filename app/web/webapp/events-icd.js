/* events-icd.js — W-98 Events: pure readers of the served ICD and of the views, for both Events
   pages (events-edit.js, events-page.js). Nothing here states a value the ICD holds: the
   recurrence grammar, the media args, the acts' vocabulary, an op's ego, and the keys the Site's
   view carries answers under are each read from the catalogue the Door serves. */
(function (root) {
  /* The declaration of op `name`, in any kind or facet; null where the ICD has none. */
  function opOf(icd, name) {
    var found = null;
    ['kinds', 'facets'].forEach(function (t) {
      Object.keys((icd && icd[t]) || {}).forEach(function (k) {
        var ops = icd[t][k].ops || {};
        if (!found && ops[name]) found = ops[name];
      });
    });
    return found;
  }
  function opServed(icd, name) { return !!opOf(icd, name); }
  function args(icd, name) { return (opOf(icd, name) || {}).args || {}; }
  function vocabulary(icd, name, arg) { return Object.keys((args(icd, name)[arg] || {}).vocabulary || {}); }

  /* ── recurrence: the grammar event.setProfile's `recurrence` declares ───────── */
  function grammar(icd) { return (args(icd, 'event.setProfile').recurrence || {}).grammar || {}; }
  function frequencies(icd) { return ((grammar(icd).FREQ || {}).oneOf || []).slice(); }
  function weekdays(icd) { return ((grammar(icd).BYDAY || {}).subsetOf || []).slice(); }
  function whole(n, min) { return typeof n === 'number' && Math.floor(n) === n && n >= min; }
  /* A rule from a choice {freq, interval, byday, count, until}, or '' for none; a choice the
     grammar refuses throws, naming the part. */
  function ruleFrom(icd, c) {
    if (!c || !c.freq) return '';
    var g = grammar(icd), parts = [];
    if (frequencies(icd).indexOf(c.freq) < 0) throw new Error('FREQ ' + c.freq);
    parts.push('FREQ=' + c.freq);
    var interval = c.interval == null ? (g.INTERVAL || {}).default : c.interval;
    if (!whole(interval, (g.INTERVAL || {}).integerMin)) throw new Error('INTERVAL ' + c.interval);
    if (interval !== (g.INTERVAL || {}).default) parts.push('INTERVAL=' + interval);
    var byday = c.byday || [];
    if (byday.length) {
      var only = (g.BYDAY || {}).onlyWith || {};
      Object.keys(only).forEach(function (k) { if (c[k.toLowerCase()] !== only[k]) throw new Error('BYDAY only with ' + k + '=' + only[k]); });
      byday.forEach(function (d) { if (weekdays(icd).indexOf(d) < 0) throw new Error('BYDAY ' + d); });
      parts.push('BYDAY=' + byday.join(','));
    }
    if (c.count != null && c.until != null) throw new Error('COUNT with UNTIL');
    if (c.count != null) {
      if (!whole(c.count, (g.COUNT || {}).integerMin)) throw new Error('COUNT ' + c.count);
      parts.push('COUNT=' + c.count);
    }
    if (c.until != null) {
      if (!whole(c.until, 1)) throw new Error('UNTIL ' + c.until);
      parts.push('UNTIL=' + c.until);
    }
    return parts.join(';');
  }
  /* A rule read back as a choice, or null where the grammar cannot express it (held as it is). */
  function parseRule(icd, rule) {
    if (!rule) return null;
    var g = grammar(icd), c = { interval: (g.INTERVAL || {}).default, byday: [] }, bad = false;
    String(rule).split(';').forEach(function (p) {
      var kv = p.split('='), k = kv[0], v = kv.slice(1).join('=');
      if (k === 'FREQ') c.freq = v;
      else if (k === 'INTERVAL') c.interval = Number(v);
      else if (k === 'BYDAY') c.byday = v ? v.split(',') : [];
      else if (k === 'COUNT') c.count = Number(v);
      else if (k === 'UNTIL') c.until = Number(v);
      else bad = true;
    });
    if (bad || !c.freq) return null;
    try { return ruleFrom(icd, c) === String(rule) ? c : null; } catch (e) { return null; }
  }

  /* The grammar's weekdays as the reader's own short day names (RFC 5545's tokens, drawn by
     Intl in the reader's locale); the tokens are what is written. */
  var RFC_DAY = { SU: 0, MO: 1, TU: 2, WE: 3, TH: 4, FR: 5, SA: 6 };
  function weekdayNames(icd) {
    var out = {};
    weekdays(icd).forEach(function (d) {
      var n = RFC_DAY[d];
      try { out[d] = n == null ? d : new Intl.DateTimeFormat(undefined, { weekday: 'short', timeZone: 'UTC' }).format(new Date(Date.UTC(2024, 0, 7 + n))); }
      catch (e) { out[d] = d; }
    });
    return out;
  }

  /* ── media: an op's prefixed args (`banner`, `bannerMime`, …) and the cap on the data ─ */
  function mediaCaps(icd, op, prefix) {
    var a = args(icd, op);
    return {
      maxLength: (a[prefix] || {}).maxLength,
      suffixes: Object.keys(a).filter(function (k) { return k.indexOf(prefix) === 0 && k !== prefix; })
        .map(function (k) { return k.slice(prefix.length); })
    };
  }
  /* The event's media as its view holds it: each register already wins over setMedia's. */
  function mediaOf(view) {
    view = view || {};
    return { banner: view.banner || null, clip: view.clip || null, photos: (view.photos || []).slice(), video: view.video_url || null };
  }

  /* ── acts: event.setLineup's `acts` ───────────────────────────────────────── */
  function actsSpec(icd) { return args(icd, 'event.setLineup').acts || {}; }
  function actRoles(icd) { return Object.keys((((actsSpec(icd).items || {}).role) || {}).vocabulary || {}); }
  function actsMax(icd) { return actsSpec(icd).maxItems; }
  function actsOf(view) {
    return ((view || {}).acts || []).map(function (a) {
      return { member: a.member || null, object: a.object || null, role: a.role, start: a.start == null ? null : a.start,
        end: a.end == null ? null : a.end, confirmed: !!a.confirmed };
    });
  }
  /* The `acts` arg: each act its keys the ICD's items name and no others; a list the fold would
     refuse throws, naming why. */
  function actsArg(icd, acts) {
    var items = actsSpec(icd).items || {}, seen = {}, max = actsMax(icd);
    if (max != null && acts.length > max) throw new Error('over ' + max + ' acts');
    return JSON.stringify(acts.map(function (a) {
      var who = a.member ? 'member' : 'object';
      if (!!a.member === !!a.object) throw new Error('an act is a member or an object, exactly one');
      if (actRoles(icd).indexOf(a.role) < 0) throw new Error('role ' + a.role);
      if (a.start != null && a.end != null && a.end < a.start) throw new Error('an act ends before it starts');
      var key = who + ':' + a[who];
      if (seen[key]) throw new Error('an act listed twice');
      seen[key] = true;
      var out = {};
      Object.keys(items).forEach(function (k) { if (a[k] != null) out[k] = a[k]; });
      return out;
    }));
  }

  /* ── who may: an op's ego, asked of THIS object ───────────────────────────── */
  function rolesOf(o) {
    var r = ((o && o.view) || {}).roles || {};
    return Array.isArray(r) ? r.reduce(function (m, p) { m[p[0]] = p[1]; return m; }, {}) : r;
  }
  function may(icd, name, o, me) {
    var d = opOf(icd, name);
    if (!d || !o || !me) return false;
    return String(d.ego || '').split('|').some(function (e) {
      if (e === 'owner') return o.owner === me;
      if (e === 'member') return (o.members || []).indexOf(me) >= 0;
      if (e.indexOf('role:') === 0) return rolesOf(o)[me] === e.slice(5);
      return false;
    });
  }
  /* How `me` saves the event's profile: the owner whole (setProfile), a co-host by editProfile. */
  function editOp(icd, event, me) {
    if (may(icd, 'event.setProfile', event, me)) return 'event.setProfile';
    if (may(icd, 'event.editProfile', event, me)) return 'event.editProfile';
    return null;
  }
  function hostsOf(event) {
    var r = rolesOf(event);
    return [event.owner].concat(Object.keys(r).filter(function (m) { return r[m] === 'admin' && m !== event.owner; }));
  }
  function visibilities(icd) { return vocabulary(icd, 'base.setVisibility', 'visibility'); }
  function visibilityOf(event) { return ((event && event.view) || {}).visibility || null; }
  /* A kind's visibility when none is set: the facet's own `defaults`. */
  function visibilityDefault(icd, kind) { return (((icd && icd.facets && icd.facets.visibility) || {}).defaults || {})[kind] || null; }
  function statuses(icd) { return vocabulary(icd, 'event.setProfile', 'status'); }

  /* The event's profile as setProfile's args: each arg the view holds under its snake-case name,
     a list (lineup's old names) as the newline text it was written as. */
  function snake(k) { return k.replace(/[A-Z]/g, function (c) { return '_' + c.toLowerCase(); }); }
  function heldProfile(icd, view) {
    var out = {};
    Object.keys(args(icd, 'event.setProfile')).forEach(function (k) {
      var v = (view || {})[snake(k)];
      if (v == null) return;
      if (Array.isArray(v)) { if (!v.length) return; v = v.join('\n'); }
      if (typeof v === 'object') return;
      out[k] = v;
    });
    return out;
  }

  /* ── the Site's answers, under the keys the ICD's own `view` names ──────────── */
  function viewKey(icd, name, i) { return Object.keys((opOf(icd, name) || {}).view || {})[i]; }
  function at(site, key, event) { return (((site && site.view) || {})[key] || {})[event]; }
  function rsvpsOf(icd, site, event) { return at(site, viewKey(icd, 'group.rsvp', 0), event) || {}; }
  function countsOf(icd, site, event) {
    var c = at(site, viewKey(icd, 'group.rsvp', 1), event) || {};
    return { going: c.going || 0, maybe: c.maybe || 0, declined: c.declined || 0, guests: c.guests || 0 };
  }
  function registrationOf(icd, site, event) { return at(site, viewKey(icd, 'group.setRegistration', 0), event) || null; }
  function rsvpStatuses(icd) { return vocabulary(icd, 'group.rsvp', 'status'); }

  root.WallFlowers = root.WallFlowers || {};
  root.WallFlowers.EventsIcd = {
    opOf: opOf, opServed: opServed, args: args,
    frequencies: frequencies, weekdays: weekdays, weekdayNames: weekdayNames, ruleFrom: ruleFrom, parseRule: parseRule,
    mediaCaps: mediaCaps, mediaOf: mediaOf,
    actRoles: actRoles, actsMax: actsMax, actsOf: actsOf, actsArg: actsArg,
    may: may, editOp: editOp, hostsOf: hostsOf, visibilities: visibilities, visibilityOf: visibilityOf, visibilityDefault: visibilityDefault, statuses: statuses,
    heldProfile: heldProfile, rsvpsOf: rsvpsOf, countsOf: countsOf, registrationOf: registrationOf, rsvpStatuses: rsvpStatuses
  };
})(window);
