# The webapp's requirements of core

What the first-party webapp (`app/web/webapp`) writes and reads, for core to build to.
Ralph, 28 Sep 2026: "Do not cripple the build by waiting for the ICD. Core has a team of A-E
agents ready to build to our spec. Core will be aligned to our requirements."

The webapp already writes and reads every shape below. Where core doesn't yet, the webapp
meets the Door's refusal and shows it; it never works around it. Each section says what
core has today, with the file and line.

Kept by WEBAPP_HUMAN, branch `webapp/ui`.

## 1. Replies, times, reactions (NC-9)

The reducer already folds every field here (`coordinator.rs` FORUM_POST ~985, FORUM_REACT
~1015). The ICD declares none of them. Declare them as the reducer reads them:

| op | args |
|---|---|
| `forum.post` | `text` (required), `ts` int ms, `reply_author` 64-hex, `reply_gen` int, `media` (MediaRef, as `MediaRef::from_args("media")` reads it) |
| `forum.react` | `target_author` 64-hex, `target_gen` int, `emoji` text, `active` int 1 = set, 0 = take back |
| `forum.vote` | unchanged: `target_author`, `target_gen`, `dir` |

`gen` stays the author's own, filled in by `authoring::build` for commutative ops. The webapp
never sends it.

## 2. Whose reaction (NC-111, the view)

`fold.rs messages_view` serves `reactions: [[emoji, count]]`. Serve `[[emoji, count, mine]]`,
where `mine` is true when the reader's own key is among the emoji's reactors. With it the
webapp marks a reaction as yours and takes it back (`active: 0`); without it, a reaction
can't be undone from any client. Don't serve the reactors' keys to other readers.

## 3. The member card (O-77)

Ralph: "All members of a community are required to publish a profile card on entry, which
includes Name, profile icon and public key." Settled with BW-D on 28 Sep, to what core
builds.

- **Written by the person, on their self record,** with an existing op:
  `group.setProfile {displayName, shape, card}`. `card` is the ContactCard JSON: `photo`
  (base64, padded, no `data:`) and `photo_mime`. The webapp makes the photo the card's
  picture before it writes it: square, 128 px, webp (or jpeg where the browser writes no
  webp), under 11,000 characters of base64. A published card holds 16,384 bytes and core
  has no image decoder. The webapp asks for this once on arrival when `/v2/me` has no
  `display_name`, and whenever the person changes it (profile menu → Your card).
- **Published by core into every group and forum the person holds,** at entry and on every
  change to the self record's name or card (BW-D).
- **Served on the group and forum view** (BW-D, o77/profiles) as
  `profiles: { <member 64-hex>: { name, icon: {mime, data} | null, gen } }`. The webapp
  also reads the pair form `[[member, {displayName, icon: data URL}]]`.
- **A departed member's card stays** (built by BW-D): every author's last card, whether
  or not they're still on the roster. This is the Community API's rule (PRO-32): their
  words keep their name.
- **The group view carries its own `card`** (group.setProfile's ContactCard), so on the
  self record a person sees their own picture.
- **`/v2/me`'s `display_name` reads the self record** (BW-D's Door). The webapp takes the
  name from there first, and from `/v2/graph`'s `me` otherwise. When the picture can't be
  published, `/v2/me` carries `card_note` in core's words, and the Your card dialog shows it.

Until it's served, the webapp names people from their connections, and otherwise shows
"New member", with a hue and a glyph. It never shows a key as a name.

## 4. Admins edit the face

Ralph: "For owners and admins, this includes an option to edit the site face." Built on
BW-B's `icd21/face-shared`, ICD 2.1.0:

- `group.editFace {face}` on the Site, and `host.editMedia {slot, media, mediaMime}` on the
  Host. Both are commutative, stamped with `gen` by the Door and core, and written by the
  owner or an admin in that object's own roles. An empty value clears. These are how every
  face is written from 2.1.0, the owner's included; the webapp's Face editor saves with
  them. `group.setFace` and `host.setMedia` stay only so old Sites still fold.

**Make admin** is in the webapp's member card; the owner sees Make admin and Remove admin:
1. `base.setRole {member, role: admin}` on the Site.
2. `POST /v2/add {object: <the Host>, member}`. The Door adds someone only if they're on
   the Site's roster and are the owner's contact. Otherwise it says why, and the card says
   so before the owner presses.
3. `base.setRole {member, role: admin}` on the Host. A role overlays a member of that
   object (`roles.rs`), so this waits on step 2. An admin who isn't on the Host can edit
   the face but not its pictures, and the card says exactly that.

