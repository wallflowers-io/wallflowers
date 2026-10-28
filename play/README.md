# play

Three pages, drawn by one WebGL2 engine (`assets/play/`):

- `play.html`: how WallFlowers works, as a scene you walk.
- `map.html`: choose places on an open map, then walk a town generated between them, in any of six looks.
- `sail.html`: the sea past the land's edge, and a boat on it. A prototype.

Not wired into the webapp yet. Nothing persists: no Delta carries a place, a land, a visit or a look.
The figures are game art (`assets/tiles/tama/cast.json`), not members.

## Run

    scripts/dev-serve.py            # http://127.0.0.1:8931/play
    scripts/cut-tiles.sh            # the map's tiles; needs `pmtiles` (go-pmtiles) on PATH
    make check-play

`/map` draws no map until the tiles are cut. They come from a dated Protomaps build, are not
committed, and are checked against `assets/map/VENDOR.json`, which also names what the map vendors,
at which version, under which licence. The map data is OpenStreetMap's (ODbL); its credit stays on
the map.
