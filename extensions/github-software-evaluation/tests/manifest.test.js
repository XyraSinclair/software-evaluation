'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const extensionRoot = path.resolve(__dirname, '..');
const manifest = JSON.parse(fs.readFileSync(path.join(extensionRoot, 'manifest.json'), 'utf8'));

test('manifest grants only the two contract hosts and no extension permissions', () => {
  assert.equal(manifest.manifest_version, 3);
  assert.deepEqual(manifest.permissions, []);
  assert.deepEqual(
    new Set(manifest.host_permissions),
    new Set(['https://github.com/*', 'http://127.0.0.1:7077/*']),
  );
  assert.deepEqual(manifest.content_scripts, [{
    matches: ['https://github.com/*'],
    js: ['config.js', 'repo-identity.js', 'panel.js', 'content.js'],
    run_at: 'document_idle',
  }]);
});
