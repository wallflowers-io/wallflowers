/* THE DOOR IS WHERE THIS PAGE CAME FROM, never what its address says (SEC-9):
   the door serves the page, so the page is same-origin with it. `?door=` points
   a loopback page at another loopback door, and nothing else: a link cannot
   send a sign-in anywhere it names. Pure, so a test can hand it any address. */
(function (root) {
  'use strict';
  var LOOPBACK = /^(localhost|127\.0\.0\.1|\[::1\])$/;

  function doorOrigin(loc) {
    if (!LOOPBACK.test(loc.hostname)) return loc.origin;
    var asked = new URLSearchParams(loc.search).get('door');
    try {
      if (asked && LOOPBACK.test(new URL(asked).hostname)) return new URL(asked).origin;
    } catch (e) { /* not a URL: this page's own door */ }
    return loc.origin;
  }

  root.doorOrigin = doorOrigin;
  if (typeof module !== 'undefined') module.exports = { doorOrigin: doorOrigin };
})(typeof self !== 'undefined' ? self : this);
