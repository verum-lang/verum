import assert from 'node:assert/strict';

/** Exercise the operator, both real Studio adapters and their local protocol peers. */
export async function exerciseHandoffs(evaluate, wait) {
  const peerText = '  Antigravity relay <b>λ</b>\nsecond line  ';
  const hostText = 'author response · <b>λ</b>\nsource occurrence 1';
  await evaluate([
    "language('en'); window.provider = id => { byId('agent-provider').value=id; byId('agent-provider').dispatchEvent(new Event('change')); };",
    "window.chat = () => document.querySelector('#agent-messages > div:not([hidden]):not(#agent-welcome)');",
    "window.replyBodies = () => [...chat().querySelectorAll('.message.agent .message-body')].map(el=>el.textContent);",
    "window.lastCard = () => [...chat().querySelectorAll('.approval')].at(-1);",
    "window.forward = () => chat().querySelector('.forward-message').click();",
    "window.selectRecipient = title => { const select=byId('handoff-recipient'); const index=[...select.options].findIndex(x=>x.text.includes(title)); if(index<0) throw Error('Missing recipient '+title); select.selectedIndex=index; };",
    "window.decision = value => byId('dialog-actions').querySelector('button[value='+value+']').click();",
    "window.mail = () => byId('mailbox-body'); window.records = [];",
    "const transport = window.fetch.bind(window); window.fetch = async (url, options) => { const call = JSON.parse(options.body); if (call.method.startsWith('agent.handoff.') || call.method==='agent.respond' || call.method==='agent.request' && call.params.method === 'turn/start') records.push(call); return transport(url,options); };",
  ].join('\n'));
  assert.deepEqual(await evaluate("[...byId('agent-provider').options].map(option => [option.value,option.text])"),
    [['codex','Codex'],['agy','Antigravity']]);
  await evaluate("provider('agy'); byId('agent-connect').click()");
  await wait("!byId('agent-login').hidden && byId('agent-send').disabled");
  await evaluate("byId('agent-login').click()");
  await wait("!byId('agent-send').disabled && byId('agent-role').hidden && byId('agent-context-info').hidden");
  await evaluate('compose(' + JSON.stringify(peerText) + ')');
  await wait("chat()?.querySelector('.forward-message') && byId('agent-stop').hidden");
  assert.equal(await evaluate('replyBodies().at(-1)'), peerText);
  assert.deepEqual(await evaluate("records.find(r=>r.method==='agent.request').params.params.input"), [{type:'text', text:peerText}]);
  // The first author's retained browser transcript predates the bounded host journal.
  await evaluate("choose(0); forward()");
  await wait("byId('handoff-recipient')");
  await evaluate("selectRecipient('Antigravity'); decision('preview')");
  await wait("byId('notice-text').textContent.includes('no longer retained')");
  assert.equal(await evaluate("byId('dialog').open"), false);
  assert.equal(await evaluate("records.filter(r=>r.method==='agent.handoff.send').length"), 0);
  await evaluate("provider('agy')");

  async function preview(recipient, expected) {
    await evaluate('forward()');
    await wait("byId('dialog').open && byId('handoff-recipient')");
    await evaluate('selectRecipient(' + JSON.stringify(recipient) + "); decision('preview')");
    await wait("byId('dialog').open && byId('dialog-body').querySelector('.handoff-text')");
    assert.equal(await evaluate("byId('dialog-body').querySelector('.handoff-text').textContent"), expected);
    assert.equal(await evaluate("byId('dialog-body').querySelector('b')"), null);
  }
  await preview('Check both source revisions', peerText);
  await evaluate("decision('cancel')");
  assert.equal(await evaluate("records.filter(r=>r.method==='agent.handoff.send').length"), 0);
  await preview('Check both source revisions', peerText);
  assert.match(await evaluate("byId('dialog-body').textContent"), /From Antigravity.*To Codex/s);
  await evaluate("language('ru')");
  assert.equal(await evaluate("byId('dialog-actions').querySelector('.primary').textContent"), 'Передать во входящие');
  await evaluate("decision('send'); language('en')");
  await wait("byId('notice-text').textContent.includes('Delivered to')");
  assert.deepEqual(await evaluate("Object.keys(records.find(r=>r.method==='agent.handoff.send').params)"), ['id']);

  await evaluate("choose(3); byId('agent-mailbox').open=true");
  await wait("mail().querySelector('.shared-message pre')");
  assert.equal(await evaluate("mail().querySelector('pre').textContent"), peerText);
  await evaluate("compose('Read inbox evidence')");
  await wait("lastCard()?.textContent.includes('Review inbox access') && lastCard().querySelector('.primary')");
  await evaluate("lastCard().querySelector('.primary').click()");
  await wait("lastCard().querySelector('[data-key=response] pre') && byId('agent-stop').hidden");
  const read = await evaluate("JSON.parse(lastCard().querySelector('[data-key=response] pre').textContent)");
  assert.equal(read.success, true);
  assert.deepEqual(read.tool_result.messages.map(message=>message.text), [peerText]);

  await preview('Antigravity', hostText);
  const selection = await evaluate("records.filter(r=>r.method==='agent.handoff.preview').at(-1).params");
  await evaluate("decision('cancel')");
  assert.notEqual(selection.from.connectionId, selection.to.connectionId);
  assert.equal(selection.from.threadId, '4');
  const sendCount = await evaluate("records.filter(r=>r.method==='agent.handoff.send').length");
  async function propose(seq) {
    const prompt = 'Propose handoff ' + JSON.stringify({...selection.to, seq});
    await evaluate('compose(' + JSON.stringify(prompt) + ')');
    await wait("lastCard()?.textContent.includes('Review message transfer') && !lastCard().querySelector('[data-key=response]')");
  }
  await propose(Number.MAX_SAFE_INTEGER);
  await wait("lastCard().querySelector('.preview-error')");
  assert.equal(await evaluate("lastCard().querySelector('.primary')"), null);
  await evaluate("lastCard().querySelector('.approval-actions button').click()");
  await wait("lastCard().textContent.includes('Request declined.') && byId('agent-stop').hidden");
  await propose(selection.seq);
  await wait("lastCard().querySelector('.handoff-text') && lastCard().querySelector('.primary')");
  assert.equal(await evaluate("lastCard().querySelector('.handoff-text').textContent"), hostText);
  assert.match(await evaluate('lastCard().textContent'), /From Codex.*To Antigravity/s);
  await evaluate("lastCard().querySelector('.primary').click()");
  await wait("lastCard().querySelector('[data-key=response] pre') && byId('agent-stop').hidden");
  const delivered = await evaluate("JSON.parse(lastCard().querySelector('[data-key=response] pre').textContent)");
  assert.equal(delivered.success, true);
  assert.equal(delivered.tool_result.text, hostText);
  assert.deepEqual(delivered.tool_result.from, selection.from);
  assert.deepEqual(delivered.tool_result.to, selection.to);
  assert.equal(await evaluate("records.filter(r=>r.method==='agent.handoff.send').length"), sendCount);

  await evaluate("provider('agy'); byId('agent-mailbox').open=true");
  await wait("mail().querySelector('.shared-message button') && byId('agent-stop').hidden");
  const turnsBefore = await evaluate("records.filter(r=>r.method==='agent.request').length");
  await evaluate("byId('agent-input').value = 'Draft to preserve'; mail().querySelector('.shared-message button').click()");
  const draft = await evaluate("byId('agent-input').value");
  assert.match(draft, /^Draft to preserve\n\nQuoted message from Codex/);
  assert.ok(draft.includes(delivered.tool_result.sha256));
  assert.ok(draft.endsWith('> author response · <b>λ</b>\n> source occurrence 1'));
  assert.equal(await evaluate("records.filter(r=>r.method==='agent.request').length"), turnsBefore);
  await evaluate("byId('agent-form').requestSubmit()");
  await wait('replyBodies().at(-1) === ' + JSON.stringify(draft) + " && byId('agent-stop').hidden");

  await evaluate('choose(3)');
  await propose(selection.seq);
  await wait("lastCard().querySelector('.handoff-text') && lastCard().querySelector('.primary')");
  await evaluate("provider('agy'); compose('disconnect')");
  await wait("byId('agent-connect').textContent === 'Connect' && byId('mailbox-scope').value === 'project'");
  await evaluate('choose(3)');
  await wait("lastCard().querySelector('.preview-error') && !lastCard().querySelector('.primary')");
  await evaluate("lastCard().querySelector('.approval-actions button').click()");
  await wait("lastCard().textContent.includes('Request declined.') && byId('agent-stop').hidden");
  assert.equal(await evaluate("records.filter(r=>r.method==='agent.handoff.send').length"), sendCount);
}

