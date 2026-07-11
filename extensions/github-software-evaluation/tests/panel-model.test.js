'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function loadPanelModel() {
  const context = vm.createContext({});
  const source = fs.readFileSync(path.resolve(__dirname, '..', 'panel.js'), 'utf8');
  vm.runInContext(source, context, { filename: 'panel.js' });
  return context.SEVAL_PANEL.model;
}

const panelModel = loadPanelModel();

test('ready model exposes independently labelled observations with their basis and limitation', () => {
  const rows = panelModel({ metrics: [
    { id: 'metrics', status: 'complete', observations: { functions: 42 }, limitations: ['Generated files are included.'] },
    { id: 'dependencies', status: 'running', observations: { nodes: 9 }, limitations: [] },
  ] });

  const plainRows = rows.map(row => ({ ...row }));
  assert.deepEqual(plainRows.map(({ extent, ...row }) => row), [
    {
      id: 'metrics', label: 'Code shape', value: 42, basis: 'functions',
      caveat: 'Generated files are included.',
    },
    {
      id: 'dependencies', label: 'Dependency shape', value: 'running', basis: 'nodes',
      caveat: 'Independent instrument; no common scale.',
    },
  ]);
  assert.ok(plainRows.every(row => Number.isFinite(row.extent)));
});

test('null or absent observations remain visibly unavailable rather than becoming zero', () => {
  const cases = [
    { observations: { p90: null }, expectedBasis: 'No scalar observation reported' },
    { observations: {}, expectedBasis: 'No scalar observation reported' },
  ];
  for (const { observations, expectedBasis } of cases) {
    const [row] = panelModel({ metrics: [{ id: 'metrics', status: 'complete', observations }] });
    assert.equal(row.value, 'Not reported');
    assert.equal(row.extent, 0);
    assert.equal(row.basis, expectedBasis);
  }
});

test('panel model displays no more than six observations and never synthesizes a composite judgment', () => {
  const metrics = Array.from({ length: 8 }, (_, index) => ({
    id: `instrument_${index}`,
    status: 'complete',
    observations: { count: index + 1 },
    limitations: [`limitation ${index}`],
  }));
  const rows = panelModel({ metrics });
  assert.equal(rows.length, 6);
  assert.deepEqual(rows.map(row => row.id), metrics.slice(0, 6).map(item => item.id));
  for (const row of rows) {
    assert.doesNotMatch(row.id, /^(score|quality_score|verdict|grade|winner)$/i);
    assert.equal(typeof row.caveat, 'string');
    assert.notEqual(row.caveat.length, 0);
  }
});
