'use strict';
const el = id => document.getElementById(id);
let token = '', page = null, pending = null, version = 0;
async function query(next = false) {
  if (pending) pending.abort();
  const current = ++version;
  const controller = new AbortController();
  pending = controller;
  token = el('token').value || token;
  el('token').value = '';
  const kind = el('kind').value;
  const request = {scope: el('scope').value.trim(), limit: 25};
  if (el('id').value.trim()) request.id = el('id').value.trim();
  if (next && page) Object.assign(request, {generation: page.generation, after: page.next_after});
  el('next').disabled = true; el('state').textContent = 'Loading…';
  const timer = setTimeout(() => controller.abort(), 5000);
  try {
    const response = await fetch('/query', {method: 'POST', headers: {'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'}, body: JSON.stringify({kind, request}), signal: controller.signal, cache: 'no-store'});
    if (!response.ok) throw new Error(response.status === 401 ? 'Read token rejected.' : `Query unavailable (HTTP ${response.status}).`);
    const value = await response.json();
    if (current !== version) return;
    page = value;
    el('results').textContent = JSON.stringify(value.data || value.error || {}, null, 2);
    el('context').replaceChildren();
    for (const key of ['scope', 'generation', 'checkpoint', 'observation_age_seconds', 'observation_recent', 'coverage', 'confidence', 'valuation_time', 'execution_eligible']) {
      const term = document.createElement('dt'), detail = document.createElement('dd');
      term.textContent = key.replaceAll('_', ' '); detail.textContent = value[key] === null || value[key] === undefined ? 'Unavailable' : String(value[key]);
      el('context').append(term, detail);
    }
    el('state').textContent = value.error ? `Query stopped: ${value.error.code}. Inspect again for the current generation.` : 'Loaded committed evidence. Recent observation does not mean fresh valuation.';
    el('next').disabled = !value.next_after;
  } catch (error) {
    if (current !== version) return;
    page = null; el('context').replaceChildren(); el('results').textContent = 'No current result.';
    el('state').textContent = error.name === 'AbortError' ? 'Query cancelled or deadline exceeded.' : error.message;
  } finally {clearTimeout(timer); if (current === version) pending = null;}
}
el('investigation').addEventListener('submit', event => {event.preventDefault(); query();});
el('next').addEventListener('click', () => query(true));
for (const name of ['scope', 'kind', 'id']) el(name).addEventListener('input', () => {++version; pending?.abort(); pending = null; page = null; el('context').replaceChildren(); el('results').textContent = 'No current result.'; el('next').disabled = true; el('state').textContent = 'Inputs changed. Inspect again to load this scope and view.';});
el('disconnect').addEventListener('click', () => {++version; pending?.abort(); pending = null; token = ''; page = null; el('token').value = ''; el('context').replaceChildren(); el('results').textContent = 'No query yet.'; el('state').textContent = 'Connection cleared.'; el('next').disabled = true;});
