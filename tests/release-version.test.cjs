const assert = require('node:assert/strict');
const fs = require('node:fs');
const test = require('node:test');
const config = require('../.versionrc.js');

for (const filename of ['Cargo.toml', 'Cargo.lock']) {
  test(`release updater bumps ${filename} without restyling other bytes`, () => {
    const updater = config.bumpFiles.find(file => file.filename === filename).updater;
    const original = fs.readFileSync(filename, 'utf8');
    const current = fs.readFileSync('VERSION', 'utf8').trim();
    assert.equal(updater.readVersion(original), current);
    const changed = updater.writeVersion(original, '0.99.0');
    assert.equal(updater.readVersion(changed), '0.99.0');
    assert.equal(updater.writeVersion(changed, current), original);
    if (filename === 'Cargo.lock') {
      assert.equal(changed.match(/version = "0.99.0"/g).length, 2);
      assert.equal(changed.split('\n')[2], 'version = 4');
    }
  });
}

test('release updaters refuse missing and inconsistent Cargo versions', () => {
  const workspace = config.bumpFiles.find(file => file.filename === 'Cargo.toml').updater;
  const lock = config.bumpFiles.find(file => file.filename === 'Cargo.lock').updater;
  assert.throws(() => workspace.readVersion(''), /Missing workspace version/);
  assert.throws(() => lock.readVersion(''), /Missing tranche-cli lock entry/);
  assert.throws(() => lock.readVersion('name = "tranche-cli"\nversion = "1"\nname = "tranche-core"\nversion = "2"'), /disagree/);
});
