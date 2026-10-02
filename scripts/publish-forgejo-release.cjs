#!/usr/bin/env node
// Publish a tagged archive and checksum through Forgejo's release-ID API.
const fs = require('node:fs');
const path = require('node:path');

async function publish() {
  const { TOKEN, SERVER, REPOSITORY, RELEASE_TAG } = process.env;
  if (!TOKEN || !SERVER || !REPOSITORY || !RELEASE_TAG) {
    throw new Error('Missing Forgejo release configuration');
  }
  const base = `${SERVER.replace(/\/$/, '')}/api/v1/repos/${REPOSITORY}`;
  async function request(route, options = {}) {
    const response = await fetch(`${base}${route}`, {
      ...options,
      headers: { Authorization: `token ${TOKEN}`, ...options.headers },
    });
    if (!response.ok) throw new Error(`Forgejo ${options.method || 'GET'} ${route}: HTTP ${response.status}`);
    return response.status === 204 ? null : response.json();
  }
  const body = fs.readFileSync('release-notes.md', 'utf8');
  if (!body.trim()) throw new Error('Empty release notes');
  const payload = { tag_name: RELEASE_TAG, name: RELEASE_TAG, body, draft: false, prerelease: false };
  const lookup = await fetch(`${base}/releases/tags/${RELEASE_TAG}`, {
    headers: { Authorization: `token ${TOKEN}` },
  });
  let release;
  if (lookup.status === 200) {
    const existing = await lookup.json();
    release = await request(`/releases/${existing.id}`, {
      method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(payload),
    });
  } else if (lookup.status === 404) {
    release = await request('/releases', {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(payload),
    });
  } else {
    throw new Error(`Forgejo release lookup: HTTP ${lookup.status}`);
  }
  const files = fs.readdirSync('dist').filter(name => /\.tar\.gz(?:\.sha256)?$/.test(name));
  if (files.length !== 2) throw new Error('Expected one archive and its checksum');
  for (const name of files) {
    const existing = (release.assets || []).find(asset => asset.name === name);
    if (existing) await request(`/releases/${release.id}/assets/${existing.id}`, { method: 'DELETE' });
    const form = new FormData();
    form.append('attachment', new Blob([fs.readFileSync(path.join('dist', name))]), name);
    await request(`/releases/${release.id}/assets?name=${encodeURIComponent(name)}`, {
      method: 'POST', body: form,
    });
  }
  const verified = await request(`/releases/${release.id}`);
  if (verified.tag_name !== RELEASE_TAG || verified.draft || verified.body !== body) {
    throw new Error('Published release metadata does not match the tag and notes');
  }
  for (const name of files) {
    const asset = verified.assets.find(asset => asset.name === name);
    if (!asset || asset.size !== fs.statSync(path.join('dist', name)).size) {
      throw new Error(`Published asset missing or wrong size: ${name}`);
    }
  }
  console.log(`Published ${verified.html_url}: ${files.join(', ')}`);
}

publish().catch(error => {
  console.error(error.message);
  process.exitCode = 1;
});
