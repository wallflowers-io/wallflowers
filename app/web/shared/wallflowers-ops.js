/* GENERATED FROM core/coordination/delta-graph.icd.json — DO NOT EDIT.
 *
 *   regenerate:  core/coordination/gen-web-ops.py
 *   check:       core/coordination/gen-web-ops.py --check
 *
 * Every op the model defines, as a function that builds its args and hands them
 * to the core. Nothing here is transcribed: the names, the op ids, the
 * authority, the commutativity and the argument types all come out of the ICD,
 * so an op added there appears here on the next build and an op removed breaks
 * it. That is the whole reason this file is generated — the audit found four
 * hand-maintained layers between the model and a screen and no two agreed.
 *
 * 141 ops · 138 reachable · 3 not, each with its reason.
 *
 * AN UNREACHABLE OP IS STILL LISTED. It has no author function, and asking for
 * one throws with the reason. A client that cannot see what it cannot do will
 * show a member a button that fails.
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.WallflowersOps = factory();
}(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  /* The catalogue, as the ICD has it. */
  var OPS = {
    "base.answerQuestion": {name:"base.answerQuestion", prefix:"base", opId:4027318274, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"choices",type:"string",required:false}, {name:"target_author",type:"string",required:true}, {name:"target_gen",type:"integer",required:true}, {name:"text",type:"string",required:false}]},
    "base.claimSpent": {name:"base.claimSpent", prefix:"base", opId:4026597379, authority:"owner|role:admitter", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"choice",type:"string",required:false}, {name:"claim",type:"string",required:true}, {name:"member",type:"string",required:true}, {name:"share",type:"string",required:false}]},
    "base.clearBacklink": {name:"base.clearBacklink", prefix:"base", opId:4027121665, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"object",type:"string",required:true}, {name:"rel",type:"string",required:true}]},
    "base.clearLocation": {name:"base.clearLocation", prefix:"base", opId:4026531841, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[]},
    "base.clearParent": {name:"base.clearParent", prefix:"base", opId:4027056129, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"parent",type:"string",required:true}]},
    "base.clearPart": {name:"base.clearPart", prefix:"base", opId:4026925057, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"part",type:"string",required:true}]},
    "base.clearRole": {name:"base.clearRole", prefix:"base", opId:4026990593, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"member",type:"string",required:true}]},
    "base.defineQuestion": {name:"base.defineQuestion", prefix:"base", opId:4027318272, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"free",type:"integer",required:false}, {name:"hint",type:"string",required:false}, {name:"max",type:"integer",required:false}, {name:"multi",type:"integer",required:false}, {name:"options",type:"string",required:false}, {name:"text",type:"string",required:true}, {name:"textMax",type:"integer",required:false}]},
    "base.memberJoined": {name:"base.memberJoined", prefix:"base", opId:4026597376, authority:"anyMember", fold:"commutative", reachable:false, why:"recorded by the MLS doors, never authored as a delta", fields:[{name:"at",type:"integer",required:true}, {name:"member",type:"string",required:true}]},
    "base.memberLeft": {name:"base.memberLeft", prefix:"base", opId:4026597377, authority:"anyMember", fold:"commutative", reachable:false, why:"recorded by the MLS doors, never authored as a delta", fields:[{name:"at",type:"integer",required:true}, {name:"member",type:"string",required:true}, {name:"reason",type:"string",required:false}]},
    "base.ownerHandover": {name:"base.ownerHandover", prefix:"base", opId:4026597378, authority:"owner", fold:"sequenced", reachable:false, why:"recorded by the MLS doors, never authored as a delta", fields:[{name:"at",type:"integer",required:true}, {name:"member",type:"string",required:true}]},
    "base.publish": {name:"base.publish", prefix:"base", opId:4026728448, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"publisher",type:"string",required:true}, {name:"slug",type:"string",required:true}]},
    "base.publishAbout": {name:"base.publishAbout", prefix:"base", opId:4027252736, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"bio",type:"string",required:true}, {name:"links",type:"string",required:false}]},
    "base.publishProfile": {name:"base.publishProfile", prefix:"base", opId:4027187200, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"card",type:"string",required:true}]},
    "base.retireQuestion": {name:"base.retireQuestion", prefix:"base", opId:4027318273, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"target_author",type:"string",required:true}, {name:"target_gen",type:"integer",required:true}]},
    "base.setBacklink": {name:"base.setBacklink", prefix:"base", opId:4027121664, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"object",type:"string",required:true}, {name:"rel",type:"string",required:true}]},
    "base.setLocation": {name:"base.setLocation", prefix:"base", opId:4026531840, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"source",type:"string",required:true}]},
    "base.setParent": {name:"base.setParent", prefix:"base", opId:4027056128, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"parent",type:"string",required:true}, {name:"role",type:"string",required:true}]},
    "base.setPart": {name:"base.setPart", prefix:"base", opId:4026925056, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"choice",type:"string",required:false}, {name:"part",type:"string",required:true}, {name:"role",type:"string",required:true}]},
    "base.setRole": {name:"base.setRole", prefix:"base", opId:4026990592, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"member",type:"string",required:true}, {name:"role",type:"string",required:true}]},
    "base.setVisibility": {name:"base.setVisibility", prefix:"base", opId:4026662912, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"visibility",type:"string",required:true}]},
    "base.unpublish": {name:"base.unpublish", prefix:"base", opId:4026728449, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[]},
    "contact.admitTicket": {name:"contact.admitTicket", prefix:"contact", opId:11, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"admittedMs",type:"integer",required:true}, {name:"ticket",type:"string",required:true}]},
    "contact.deliverTicket": {name:"contact.deliverTicket", prefix:"contact", opId:6, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"listing",type:"string",required:true}, {name:"ticket",type:"string",required:true}, {name:"tickets",type:"string",required:true}]},
    "contact.discoverRequest": {name:"contact.discoverRequest", prefix:"contact", opId:7, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"budget",type:"integer",required:true}, {name:"hops",type:"integer",required:true}, {name:"kind",type:"string",required:true}, {name:"origin",type:"string",required:true}, {name:"rid",type:"string",required:true}, {name:"tags",type:"string",required:false}]},
    "contact.discoverResponse": {name:"contact.discoverResponse", prefix:"contact", opId:8, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"holder",type:"string",required:true}, {name:"hops",type:"integer",required:true}, {name:"items",type:"string",required:true}, {name:"origin",type:"string",required:true}, {name:"rid",type:"string",required:true}]},
    "contact.invite": {name:"contact.invite", prefix:"contact", opId:9, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"event",type:"string",required:true}, {name:"note",type:"string",required:false}, {name:"startMs",type:"integer",required:true}, {name:"title",type:"string",required:true}, {name:"venue",type:"string",required:false}]},
    "contact.inviteReply": {name:"contact.inviteReply", prefix:"contact", opId:10, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"event",type:"string",required:true}, {name:"going",type:"integer",required:true}]},
    "contact.prekeyConsume": {name:"contact.prekeyConsume", prefix:"contact", opId:1, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"kp_id",type:"string",required:true}, {name:"prekeyId",type:"string",required:true}]},
    "contact.prekeyRevoke": {name:"contact.prekeyRevoke", prefix:"contact", opId:2, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"kp_id",type:"string",required:true}, {name:"prekeyId",type:"string",required:true}]},
    "contact.prekeySupply": {name:"contact.prekeySupply", prefix:"contact", opId:0, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"bundle",type:"string",required:true}, {name:"intro_tag",type:"string",required:true}, {name:"kp",type:"string",required:true}, {name:"kp_id",type:"string",required:true}, {name:"not_after",type:"integer",required:false}]},
    "contact.publishListing": {name:"contact.publishListing", prefix:"contact", opId:4, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"area",type:"string",required:false}, {name:"deadline",type:"integer",required:false}, {name:"descriptor",type:"string",required:false}, {name:"hops",type:"integer",required:true}, {name:"listing",type:"string",required:true}, {name:"origin",type:"string",required:true}, {name:"photo",type:"string",required:false}, {name:"photoMime",type:"string",required:false}, {name:"posture",type:"string",required:true}, {name:"price",type:"string",required:false}, {name:"reach",type:"string",required:true}, {name:"rev",type:"integer",required:true}, {name:"thingId",type:"string",required:true}, {name:"title",type:"string",required:true}, {name:"withdrawn",type:"integer",required:false}]},
    "contact.publishProfile": {name:"contact.publishProfile", prefix:"contact", opId:3, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"card",type:"string",required:true}, {name:"displayName",type:"string",required:true}, {name:"shape",type:"string",required:true}]},
    "contact.requestTicket": {name:"contact.requestTicket", prefix:"contact", opId:5, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"eventId",type:"string",required:true}, {name:"listing",type:"string",required:true}, {name:"pi",type:"string",required:true}, {name:"qty",type:"integer",required:true}]},
    "contact.setLink": {name:"contact.setLink", prefix:"contact", opId:12, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"active",type:"integer",required:true}]},
    "event.addPhoto": {name:"event.addPhoto", prefix:"event", opId:10, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"id",type:"string",required:true}, {name:"photo",type:"string",required:false}, {name:"photoBytes",type:"integer",required:false}, {name:"photoDigest",type:"string",required:false}, {name:"photoH",type:"integer",required:false}, {name:"photoKey",type:"string",required:false}, {name:"photoKind",type:"string",required:false}, {name:"photoMime",type:"string",required:false}, {name:"photoMs",type:"integer",required:false}, {name:"photoSecret",type:"string",required:false}, {name:"photoSession",type:"string",required:false}, {name:"photoVia",type:"string",required:false}, {name:"photoW",type:"integer",required:false}]},
    "event.editProfile": {name:"event.editProfile", prefix:"event", opId:13, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"allDay",type:"integer",required:false}, {name:"descriptor",type:"string",required:false}, {name:"descriptorFormat",type:"string",required:false}, {name:"endMs",type:"integer",required:false}, {name:"lineup",type:"string",required:false}, {name:"online",type:"string",required:false}, {name:"recurrence",type:"string",required:false}, {name:"startMs",type:"integer",required:true}, {name:"status",type:"string",required:false}, {name:"ticketUrl",type:"string",required:false}, {name:"title",type:"string",required:true}, {name:"tz",type:"string",required:false}, {name:"venue",type:"string",required:false}, {name:"videoUrl",type:"string",required:false}]},
    "event.recordSale": {name:"event.recordSale", prefix:"event", opId:2, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"buyer",type:"string",required:true}, {name:"currency",type:"string",required:true}, {name:"feeCents",type:"integer",required:true}, {name:"pi",type:"string",required:true}, {name:"qty",type:"integer",required:true}, {name:"tickets",type:"string",required:true}, {name:"unitCents",type:"integer",required:true}]},
    "event.redeem": {name:"event.redeem", prefix:"event", opId:3, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"ticket",type:"string",required:true}]},
    "event.removePhoto": {name:"event.removePhoto", prefix:"event", opId:11, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"id",type:"string",required:true}]},
    "event.setBanner": {name:"event.setBanner", prefix:"event", opId:9, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"banner",type:"string",required:false}, {name:"bannerBytes",type:"integer",required:false}, {name:"bannerDigest",type:"string",required:false}, {name:"bannerH",type:"integer",required:false}, {name:"bannerKey",type:"string",required:false}, {name:"bannerKind",type:"string",required:false}, {name:"bannerMime",type:"string",required:false}, {name:"bannerMs",type:"integer",required:false}, {name:"bannerSecret",type:"string",required:false}, {name:"bannerSession",type:"string",required:false}, {name:"bannerVia",type:"string",required:false}, {name:"bannerW",type:"integer",required:false}]},
    "event.setClip": {name:"event.setClip", prefix:"event", opId:12, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"clip",type:"string",required:false}, {name:"clipBytes",type:"integer",required:false}, {name:"clipDigest",type:"string",required:false}, {name:"clipH",type:"integer",required:false}, {name:"clipKey",type:"string",required:false}, {name:"clipKind",type:"string",required:false}, {name:"clipMime",type:"string",required:false}, {name:"clipMs",type:"integer",required:false}, {name:"clipSecret",type:"string",required:false}, {name:"clipSession",type:"string",required:false}, {name:"clipVia",type:"string",required:false}, {name:"clipW",type:"integer",required:false}]},
    "event.setLineup": {name:"event.setLineup", prefix:"event", opId:8, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"acts",type:"string",required:true}]},
    "event.setMedia": {name:"event.setMedia", prefix:"event", opId:4, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"banner",type:"string",required:false}, {name:"bannerMime",type:"string",required:false}, {name:"clip",type:"string",required:false}, {name:"clipMime",type:"string",required:false}, {name:"photos",type:"string",required:false}]},
    "event.setProfile": {name:"event.setProfile", prefix:"event", opId:0, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"allDay",type:"integer",required:false}, {name:"descriptor",type:"string",required:false}, {name:"descriptorFormat",type:"string",required:false}, {name:"endMs",type:"integer",required:false}, {name:"lineup",type:"string",required:false}, {name:"online",type:"string",required:false}, {name:"recurrence",type:"string",required:false}, {name:"startMs",type:"integer",required:true}, {name:"status",type:"string",required:false}, {name:"ticketUrl",type:"string",required:false}, {name:"title",type:"string",required:true}, {name:"tz",type:"string",required:false}, {name:"venue",type:"string",required:false}, {name:"videoUrl",type:"string",required:false}]},
    "event.setTickets": {name:"event.setTickets", prefix:"event", opId:1, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"acct",type:"string",required:false}, {name:"capacity",type:"integer",required:true}, {name:"currency",type:"string",required:true}, {name:"delegate",type:"string",required:false}, {name:"open",type:"integer",required:true}, {name:"priceCents",type:"integer",required:true}, {name:"rev",type:"integer",required:false}, {name:"terms",type:"string",required:false}]},
    "event.setVenue": {name:"event.setVenue", prefix:"event", opId:7, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"name",type:"string",required:true}, {name:"place",type:"string",required:true}]},
    "forum.editDescription": {name:"forum.editDescription", prefix:"forum", opId:7, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"description",type:"string",required:true}]},
    "forum.post": {name:"forum.post", prefix:"forum", opId:0, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"media",type:"string",required:false}, {name:"mediaBytes",type:"integer",required:false}, {name:"mediaDigest",type:"string",required:false}, {name:"mediaH",type:"integer",required:false}, {name:"mediaKey",type:"string",required:false}, {name:"mediaKind",type:"string",required:false}, {name:"mediaMime",type:"string",required:false}, {name:"mediaMs",type:"integer",required:false}, {name:"mediaSecret",type:"string",required:false}, {name:"mediaSession",type:"string",required:false}, {name:"mediaVia",type:"string",required:false}, {name:"mediaW",type:"integer",required:false}, {name:"reply_author",type:"string",required:false}, {name:"reply_gen",type:"integer",required:false}, {name:"text",type:"string",required:true}, {name:"ts",type:"integer",required:false}]},
    "forum.react": {name:"forum.react", prefix:"forum", opId:1, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"active",type:"integer",required:false}, {name:"emoji",type:"string",required:true}, {name:"target",type:"string",required:true}, {name:"target_author",type:"string",required:true}, {name:"target_gen",type:"integer",required:true}]},
    "forum.receipt": {name:"forum.receipt", prefix:"forum", opId:2, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"refs",type:"string",required:true}, {name:"status",type:"integer",required:true}, {name:"upto",type:"integer",required:true}]},
    "forum.retract": {name:"forum.retract", prefix:"forum", opId:6, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"target_author",type:"string",required:true}, {name:"target_gen",type:"integer",required:true}]},
    "forum.vote": {name:"forum.vote", prefix:"forum", opId:3, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"dir",type:"integer",required:true}, {name:"target_author",type:"string",required:true}, {name:"target_gen",type:"integer",required:true}]},
    "group.clearAffiliation": {name:"group.clearAffiliation", prefix:"group", opId:6, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"peer",type:"string",required:true}]},
    "group.clearClaimIssuer": {name:"group.clearClaimIssuer", prefix:"group", opId:16, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"kid",type:"string",required:true}]},
    "group.clearOffice": {name:"group.clearOffice", prefix:"group", opId:11, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"office",type:"string",required:true}]},
    "group.editFace": {name:"group.editFace", prefix:"group", opId:17, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"face",type:"string",required:true}]},
    "group.joinedObject": {name:"group.joinedObject", prefix:"group", opId:12, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"arc",type:"string",required:true}, {name:"at",type:"integer",required:true}, {name:"kind",type:"string",required:true}, {name:"object",type:"string",required:true}, {name:"tag",type:"string",required:true}]},
    "group.leftObject": {name:"group.leftObject", prefix:"group", opId:13, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"object",type:"string",required:true}, {name:"reason",type:"string",required:false}]},
    "group.publishListing": {name:"group.publishListing", prefix:"group", opId:21, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"area",type:"string",required:false}, {name:"deadline",type:"integer",required:false}, {name:"descriptor",type:"string",required:false}, {name:"photo",type:"string",required:false}, {name:"photoMime",type:"string",required:false}, {name:"posture",type:"string",required:true}, {name:"price",type:"string",required:false}, {name:"reach",type:"string",required:true}, {name:"rev",type:"integer",required:true}, {name:"site",type:"integer",required:false}, {name:"thingId",type:"string",required:true}, {name:"title",type:"string",required:true}, {name:"withdrawn",type:"integer",required:false}]},
    "group.removeListing": {name:"group.removeListing", prefix:"group", opId:22, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"author",type:"string",required:true}, {name:"thingId",type:"string",required:true}]},
    "group.revokeCredential": {name:"group.revokeCredential", prefix:"group", opId:3, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"id",type:"string",required:true}]},
    "group.rsvp": {name:"group.rsvp", prefix:"group", opId:18, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"event",type:"string",required:true}, {name:"guestNames",type:"string",required:false}, {name:"guests",type:"integer",required:false}, {name:"occurrence",type:"integer",required:false}, {name:"status",type:"string",required:true}]},
    "group.rsvpDecide": {name:"group.rsvpDecide", prefix:"group", opId:20, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"decision",type:"string",required:true}, {name:"event",type:"string",required:true}, {name:"member",type:"string",required:true}]},
    "group.setAffiliation": {name:"group.setAffiliation", prefix:"group", opId:5, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"name",type:"string",required:true}, {name:"peer",type:"string",required:true}, {name:"rel",type:"string",required:true}, {name:"tether",type:"string",required:false}]},
    "group.setClaimIssuer": {name:"group.setClaimIssuer", prefix:"group", opId:15, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"key",type:"string",required:true}, {name:"kid",type:"string",required:true}]},
    "group.setCover": {name:"group.setCover", prefix:"group", opId:9, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"data",type:"string",required:false}, {name:"mime",type:"string",required:false}]},
    "group.setFace": {name:"group.setFace", prefix:"group", opId:14, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"face",type:"string",required:true}]},
    "group.setOffice": {name:"group.setOffice", prefix:"group", opId:10, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"holder",type:"string",required:true}, {name:"office",type:"string",required:true}, {name:"status",type:"string",required:true}]},
    "group.setPresence": {name:"group.setPresence", prefix:"group", opId:1, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"identityKey",type:"string",required:false}, {name:"inviteHint",type:"string",required:false}, {name:"kind",type:"string",required:true}, {name:"spaceId",type:"string",required:false}]},
    "group.setProfile": {name:"group.setProfile", prefix:"group", opId:0, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"card",type:"string",required:false}, {name:"displayName",type:"string",required:true}, {name:"shape",type:"string",required:true}]},
    "group.setRegistration": {name:"group.setRegistration", prefix:"group", opId:19, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"approval",type:"integer",required:false}, {name:"capacity",type:"integer",required:false}, {name:"closesMs",type:"integer",required:false}, {name:"event",type:"string",required:true}, {name:"guestsMax",type:"integer",required:false}, {name:"location",type:"string",required:false}, {name:"maybe",type:"integer",required:false}, {name:"waitlist",type:"integer",required:false}]},
    "group.storeCredential": {name:"group.storeCredential", prefix:"group", opId:2, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"id",type:"string",required:true}, {name:"kind",type:"string",required:true}, {name:"label",type:"string",required:true}]},
    "host.define": {name:"host.define", prefix:"host", opId:0, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"name",type:"string",required:true}]},
    "host.editMedia": {name:"host.editMedia", prefix:"host", opId:3, authority:"owner|role:admin", fold:"commutative", reachable:true, why:null, fields:[{name:"media",type:"string",required:true}, {name:"mediaMime",type:"string",required:false}, {name:"slot",type:"string",required:true}]},
    "host.hydrate": {name:"host.hydrate", prefix:"host", opId:1, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"fetchedAt",type:"integer",required:true}, {name:"key",type:"string",required:true}, {name:"payload",type:"string",required:true}, {name:"rev",type:"integer",required:true}, {name:"withdrawn",type:"string",required:false}]},
    "host.setMedia": {name:"host.setMedia", prefix:"host", opId:2, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"media",type:"string",required:true}, {name:"mediaMime",type:"string",required:false}, {name:"slot",type:"string",required:true}]},
    "note.comment": {name:"note.comment", prefix:"note", opId:2, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"comment",type:"string",required:true}, {name:"note",type:"string",required:true}, {name:"text",type:"string",required:true}]},
    "note.promote": {name:"note.promote", prefix:"note", opId:4, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"note",type:"string",required:true}, {name:"object",type:"string",required:true}]},
    "note.react": {name:"note.react", prefix:"note", opId:3, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"emoji",type:"string",required:true}, {name:"note",type:"string",required:true}]},
    "note.retract": {name:"note.retract", prefix:"note", opId:1, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"note",type:"string",required:true}]},
    "note.write": {name:"note.write", prefix:"note", opId:0, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"created",type:"integer",required:false}, {name:"note",type:"string",required:true}, {name:"source",type:"string",required:false}, {name:"text",type:"string",required:true}, {name:"title",type:"string",required:false}]},
    "place.clearLand": {name:"place.clearLand", prefix:"place", opId:4, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[]},
    "place.post": {name:"place.post", prefix:"place", opId:5, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"note",type:"string",required:true}, {name:"text",type:"string",required:true}, {name:"title",type:"string",required:false}]},
    "place.setAccess": {name:"place.setAccess", prefix:"place", opId:1, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"access",type:"string",required:true}]},
    "place.setDoorbell": {name:"place.setDoorbell", prefix:"place", opId:2, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"doorbell",type:"string",required:true}]},
    "place.setLand": {name:"place.setLand", prefix:"place", opId:3, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"claim",type:"string",required:true}]},
    "place.setProfile": {name:"place.setProfile", prefix:"place", opId:0, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"descriptor",type:"string",required:false}, {name:"name",type:"string",required:true}]},
    "post.addAsset": {name:"post.addAsset", prefix:"post", opId:6, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"alt",type:"string",required:false}, {name:"asset",type:"string",required:false}, {name:"assetBytes",type:"integer",required:false}, {name:"assetDigest",type:"string",required:false}, {name:"assetH",type:"integer",required:false}, {name:"assetKey",type:"string",required:false}, {name:"assetKind",type:"string",required:false}, {name:"assetMime",type:"string",required:false}, {name:"assetMs",type:"integer",required:false}, {name:"assetSecret",type:"string",required:false}, {name:"assetSession",type:"string",required:false}, {name:"assetVia",type:"string",required:false}, {name:"assetW",type:"integer",required:false}, {name:"at",type:"integer",required:true}, {name:"id",type:"string",required:true}]},
    "post.react": {name:"post.react", prefix:"post", opId:3, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"emoji",type:"string",required:true}]},
    "post.removeAsset": {name:"post.removeAsset", prefix:"post", opId:7, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"id",type:"string",required:true}]},
    "post.retract": {name:"post.retract", prefix:"post", opId:2, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[]},
    "post.setDocument": {name:"post.setDocument", prefix:"post", opId:5, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"document",type:"string",required:false}, {name:"documentBytes",type:"integer",required:false}, {name:"documentDigest",type:"string",required:false}, {name:"documentH",type:"integer",required:false}, {name:"documentKey",type:"string",required:false}, {name:"documentKind",type:"string",required:false}, {name:"documentMime",type:"string",required:false}, {name:"documentMs",type:"integer",required:false}, {name:"documentSecret",type:"string",required:false}, {name:"documentSession",type:"string",required:false}, {name:"documentVia",type:"string",required:false}, {name:"documentW",type:"integer",required:false}, {name:"name",type:"string",required:false}]},
    "post.setMedia": {name:"post.setMedia", prefix:"post", opId:1, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"banner",type:"string",required:false}, {name:"bannerAlt",type:"string",required:false}, {name:"bannerMime",type:"string",required:false}, {name:"icon",type:"string",required:false}, {name:"iconMime",type:"string",required:false}]},
    "post.setProfile": {name:"post.setProfile", prefix:"post", opId:0, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"body",type:"string",required:false}, {name:"bodyFormat",type:"string",required:false}, {name:"excerpt",type:"string",required:false}, {name:"form",type:"string",required:false}, {name:"link",type:"string",required:false}, {name:"title",type:"string",required:true}]},
    "project.addDependency": {name:"project.addDependency", prefix:"project", opId:5, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"edgeId",type:"string",required:true}, {name:"from",type:"string",required:true}, {name:"kind",type:"string",required:true}, {name:"to",type:"string",required:true}]},
    "project.addLocation": {name:"project.addLocation", prefix:"project", opId:16, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"locationId",type:"string",required:true}, {name:"name",type:"string",required:true}]},
    "project.addStakeholder": {name:"project.addStakeholder", prefix:"project", opId:14, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"name",type:"string",required:true}, {name:"note",type:"string",required:false}, {name:"stakeholderId",type:"string",required:true}]},
    "project.addTimelineItem": {name:"project.addTimelineItem", prefix:"project", opId:1, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"itemId",type:"string",required:true}, {name:"kind",type:"string",required:true}, {name:"title",type:"string",required:true}]},
    "project.configure": {name:"project.configure", prefix:"project", opId:0, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"status",type:"string",required:true}, {name:"title",type:"string",required:true}]},
    "project.removeDependency": {name:"project.removeDependency", prefix:"project", opId:6, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"edgeId",type:"string",required:true}]},
    "project.removeKpi": {name:"project.removeKpi", prefix:"project", opId:19, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"kpiId",type:"string",required:true}]},
    "project.removeLocation": {name:"project.removeLocation", prefix:"project", opId:17, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"locationId",type:"string",required:true}]},
    "project.removeStakeholder": {name:"project.removeStakeholder", prefix:"project", opId:15, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"stakeholderId",type:"string",required:true}]},
    "project.setAssignee": {name:"project.setAssignee", prefix:"project", opId:4, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"assigned",type:"integer",required:true}, {name:"itemId",type:"string",required:true}, {name:"member",type:"string",required:true}]},
    "project.setItemField": {name:"project.setItemField", prefix:"project", opId:10, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"field",type:"string",required:true}, {name:"itemId",type:"string",required:true}, {name:"value",type:"string",required:true}]},
    "project.setItemProgress": {name:"project.setItemProgress", prefix:"project", opId:9, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"itemId",type:"string",required:true}, {name:"status",type:"string",required:true}]},
    "project.setItemSchedule": {name:"project.setItemSchedule", prefix:"project", opId:3, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"endDate",type:"integer",required:false}, {name:"itemId",type:"string",required:true}]},
    "project.setKpi": {name:"project.setKpi", prefix:"project", opId:18, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"kpiId",type:"string",required:true}, {name:"label",type:"string",required:true}, {name:"target",type:"string",required:true}]},
    "project.setObjective": {name:"project.setObjective", prefix:"project", opId:12, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"goal",type:"string",required:true}, {name:"headline",type:"string",required:true}]},
    "project.setRole": {name:"project.setRole", prefix:"project", opId:13, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"member",type:"string",required:true}, {name:"role",type:"string",required:true}]},
    "project.subscribe": {name:"project.subscribe", prefix:"project", opId:7, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"disclosure",type:"string",required:true}, {name:"kind",type:"string",required:true}, {name:"originItem",type:"string",required:false}, {name:"subId",type:"string",required:true}, {name:"target",type:"string",required:true}]},
    "project.suppressItem": {name:"project.suppressItem", prefix:"project", opId:2, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"itemId",type:"string",required:true}, {name:"suppressed",type:"integer",required:true}]},
    "project.touchSubscription": {name:"project.touchSubscription", prefix:"project", opId:11, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"subId",type:"string",required:true}]},
    "project.unsubscribe": {name:"project.unsubscribe", prefix:"project", opId:8, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"subId",type:"string",required:true}]},
    "ratify.close": {name:"ratify.close", prefix:"ratify", opId:61442, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"ballots",type:"string",required:false}, {name:"electorate",type:"string",required:false}, {name:"target_author",type:"string",required:true}, {name:"target_gen",type:"integer",required:true}]},
    "ratify.propose": {name:"ratify.propose", prefix:"ratify", opId:61440, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"payload",type:"string",required:true}, {name:"rule",type:"integer",required:true}]},
    "ratify.vote": {name:"ratify.vote", prefix:"ratify", opId:61441, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"ballot",type:"integer",required:true}, {name:"target_author",type:"string",required:true}, {name:"target_gen",type:"integer",required:true}]},
    "system.define": {name:"system.define", prefix:"system", opId:0, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"connector",type:"string",required:true}, {name:"name",type:"string",required:true}, {name:"scope",type:"string",required:false}]},
    "system.hydrate": {name:"system.hydrate", prefix:"system", opId:10, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"fetchedAt",type:"integer",required:true}, {name:"key",type:"string",required:true}, {name:"payload",type:"string",required:true}, {name:"rev",type:"integer",required:true}, {name:"withdrawn",type:"integer",required:false}]},
    "thing.addPhoto": {name:"thing.addPhoto", prefix:"thing", opId:4, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"id",type:"string",required:true}, {name:"photo",type:"string",required:false}, {name:"photoBytes",type:"integer",required:false}, {name:"photoDigest",type:"string",required:false}, {name:"photoH",type:"integer",required:false}, {name:"photoKey",type:"string",required:false}, {name:"photoKind",type:"string",required:false}, {name:"photoMime",type:"string",required:false}, {name:"photoMs",type:"integer",required:false}, {name:"photoSecret",type:"string",required:false}, {name:"photoSession",type:"string",required:false}, {name:"photoVia",type:"string",required:false}, {name:"photoW",type:"integer",required:false}]},
    "thing.clearPosture": {name:"thing.clearPosture", prefix:"thing", opId:2, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[]},
    "thing.removePhoto": {name:"thing.removePhoto", prefix:"thing", opId:5, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"id",type:"string",required:true}]},
    "thing.setDisposition": {name:"thing.setDisposition", prefix:"thing", opId:6, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"for",type:"string",required:false}, {name:"state",type:"string",required:true}, {name:"transaction",type:"string",required:false}, {name:"until",type:"integer",required:false}]},
    "thing.setPhoto": {name:"thing.setPhoto", prefix:"thing", opId:3, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"mime",type:"string",required:false}, {name:"photo",type:"string",required:false}]},
    "thing.setPosture": {name:"thing.setPosture", prefix:"thing", opId:1, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"area",type:"string",required:false}, {name:"deadline",type:"integer",required:false}, {name:"posture",type:"string",required:true}, {name:"price",type:"string",required:false}, {name:"reach",type:"string",required:false}]},
    "thing.setProfile": {name:"thing.setProfile", prefix:"thing", opId:0, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"category",type:"string",required:false}, {name:"condition",type:"string",required:false}, {name:"description",type:"string",required:false}, {name:"descriptor",type:"string",required:false}, {name:"name",type:"string",required:true}]},
    "transaction.accept": {name:"transaction.accept", prefix:"transaction", opId:1, authority:"role:buyer", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"terms",type:"integer",required:true}]},
    "transaction.cancel": {name:"transaction.cancel", prefix:"transaction", opId:5, authority:"role:buyer|role:seller", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"reason",type:"string",required:false}]},
    "transaction.confirmReceipt": {name:"transaction.confirmReceipt", prefix:"transaction", opId:2, authority:"role:buyer", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}]},
    "transaction.confirmSale": {name:"transaction.confirmSale", prefix:"transaction", opId:4, authority:"role:seller", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}]},
    "transaction.dispute": {name:"transaction.dispute", prefix:"transaction", opId:3, authority:"role:buyer|role:seller", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"reason",type:"string",required:true}]},
    "transaction.fraudSignal": {name:"transaction.fraudSignal", prefix:"transaction", opId:7, authority:"role:settler", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"score",type:"integer",required:false}, {name:"signal",type:"string",required:true}]},
    "transaction.rate": {name:"transaction.rate", prefix:"transaction", opId:8, authority:"role:buyer|role:seller", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"stars",type:"integer",required:true}, {name:"text",type:"string",required:false}]},
    "transaction.resolve": {name:"transaction.resolve", prefix:"transaction", opId:6, authority:"role:settler", fold:"commutative", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"note",type:"string",required:false}, {name:"outcome",type:"string",required:true}]},
    "transaction.setSite": {name:"transaction.setSite", prefix:"transaction", opId:9, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"at",type:"integer",required:true}, {name:"site",type:"string",required:true}, {name:"thingId",type:"string",required:true}]},
    "transaction.setTerms": {name:"transaction.setTerms", prefix:"transaction", opId:0, authority:"role:seller", fold:"commutative", reachable:true, why:null, fields:[{name:"acceptBy",type:"integer",required:false}, {name:"amount",type:"integer",required:true}, {name:"at",type:"integer",required:true}, {name:"confirmWithin",type:"integer",required:false}, {name:"currency",type:"string",required:true}, {name:"delivery",type:"string",required:true}, {name:"descriptor",type:"string",required:false}, {name:"qty",type:"integer",required:true}, {name:"shipBy",type:"integer",required:false}, {name:"thing",type:"string",required:false}]},
    "treasury.attestBalance": {name:"treasury.attestBalance", prefix:"treasury", opId:3, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"amount",type:"integer",required:true}, {name:"at",type:"integer",required:true}]},
    "treasury.attestSettlement": {name:"treasury.attestSettlement", prefix:"treasury", opId:2, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"amount",type:"integer",required:true}, {name:"at",type:"integer",required:true}, {name:"memo",type:"string",required:false}, {name:"proposal",type:"string",required:false}, {name:"reference",type:"string",required:true}]},
    "treasury.recordDeposit": {name:"treasury.recordDeposit", prefix:"treasury", opId:1, authority:"anyMember", fold:"commutative", reachable:true, why:null, fields:[{name:"amount",type:"integer",required:true}, {name:"at",type:"integer",required:true}, {name:"reference",type:"string",required:true}, {name:"source",type:"string",required:false}]},
    "treasury.setPolicy": {name:"treasury.setPolicy", prefix:"treasury", opId:0, authority:"owner", fold:"sequenced", reachable:true, why:null, fields:[{name:"account",type:"string",required:false}, {name:"bands",type:"string",required:true}, {name:"cooloffHours",type:"integer",required:true}, {name:"currency",type:"string",required:true}, {name:"disclosure",type:"string",required:false}]},
  };

  /* Validate against the ICD's own schema before the core is troubled. The
     core validates authoritatively — this is only so a caller finds out in
     the same tick, with the field named. */
  function check(spec, args) {
    args = args || {};
    var out = {};
    for (var i = 0; i < spec.fields.length; i++) {
      var f = spec.fields[i], v = args[f.name];
      if (v === undefined || v === null) {
        if (f.required) throw new Error(spec.name + ': ' + f.name + ' is required');
        continue;
      }
      if (f.type === 'integer') {
        if (typeof v !== 'number' || !Number.isInteger(v)) {
          throw new Error(spec.name + ': ' + f.name + ' must be an integer (the core\'s ArgVal has no float)');
        }
      } else if (typeof v !== 'string') {
        throw new Error(spec.name + ': ' + f.name + ' must be a string');
      }
      out[f.name] = v;
    }
    for (var k in args) {
      if (Object.prototype.hasOwnProperty.call(args, k) && !(k in out)) {
        throw new Error(spec.name + ': ' + k + ' is not an argument of this op ' +
          '(the ICD sets additionalProperties:false)');
      }
    }
    return out;
  }

  /* One draft: {op, opId, args}. The core resolves the object's kind and
     picks the door; this never guesses which one. */
  function draft(name, args) {
    var spec = OPS[name];
    if (!spec) throw new Error('no op ' + name + ' in the ICD');
    if (!spec.reachable) throw new Error(name + ' cannot be authored: ' + spec.why);
    return { op: spec.name, opId: spec.opId, args: check(spec, args) };
  }

  var api = { OPS: OPS, draft: draft, check: check,
              names: Object.keys(OPS),
              reachable: Object.keys(OPS).filter(function (k) { return OPS[k].reachable; }),
              blocked: Object.keys(OPS).filter(function (k) { return !OPS[k].reachable; }) };

  /* One function per op, named for it. Unreachable ops get a function that
     throws the reason rather than no function at all — a missing name reads
     as a typo; a refusal reads as the truth. */
  api.baseAnswerQuestion = function (a) { return draft("base.answerQuestion", a); };
  api.baseClaimSpent = function (a) { return draft("base.claimSpent", a); };
  api.baseClearBacklink = function (a) { return draft("base.clearBacklink", a); };
  api.baseClearLocation = function (a) { return draft("base.clearLocation", a); };
  api.baseClearParent = function (a) { return draft("base.clearParent", a); };
  api.baseClearPart = function (a) { return draft("base.clearPart", a); };
  api.baseClearRole = function (a) { return draft("base.clearRole", a); };
  api.baseDefineQuestion = function (a) { return draft("base.defineQuestion", a); };
  api.baseMemberJoined = function (a) { return draft("base.memberJoined", a); };
  api.baseMemberLeft = function (a) { return draft("base.memberLeft", a); };
  api.baseOwnerHandover = function (a) { return draft("base.ownerHandover", a); };
  api.basePublish = function (a) { return draft("base.publish", a); };
  api.basePublishAbout = function (a) { return draft("base.publishAbout", a); };
  api.basePublishProfile = function (a) { return draft("base.publishProfile", a); };
  api.baseRetireQuestion = function (a) { return draft("base.retireQuestion", a); };
  api.baseSetBacklink = function (a) { return draft("base.setBacklink", a); };
  api.baseSetLocation = function (a) { return draft("base.setLocation", a); };
  api.baseSetParent = function (a) { return draft("base.setParent", a); };
  api.baseSetPart = function (a) { return draft("base.setPart", a); };
  api.baseSetRole = function (a) { return draft("base.setRole", a); };
  api.baseSetVisibility = function (a) { return draft("base.setVisibility", a); };
  api.baseUnpublish = function (a) { return draft("base.unpublish", a); };
  api.contactAdmitTicket = function (a) { return draft("contact.admitTicket", a); };
  api.contactDeliverTicket = function (a) { return draft("contact.deliverTicket", a); };
  api.contactDiscoverRequest = function (a) { return draft("contact.discoverRequest", a); };
  api.contactDiscoverResponse = function (a) { return draft("contact.discoverResponse", a); };
  api.contactInvite = function (a) { return draft("contact.invite", a); };
  api.contactInviteReply = function (a) { return draft("contact.inviteReply", a); };
  api.contactPrekeyConsume = function (a) { return draft("contact.prekeyConsume", a); };
  api.contactPrekeyRevoke = function (a) { return draft("contact.prekeyRevoke", a); };
  api.contactPrekeySupply = function (a) { return draft("contact.prekeySupply", a); };
  api.contactPublishListing = function (a) { return draft("contact.publishListing", a); };
  api.contactPublishProfile = function (a) { return draft("contact.publishProfile", a); };
  api.contactRequestTicket = function (a) { return draft("contact.requestTicket", a); };
  api.contactSetLink = function (a) { return draft("contact.setLink", a); };
  api.eventAddPhoto = function (a) { return draft("event.addPhoto", a); };
  api.eventEditProfile = function (a) { return draft("event.editProfile", a); };
  api.eventRecordSale = function (a) { return draft("event.recordSale", a); };
  api.eventRedeem = function (a) { return draft("event.redeem", a); };
  api.eventRemovePhoto = function (a) { return draft("event.removePhoto", a); };
  api.eventSetBanner = function (a) { return draft("event.setBanner", a); };
  api.eventSetClip = function (a) { return draft("event.setClip", a); };
  api.eventSetLineup = function (a) { return draft("event.setLineup", a); };
  api.eventSetMedia = function (a) { return draft("event.setMedia", a); };
  api.eventSetProfile = function (a) { return draft("event.setProfile", a); };
  api.eventSetTickets = function (a) { return draft("event.setTickets", a); };
  api.eventSetVenue = function (a) { return draft("event.setVenue", a); };
  api.forumEditDescription = function (a) { return draft("forum.editDescription", a); };
  api.forumPost = function (a) { return draft("forum.post", a); };
  api.forumReact = function (a) { return draft("forum.react", a); };
  api.forumReceipt = function (a) { return draft("forum.receipt", a); };
  api.forumRetract = function (a) { return draft("forum.retract", a); };
  api.forumVote = function (a) { return draft("forum.vote", a); };
  api.groupClearAffiliation = function (a) { return draft("group.clearAffiliation", a); };
  api.groupClearClaimIssuer = function (a) { return draft("group.clearClaimIssuer", a); };
  api.groupClearOffice = function (a) { return draft("group.clearOffice", a); };
  api.groupEditFace = function (a) { return draft("group.editFace", a); };
  api.groupJoinedObject = function (a) { return draft("group.joinedObject", a); };
  api.groupLeftObject = function (a) { return draft("group.leftObject", a); };
  api.groupPublishListing = function (a) { return draft("group.publishListing", a); };
  api.groupRemoveListing = function (a) { return draft("group.removeListing", a); };
  api.groupRevokeCredential = function (a) { return draft("group.revokeCredential", a); };
  api.groupRsvp = function (a) { return draft("group.rsvp", a); };
  api.groupRsvpDecide = function (a) { return draft("group.rsvpDecide", a); };
  api.groupSetAffiliation = function (a) { return draft("group.setAffiliation", a); };
  api.groupSetClaimIssuer = function (a) { return draft("group.setClaimIssuer", a); };
  api.groupSetCover = function (a) { return draft("group.setCover", a); };
  api.groupSetFace = function (a) { return draft("group.setFace", a); };
  api.groupSetOffice = function (a) { return draft("group.setOffice", a); };
  api.groupSetPresence = function (a) { return draft("group.setPresence", a); };
  api.groupSetProfile = function (a) { return draft("group.setProfile", a); };
  api.groupSetRegistration = function (a) { return draft("group.setRegistration", a); };
  api.groupStoreCredential = function (a) { return draft("group.storeCredential", a); };
  api.hostDefine = function (a) { return draft("host.define", a); };
  api.hostEditMedia = function (a) { return draft("host.editMedia", a); };
  api.hostHydrate = function (a) { return draft("host.hydrate", a); };
  api.hostSetMedia = function (a) { return draft("host.setMedia", a); };
  api.noteComment = function (a) { return draft("note.comment", a); };
  api.notePromote = function (a) { return draft("note.promote", a); };
  api.noteReact = function (a) { return draft("note.react", a); };
  api.noteRetract = function (a) { return draft("note.retract", a); };
  api.noteWrite = function (a) { return draft("note.write", a); };
  api.placeClearLand = function (a) { return draft("place.clearLand", a); };
  api.placePost = function (a) { return draft("place.post", a); };
  api.placeSetAccess = function (a) { return draft("place.setAccess", a); };
  api.placeSetDoorbell = function (a) { return draft("place.setDoorbell", a); };
  api.placeSetLand = function (a) { return draft("place.setLand", a); };
  api.placeSetProfile = function (a) { return draft("place.setProfile", a); };
  api.postAddAsset = function (a) { return draft("post.addAsset", a); };
  api.postReact = function (a) { return draft("post.react", a); };
  api.postRemoveAsset = function (a) { return draft("post.removeAsset", a); };
  api.postRetract = function (a) { return draft("post.retract", a); };
  api.postSetDocument = function (a) { return draft("post.setDocument", a); };
  api.postSetMedia = function (a) { return draft("post.setMedia", a); };
  api.postSetProfile = function (a) { return draft("post.setProfile", a); };
  api.projectAddDependency = function (a) { return draft("project.addDependency", a); };
  api.projectAddLocation = function (a) { return draft("project.addLocation", a); };
  api.projectAddStakeholder = function (a) { return draft("project.addStakeholder", a); };
  api.projectAddTimelineItem = function (a) { return draft("project.addTimelineItem", a); };
  api.projectConfigure = function (a) { return draft("project.configure", a); };
  api.projectRemoveDependency = function (a) { return draft("project.removeDependency", a); };
  api.projectRemoveKpi = function (a) { return draft("project.removeKpi", a); };
  api.projectRemoveLocation = function (a) { return draft("project.removeLocation", a); };
  api.projectRemoveStakeholder = function (a) { return draft("project.removeStakeholder", a); };
  api.projectSetAssignee = function (a) { return draft("project.setAssignee", a); };
  api.projectSetItemField = function (a) { return draft("project.setItemField", a); };
  api.projectSetItemProgress = function (a) { return draft("project.setItemProgress", a); };
  api.projectSetItemSchedule = function (a) { return draft("project.setItemSchedule", a); };
  api.projectSetKpi = function (a) { return draft("project.setKpi", a); };
  api.projectSetObjective = function (a) { return draft("project.setObjective", a); };
  api.projectSetRole = function (a) { return draft("project.setRole", a); };
  api.projectSubscribe = function (a) { return draft("project.subscribe", a); };
  api.projectSuppressItem = function (a) { return draft("project.suppressItem", a); };
  api.projectTouchSubscription = function (a) { return draft("project.touchSubscription", a); };
  api.projectUnsubscribe = function (a) { return draft("project.unsubscribe", a); };
  api.ratifyClose = function (a) { return draft("ratify.close", a); };
  api.ratifyPropose = function (a) { return draft("ratify.propose", a); };
  api.ratifyVote = function (a) { return draft("ratify.vote", a); };
  api.systemDefine = function (a) { return draft("system.define", a); };
  api.systemHydrate = function (a) { return draft("system.hydrate", a); };
  api.thingAddPhoto = function (a) { return draft("thing.addPhoto", a); };
  api.thingClearPosture = function (a) { return draft("thing.clearPosture", a); };
  api.thingRemovePhoto = function (a) { return draft("thing.removePhoto", a); };
  api.thingSetDisposition = function (a) { return draft("thing.setDisposition", a); };
  api.thingSetPhoto = function (a) { return draft("thing.setPhoto", a); };
  api.thingSetPosture = function (a) { return draft("thing.setPosture", a); };
  api.thingSetProfile = function (a) { return draft("thing.setProfile", a); };
  api.transactionAccept = function (a) { return draft("transaction.accept", a); };
  api.transactionCancel = function (a) { return draft("transaction.cancel", a); };
  api.transactionConfirmReceipt = function (a) { return draft("transaction.confirmReceipt", a); };
  api.transactionConfirmSale = function (a) { return draft("transaction.confirmSale", a); };
  api.transactionDispute = function (a) { return draft("transaction.dispute", a); };
  api.transactionFraudSignal = function (a) { return draft("transaction.fraudSignal", a); };
  api.transactionRate = function (a) { return draft("transaction.rate", a); };
  api.transactionResolve = function (a) { return draft("transaction.resolve", a); };
  api.transactionSetSite = function (a) { return draft("transaction.setSite", a); };
  api.transactionSetTerms = function (a) { return draft("transaction.setTerms", a); };
  api.treasuryAttestBalance = function (a) { return draft("treasury.attestBalance", a); };
  api.treasuryAttestSettlement = function (a) { return draft("treasury.attestSettlement", a); };
  api.treasuryRecordDeposit = function (a) { return draft("treasury.recordDeposit", a); };
  api.treasurySetPolicy = function (a) { return draft("treasury.setPolicy", a); };

  return api;
}));
