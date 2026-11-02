#!/usr/bin/env node
// Both conversations stream the same item ID. Only the reviewer requests a read.
import { createInterface } from 'node:readline';

const send = packet => process.stdout.write(JSON.stringify(packet) + '\n');
const roles = new Map();
let next = 0;
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
    reply({turn});
    send({method: 'turn/started', params: {...params, turn}});
    send({method: 'item/agentMessage/delta', params: {...params, itemId: 'shared', delta: role + ' response'}});
    if (role === 'reviewer') {
      send({id: 'read', method: 'item/tool/call', params: {...params, tool: 'verum_read', arguments: {path: 'experiment.vr'}}});
    }
  }
  if (method === 'turn/interrupt') {
    reply({});
    send({method: 'turn/completed', params: {threadId: p.threadId, turn: {id: p.turnId, status: 'interrupted'}}});
  }
  if (method === undefined && id === 'read') {
    send({method: 'turn/completed', params: {threadId: '2', turn: {id: 'T-2', status: 'completed'}}});
  }
}
