'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { TextEncoder } = require('node:util');

const extensionRoot = path.resolve(__dirname, '..');

function loadBackground({ fetchImpl, setTimeoutImpl } = {}) {
  const connectListeners = [];
  const context = vm.createContext({
    __SEVAL_EXTENSION_TEST__: true,
    URL,
    TextEncoder,
    AbortController,
    setTimeout: setTimeoutImpl || setTimeout,
    clearTimeout,
    fetch: fetchImpl || (async () => { throw new Error('network is forbidden in contract tests'); }),
    chrome: {
      runtime: {
        id: 'test-extension-id',
        onConnect: { addListener(listener) { connectListeners.push(listener); } },
      },
    },
  });
  context.importScripts = (...names) => {
    for (const name of names) {
      const source = fs.readFileSync(path.join(extensionRoot, name), 'utf8');
      vm.runInContext(source, context, { filename: name });
    }
  };
  const source = fs.readFileSync(path.join(extensionRoot, 'background.js'), 'utf8');
  vm.runInContext(source, context, { filename: 'background.js' });
  return { helpers: context.SEVAL_BACKGROUND_TEST, config: context.SEVAL_CONFIG };
}

const { helpers, config } = loadBackground();

test('analysis URLs cannot escape the fixed local API origin or analyses routes', () => {
  assert.equal(helpers.buildAnalysisUrl(), 'http://127.0.0.1:7077/v1/analyses');
  assert.equal(helpers.buildAnalysisUrl('job_123-ABC'), 'http://127.0.0.1:7077/v1/analyses/job_123-ABC');
  assert.equal(
    helpers.buildAnalysisUrl('../healthz?redirect=https://evil.example'),
    'http://127.0.0.1:7077/v1/analyses/..%2Fhealthz%3Fredirect%3Dhttps%3A%2F%2Fevil.example',
  );
});

test('poll delays clamp every numeric and malformed input to the configured interval', () => {
  const cases = [
    { input: -1, expected: config.pollMinDelayMs },
    { input: 0, expected: config.pollMinDelayMs },
    { input: config.pollMinDelayMs + 1, expected: config.pollMinDelayMs + 1 },
    { input: config.pollMaxDelayMs + 1, expected: config.pollMaxDelayMs },
    { input: Infinity, expected: config.pollMinDelayMs },
    { input: 'not-a-number', expected: config.pollMinDelayMs },
  ];
  for (const { input, expected } of cases) assert.equal(helpers.clampPollDelay(input), expected, String(input));
});

test('response normalization accepts compact data with null observations without inventing values', () => {
  const response = {
    analysis_id: 'job_1',
    status: 'completed_partial',
    result: {
      metrics: { p90: null, functions: 12 },
      observations: ['A bounded observation'],
      limitations: ['History was not analyzed'],
    },
  };
  assert.equal(helpers.normalizeResponse(response), response);
  assert.equal(response.result.metrics.p90, null);
});

test('response normalization rejects non-object envelopes and forbidden judgments at any depth', () => {
  for (const value of [null, [], 'completed', 7]) {
    assert.throws(() => helpers.normalizeResponse(value), /Invalid service response/);
  }
  for (const key of ['score', 'quality_score', 'verdict', 'grade', 'winner', 'QUALITY_SCORE']) {
    const response = { status: 'completed', result: { instruments: [{ observations: [{ [key]: 99 }] }] } };
    assert.equal(helpers.containsForbiddenJudgmentKeys(response), true, key);
    assert.throws(() => helpers.normalizeResponse(response), /Invalid service response/, key);
  }
  assert.equal(helpers.containsForbiddenJudgmentKeys({ observations: [{ scored_files: 3 }] }), false);
});

test('sender validation accepts only this extension on HTTPS github.com', () => {
  assert.equal(helpers.validSender({ id: 'test-extension-id', url: 'https://github.com/owner/repo' }), true);
  for (const sender of [
    { id: 'another-extension', url: 'https://github.com/owner/repo' },
    { id: 'test-extension-id', url: 'http://github.com/owner/repo' },
    { id: 'test-extension-id', url: 'https://github.example/owner/repo' },
    { id: 'test-extension-id', url: 'not a url' },
    null,
  ]) assert.equal(helpers.validSender(sender), false);
});

