/* ═══════════════════════════════════════════════════════════════════════════
   perf.js — what a person waits on, timed (Ralph, 28 Sep: latency first).

   User Timing on the Performance timeline, one measure per step a person sees,
   named `wf:<step>`. NOTHING IS SENT AND NOTHING IS DRAWN: the numbers stay in
   this tab, for DevTools' Performance panel, for `WallFlowersPerf.summary()` in
   the console, and for a harness that calls it.

   Served by the Door at /door/perf.js from its binary, so the sign-in window
   loads it with no webapp files; the webapp loads the same route. A page where
   it did not load keeps working: every caller falls back to a stub.

     var t = WallFlowersPerf.begin();         a start
     WallFlowersPerf.end('step', t)           measured now
     WallFlowersPerf.shown('step', t)         measured once the next frame is painted
     WallFlowersPerf.shown('step', 0)         from navigation, for a first render
     WallFlowersPerf.summary()                n, p50, p95, max per step; printed
   ═══════════════════════════════════════════════════════════════════════════ */
(function () {
  'use strict';
  var P = window.performance;
  var ok = !!(P && P.now && P.measure && P.getEntriesByType);
  function now() { return ok ? P.now() : 0; }

  function measure(step, t0, detail) {
    if (!ok || t0 == null) return;
    try { P.measure('wf:' + step, { start: t0, end: P.now(), detail: detail || null }); } catch (e) {}
  }

  /* On screen: the frame after the change has been painted. rAF runs before the
     paint; the task it queues runs after it. */
  function frame(f) {
    if (typeof requestAnimationFrame !== 'function') return f();
    requestAnimationFrame(function () { setTimeout(f, 0); });
  }

  /* The browser's own: first paint and the largest one, and main-thread tasks
     over 50 ms (Chromium only). Kept as they arrive. */
  var lcp = null, long = { n: 0, ms: 0, max: 0 };
  function observe(type, f) {
    try { new PerformanceObserver(function (l) { l.getEntries().forEach(f); }).observe({ type: type, buffered: true }); } catch (e) {}
  }
  if (ok && typeof PerformanceObserver === 'function') {
    observe('largest-contentful-paint', function (e) { lcp = e.startTime; });
    observe('longtask', function (e) { long.n++; long.ms += e.duration; long.max = Math.max(long.max, e.duration); });
  }
  // The default 250 fills in a long session; a harness reads the whole landing.
  try { if (P.setResourceTimingBufferSize) P.setResourceTimingBufferSize(1000); } catch (e) {}

  function rank(xs, q) { return xs[Math.min(xs.length - 1, Math.max(0, Math.ceil(q * xs.length) - 1))]; }
  function ms(x) { return x == null ? null : Math.round(x); }

  function summary(quiet) {
    if (!ok) return null;
    var by = {};
    P.getEntriesByType('measure').forEach(function (m) {
      if (m.name.indexOf('wf:') !== 0) return;
      (by[m.name.slice(3)] = by[m.name.slice(3)] || []).push(m.duration);
    });
    var steps = {};
    Object.keys(by).sort().forEach(function (k) {
      var xs = by[k], last = xs[xs.length - 1], s = xs.slice().sort(function (a, b) { return a - b; });
      steps[k] = { n: s.length, p50: ms(rank(s, 0.5)), p95: ms(rank(s, 0.95)), max: ms(s[s.length - 1]), last: ms(last) };
    });
    var nav = P.getEntriesByType('navigation')[0] || {}, paint = {};
    P.getEntriesByType('paint').forEach(function (e) { paint[e.name] = ms(e.startTime); });
    var res = { n: 0, transfer: 0, encoded: 0, decoded: 0 };
    P.getEntriesByType('resource').forEach(function (r) {
      res.n++; res.transfer += r.transferSize || 0; res.encoded += r.encodedBodySize || 0; res.decoded += r.decodedBodySize || 0;
    });
    var out = {
      at: location.pathname, timeOrigin: P.timeOrigin, steps: steps,
      page: { ttfb: ms(nav.responseStart), domContentLoaded: ms(nav.domContentLoadedEventEnd), load: ms(nav.loadEventEnd),
              fcp: paint['first-contentful-paint'] || null, lcp: ms(lcp), bytes: nav.transferSize || 0 },
      longtasks: { n: long.n, ms: ms(long.ms), max: ms(long.max) },
      resources: res
    };
    if (!quiet && window.console && console.table) console.table(steps);
    return out;
  }

  window.WallFlowersPerf = {
    begin: now,
    end: measure,
    shown: function (step, t0, detail) { if (ok && t0 != null) frame(function () { measure(step, t0, detail); }); },
    summary: summary
  };
})();
