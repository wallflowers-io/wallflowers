// palette.js: every Face editor theme, and a few hard ones, make a palette that reads.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const { from, contrast, PAPER } = createRequire(import.meta.url)('./palette.js');

// The Face editor's own themes (website assets/face/face.js PRESETS), by their colours alone.
const LOOKS = {
  paper: PAPER,
  ink: { background: '#000000', cards: '#14181E', buttons: '#FAF8F3', buttonText: '#14181E', text: '#E6EDE9', title: '#FAF8F3' },
  marker: { background: '#F4EFB4', cards: '#FFFCE8', buttons: '#14181E', buttonText: '#FFFFFF', text: '#14181E', title: '#14181E' },
  tulip: { background: '#CD5B7A', cards: '#FFFFFF', buttons: '#FFFFFF', buttonText: '#FFFFFF', text: '#FFFFFF', title: '#FFFFFF' },
  dusk: { background: '#1C1A3A', cards: '#FFFFFF', buttons: '#FFFFFF', buttonText: '#FFFFFF', text: '#FFFFFF', title: '#FFFFFF' },
  terminal: { background: '#0B0F0C', cards: '#0B0F0C', buttons: '#7CFFB2', buttonText: '#14181E', text: '#B8F5CF', title: '#7CFFB2' },
  sunflower: { background: '#EADD3E', cards: '#FFF9D6', buttons: '#14181E', buttonText: '#FFFFFF', text: '#14181E', title: '#14181E' },
  riso: { background: '#F3EDE0', cards: '#F3EDE0', buttons: '#546CAC', buttonText: '#FFFFFF', text: '#A83A5A', title: '#A83A5A' },
  egregore: { background: '#243819', cards: '#FFFFFF', buttons: '#EADD3E', buttonText: '#14181E', text: '#F3F1E4', title: '#EADD3E' },
  // an unreadable one: the words the colour of the ground
  clash: { background: '#808080', cards: '#808080', buttons: '#808080', buttonText: '#808080', text: '#808080', title: '#808080' }
};

for (const [name, colours] of Object.entries(LOOKS)) {
  test(`${name}: words, titles and the accent read on the pane`, () => {
    const { vars } = from({ colours });
    const on = (k) => contrast(vars[k], vars['--s0']);
    assert.ok(on('--ink') >= 9, `ink ${on('--ink').toFixed(2)}`);
    assert.ok(on('--i2') >= 5.5, `i2 ${on('--i2').toFixed(2)}`);
    assert.ok(on('--i3') >= 3.4, `i3 ${on('--i3').toFixed(2)}`);
    assert.ok(on('--title') >= 4.5, `title ${on('--title').toFixed(2)}`);
    assert.ok(on('--accent') >= 3, `accent ${on('--accent').toFixed(2)}`);
    assert.ok(contrast(vars['--on-accent'], vars['--accent']) >= 3, 'on-accent');
    for (const k of ['--s0', '--s1', '--s2', '--s3', '--hair', '--a1', '--a2', '--a3', '--a4', '--a5', '--a6']) {
      assert.match(vars[k], /^#[0-9A-F]{6}$/, k);
    }
  });
}

test('the ground stays quiet: a loud background is not the pane', () => {
  const { vars } = from({ colours: LOOKS.tulip });
  assert.notEqual(vars['--s0'], '#CD5B7A');
  assert.ok(contrast(vars['--s0'], '#FFFFFF') < 1.25, 'the pane of a pink Face is a pale ground, not pink');
});

test('the accent shades run from a tint to a deep', () => {
  const { vars } = from({ colours: LOOKS.riso });
  const toPane = ['--a1', '--a2', '--a3', '--a4', '--a5', '--a6'].map((k) => contrast(vars[k], vars['--s0']));
  for (let i = 1; i < toPane.length; i++) assert.ok(toPane[i] > toPane[i - 1], `a${i + 1} stronger than a${i}`);
});

test('dark Faces make a dark room, light ones a light room', () => {
  assert.equal(from({ colours: LOOKS.egregore }).dark, true);
  assert.equal(from({ colours: LOOKS.terminal }).dark, true);
  assert.equal(from({ colours: LOOKS.riso }).dark, false);
  assert.equal(from({ colours: LOOKS.sunflower }).dark, false);
});

test('no look, or a broken one, is Paper', () => {
  assert.deepEqual(from(null).vars, from({ colours: PAPER }).vars);
  assert.deepEqual(from({ colours: { background: 'red', text: 'url(x)' } }).vars, from({ colours: PAPER }).vars);
});

test('white buttons give way to the Face\'s own colour for the accent', () => {
  const { vars } = from({ colours: LOOKS.tulip });
  const { vars: grey } = from({ colours: { ...LOOKS.tulip, background: '#EEEEEE' } });
  assert.notEqual(vars['--a4'], grey['--a4']);
  assert.ok(contrast(vars['--a4'], '#CD5B7A') < 2, `tulip's accent ${vars['--a4']} is its pink, held readable`);
});