export async function inspectRestartedArchive(evaluate, wait) {
  await wait("document.querySelector('#agent-provider option') && !document.querySelector('#app').inert");
  await evaluate("window.byId=id=>document.getElementById(id); window.calls=[]; const original=fetch.bind(window); window.fetch=(url,options)=>{calls.push(JSON.parse(options.body));return original(url,options)}; if(byId('agent-body').hidden) byId('agent-restore').click(); byId('agent-mailbox').open=true;");
  await wait("byId('mailbox-scope').value === 'project' && byId('mailbox-body').querySelectorAll('.shared-message').length===2");
  assert.equal(await evaluate("byId('agent-connect').textContent"), 'Connect');
  assert.deepEqual(await evaluate("calls.find(call=>call.method==='agent.handoff.history').params"), {});
  const texts = await evaluate("[...byId('mailbox-body').querySelectorAll('.shared-message pre')].map(pre=>pre.textContent)");
  assert.ok(texts.includes('author response · <b>λ</b>\nsource occurrence 1'));
  assert.ok(texts.includes('  Antigravity relay <b>λ</b>\nsecond line  '));
  await evaluate("byId('mailbox-body').querySelector('.shared-message button').click()");
  assert.match(await evaluate("byId('agent-input').value"), /SHA-256: [0-9a-f]{64}/);
  assert.equal(await evaluate("calls.some(call=>['agent.connect','agent.request','agent.handoff.send'].includes(call.method))"), false);
}
