import test from 'node:test';
import assert from 'node:assert/strict';
import { providerCatalog } from '../src/providers.ts';

test('any host driver can expose its own context, model and review capabilities', () => {
  const driver = { id:'local-test', name:'Local Test', connectionId:null, executableAvailable:false,
    capabilities:{reviewedTools:false, workspaceContext:false, modelSelection:true} };
  assert.deepEqual(providerCatalog({providers:[driver]}), [driver]);
  for (const entries of [[driver, driver], [{...driver, capabilities:{...driver.capabilities, workspaceContext:'false'}}],
    [{...driver, connectionId:123}], [null]]) assert.throws(() => providerCatalog({providers:entries}));
});
