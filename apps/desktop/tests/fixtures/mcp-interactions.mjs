// Isolated, read-only MCP server for opt-in Agent acceptance validation.
import { createInterface } from 'node:readline';
import { appendFileSync } from 'node:fs';
const pending = new Map();
let id = 0;
const send = value => process.stdout.write(JSON.stringify(value) + '\n');
const ask = params => new Promise(resolve => {
  const requestId = ++id;
  pending.set(requestId, resolve);
  send({jsonrpc:'2.0', id:requestId, method:'elicitation/create', params});
});
const tools = ['fixture_read','fixture_form','fixture_url','fixture_approval'].map(name => ({
  name, description:`Lens acceptance fixture ${name}. Use only for the explicit validation request.`,
  inputSchema:{type:'object',properties:{},additionalProperties:false},
  annotations:{readOnlyHint:name !== "fixture_approval",destructiveHint:name === "fixture_approval",idempotentHint:true,openWorldHint:false}
}));
createInterface({input:process.stdin}).on('line', async line => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if ('result' in message || 'error' in message) {
    pending.get(message.id)?.(message.result ?? {action:'cancel'}); pending.delete(message.id); return;
  }
  if (!('id' in message)) return;
  const result = await (async () => {
    switch(message.method) {
      case 'initialize': return {protocolVersion:message.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'lens-fixture',version:'1.0.0'}};
      case 'ping': return {};
      case 'tools/list': return {tools};
      case 'tools/call': {
        if (process.env.LENS_FIXTURE_TRACE) appendFileSync(process.env.LENS_FIXTURE_TRACE, JSON.stringify({tool:message.params.name}) + '\n');
        let token = 'LENS_FIXTURE_READ_7B4A';
        if (message.params.name === 'fixture_approval') {
          const response = await ask({mode:'form',message:'Lens MCP approval fixture',requestedSchema:{type:'object',properties:{}},_meta:{codex_approval_kind:'mcp_tool_call'}});
          token = response.action === 'cancel' || response.action === 'decline' ? 'LENS_FIXTURE_APPROVAL_DENIED_7B4A' : 'LENS_FIXTURE_APPROVAL_ACCEPTED_7B4A';
        } else if (message.params.name === 'fixture_form') {
          const response = await ask({mode:'form',message:'Lens provider form fixture',requestedSchema:{type:'object',properties:{answer:{type:'string',minLength:1}},required:['answer']}});
          token = response.action === 'accept' && response.content?.answer === 'fixture-answer' ? 'LENS_FIXTURE_FORM_ACCEPTED_7B4A' : 'LENS_FIXTURE_FORM_DECLINED_7B4A';
        } else if(message.params.name === 'fixture_url') {
          const response = await ask({mode:'url',message:'Lens provider URL fixture',url:'http://127.0.0.1:9/lens-validation',elicitationId:'lens-url-fixture'});
          token = response.action === 'decline' ? 'LENS_FIXTURE_URL_DECLINED_7B4A' : 'LENS_FIXTURE_URL_OTHER_7B4A';
        } else if(message.params.name !== 'fixture_read') return {isError:true,content:[{type:'text',text:'Unknown fixture tool'}]};
        return {content:[{type:'text',text:token}]};
      }
      default: return undefined;
    }
  })();
  send(result === undefined ? {jsonrpc:'2.0',id:message.id,error:{code:-32601,message:'Unsupported fixture method'}} : {jsonrpc:'2.0',id:message.id,result});
});
