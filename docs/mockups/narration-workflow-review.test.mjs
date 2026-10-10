import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { JSDOM, VirtualConsole } from 'jsdom';

// Exercise the exported mockup, without network resources, audio or microphone access.
const exported = readFileSync(new URL('./narration-workflow-review.html', import.meta.url), 'utf8');
const wrapper = new JSDOM(exported);
const fragment = wrapper.window.document.querySelector('iframe').getAttribute('data-srcdoc');
wrapper.window.close();

function mount(t, design) {
  const dom = new JSDOM(fragment, { runScripts: 'outside-only', virtualConsole: new VirtualConsole() });
  const script = [...dom.window.document.scripts].find(s => s.textContent.includes("const root = document.getElementById('narration-workflow')"));
  assert.ok(script, 'export retains the editable prototype interactions');
  dom.window.eval(script.textContent);
  t.after(() => dom.window.close());
  const app = dom.window.document.querySelector(`[data-design="${design}"]`);
  const q = selector => app.querySelector(selector);
  const click = action => q(`[data-action="${action}"]`).click();
  const input = (selector, value) => {
    q(selector).value = value;
    q(selector).dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  };
  return { dom, app, q, click, input };
}

for (const design of ['slide-first', 'workspace']) {
  test(`${design}: script edit affects only its slide and old takes remain selectable`, t => {
    const { q, click, input } = mount(t, design);
    const originalScript = q('[data-script]').value;
    input('[data-script]', 'A revised script for this slide.');
    assert.match(q('[data-audio-status]').textContent, /needs to be generated/);
    click('generate');
    assert.equal(q('[data-take-count]').textContent, '(3)');
    assert.equal(q('[data-use="2"]').textContent, 'Use & script');
    q('[data-use="2"]').click();
    assert.match(q('[data-takes]').textContent, /Take 1.*Used in video/s);
    q('[data-slide="0"]').click();
    assert.match(q('[data-audio-status]').textContent, /ready for video/);
    q('[data-slide="1"]').click();
    assert.equal(q('[data-script]').value, originalScript);
  });

  test(`${design}: recording prep precedes capture; preview name is editable and retry preserves it`, t => {
    const { q, click, input } = mount(t, design);
    click('voice');
    assert.equal(q('[data-voice-name]').value, '');
    input('[data-voice-name]', 'Workshop voice');
    click('record-prep');
    assert.equal(q('[data-record-prep]').hidden, false);
    assert.equal(q('[data-reference-fields]').hidden, true);
    assert.ok(q('[data-warmup]').compareDocumentPosition(q('[data-passage]')) & 4);
    click('start-record');
    assert.match(q('[data-reference-status]').textContent, /warm-up discarded/);
    assert.equal(q('[data-action="voice-preview"]').disabled, true);
    q('[data-consent]').click();
    click('voice-preview');
    assert.equal(q('[data-action="save-voice"]').disabled, false);
    input('[data-voice-name]', '');
    assert.equal(q('[data-action="save-voice"]').disabled, true);
    input('[data-voice-name]', 'Workshop presenter');
    click('record-again');
    assert.equal(q('[data-voice-name]').value, 'Workshop presenter');
    assert.equal(q('[data-record-prep]').hidden, false);
    assert.equal(q('[data-reference-fields]').hidden, true);
  });

  test(`${design}: import → preview → save selects a presenter only for the current slide`, t => {
    const { q, click, input } = mount(t, design);
    click('voice');
    input('[data-voice-name]', 'Reference presenter');
    click('import-reference');
    q('[data-consent]').click();
    click('voice-preview');
    input('[data-voice-name]', 'Final presenter');
    click('save-voice');
    assert.equal(q('[data-overlay]').hidden, true);
    assert.equal(q('[data-presenter]').value, 'Final presenter');
    q('[data-slide="0"]').click();
    assert.equal(q('[data-presenter]').value, 'Ryan · English');
    assert.match(q('[data-audio-status]').textContent, /ready for video/);
    q('[data-slide="1"]').click();
    assert.equal(q('[data-presenter]').value, 'Final presenter');
  });

  test(`${design}: export prepares missing audio before simulated export`, t => {
    const { q, click } = mount(t, design);
    click('export');
    assert.match(q('[data-dialog]').textContent, /4 slides need current audio/);
    assert.equal(q('[data-action="finish-export"]'), null);
    click('prepare-audio');
    assert.match(q('[data-dialog]').textContent, /All slides are ready/);
    click('finish-export');
    assert.match(q('[data-notice]').textContent, /no file was created/);
  });

  test(`${design}: blank script uses five seconds; Chat switches preserve edits`, t => {
    const { q, input, click } = mount(t, design);
    input('[data-script]', '');
    assert.equal(q('[data-silent-control]').hidden, false);
    assert.equal(q('[data-silent-control] input').value, '5');
    q('[data-tab="chat"]').click();
    assert.equal(q('[data-chat]').hidden, false);
    click('narration');
    assert.equal(q('[data-narration]').hidden, false);
    assert.equal(q('[data-script]').value, '');
    click('generate');
    assert.match(q('[data-notice]').textContent, /silent slide.*5 seconds/);
  });
}

test('Separate workspace has distinct slide and presentation tasks', t => {
  const { q } = mount(t, 'workspace');
  assert.equal(q('[data-subtabs]').hidden, false);
  assert.equal(q('[data-footer]').hidden, true);
  q('[data-scope="deck"]').click();
  assert.equal(q('[data-slide-panel]').hidden, true);
  assert.equal(q('[data-deck-panel]').hidden, false);
  assert.equal(q('[data-footer]').hidden, false);
  assert.equal(q('[data-deck-panel]').querySelectorAll('[data-jump], .ss-deck-row').length, 0);
  assert.equal(q('.ss-thumbs').querySelectorAll('[data-slide]').length, 6);
  q('[data-slide="3"]').click();
  assert.equal(q('[data-slide-panel]').hidden, false);
  assert.equal(q('[data-deck-panel]').hidden, true);
  assert.equal(q('[data-panel-title]').textContent, 'Build it into the workflow');
});
