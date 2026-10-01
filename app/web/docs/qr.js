/* ═══════════════════════════════════════════════════════════════════════════
   QR — the smallest encoder that draws a pairing offer, and a reader for it.

   WHY THIS IS HERE AND NOT A DEPENDENCY. Pacific's embed promise is one script
   tag, no build step, and nothing fetched from a third origin at runtime. A QR
   library from a CDN would break all three, and the one thing this code draws is
   a public key that authorises a device — a script from someone else's origin is
   the last thing that should be holding it. So: ~300 lines, here, readable.

   WHAT IT COVERS, AND WHAT IT REFUSES. Versions 1–6, error level M, alphanumeric
   and byte mode. That is a deliberate floor rather than an unfinished ceiling:

     · LEVEL M because that is what iOS already generates with
       (`ContactExport.swift` sets correctionLevel "M"), and two mirrors of one
       core should not disagree about how much damage a code survives.
     · VERSIONS 1–6 because version 7 is where the format gains a second
       embedded block — the version information — and a pairing offer is 79
       characters. v6-M holds 108 data codewords; the offer needs 56. There is
       room for the payload to grow by half before this has to learn v7, and an
       encoder that stops loudly at a boundary it has actually tested is worth
       more than one that guesses past it.
     · ALPHANUMERIC because the payload is built to earn it — see PAIR_PREFIX in
       pacific-qr-link.html. Uppercase hex plus `:` are all in QR's alphanumeric
       set, which costs 5.5 bits a character where byte mode costs 8.

   `fit` throws rather than truncating. A QR that silently dropped the tail of a
   key would encode a DIFFERENT key, scan cleanly, and pair two devices to
   nothing — which is the failure this whole file exists to not have.
   ═══════════════════════════════════════════════════════════════════════════ */
