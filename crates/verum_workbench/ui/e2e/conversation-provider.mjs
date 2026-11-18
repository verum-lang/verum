#!/usr/bin/env node
// Stream shared item IDs across roles and reuse one provider ID for successive execution reviews.
import { createInterface } from 'node:readline';

const send = packet => process.stdout.write(JSON.stringify(packet) + '\n');
const roles = new Map();
const turnCounts = new Map(), reviewedTurns = new Map();
let next = 0;
const execution = { threadId: '3', turnId: 'T-3' };
const path = 'experiment.vr', source = 'print("<b>λ</b>");\r\n';
const operations = [
  { tool: 'verum_run', arguments: { path, source, source_mode: 'script' } },
  { tool: 'verum_run', arguments: { path, source, timeout_ms: 0 } },
  { tool: 'verum_check', arguments: { path, source, source_mode: 'script' } },
];
function requestExecution() {
  send({id: 'operation', method: 'item/tool/call', params: {...execution, ...operations.shift()}});
}
const checks = ['fn main() -> Int { 42 }', 'fn main() -> Int { missing_receipt_value }'];
function requestCheck() {
  if (!checks.length) {
    send({id: 'inspect-order', method: 'item/tool/call', params: {
      threadId: '4', turnId: 'T-4', tool: 'verum_action_sequence', arguments: {},
    }});
    return;
  }
  send({id: 'check', method: 'item/tool/call', params: {
    threadId: '4', turnId: 'T-4', tool: 'verum_check', arguments: {path, source: checks.shift()},
  }});
}
for await (const line of createInterface({input: process.stdin})) {
  const {id, method, params: p} = JSON.parse(line);
  const reply = result => send({id, result});
  if (method === 'initialize') reply({});
  if (method === 'account/read') reply({account: {planType: 'fixture'}});
  if (method === 'model/list') reply({data: [{id: 'review-model'}]});
  if (method === 'thread/start') {
    const thread = String(++next);
    const role = p.dynamicTools.some(tool => tool.name === 'verum_propose') ? 'author' : 'reviewer';
    roles.set(thread, role);
    reply({thread: {id: thread}});
  }
  if (method === 'turn/start') {
    const threadId = p.threadId, role = roles.get(threadId);
    const number = (turnCounts.get(threadId) || 0) + 1; turnCounts.set(threadId, number);
    const turn = {id: 'T-' + threadId + (number > 1 ? '-' + number : ''), status: 'inProgress'};
    const params = {threadId, turnId: turn.id};
    send({method: 'turn/started', params: {...params, turn}});
    send({method: 'item/agentMessage/delta', params: {...params, itemId: 'shared', delta: role + ' response'}});
    send({method:'item/completed', params:{...params, item:{id:'shared', type:'agentMessage', text:role + ' response · <b>λ</b>\nsource occurrence ' + number}}});
    const prompt = p.input[0].text.split('\n')[0];
    if (prompt.startsWith('Propose handoff ') || prompt === 'Read inbox evidence') {
      const handoff = prompt.startsWith('Propose handoff ');
      reviewedTurns.set('transfer-review', params);
      send({id:'transfer-review',method:'item/tool/call',params:{...params,tool:handoff ? 'verum_handoff' : 'verum_inbox',
        arguments:handoff ? JSON.parse(prompt.slice('Propose handoff '.length)) : {}}});
      reply({turn}); continue;
    }
    if (role === 'reviewer') {
      // Provider prose and provider-shaped assertions have no standing in the host action check.
      send({method: 'studio/actionSequence', params: {...params, status: 'checked', succeeded: 999}});
      send({id: 'read', method: 'item/tool/call', params: {...params, tool: 'verum_read', arguments: {path: 'experiment.vr'}}});
    }
    if (threadId === execution.threadId) {
      requestExecution();
      // Age this live occurrence out of the host journal before the first scoped poll.
      for (let i = 0; i < 513; i++) send({method: 'item/completed', params: {threadId: '2', item: {id: String(i), type: 'reasoning'}}});
    }
    if (threadId === '4') requestCheck();
    reply({turn});
  }
  if (method === 'turn/interrupt') {
    reply({});
    send({method: 'turn/completed', params: {threadId: p.threadId, turn: {id: p.turnId, status: 'interrupted'}}});
  }
  if (method === undefined && id === 'read') {
    send({method: 'turn/completed', params: {threadId: '2', turn: {id: 'T-2', status: 'completed'}}});
  }
  if (method === undefined && id === 'operation' && operations.length) requestExecution();
  if (method === undefined && id === 'check') requestCheck();
  if (method === undefined && id === 'inspect-order') {
    send({method: 'turn/completed', params: {threadId: '4', turn: {id: 'T-4', status: 'completed'}}});
  }
  if (method === undefined && reviewedTurns.has(id)) {
    const {threadId, turnId} = reviewedTurns.get(id); reviewedTurns.delete(id);
    send({method:'turn/completed', params:{threadId, turn:{id:turnId,status:'completed'}}});
  }
}
