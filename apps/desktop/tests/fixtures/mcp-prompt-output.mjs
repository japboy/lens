// Isolated publication sink for real-provider prompt acceptance; not production admission.
import { createInterface } from 'node:readline';
import { appendFileSync, readFileSync } from 'node:fs';
const send = value => process.stdout.write(JSON.stringify(value) + '\n');
const tool = {
  name: 'publish_html',
  description: "Publish a self-contained HTML artifact for display in Lens's Hero area. Supply html and the exact turn_id from the current lens_output_publication prompt metadata. Pass HTML itself, not a file path, URL, Markdown link, or fenced code block. Use expressive HTML and CSS, inline SVG and data-URL images. JavaScript, external resources and form submission are disabled. Publish at most one artifact per turn; an identical retry is accepted. HTML is registered directly with Lens without writing files or fetching network resources. Ordinary answers can remain text.",
  inputSchema: {type:'object', properties:{html:{type:'string'},turn_id:{type:'string'}}, required:['html','turn_id'],additionalProperties:false},
  annotations:{readOnlyHint:true,destructiveHint:false,idempotentHint:false,openWorldHint:false},
};
const published = new Map();
createInterface({input:process.stdin}).on('line', line => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (!('id' in message)) return;
  let result;
  if (message.method === 'initialize') result = {protocolVersion:message.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'lens-output-validation',version:'1'}};
  else if (message.method === 'ping') result = {};
  else if (message.method === 'tools/list') result = {tools:[tool]};
  else if (message.method === 'tools/call') {
    const {html, turn_id} = message.params.arguments ?? {};
    const active = readFileSync(process.env.LENS_PROMPT_TURN, 'utf8').trim();
    const valid = message.params.name === 'publish_html' && turn_id === active && typeof html === 'string' && html.trim() && Buffer.byteLength(html) <= 524288 && (!published.has(turn_id) || published.get(turn_id) === html);
    if (!valid) result = {isError:true,content:[{type:'text',text:'Invalid, stale, or conflicting publication'}]};
    else {
      published.set(turn_id, html);
      appendFileSync(process.env.LENS_PROMPT_TRACE, JSON.stringify({turn_id,html}) + '\n');
      result = {content:[{type:'text',text:JSON.stringify({accepted:true,publication_id:turn_id})}]};
    }
  }
  send(result === undefined ? {jsonrpc:'2.0',id:message.id,error:{code:-32601,message:'Unsupported validation method'}} : {jsonrpc:'2.0',id:message.id,result});
});