function liveServiceResponses(terminalState = 'completed_partial') {
  return [
    {
      analysis_id: 'job_live_contract',
      state: 'queued',
      created_at_ms: 1_752_000_000_000,
      updated_at_ms: 1_752_000_000_000,
    },
    {
      analysis_id: 'job_live_contract',
      state: 'resolving',
      created_at_ms: 1_752_000_000_000,
      updated_at_ms: 1_752_000_000_100,
    },
    {
      analysis_id: 'job_live_contract',
      state: terminalState,
      created_at_ms: 1_752_000_000_000,
      updated_at_ms: 1_752_000_000_200,
      result: {
        repository: {
          full_name: 'Canonical-Owner/Canonical.Repo',
          repository_id: 8675309,
          commit: '0123456789abcdef0123456789abcdef01234567',
          cached: true,
        },
        instruments: {
          metrics: {
            analyzer: 'seval-metrics-v2',
            state: 'complete',
            coverage: { files_analyzed: 7, files_total: 10, ratio: null },
            observations: { functions: 12, p90: null },
            limitations: ['Generated files are included.'],
          },
          dependencies: {
            analyzer: 'seval-dependencies-v1',
            state: 'failed',
            coverage: { manifests_analyzed: 1, manifests_total: 2 },
            observations: { direct: null },
            limitations: ['One manifest could not be parsed.'],
            error: 'bounded analyzer failure',
          },
        },
        completed_instruments: 1,
        failed_instruments: 1,
      },
    },
  ];
}

test('service adapter maps the live Rust terminal DTO to deterministic panel data without a composite score', () => {
  const terminal = liveServiceResponses()[2];
  assert.equal(
    typeof helpers.adaptServiceResponse,
    'function',
    'background must expose a pure service-to-panel adapter for contract testing',
  );

  const adapted = JSON.parse(JSON.stringify(helpers.adaptServiceResponse(terminal)));
  assert.equal(adapted.analysis_id, 'job_live_contract');
  assert.equal(adapted.status, 'completed_partial');
  assert.ok(adapted.result, 'adapter must place snapshot and metrics under result for the panel consumer');
  assert.deepEqual(adapted.result.snapshot, {
    full_name: 'Canonical-Owner/Canonical.Repo',
    repository_id: 8675309,
    commit: '0123456789abcdef0123456789abcdef01234567',
    cache: { hit: true },
  });
  assert.deepEqual(adapted.result.metrics.map(metric => metric.id), ['dependencies', 'metrics']);
  assert.deepEqual(adapted.result.metrics[0], {
    id: 'dependencies',
    analyzer: 'seval-dependencies-v1',
    status: 'failed',
    coverage: { manifests_analyzed: 1, manifests_total: 2 },
    observations: { direct: null },
    limitations: ['One manifest could not be parsed.'],
    error: 'bounded analyzer failure',
  });
  assert.deepEqual(adapted.result.metrics[1], {
    id: 'metrics',
    analyzer: 'seval-metrics-v2',
    status: 'complete',
    coverage: { files_analyzed: 7, files_total: 10, ratio: null },
    observations: { functions: 12, p90: null },
    limitations: ['Generated files are included.'],
  });
  assert.equal(helpers.containsForbiddenJudgmentKeys(adapted), false);
});

test('service adapter maps every live terminal state and keeps nonterminal state available to the panel', () => {
  for (const state of ['completed', 'completed_partial']) {
    const adapted = helpers.adaptServiceResponse(liveServiceResponses(state)[2]);
    assert.equal(adapted.status, state);
  }
  for (const response of liveServiceResponses().slice(0, 2)) {
    const adapted = helpers.adaptServiceResponse(response);
    assert.equal(adapted.status, response.state);
    assert.equal(adapted.analysis_id, response.analysis_id);
  }
});

test('analyze submits once, polls live state responses, and stops on a terminal Rust DTO', async () => {
  const responses = liveServiceResponses();
  const requests = [];
  const fetchImpl = async (url, options) => {
    requests.push({ url, method: options.method });
    const body = responses.shift();
    if (!body) throw new Error('analyze polled after the terminal state');
    const text = JSON.stringify(body);
    return {
      ok: true,
      headers: { get(name) { return name.toLowerCase() === 'content-length' ? String(text.length) : null; } },
      async text() { return text; },
    };
  };
  const immediateTimeout = callback => { queueMicrotask(callback); return 1; };
  const { helpers: isolatedHelpers } = loadBackground({ fetchImpl, setTimeoutImpl: immediateTimeout });
  const progress = [];

  const rawResult = await isolatedHelpers.analyze(
    { owner: 'canonical-owner', repo: 'canonical.repo' },
    message => progress.push(message.result),
  );
  const result = JSON.parse(JSON.stringify(rawResult));

  assert.deepEqual(requests, [
    { url: 'http://127.0.0.1:7077/v1/analyses', method: 'POST' },
    { url: 'http://127.0.0.1:7077/v1/analyses/job_live_contract', method: 'GET' },
    { url: 'http://127.0.0.1:7077/v1/analyses/job_live_contract', method: 'GET' },
  ]);
  assert.deepEqual(progress.map(item => item.status), ['queued', 'resolving']);
  assert.equal(result.status, 'completed_partial');
  assert.ok(result.result, 'analyze must return the adapted panel envelope, not the raw or flattened service DTO');
  assert.deepEqual(result.result.metrics.map(metric => metric.id), ['dependencies', 'metrics']);
  assert.equal(result.result.snapshot.commit, '0123456789abcdef0123456789abcdef01234567');
  assert.equal(result.result.snapshot.cache.hit, true);
  assert.equal(helpers.containsForbiddenJudgmentKeys(result), false);
});
