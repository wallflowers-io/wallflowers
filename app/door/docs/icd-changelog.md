# ICD changelog

What each release of the model (core/coordination/delta-graph.icd.json) added, changed, retired or renamed, for
the people building on the Community API. Newest first. The release is its `icd/*` tag and pin; the lists between
the markers are computed from the released files by `icd-changelog.mjs`, and `--check` refuses them if they differ.
Method: deploy.md § 10.

Every op is written the same way: `POST /v2/apply` with `{object, op, args}` on an object that exists, or
`POST /v2/mint` to make one. `GET /v2/icd` serves the model production runs; model.html lists every op and its args.

## 2.3.1

<!-- generated 2.3.1: icd-changelog.mjs --write; do not edit between the markers -->
- Release: `icd/2.3.1`, 2026-10-01; the pin `a71fb32573731ca2de99e637834c279fd44d661a947b9542c2d3b088dd10cb13`
- Against 2.2.0 (`b35c3c076e8dfd52…`):
  - added: base.answerQuestion (facet questions, op 4027318274, member, commutative)
  - added: base.defineQuestion (facet questions, op 4027318272, owner|role:admin, commutative)
  - added: base.publishAbout (facet about, op 4027252736, member, commutative)
  - added: base.retireQuestion (facet questions, op 4027318273, owner|role:admin, commutative)
  - added: contact.setLink (contact, op 12, member, commutative)
  - added: event.addPhoto (event, op 10, owner|role:admin, commutative)
  - added: event.editProfile (event, op 13, owner|role:admin, commutative)
  - added: event.removePhoto (event, op 11, owner|role:admin, commutative)
  - added: event.setBanner (event, op 9, owner|role:admin, commutative)
  - added: event.setClip (event, op 12, owner|role:admin, commutative)
  - added: event.setLineup (event, op 8, owner|role:admin, commutative)
  - added: forum.editDescription (forum, op 7, owner|role:admin, commutative)
  - added: group.publishListing (group, op 21, member, commutative)
  - added: group.removeListing (group, op 22, owner|role:admin, commutative)
  - added: group.rsvp (group, op 18, member, commutative)
  - added: group.rsvpDecide (group, op 20, owner|role:admin, commutative)
  - added: group.setRegistration (group, op 19, owner|role:admin, commutative)
  - added: post.addAsset (post, op 6, owner, sequenced)
  - added: post.removeAsset (post, op 7, owner, sequenced)
  - added: post.setDocument (post, op 5, owner, sequenced)
  - added: thing.addPhoto (thing, op 4, owner, sequenced)
  - added: thing.removePhoto (thing, op 5, owner, sequenced)
  - added: thing.setDisposition (thing, op 6, owner, sequenced)
  - added: transaction.accept (transaction, op 1, role:buyer, commutative)
  - added: transaction.cancel (transaction, op 5, role:buyer|role:seller, commutative)
  - added: transaction.confirmReceipt (transaction, op 2, role:buyer, commutative)
  - added: transaction.confirmSale (transaction, op 4, role:seller, commutative)
  - added: transaction.dispute (transaction, op 3, role:buyer|role:seller, commutative)
  - added: transaction.fraudSignal (transaction, op 7, role:settler, commutative)
  - added: transaction.rate (transaction, op 8, role:buyer|role:seller, commutative)
  - added: transaction.resolve (transaction, op 6, role:settler, commutative)
  - added: transaction.setSite (transaction, op 9, owner, sequenced)
  - added: transaction.setTerms (transaction, op 0, role:seller, commutative)
  - changed: base.claimSpent: ego member → owner|role:admitter
  - changed: event.setProfile: +allDay (integer); +descriptorFormat (string); +online (string max 2048); +status (string); ticketUrl: string → string max 2048; +tz (string max 64); +videoUrl (string max 2048)
  - changed: forum.post: +mediaKey (string); +mediaSecret (string)
  - changed: post.setMedia: banner: string → string max 716800; +bannerAlt (string max 191); icon: string → string max 512000
  - changed: post.setProfile: +bodyFormat (string); +excerpt (string max 300)
  - changed: thing.setProfile: +condition (string); +description (string max 5000)
  - model sections changed besides ops: facets, kinds, relations
<!-- end generated 2.3.1 -->

Departures: `base.claimSpent`'s ego, released as `member`, is `owner|role:admitter`: what the fold already enforced
(NC-135). Named in the pin's W- row.

For sites (W-98; each lands with its lane, and its docs page when the release ships):
- Rooms: a room's description, `forum.editDescription`; deleting your post, `forum.retract` (2.1.0). rooms.html.
- Events: lineup, banner, photos, clip, `event.editProfile`; RSVP and registration, `group.rsvp`,
  `group.setRegistration`, `group.rsvpDecide`. public.html.
