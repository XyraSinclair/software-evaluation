'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function loadIdentity() {
  const context = vm.createContext({});
  const source = fs.readFileSync(path.resolve(__dirname, '..', 'repo-identity.js'), 'utf8');
  vm.runInContext(source, context, { filename: 'repo-identity.js' });
  return context.SEVAL_REPO_IDENTITY;
}

function metadataDocument(nwo) {
  return {
    querySelector(selector) {
      assert.equal(selector, 'meta[name="octolytics-dimension-repository_nwo"]');
      return nwo === null ? null : { getAttribute: (name) => name === 'content' ? nwo : null };
    },
  };
}

function locationFor(pathname, overrides = {}) {
  return { protocol: 'https:', hostname: 'github.com', pathname, search: '', ...overrides };
}

const identity = loadIdentity();

test('parseNwo accepts the full owner/repository grammar boundaries', () => {
  const owner39 = `a${'b'.repeat(37)}z`;
  const repo100 = `r${'x'.repeat(99)}`;
  assert.deepEqual({ ...identity.parseNwo('Owner/repo.js') }, { owner: 'Owner', repo: 'repo.js' });
  assert.deepEqual({ ...identity.parseNwo(`${owner39}/${repo100}`) }, { owner: owner39, repo: repo100 });
});

test('parseNwo rejects reserved routes and identity-shaped injection inputs', () => {
  for (const value of [
    'settings/profile', 'About/repository', 'owner/.', 'owner/..', '-owner/repo',
    'owner-/repo', 'owner/repo/name', 'https://github.com/owner/repo', 'owner/repo?ref=main',
    'owner/repo#main', 'ownér/repo', `a${'b'.repeat(39)}/repo`, `owner/${'r'.repeat(101)}`,
  ]) {
    assert.equal(identity.parseNwo(value), null, value);
  }
});

test('document identity requires HTTPS GitHub route and matching GitHub metadata', () => {
  const doc = metadataDocument('Canonical/Repository');
  assert.deepEqual(
    { ...identity.identityFromDocument(doc, locationFor('/canonical/repository/issues/1')) },
    { owner: 'canonical', repo: 'repository' },
  );

  for (const [name, candidateDoc, location] of [
    ['metadata mismatch', metadataDocument('other/repository'), locationFor('/canonical/repository')],
    ['missing metadata', metadataDocument(null), locationFor('/canonical/repository')],
    ['reserved route', metadataDocument('settings/profile'), locationFor('/settings/profile')],
    ['one segment', metadataDocument('canonical/repository'), locationFor('/canonical')],
    ['query-bearing route', doc, locationFor('/canonical/repository', { search: '?ref=main' })],
    ['non-HTTPS', doc, locationFor('/canonical/repository', { protocol: 'http:' })],
    ['non-GitHub', doc, locationFor('/canonical/repository', { hostname: 'github.example' })],
  ]) {
    assert.equal(identity.identityFromDocument(candidateDoc, location), null, name);
  }
});
