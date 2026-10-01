/* THE `site_group` RULE, under node.

   Run:  node world.site-group.test.cjs      (or: node --test world.site-group.test.cjs)

   A SEPARATE FILE ON PURPOSE, and a temporary one. world.test.cjs is the home
   for this — same subject, same fixtures — and these cases belong pasted into
   it. They are here because the cutover that added the rule did not own that
   file, and adding a case to somebody else's suite in a parallel tree is how
   two tracks end up editing one file. Fold it in and delete this.

   WHAT THE RULE IS. `worldFromFold` derived the site from the exporter's own
   INDIVIDUAL group — the archive's "a person IS a Group" case. A community's
   archive has no such group to stand for it: STOMA is an ORGANISATION, and its
   members' individual groups are all somebody else. So a fold may DECLARE its
   site — `site_group`, an edge exactly like `own_group` and
   `peers[].identity_group` — and the mapping reads the edge instead of
   guessing. A fold that declares nothing behaves exactly as it did, which is
   the last case below and the one that matters most. */
'use strict';
process.env.TZ = 'UTC';

const test = require('node:test');
const assert = require('node:assert/strict');
const { worldFromFold } = require('./world.js');

const key = (c) => c.repeat(64);
const gid = (c) => c.repeat(64);
const ME = key('a'), BOB = key('b');

/* Ada's archive of STOMA: her own group, Bob's, the site, and a forum. The two
   rosters disagree about Bob on purpose — 'Viewer' on Ada's own group, 'Guest'
   on the site's — because which one the mapping reads is the question. */
function fixture() {
  return {
    v: 1, exported_by: ME, display_name: 'Ada',
    own_group: gid('6'), site_group: gid('8'),
    peers: [{ id: BOB, name: 'bob', status: 'connected', identity_group: gid('7') }],
    groups: [
      { id: gid('6'), kind: 'group', name: 'Ada', owner: ME, members: [ME],
        view: { display_name: 'Ada Lovelace', shape: 'individual', presence: 'onPlatform',
                roles: [[ME, 'Owner'], [BOB, 'Viewer']] } },
      { id: gid('7'), kind: 'group', name: 'Bob', owner: BOB, members: [BOB],
        view: { display_name: 'Bob B', shape: 'individual', presence: 'onPlatform', roles: [] } },
      { id: gid('8'), kind: 'group', name: 'STOMA', owner: ME, members: [ME, BOB],
        view: { display_name: 'STOMA', shape: 'organisation', presence: 'onPlatform',
                roles: [[ME, 'Owner'], [BOB, 'Guest']] } },
      { id: gid('1'), kind: 'forum', name: 'Cookbook', owner: ME, members: [ME, BOB],
        view: { messages: [] } }
    ]
  };
}

test('a declared organisation is the site, keyed and named the way a person is', () => {
  const w = worldFromFold(fixture(), ME);
  assert.equal(w.site, 'stoma');
  assert.deepEqual(w.sites,
    { stoma: { name: 'STOMA', kind: 'Community', disc: 'chat', shape: 'organisation' } });
  assert.equal(w.me, 'ada-lovelace');
  assert.equal(w.people['ada-lovelace'].home, 'stoma');
});

test('the roles are the SITE group\'s, not the exporter\'s own', () => {
  const w = worldFromFold(fixture(), ME);
  assert.equal(w.people['ada-lovelace'].role, 'Owner');
  assert.equal(w.people['bob-b'].role, 'Guest');
});

test('a team site is a Team, and the site group is still not a person', () => {
  const f = fixture();
  f.groups[2].view.shape = 'team';
  const w = worldFromFold(f, ME);
  assert.equal(w.sites.stoma.kind, 'Team');
  assert.deepEqual(Object.keys(w.people).sort(), ['ada-lovelace', 'bob-b']);
});

test('a nameless site group is called by its id, as a nameless person is', () => {
  const f = fixture();
  f.groups[2].view.display_name = '';
  f.groups[2].name = '';
  assert.equal(worldFromFold(f, ME).site, '88888888');
});

test('a fold that declares nothing behaves exactly as it did', () => {
  const f = fixture();
  delete f.site_group;
  const w = worldFromFold(f, ME);
  assert.equal(w.site, 'ada-lovelace');
  assert.equal(w.sites['ada-lovelace'].kind, 'Individual');
  assert.equal(w.sites['ada-lovelace'].shape, 'individual');
  assert.equal(w.people['bob-b'].role, 'Viewer');
});

test('a site_group it cannot resolve is refused, by name', () => {
  const no = fixture(); no.site_group = gid('9');
  assert.throws(() => worldFromFold(no, ME), /site_group 9{16}… names no group/);

  const forum = fixture(); forum.site_group = gid('1');
  assert.throws(() => worldFromFold(forum, ME), /is a forum, and a site is a group/);

  const odd = fixture(); odd.groups[2].view.shape = 'collective';
  assert.throws(() => worldFromFold(odd, ME), /not a GroupShape/);
});