(function (root) {
  'use strict';

  /* ── GF(256) ────────────────────────────────────────────────────────────
     The Reed–Solomon field, primitive polynomial 0x11D. Log/antilog tables
     built once so multiplication is two lookups and an add. */
  var EXP = new Uint8Array(512), LOG = new Uint8Array(256);
  (function () {
    for (var i = 0, x = 1; i < 255; i++) {
      EXP[i] = x; LOG[x] = i;
      x <<= 1; if (x & 0x100) x ^= 0x11D;
    }
    for (var j = 255; j < 512; j++) EXP[j] = EXP[j - 255];
  })();
  function mul(a, b) { return (a === 0 || b === 0) ? 0 : EXP[LOG[a] + LOG[b]]; }

  /* The generator polynomial for `n` EC codewords: ∏(x − α^i). */
  function gen(n) {
    var p = [1];
    for (var i = 0; i < n; i++) {
      var q = p.concat([0]);
      for (var j = 0; j < p.length; j++) q[j + 1] ^= mul(p[j], EXP[i]);
      p = q;
    }
    return p;
  }

  /* The EC codewords for one block — polynomial division, remainder out. */
  function ecc(data, n) {
    var g = gen(n), r = new Array(data.length + n).fill(0), i, j;
    for (i = 0; i < data.length; i++) r[i] = data[i];
    for (i = 0; i < data.length; i++) {
      var f = r[i];
      if (!f) continue;
      for (j = 0; j < g.length; j++) r[i + j] ^= mul(g[j], f);
    }
    return r.slice(data.length);
  }

  /* ── the version tables, level M only ───────────────────────────────────
     [total codewords, EC per block, blocks in group 1, data per block in g1,
      blocks in group 2, data per block in g2] — the standard's Table 9 rows
     for M, transcribed for 1..6 and no further. */
  var M = {
    1: [26, 10, 1, 16, 0, 0],
    2: [44, 16, 1, 28, 0, 0],
    3: [70, 26, 1, 44, 0, 0],
    4: [100, 18, 2, 32, 0, 0],
    5: [134, 24, 2, 43, 0, 0],
    6: [172, 16, 4, 27, 0, 0]
  };
  var MAXVER = 6;
  /* Alignment pattern centres per version. v1 has none. */
  var ALIGN = { 1: [], 2: [6, 18], 3: [6, 22], 4: [6, 26], 5: [6, 30], 6: [6, 34] };
  /* Bits of padding after the data, before the first codeword boundary. */
  var REMAINDER = { 1: 0, 2: 7, 3: 7, 4: 7, 5: 7, 6: 7 };

  function dataCodewords(v) {
    var t = M[v];
    return t[2] * t[3] + t[4] * t[5];
  }

  /* ── modes ──────────────────────────────────────────────────────────────
     Alphanumeric's 45-character set, in its own index order. `:` is in it,
     which is what lets a scheme prefix stay in the cheap mode. */
  var ALNUM = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:';
  function isAlnum(s) {
    for (var i = 0; i < s.length; i++) if (ALNUM.indexOf(s[i]) < 0) return false;
    return true;
  }

  /* A growable bit buffer. QR is bit-packed end to end, so this is the only
     place lengths are reasoned about. */
  function Bits() { this.b = []; }
  Bits.prototype.put = function (val, len) {
    for (var i = len - 1; i >= 0; i--) this.b.push((val >>> i) & 1);
  };
  Bits.prototype.len = function () { return this.b.length; };
  Bits.prototype.bytes = function () {
    var out = [];
    for (var i = 0; i < this.b.length; i += 8) {
      var byte = 0;
      for (var j = 0; j < 8; j++) byte = (byte << 1) | (this.b[i + j] || 0);
      out.push(byte);
    }
    return out;
  };

  /* Character-count bit widths. Versions 1–9 only, which is all this file goes
     to — the widths step up at v10 and again at v27. */
  function countBits(mode) { return mode === 'alnum' ? 9 : 8; }

  function encodeData(s, mode) {
    var bits = new Bits(), i;
    bits.put(mode === 'alnum' ? 0b0010 : 0b0100, 4);
    /* Byte mode counts BYTES, not characters. They differ the moment the
       payload leaves ASCII, and a count taken from `s.length` encodes a length
       shorter than the data that follows it — which decodes as a truncated
       string rather than as an error. */
    var u8 = mode === 'byte' ? new TextEncoder().encode(s) : null;
    bits.put(mode === 'byte' ? u8.length : s.length, countBits(mode));
    if (mode === 'alnum') {
      for (i = 0; i + 1 < s.length; i += 2) {
        bits.put(ALNUM.indexOf(s[i]) * 45 + ALNUM.indexOf(s[i + 1]), 11);
      }
      if (i < s.length) bits.put(ALNUM.indexOf(s[i]), 6);
    } else {
      for (i = 0; i < u8.length; i++) bits.put(u8[i], 8);
    }
    return bits;
  }

  /* The smallest version whose data capacity holds this payload. Throws rather
     than truncating — see the header. */
  function fit(s, mode) {
    var need = 4 + countBits(mode) +
      (mode === 'alnum'
        ? Math.floor(s.length / 2) * 11 + (s.length % 2 ? 6 : 0)
        : new TextEncoder().encode(s).length * 8);
    for (var v = 1; v <= MAXVER; v++) {
      if (dataCodewords(v) * 8 >= need) return v;
    }
    throw new Error('payload needs a QR version above ' + MAXVER +
                    ' (' + need + ' bits, v' + MAXVER + ' holds ' +
                    dataCodewords(MAXVER) * 8 + ') — this encoder stops here on purpose');
  }

  /* ── codewords ──────────────────────────────────────────────────────────
     Pad to capacity, split into blocks, compute EC per block, then INTERLEAVE:
     the standard reads one codeword from each block in turn so a burst of
     damage is spread across blocks rather than destroying one entirely. */
  function codewords(s, mode, v) {
    var t = M[v], cap = dataCodewords(v) * 8;
    var bits = encodeData(s, mode);
    /* Terminator: up to four zero bits, fewer if the payload nearly fills. */
    var term = Math.min(4, cap - bits.len());
    bits.put(0, term);
    while (bits.len() % 8) bits.put(0, 1);
    var d = bits.bytes();
    /* The two pad bytes the standard names, alternating, to the capacity. */
    var PAD = [0xEC, 0x11], k = 0;
    while (d.length < dataCodewords(v)) d.push(PAD[k++ % 2]);

    var blocks = [], eccs = [], at = 0, i;
    for (i = 0; i < t[2]; i++) { blocks.push(d.slice(at, at + t[3])); at += t[3]; }
    for (i = 0; i < t[4]; i++) { blocks.push(d.slice(at, at + t[5])); at += t[5]; }
    for (i = 0; i < blocks.length; i++) eccs.push(ecc(blocks[i], t[1]));

    var out = [], c, longest = Math.max.apply(null, blocks.map(function (b) { return b.length; }));
    for (c = 0; c < longest; c++) {
      for (i = 0; i < blocks.length; i++) if (c < blocks[i].length) out.push(blocks[i][c]);
    }
    for (c = 0; c < t[1]; c++) {
      for (i = 0; i < eccs.length; i++) out.push(eccs[i][c]);
    }
    return out;
  }

  /* ── the matrix ─────────────────────────────────────────────────────────
     Function patterns first, then the data snake, then the mask. `fn` marks
     every module the data must step over. */
  function size(v) { return v * 4 + 17; }

  function frame(v) {
    var n = size(v), m = [], fn = [], y, x, i;
    for (y = 0; y < n; y++) { m.push(new Array(n).fill(0)); fn.push(new Array(n).fill(0)); }

    function finder(cy, cx) {
      for (y = -1; y <= 7; y++) for (x = -1; x <= 7; x++) {
        var py = cy + y, px = cx + x;
        if (py < 0 || py >= n || px < 0 || px >= n) continue;
        var on = (y >= 0 && y <= 6 && (x === 0 || x === 6)) ||
                 (x >= 0 && x <= 6 && (y === 0 || y === 6)) ||
                 (y >= 2 && y <= 4 && x >= 2 && x <= 4);
        m[py][px] = on ? 1 : 0; fn[py][px] = 1;
      }
    }
    finder(0, 0); finder(0, n - 7); finder(n - 7, 0);

    /* Timing: the alternating rows that let a scanner find the module pitch. */
    for (i = 8; i < n - 8; i++) {
      m[6][i] = m[i][6] = (i % 2 === 0) ? 1 : 0;
      fn[6][i] = fn[i][6] = 1;
    }

    /* Alignment patterns, at every centre pair except the three that would
       collide with a finder. */
    var a = ALIGN[v];
    for (var ai = 0; ai < a.length; ai++) for (var aj = 0; aj < a.length; aj++) {
      var cy = a[ai], cx = a[aj];
      if ((cy <= 8 && cx <= 8) || (cy <= 8 && cx >= n - 9) || (cy >= n - 9 && cx <= 8)) continue;
      for (y = -2; y <= 2; y++) for (x = -2; x <= 2; x++) {
        m[cy + y][cx + x] = (Math.max(Math.abs(y), Math.abs(x)) !== 1) ? 1 : 0;
        fn[cy + y][cx + x] = 1;
      }
    }

    /* The dark module — always set, always here. */
    m[n - 8][8] = 1; fn[n - 8][8] = 1;
    /* Reserve the two format-information strips. */
    for (i = 0; i < 9; i++) { fn[8][i] = 1; fn[i][8] = 1; }
    for (i = 0; i < 8; i++) { fn[8][n - 1 - i] = 1; fn[n - 1 - i][8] = 1; }
    return { m: m, fn: fn, n: n };
  }

  /* The data snake: two columns at a time, right to left, alternating upward
     and downward, skipping column 6 because the vertical timing pattern owns it. */
  function place(f, cw, v) {
    var n = f.n, bits = [], i, j;
    for (i = 0; i < cw.length; i++) for (j = 7; j >= 0; j--) bits.push((cw[i] >>> j) & 1);
    for (i = 0; i < REMAINDER[v]; i++) bits.push(0);

    var k = 0, up = true;
    for (var col = n - 1; col > 0; col -= 2) {
      if (col === 6) col--;
      for (var r = 0; r < n; r++) {
        var y = up ? n - 1 - r : r;
        for (var c = 0; c < 2; c++) {
          var x = col - c;
          if (f.fn[y][x]) continue;
          f.m[y][x] = bits[k++] || 0;
        }
      }
      up = !up;
    }
  }

  var MASKS = [
    function (y, x) { return (y + x) % 2 === 0; },
    function (y) { return y % 2 === 0; },
    function (y, x) { return x % 3 === 0; },
    function (y, x) { return (y + x) % 3 === 0; },
    function (y, x) { return (Math.floor(y / 2) + Math.floor(x / 3)) % 2 === 0; },
    function (y, x) { return ((y * x) % 2) + ((y * x) % 3) === 0; },
    function (y, x) { return (((y * x) % 2) + ((y * x) % 3)) % 2 === 0; },
    function (y, x) { return (((y + x) % 2) + ((y * x) % 3)) % 2 === 0; }
  ];

  /* The standard's four penalty rules. Lower is better; the winner is the mask
     that leaves the fewest features a scanner could mistake for structure. */
  function penalty(m, n) {
    var p = 0, y, x, i, run, dark = 0;
    function line(get) {
      for (y = 0; y < n; y++) {
        run = 1;
        for (x = 1; x < n; x++) {
          if (get(y, x) === get(y, x - 1)) { run++; }
          else { if (run >= 5) p += 3 + (run - 5); run = 1; }
        }
        if (run >= 5) p += 3 + (run - 5);
      }
    }
    line(function (y, x) { return m[y][x]; });
    line(function (y, x) { return m[x][y]; });
    for (y = 0; y < n - 1; y++) for (x = 0; x < n - 1; x++) {
      var s = m[y][x] + m[y][x + 1] + m[y + 1][x] + m[y + 1][x + 1];
      if (s === 0 || s === 4) p += 3;
    }
    /* The 1:1:3:1:1 finder-lookalike, either orientation. */
    var pat1 = [1, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0], pat2 = [0, 0, 0, 0, 1, 0, 1, 1, 1, 0, 1];
    function scan(get) {
      for (y = 0; y < n; y++) for (x = 0; x + 11 <= n; x++) {
        var a = true, b = true;
        for (i = 0; i < 11; i++) {
          if (get(y, x + i) !== pat1[i]) a = false;
          if (get(y, x + i) !== pat2[i]) b = false;
        }
        if (a || b) p += 40;
      }
    }
    scan(function (y, x) { return m[y][x]; });
    scan(function (y, x) { return m[x][y]; });
    for (y = 0; y < n; y++) for (x = 0; x < n; x++) dark += m[y][x];
    p += Math.floor(Math.abs(dark * 100 / (n * n) - 50) / 5) * 10;
    return p;
  }

  /* Format info: 5 bits (EC level + mask) through BCH(15,5), then XOR 0x5412
     so an all-zero format still has structure. Level M is 0b00. */
  function formatBits(mask) {
    var v = (0b00 << 3) | mask, d = v << 10;
    for (var i = 4; i >= 0; i--) if (d & (1 << (i + 10))) d ^= 0x537 << i;
    return ((v << 10) | d) ^ 0x5412;
  }

  function putFormat(m, n, mask) {
    var f = formatBits(mask), i;
    for (i = 0; i < 15; i++) {
      /* MSB FIRST along the path: bit 14 lands at (8,0), not bit 0. Placing it
         the other way round still produces a well-formed 15-bit word in a
         plausible place, so this file's own reader — walking the same path with
         the same reversal — decodes it perfectly and reports a pass. Every real
         scanner sees a code with an unreadable format and gives up. Derived
         from Apple's own generator for a payload whose data area is byte-identical
         to ours, which is the only reason it was findable at all. */
      var bit = (f >>> (14 - i)) & 1;
      /* The copy around the top-left finder… */
      if (i < 6) m[8][i] = bit;
      else if (i === 6) m[8][7] = bit;
      else if (i === 7) m[8][8] = bit;
      else if (i === 8) m[7][8] = bit;
      else m[14 - i][8] = bit;
      /* …and the second copy, split across the other two corners, so a damaged
         corner never costs both.

         THE SPLIT IS AT 7, NOT 8. Bits 0–6 run up the left column from the
         bottom; bits 7–14 run along row 8 from column n−8. Splitting at 8 puts
         bit 7 in (n−8, 8) — which is the DARK MODULE, always set and not part
         of the format — and leaves column n−8 unwritten. The result still
         decodes from copy 1, so a reader that only consults copy 1 (this file's
         own, and the round-trip test with it) reports success while no real
         scanner sees a code at all. Caught by Apple's CIDetector, which is the
         whole reason a second implementation is in the loop. */
      if (i < 7) m[n - 1 - i][8] = bit;
      else m[8][n - 15 + i] = bit;
    }
  }

  /* Encode `text` to a module matrix. Returns {m, n, version, mask, mode}. */
  function encode(text) {
    var mode = isAlnum(text) ? 'alnum' : 'byte';
    var v = fit(text, mode);
    var cw = codewords(text, mode, v);
    var best = null;
    for (var mask = 0; mask < 8; mask++) {
      var f = frame(v);
      place(f, cw, v);
      for (var y = 0; y < f.n; y++) for (var x = 0; x < f.n; x++) {
        if (!f.fn[y][x] && MASKS[mask](y, x)) f.m[y][x] ^= 1;
      }
      putFormat(f.m, f.n, mask);
      var p = penalty(f.m, f.n);
      if (!best || p < best.p) best = { p: p, m: f.m, n: f.n, mask: mask };
    }
    return { m: best.m, n: best.n, version: v, mask: best.mask, mode: mode };
  }

  /* ── reading back ───────────────────────────────────────────────────────
     A GRID reader, not a camera one: it is handed a clean matrix, so there is
     no finder search and no perspective to undo. That is exactly enough to
     close the loop in a browser — the optics are proven separately, by Apple's
     own detector, against the PNG this draws (tools/qr-verify.swift).

     No error correction either. On a clean grid the EC codewords are
     redundant, and a reader that quietly repaired damage would hide the very
     drift this is here to catch. A wrong module should fail, loudly. */
  function decode(m, n) {
    /* Recover the mask from the format strip and undo it. */
    var f = 0, i;
    /* The same path as `putFormat`, read MSB first. */
    function fbit(i, v) { f |= v << (14 - i); }
    for (i = 0; i < 6; i++) fbit(i, m[8][i]);
    fbit(6, m[8][7]); fbit(7, m[8][8]); fbit(8, m[7][8]);
    for (i = 9; i < 15; i++) fbit(i, m[14 - i][8]);
    f ^= 0x5412;
    var mask = (f >>> 10) & 0b111;
    var v = (n - 17) / 4;
    if (!M[v]) throw new Error('unsupported version ' + v);

    var fr = frame(v), g = [], y, x;
    for (y = 0; y < n; y++) {
      g.push([]);
      for (x = 0; x < n; x++) {
        g[y][x] = (!fr.fn[y][x] && MASKS[mask](y, x)) ? m[y][x] ^ 1 : m[y][x];
      }
    }

    /* Walk the snake in the same order and read the bits back out. */
    var bits = [], up = true;
    for (var col = n - 1; col > 0; col -= 2) {
      if (col === 6) col--;
      for (var r = 0; r < n; r++) {
        y = up ? n - 1 - r : r;
        for (var c = 0; c < 2; c++) {
          x = col - c;
          if (fr.fn[y][x]) continue;
          bits.push(g[y][x]);
        }
      }
      up = !up;
    }

    /* De-interleave back to block order, then take group 1's data. The blocks
       are equal-length at every version this file supports, which is what makes
       the un-shuffle a stride rather than a table. */
    var t = M[v], nblocks = t[2] + t[4], per = t[3];
    var cw = [];
    for (i = 0; i + 8 <= bits.length; i += 8) {
      var b = 0;
      for (var j = 0; j < 8; j++) b = (b << 1) | bits[i + j];
      cw.push(b);
    }
    var data = new Array(nblocks * per);
    for (i = 0; i < nblocks * per; i++) {
      data[(i % nblocks) * per + Math.floor(i / nblocks)] = cw[i];
    }

    /* Read the header off the flattened data stream. */
    var s = [], k = 0;
    function take(len) {
      var out = 0;
      for (var q = 0; q < len; q++) {
        var idx = k + q, byte = data[idx >> 3];
        out = (out << 1) | ((byte >>> (7 - (idx & 7))) & 1);
      }
      k += len;
      return out;
    }
    var mode = take(4);
    if (mode === 0b0010) {
      var count = take(9), pairs = Math.floor(count / 2);
      for (i = 0; i < pairs; i++) {
        var pv = take(11);
        s.push(ALNUM[Math.floor(pv / 45)], ALNUM[pv % 45]);
      }
      if (count % 2) s.push(ALNUM[take(6)]);
    } else if (mode === 0b0100) {
      var cnt = take(8), bytes = [];
      for (i = 0; i < cnt; i++) bytes.push(take(8));
      return new TextDecoder().decode(new Uint8Array(bytes));
    } else {
      throw new Error('unsupported mode ' + mode.toString(2));
    }
    return s.join('');
  }

  /* ── drawing ────────────────────────────────────────────────────────────
     A QUIET ZONE IS NOT DECORATION. The standard asks for four modules of
     clear margin, and a scanner that cannot find it will not see the code at
     all — which looks exactly like a broken encoder. */
  var QUIET = 4;

  function draw(canvas, q, scale) {
    scale = scale || 6;
    var n = q.n, side = (n + QUIET * 2) * scale;
    canvas.width = side; canvas.height = side;
    var g = canvas.getContext('2d');
    g.fillStyle = '#fff'; g.fillRect(0, 0, side, side);
    g.fillStyle = '#000';
    for (var y = 0; y < n; y++) for (var x = 0; x < n; x++) {
      if (q.m[y][x]) g.fillRect((x + QUIET) * scale, (y + QUIET) * scale, scale, scale);
    }
    return { side: side, scale: scale, quiet: QUIET, n: n };
  }

  /* Sample a drawn code back off the canvas — the reader's half of `draw`, and
     the reason the two are in one file: the geometry is stated once. */
  function read(canvas, geo) {
    var g = canvas.getContext('2d');
    var img = g.getImageData(0, 0, canvas.width, canvas.height).data;
    var half = Math.floor(geo.scale / 2), m = [];
    for (var y = 0; y < geo.n; y++) {
      m.push([]);
      for (var x = 0; x < geo.n; x++) {
        var px = (x + geo.quiet) * geo.scale + half, py = (y + geo.quiet) * geo.scale + half;
        var o = (py * canvas.width + px) * 4;
        m[y][x] = img[o] < 128 ? 1 : 0;
      }
    }
    return decode(m, geo.n);
  }

  root.QR = { encode: encode, decode: decode, draw: draw, read: read, fit: fit, isAlnum: isAlnum };
})(this);
