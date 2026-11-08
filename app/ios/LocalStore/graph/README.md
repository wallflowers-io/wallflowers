# LocalStore / graph — the on-device bi-temporal graph

The read index behind the **LIFE tab**. `arc-resolver/` is one Rust crate, linked into the app
through `arc-resolver/swift/ArcResolverKit`, that owns `graph.lode` on the device and drives the
`lodedb-graph` engine: SQLite topology as the source of truth, plus a rebuildable `lodedb-core`
semantic index over entity labels and fact text.

The graph is derived, never authoritative. The per-object delta logs are the truth; the graph
is the fold that makes them cheap to read.

## Writes

- **Object projection** (`project_objects`) — the app's minted GroupObjects (groups, projects,
  things, places, events), their rosters, their Group↔Group affiliations and an Event's venue,
  folded into entities and facts deterministically. Re-running an unchanged snapshot writes
  nothing. A relationship that disappears from the snapshot is invalidated, never deleted, so
  history stays queryable.
- **Promotion** (`rehome_entity`) — re-points a text-lifted node onto a newly minted
  GroupObject's id, carrying its facts, note provenance and summary with it. Graphs written by
  older builds still hold such nodes.

No language model runs here and nothing in the crate calls the network. Extracting entities
and facts from notes and chats with a model has been removed, together with the Python
prototype and prompt bench that used to sit in this directory.

## Reads

`entities`, `neighbors`, `history`, `get_entity`, `episode_index`, `semantic_entities` and
`stats`, over a full bi-temporal model: event time (`valid_at` / `invalid_at`), transaction
time (`created_at` / `expired_at`), invalidation rather than deletion, and as-of queries.
`semantic_entities` embeds the query with the platform's own embedder, injected through a C
callback, so search never leaves the device.

## Building

    cd arc-resolver
    cargo test --no-default-features
    swift/ArcResolverKit/scripts/package_xcframework.sh

Mobile builds use `--no-default-features` because `ort` has no iOS prebuilt; the app injects its
embedder through the callback instead. The default `ort-embedder` feature is for desktop
development.

## Also here

- `EVENTS-EXTRACTION.md` — the shape an Event node takes, and why recurrence stays off the
  bi-temporal axis.
