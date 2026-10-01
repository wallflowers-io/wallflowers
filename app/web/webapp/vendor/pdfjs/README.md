# PDF.js, vendored

pdfjs-dist 6.3.289 (Apache-2.0, LICENSE beside it), `build/pdf.min.mjs` and `build/pdf.worker.min.mjs`
as npm published them, unchanged. The webapp's PDF viewer renders a Resource that is a PDF
(Ralph, 28 Sep: "Resources is required to handle PDFs, and should render them in a modal
viewer"), page by page onto canvases, the same in Safari, iOS Safari and Chrome. Served by the
Door from this directory, as everything the webapp runs is: no CDN is asked for it.

    shasum -a 256
    f80490490320511e5df18c580b9edd6b5db8058dceebaf6f161992e0a964b9e2  pdf.min.mjs
    8ab0e5e30031b4a06ecfddd5ae9562f0227f830ee7ec9ed1a968b134243d2386  pdf.worker.min.mjs
