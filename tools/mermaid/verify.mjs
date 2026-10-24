import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { JSDOM } from 'jsdom';

const dom = new JSDOM('<!doctype html><html><body></body></html>');
globalThis.window = dom.window;
globalThis.document = dom.window.document;
const { default: mermaid } = await import('mermaid');
const { decodeEntities } = await import('./node_modules/mermaid/dist/chunks/mermaid.core/chunk-7S6XSTFO.mjs');
mermaid.initialize({ startOnLoad: false, securityLevel: 'strict' });
const fixture = JSON.parse(await readFile(process.argv[2], 'utf8'));
await mermaid.parse(fixture.text);
const diagram = await mermaid.mermaidAPI.getDiagramFromText(fixture.text);
assert.equal(diagram.type, 'mindmap');
const actual = [];
function visit(node, parent) {
  // Use Mermaid's real entity decoder and browser text interpretation, not a second encoder.
  const element = document.createElement('div');
  element.innerHTML = decodeEntities(node.descr);
  assert.equal(element.querySelector('*'), null, 'a label introduced markup');
  assert.equal(node.icon, undefined);
  assert.equal(node.class, undefined);
  actual.push({ id: node.nodeId, label: element.textContent, parent });
  for (const child of node.children) visit(child, node.nodeId);
}
visit(diagram.db.getMindmap(), null);
assert.deepEqual(actual, fixture.nodes);
console.log('Pinned Mermaid parser preserved synthetic labels, hierarchy and order.');
dom.window.close();