Remove admin is `base.clearRole` on both. Their earlier edits then stop counting, and the
face falls back.

## 5. Resources: PDFs and full-size pictures

- `post.setProfile.form` gains `pdf`. The webapp today tells a PDF by a link ending in
  `.pdf`, which is the guessing post.setProfile's own ICD summary forbids.
- `post.setProfile.link` is allowed on `image` (the full-size source) and `pdf` (where the
  PDF lives) as well as `link`. Today the fold refuses a link on any other form.
- `post.setMedia` gains `bannerMime` and `iconMime`, as `host.setMedia` has `mediaMime`.
  The webapp sniffs the bytes today.
- Later, the file store the Community API describes: a PDF or picture stored by
  WallFlowers, encrypted, its key in the Delta.

## 6. An event's end

`event.setProfile` declares `endMs`. `event.rs:486` already reads it and `fold.rs
event_view` serves `end_ms`. The webapp's event sheet sends it.

## 7. A picture in a room (next)

With § 1's `media` declared, a message can carry a picture. `fold.rs messages_view` doesn't
serve a message's media yet (it serves author, gen, text, ts, reply_to, up, down,
reactions), so the view needs it too. The webapp doesn't attach pictures yet; this is the
next thing it will build.

## 8. Threads

Ralph, 29 Sep: rooms read like threads on a subreddit, not a group chat. Core already has
the projection, `ForumState::thread()` (`coordinator.rs` ~690): pre-order, with depth and a
count of descendants; siblings by causal `(gen, author)`; an orphan or a cycle surfaces as a
root; every message exactly once. `messages_view` serves `detailed()` (flat), so the webapp
computes the same projection itself.

Wanted: the view serves each message's `depth` and `descendants`, in `thread()` order, so
every client threads alike with one implementation.

## 9. Rich media: full-size pictures and PDFs (R2)

Ralph, 29 Sep: "Allow full size, we need rich media, including PDFs."

- **Built, and on production:** the relay's half. arc.env names `RELAY_R2_*`, and the relay
  presigns uploads and downloads (`Frame::MediaPut` / `MediaGet`,
  `arc/planes/relay/src/media.rs`). Clients put ciphertext straight to R2, so the relay
  never sees the bytes. The cap is `RELAY_MEDIA_MAX_BYTES`, default 25 MiB. (BUILD, 29 Sep.)
- **Deployed and unused** (SCM/DOCS, read-only on production, 29 Sep): the bucket is
  `pacific-media`, the relay logged "media offload enabled" (25 MiB per object, 9 GB
  budget), and every stats line since reads `media_presigned=0`. Nothing has uploaded.
- **The reference is an ICD addition, in 2.3.0 (O-79).** No op or arg carries a media
  reference today (object key, size, mime, content hash, decryption key); every picture is
  inline base64. Ralph chose to ship 2.1.0 as pinned (W-86), and approved O-75 (history for
  newcomers, with cards and posts) as 2.2.0, so rich media is 2.3.0: the reference type
  declared together with the client half that writes and reads it (MANAGE, 29 Sep). This
  section is its input.
- **Not built:** the client half. Nothing in core, core-wasm or the Door uses it, and
  `MediaRef`'s detached form carries no R2 key or content key. It's about 2 days of work,
  a revision after 2.1.0.
- **What the webapp needs from it:** a Door route that stores a blob for the session,
  e.g. `POST /v2/blob` (the bytes), answering a reference (`MediaRef`: where it is, its
  type, its size, and its content key sealed as the Delta carries it). Each of these then
  carries that reference instead of inline base64: the card's photo (§ 3), a Host's
  picture (`host.editMedia`), a post's banner and file (`post.setMedia`, a `pdf` form's
  file, § 5), and a message's `media` (§ 7). The view serves the reference, and the Door
  serves the bytes, decrypted, to whoever the object's roster admits (e.g.
  `GET /v2/blob/<ref>`).
- **Caps today** (DOCS): group.setFace 16,384 bytes; host.setMedia 156,000 base64 per slot;
  post.setMedia icon 500 KB and banner 700 KB of base64; a Delta 1 MiB on production
  (`RELAY_MAX_BLOB_BYTES`). With offload, a Delta carries a reference of tens of bytes,
  and an object may be up to 25 MiB.
- **Until then:** the webapp writes the card's photo small enough that the published card
  fits whole: 128 px, under 11,000 base64 characters (Ralph, 29 Sep: "if we can fit
  smaller profile pictures, that'd be great"). With 2.3.0 it writes full size, as a
  reference. A PDF resource is a link to a hosted file (form `pdf` from 2.1.0 row 6).
