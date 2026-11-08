// SPDX-License-Identifier: AGPL-3.0-or-later
// C ABI for arc-resolver — the on-device bi-temporal LIFE resolver.
//
// One handle owns `graph.lode`: write with the object projection verbs, read with the query
// verbs. Every verb takes a NUL-terminated JSON C string and returns a heap JSON C string the
// caller frees with arc_resolver_string_free (read/write failures come back as {"error": "..."}).
// arc_resolver_open returns NULL and sets *err_out on failure. Not thread-safe — serialize
// calls on a handle externally (writes mutate; reads are shared).
#ifndef ARC_RESOLVER_H
#define ARC_RESOLVER_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ArcResolverHandle ArcResolverHandle;

// The platform embedder callback (the MOBILE path — the app's own ONNX runtime). Fill
// `count * dim` floats into `out`, row-major (text i -> out[i*dim .. (i+1)*dim]). Return 0 on
// success, non-zero on failure. `ctx` is the opaque handle passed to arc_resolver_open.
typedef int (*ArcEmbedCallback)(void *ctx, const char *const *texts, size_t count, size_t dim,
                                float *out);

// Open a resolver over config JSON:
//   {graph_path, vector_dim?=768, index_facts?=true, embed_model?, embed_tokenizer?}
// If embed_callback is non-NULL it is used (embed_dim = its output dimension); otherwise, on a
// desktop build with the ort feature, embed_model/embed_tokenizer load an in-Rust embedder.
// Returns a handle, or NULL with *err_out set to a JSON error string (free it too).
ArcResolverHandle *arc_resolver_open(const char *config_json, ArcEmbedCallback embed_callback,
                                     void *embed_ctx, size_t embed_dim, char **err_out);

void arc_resolver_free(ArcResolverHandle *h);
void arc_resolver_string_free(char *s);

// Write: the ratified-delta projection — minted GroupObjects reconciled into the graph,
// deterministically. Request:
//   {now_ms, objects: [{id, kind, name, shape?, members?: [{space, name?}],
//     affiliations?: [{peer, rel, name?, tether?, at}], at?}, ...]}
// `objects` is the COMPLETE snapshot: a previously projected relationship absent from it is
// invalidated at now_ms (history retained). Returns ProjectStats JSON.
char *arc_resolver_project_objects(ArcResolverHandle *h, const char *req_json);

// Write: promotion — re-point a text-lifted ghost entity onto a freshly minted GroupObject's
// node (facts, provenance, summary move; the ghost is removed).
// Request: {ghost_id, object_id, object_type, name}. Returns {"facts_moved": n}.
char *arc_resolver_rehome_entity(ArcResolverHandle *h, const char *req_json);

// Reads.
char *arc_resolver_entities(const ArcResolverHandle *h, const char *req_json);          // {type?, as_of?}
char *arc_resolver_neighbors(const ArcResolverHandle *h, const char *req_json);         // {id, direction?, relation?, as_of?}
char *arc_resolver_history(const ArcResolverHandle *h, const char *req_json);           // {id}
char *arc_resolver_get_entity(const ArcResolverHandle *h, const char *req_json);        // {id}
char *arc_resolver_semantic_entities(const ArcResolverHandle *h, const char *req_json); // {query, k?, type?, as_of?}
char *arc_resolver_episode_index(const ArcResolverHandle *h);                           // -> [{episode_id, source, occurred_at, entities}]
char *arc_resolver_stats(const ArcResolverHandle *h);

#ifdef __cplusplus
}
#endif

#endif  // ARC_RESOLVER_H