- Resources: documents and their assets, `post.setDocument`, `post.addAsset`, `post.removeAsset`. public.html.
- Members: an about text and the owner's questions, `base.publishAbout`, `base.defineQuestion`,
  `base.answerQuestion`, `base.retireQuestion`; links between members, `contact.setLink`. members.html.
- Trade: listings, `group.publishListing`, `group.removeListing`, `thing.*`; a deal's life,
  `transaction.setTerms` through `transaction.rate`. Its page comes with the Trade lane.

## 2.2.0

<!-- generated 2.2.0: icd-changelog.mjs --write; do not edit between the markers -->
- Release: `icd/2.2.0`, 2026-09-29; the pin `b35c3c076e8dfd52e832aedd3e21f247a9ba8e43583a37a6efd2ee6dafa3b257`
- Against 2.1.0 (`4647a17c6465ffc7…`):
  - model sections changed besides ops: mls
<!-- end generated 2.2.0 -->

Message history (Ralph, 29 Sep: "Use the MLS welcome history package, delivered by the arc"): a member admitted
later reads the room's posts and cards from before. No op changed; the model's `mls` section states the rule.
No departures. For sites: history arrives with `GET /v2/graph` after a join. views.html.

## 2.1.0

<!-- generated 2.1.0: icd-changelog.mjs --write; do not edit between the markers -->
- Release: `icd/2.1.0`, 2026-09-29; the pin `4647a17c6465ffc74c92ce9ad91e4a0df52824128d29a476f9960e0625409197`
- Against 2.0.0 (`d3d5ef2878a0259e…`):
  - added: base.publishProfile (facet profiles, op 4027187200, member, commutative)
  - added: forum.retract (forum, op 6, member, commutative)
  - added: group.editFace (group, op 17, owner|role:admin, commutative)
  - added: host.editMedia (host, op 3, owner|role:admin, commutative)
  - changed: base.claimSpent: +choice (string); +share (string)
  - changed: base.setPart: +choice (string)
  - changed: contact.deliverTicket: +listing (string required); +tickets (string required)
  - changed: contact.prekeyConsume: +kp_id (string required)
  - changed: contact.prekeyRevoke: +kp_id (string required)
  - changed: contact.prekeySupply: +intro_tag (string required); +kp (string required); +kp_id (string required); +not_after (integer)
  - changed: contact.publishListing: +area (string); +deadline (integer); +descriptor (string); +photo (string); +photoMime (string); +posture (string required); +price (string); +reach (string required); +rev (integer required); +thingId (string required); +title (string required); +withdrawn (integer)
  - changed: contact.publishProfile: +displayName (string required); +shape (string required)
  - changed: contact.requestTicket: +listing (string required); +pi (string required)
  - changed: event.recordSale: +buyer (string required)
  - changed: event.setMedia: +photos (string)
  - changed: event.setProfile: +endMs (integer)
  - changed: event.setTickets: +delegate (string)
  - changed: forum.post: +gen (integer required); +media (string); +mediaBytes (integer); +mediaDigest (string); +mediaH (integer); +mediaKind (string); +mediaMime (string); +mediaMs (integer); +mediaSession (string); +mediaVia (string); +mediaW (integer); +reply_author (string); +reply_gen (integer); +ts (integer)
  - changed: forum.react: +active (integer); +target_author (string required); +target_gen (integer required)
  - changed: forum.receipt: +refs (string required); +status (integer required)
  - changed: group.setPresence: +identityKey (string)
  - changed: post.setMedia: +bannerMime (string); +iconMime (string)
  - changed: thing.setPosture: +deadline (integer)
  - changed: thing.setProfile: +category (string)
  - model sections changed besides ops: facets, kinds, principals
<!-- end generated 2.1.0 -->

Departures (W-86): 27 args newly declared required that their reducers already required (str.md run 71); the
retired op ids recorded and their reuse refused.

For sites:
- Delete your post: `forum.retract`. rooms.html.
- The page's look and pictures: `group.editFace`, `host.editMedia`. page.html.
- Member cards: `base.publishProfile`. members.html.
- Claims carry the visitor's choice: `base.claimSpent` `choice`, `base.setPart` `choice`.

## 2.0.0

<!-- generated 2.0.0: icd-changelog.mjs --write; do not edit between the markers -->
- Release: `icd/2.0.0`, 2026-09-28; the pin `d3d5ef2878a0259e62725979263ace086129d8ec6869871c3cfc90f7e23443cb`
- The first release: every op is new.
<!-- end generated 2.0.0 -->

The model production ran at the mint, 28 Sep. Everything after is stated against it.
