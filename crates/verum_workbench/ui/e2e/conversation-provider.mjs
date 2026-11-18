#!/usr/bin/env node
// Stream shared item IDs across roles and reuse one provider ID for successive execution reviews.
import { createInterface } from 'node:readline';

const send = packet => process.stdout.write(JSON.stringify(packet) + '\n');
const roles = new Map();
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
    send({method: 'turn/completed', params: {threadId: '4', turn: {id: 'T-4', status: 'completed'}}});
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
    const turn = {id: 'T-' + threadId, status: 'inProgress'};
    const params = {threadId, turnId: turn.id};
    send({method: 'turn/started', params: {...params, turn}});
    send({method: 'item/agentMessage/delta', params: {...params, itemId: 'shared', delta: role + ' response'}});
    if (role === 'reviewer') {
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
}
