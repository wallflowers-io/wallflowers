/* ═══════════════════════════════════════════════════════════════════════════
   embed.js — a site's anchor into WallFlowers.

       <script src="https://…/webapp/embed.js" data-site="<group object id>"></script>

   or, declared rather than scripted:

       <wallflowers-site site="<group object id>"></wallflowers-site>

   What pacific.js did for a host page, and only that: one <div> on <body>,
   holding a shadow root that `:host { all: initial }` closes, so the page's CSS
   cannot reach in and nothing here reaches out. Mounted on <body>, not where
   the tag sits, because `position: fixed` is measured against the nearest
   transformed ancestor and a host may have one anywhere. It takes no layout.

   The mark is an ANCHOR to the webapp, landing on the site. The interior does
   not open over the host: the door's session is a cookie on the webapp's
   origin, and inside a third-party frame that cookie is refused.

   The webapp's address is this script's own, so there is nothing to configure.
   ═══════════════════════════════════════════════════════════════════════════ */
(function () {
  'use strict';

  var tag = document.currentScript;
  var BASE = new URL('.', (tag && tag.src) || location.href).href;

  var CSS =
    /* !important on the host: the host element is in the page's document, and
       there an ordinary :host rule loses to the page's own. */
    ':host{all:initial!important;position:fixed!important;top:20px!important;right:20px!important;' +
    'z-index:2147483000!important;display:block!important}' +
    'a{display:block;width:44px;height:44px;border-radius:11px;outline:0;' +
    'transition:transform .16s cubic-bezier(.16,1,.3,1)}' +
    'a:hover{transform:scale(1.06)}' +
    'a:focus-visible{box-shadow:0 0 0 3px rgba(20,24,30,.35)}' +
    'img{display:block;width:100%;height:100%}';

  function mount(site) {
    var host = document.createElement('div');
    host.setAttribute('data-wallflowers', site || '');
    document.body.appendChild(host);
    var root = host.attachShadow({ mode: 'closed' });
    var a = document.createElement('a');
    a.href = BASE + (site ? '#site=' + encodeURIComponent(site) : '#signin');
    a.target = '_blank';
    a.rel = 'noopener';
    a.setAttribute('aria-label', 'WallFlowers');
    var img = document.createElement('img');
    img.src = BASE + 'brand/wallflowers-icon.svg';
    img.alt = '';
    a.appendChild(img);
    var style = document.createElement('style');
    style.textContent = CSS;
    root.appendChild(style);
    root.appendChild(a);
    return host;
  }

  if (typeof customElements !== 'undefined' && !customElements.get('wallflowers-site')) {
    customElements.define('wallflowers-site', class extends HTMLElement {
      connectedCallback() {
        if (this._up) return;
        this._up = 1;
        this.style.display = 'none';
        mount(this.getAttribute('site') || '');
      }
    });
  }

  if (tag && tag.hasAttribute('data-site')) {
    var boot = function () { mount(tag.dataset.site); };
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', boot);
    else boot();
  }
})();
