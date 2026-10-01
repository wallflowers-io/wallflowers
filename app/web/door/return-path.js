/* Where the window sends a person after sign-in: a place on this origin, and
   nowhere else (CS-39, SEC-41). Resolved as a URL, never judged by its spelling:
   the parser drops tab, CR and LF, so "/\t/evil.example" is "//evil.example".
   Pure, so a test can hand it anything. */
(function (root) {
  'use strict';

  function returnPath(r, origin) {
    try {
      var u = new URL(r || '/', origin);
      // The absolute URL: a path of "//host" would be read as another origin.
      return u.origin === origin ? u.href : origin + '/';
    } catch (e) {
      return origin + '/';
    }
  }

  root.returnPath = returnPath;
  if (typeof module !== 'undefined') module.exports = { returnPath: returnPath };
})(globalThis);
