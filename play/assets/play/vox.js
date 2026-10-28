/* vox.js — a block of voxels, the triangles that show it, and what stops a body in it.
 *
 * /play is the how-it-works scene (assets/how/ceremony.js) built instead of
 * drawn, one voxel to one of its art pixels. This file knows nothing of that
 * scene: it stores ids, meshes them, and answers "is this box in something".
 * What an id looks like is the palette's business (world.js).
 *
 * STORED IN CHUNKS OF 32, and a chunk that is all air is not stored. The sky is
 * most of the block.
 *
 * MESHED GREEDILY. A face shows when the voxel behind it is air; on each slice,
 * neighbouring faces of the same id merge into one rectangle. A vertex is six
 * shorts: x, y, z, id, which of the six ways it faces, and a spare.
 *
 * NOTHING HERE TOUCHES THE DOM OR THE GPU, so node can run it (tests/test_play.js).
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.Vox = factory();
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  var CS = 32, CS2 = CS * CS, CS3 = CS2 * CS, PS = CS + 2;
  var STRIDE = [1, PS * PS, PS];           // a step along x, y, z in a padded chunk

  function World(w, h, d) {
    this.w = w; this.h = h; this.d = d;
    this.nx = Math.ceil(w / CS); this.ny = Math.ceil(h / CS); this.nz = Math.ceil(d / CS);
    this.chunks = new Array(this.nx * this.ny * this.nz);
    this.dirty = new Uint8Array(this.chunks.length);
    this.soft = new Uint8Array(65536);     // ids a body walks through: grass, a bush
  }
  World.prototype.index = function (cx, cy, cz) { return (cy * this.nz + cz) * this.nx + cx; };
  World.prototype.inside = function (x, y, z) {
    return x >= 0 && y >= 0 && z >= 0 && x < this.w && y < this.h && z < this.d;
  };
  World.prototype.get = function (x, y, z) {
    if (x < 0 || y < 0 || z < 0 || x >= this.w || y >= this.h || z >= this.d) return 0;
    var c = this.chunks[((y >> 5) * this.nz + (z >> 5)) * this.nx + (x >> 5)];
    return c ? c[((y & 31) * CS + (z & 31)) * CS + (x & 31)] : 0;
  };
  World.prototype.set = function (x, y, z, id) {
    if (x < 0 || y < 0 || z < 0 || x >= this.w || y >= this.h || z >= this.d) return;
    var cx = x >> 5, cy = y >> 5, cz = z >> 5, i = (cy * this.nz + cz) * this.nx + cx, c = this.chunks[i];
    if (!c) { if (!id) return; c = this.chunks[i] = new Uint16Array(CS3); }
    c[((y & 31) * CS + (z & 31)) * CS + (x & 31)] = id;
    this.dirty[i] = 1;
    /* a face on a chunk's edge belongs to the mesh next door */
    var lx = x & 31, ly = y & 31, lz = z & 31;
    if (lx === 0 && cx > 0) this.dirty[i - 1] = 1;
    if (lx === 31 && cx < this.nx - 1) this.dirty[i + 1] = 1;
    if (lz === 0 && cz > 0) this.dirty[i - this.nx] = 1;
    if (lz === 31 && cz < this.nz - 1) this.dirty[i + this.nx] = 1;
    if (ly === 0 && cy > 0) this.dirty[i - this.nx * this.nz] = 1;
    if (ly === 31 && cy < this.ny - 1) this.dirty[i + this.nx * this.nz] = 1;
  };
  World.prototype.solid = function (x, y, z) {
    var id = this.get(x, y, z);
    return id !== 0 && !this.soft[id];
  };
  /* Is the box in anything a body cannot enter? The block's own sides and its
     floor are walls; above it is open sky. */
  World.prototype.hits = function (x0, y0, z0, x1, y1, z1) {
    if (x0 < 0 || z0 < 0 || y0 < 0 || x1 > this.w || z1 > this.d) return true;
    var ax = Math.floor(x0), ay = Math.floor(y0), az = Math.floor(z0);
    var bx = Math.ceil(x1), by = Math.min(this.h, Math.ceil(y1)), bz = Math.ceil(z1);
    for (var y = ay; y < by; y++) for (var z = az; z < bz; z++) for (var x = ax; x < bx; x++) {
      var id = this.get(x, y, z);
      if (id !== 0 && !this.soft[id]) return true;
    }
    return false;
  };
  /* The top of whatever is solid under a point: where a shadow lies. */
  World.prototype.floorUnder = function (x, y, z) {
    x = Math.floor(x); z = Math.floor(z);
    for (var yy = Math.min(this.h - 1, Math.floor(y)); yy >= 0; yy--) if (this.solid(x, yy, z)) return yy + 1;
    return 0;
  };

  /* A list that grows, in the type the GPU takes. */
  function Buf(Type, n) { this.a = new Type(n || 1024); this.n = 0; }
  Buf.prototype.room = function (k) {
    if (this.n + k <= this.a.length) return;
    var b = new this.a.constructor(Math.max(this.a.length * 2, this.n + k));
    b.set(this.a.subarray(0, this.n)); this.a = b;
  };
  Buf.prototype.done = function () { return this.a.slice(0, this.n); };

  function quad(V, I, p0, p1, p2, p3, id, n, front) {
    V.room(24); I.room(6);
    var a = V.a, k = V.n, base = k / 6, c = [p0, p1, p2, p3];
    for (var j = 0; j < 4; j++) {
      a[k++] = c[j][0]; a[k++] = c[j][1]; a[k++] = c[j][2]; a[k++] = id; a[k++] = n; a[k++] = 0;
    }
    V.n = k;
    var ia = I.a, m = I.n;
    ia[m++] = base; ia[m++] = base + (front ? 1 : 2); ia[m++] = base + (front ? 2 : 1);
    ia[m++] = base; ia[m++] = base + (front ? 2 : 3); ia[m++] = base + (front ? 3 : 2);
    I.n = m;
  }

  var PAD = new Uint16Array(PS * PS * PS), MASK = new Uint16Array(CS2);
  /* One chunk's faces into V and I, at the chunk's own place in the block. */
  function meshInto(world, cx, cy, cz, V, I) {
    if (!world.chunks[world.index(cx, cy, cz)]) return;
    var ox = cx * CS, oy = cy * CS, oz = cz * CS, x, y, z, p = 0;
    for (y = -1; y <= CS; y++) for (z = -1; z <= CS; z++) for (x = -1; x <= CS; x++) PAD[p++] = world.get(ox + x, oy + y, oz + z);
    var o = [ox, oy, oz];
    for (var d = 0; d < 3; d++) {
      var u = (d + 1) % 3, v = (d + 2) % 3, sd = STRIDE[d], su = STRIDE[u], sv = STRIDE[v];
      for (var s = -1; s <= 1; s += 2) {
        var n = d * 2 + (s > 0 ? 1 : 0);
        for (var i = 0; i < CS; i++) {
          var any = false, a, b, id;
          for (b = 0; b < CS; b++) for (a = 0; a < CS; a++) {
            p = (i + 1) * sd + (a + 1) * su + (b + 1) * sv;
            id = PAD[p];
            if (id && PAD[p + s * sd]) id = 0;
            MASK[b * CS + a] = id;
            if (id) any = true;
          }
          if (!any) continue;
          for (b = 0; b < CS; b++) for (a = 0; a < CS;) {
            id = MASK[b * CS + a];
            if (!id) { a++; continue; }
            var w = 1, h = 1, k;
            while (a + w < CS && MASK[b * CS + a + w] === id) w++;
            grow: for (; b + h < CS; h++) for (k = 0; k < w; k++) if (MASK[(b + h) * CS + a + k] !== id) break grow;
            for (var hh = 0; hh < h; hh++) for (k = 0; k < w; k++) MASK[(b + hh) * CS + a + k] = 0;
            var q = [[0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0]], at = i + (s > 0 ? 1 : 0);
            var cu = [a, a + w, a + w, a], cv = [b, b, b + h, b + h];
            for (k = 0; k < 4; k++) { q[k][d] = o[d] + at; q[k][u] = o[u] + cu[k]; q[k][v] = o[v] + cv[k]; }
            quad(V, I, q[0], q[1], q[2], q[3], id, n, s > 0);
            a += w;
          }
        }
      }
    }
  }
  function meshChunk(world, cx, cy, cz) {
    var V = new Buf(Uint16Array, 4096), I = new Buf(Uint32Array, 1024);
    meshInto(world, cx, cy, cz, V, I);
    return I.n ? { v: V.done(), i: I.done() } : null;
  }
  /* A whole (small) block as one mesh: a board, a leaf, a key. */
  function meshAll(world) {
    var V = new Buf(Uint16Array, 4096), I = new Buf(Uint32Array, 1024);
    for (var cy = 0; cy < world.ny; cy++) for (var cz = 0; cz < world.nz; cz++) for (var cx = 0; cx < world.nx; cx++)
      meshInto(world, cx, cy, cz, V, I);
    return { v: V.done(), i: I.done() };
  }

  /* Single cubes, in order, 36 indices each — so the first k of them can be
     drawn by drawing the first 36k indices. A root is shown a voxel at a time. */
  var FACES = [                      // per way it faces: four corners, counter-clockwise from outside
    [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]], [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]],
    [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]], [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]],
    [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]], [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]]
  ];
  function meshCubes(cubes) {        // [x, y, z, id, ...]
    var V = new Buf(Uint16Array, cubes.length * 36), I = new Buf(Uint32Array, cubes.length * 9);
    for (var c = 0; c < cubes.length; c += 4) for (var n = 0; n < 6; n++) {
      var f = FACES[n], q = [];
      for (var k = 0; k < 4; k++) q.push([cubes[c] + f[k][0], cubes[c + 1] + f[k][1], cubes[c + 2] + f[k][2]]);
      quad(V, I, q[0], q[1], q[2], q[3], cubes[c + 3], n, true);
    }
    return { v: V.done(), i: I.done() };
  }

  return { CS: CS, World: World, meshChunk: meshChunk, meshAll: meshAll, meshCubes: meshCubes };
});
