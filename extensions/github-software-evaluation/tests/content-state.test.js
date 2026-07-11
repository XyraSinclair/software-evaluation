'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function makePort() {
  const messages = [];
  const disconnects = [];
  return {
    posted: [], disconnected: false,
    onMessage: { addListener(fn) { messages.push(fn); } },
    onDisconnect: { addListener(fn) { disconnects.push(fn); } },
    postMessage(value) { this.posted.push(value); },
    disconnect() { this.disconnected = true; for (const fn of disconnects) fn(); },
    emit(value) { for (const fn of messages) fn(value); },
  };
}

function harness() {
  let activeHost = null;
  let identity = { owner: 'first', repo: 'repo' };
  const ports = [];
  const views = [];
  const document = {
    body: { append(host) { activeHost = host; } },
    getElementById(id) { return activeHost && activeHost.id === id ? activeHost : null; },
    addEventListener() {},
    documentElement: {},
  };
  const panelApi = {
    createPanel(_document, onRetry) {
      const host = { id: 'seval-github-panel-host', remove() { if (activeHost === host) activeHost = null; } };
      return { host, update(view) { views.push(view); }, retry: onRetry };
    },
    model(result) { return result.metrics || []; },
  };
  const chrome = { runtime: { connect() { const port = makePort(); ports.push(port); return port; } } };
  const parser = { identityFromDocument() { return identity; } };
  const context = vm.createContext({
    __SEVAL_EXTENSION_TEST__: true,
    SEVAL_PANEL: panelApi,
    SEVAL_REPO_IDENTITY: parser,
    document,
    location: {},
    chrome,
    queueMicrotask() {},
    addEventListener() {},
    MutationObserver: class { observe() {} },
  });
  const source = fs.readFileSync(path.resolve(__dirname, '..', 'content.js'), 'utf8');
  vm.runInContext(source, context, { filename: 'content.js' });
  const controller = context.SEVAL_CONTENT_TEST.createController({
    document, location: {}, chrome, SEVAL_PANEL: panelApi, SEVAL_REPO_IDENTITY: parser,
  });
  return {
    controller, ports, views,
    setIdentity(next) { identity = next; },
    get activeHost() { return activeHost; },
  };
}

test('controller maps loading, bounded progress, ready/cache, and retryable error into pure panel views', () => {
  const h = harness();
  h.controller.reconcile();
  assert.equal(h.views.at(-1).state, 'loading');
  assert.deepEqual(h.ports[0].posted.map(message => ({ ...message })), [{ type: 'analyze', requestedIdentity: true, owner: 'first', repo: 'repo' }]);

  h.ports[0].emit({ type: 'progress', result: {
    status: 'analyzing', progress: { completed_instruments: 2, total_instruments: 5 },
    metrics: [{ id: 'metrics' }],
  } });
  assert.equal(h.views.at(-1).state, 'progress');
  assert.match(h.views.at(-1).message, /analyzing · 2\/5 instruments/);

  h.ports[0].emit({ type: 'result', result: {
    status: 'completed_partial',
    snapshot: { commit: '0123456789abcdef', cache: { hit: true, bundle_version: 'bundle-v7' } },
    metrics: [{ id: 'metrics', limitations: ['Generated files included.'] }],
  } });
  const ready = h.views.at(-1);
  assert.equal(ready.state, 'ready');
  assert.equal(ready.cache, true);
  assert.match(ready.snapshot, /commit 0123456789ab · analyzer bundle-v7/);
  assert.match(ready.message, /partial coverage/);
  assert.deepEqual([...ready.limitations], ['metrics: Generated files included.']);

  h.ports[0].emit({ type: 'error', message: 'Service unavailable.', retryable: true });
  assert.deepEqual(
    { state: h.views.at(-1).state, message: h.views.at(-1).message, retryable: h.views.at(-1).retryable },
    { state: 'error', message: 'Service unavailable.', retryable: true },
  );
});

test('generation changes disconnect old work and suppress its stale response', () => {
  const h = harness();
  h.controller.reconcile();
  const stalePort = h.ports[0];
  const viewsBeforeNavigation = h.views.length;

  h.setIdentity({ owner: 'second', repo: 'repo' });
  h.controller.stale();
  h.controller.reconcile();
  assert.equal(stalePort.disconnected, true);
  assert.equal(h.views.length, viewsBeforeNavigation + 1);
  assert.equal(h.views.at(-1).state, 'loading');

  stalePort.emit({ type: 'result', result: { status: 'completed', snapshot: { cache: { hit: true } }, metrics: [] } });
  assert.equal(h.views.at(-1).state, 'loading');

  h.ports[1].emit({ type: 'result', result: { status: 'completed', snapshot: { commit: 'abcdef', cache: { hit: false } }, metrics: [] } });
  assert.equal(h.views.at(-1).state, 'ready');
  assert.equal(h.views.at(-1).cache, false);
});

test('same-repository reconcile is idempotent and route-away removes host and work', () => {
  const h = harness();
  h.controller.reconcile();
  h.controller.reconcile();
  assert.equal(h.ports.length, 1);
  assert.ok(h.activeHost);

  h.setIdentity(null);
  h.controller.reconcile();
  assert.equal(h.activeHost, null);
  assert.equal(h.ports[0].disconnected, true);
});
