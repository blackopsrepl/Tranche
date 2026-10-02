/* Tranche workbench. Pure query helpers also run under node:test. */
(function (root) {
  'use strict';
  const normalize = value => String(value ?? '').normalize('NFKD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
  const words = value => normalize(value).match(/[\p{L}\p{N}_-]+/gu) || [];
  // Optimal string alignment distance: insertion, deletion, substitution, transposition.
  function distance(a, b) {
    const matrix = Array.from({length: a.length + 1}, (_, i) => [i]);
    for (let j = 0; j <= b.length; j++) matrix[0][j] = j;
    for (let i = 1; i <= a.length; i++) {
      for (let j = 1; j <= b.length; j++) {
        matrix[i][j] = Math.min(matrix[i - 1][j] + 1, matrix[i][j - 1] + 1,
          matrix[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
        if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) {
          matrix[i][j] = Math.min(matrix[i][j], matrix[i - 2][j - 2] + 1);
        }
      }
    }
    return matrix[a.length][b.length];
  }
  function index(pr) {
    return {text: normalize(`${pr.title} ${pr.author} ${pr.number} ${pr.body}`),
      tokens: words(`${pr.title} ${pr.author} ${pr.body}`)};
  }
  function matches(pr, query, searchIndex, wordCache) {
    const terms = normalize(query).trim().split(/\s+/).filter(Boolean);
    if (!terms.length) return true;
    const haystack = searchIndex || index(pr);
    return terms.every(term => {
      if (/^#?\d+$/.test(term)) return String(pr.number) === term.replace(/^#/, '');
      if (term.startsWith('@')) return normalize(pr.author) === term.slice(1);
      if (haystack.text.includes(term)) return true;
      const tolerance = term.length >= 8 ? 2 : term.length >= 4 ? 1 : 0;
      return tolerance > 0 && haystack.tokens.some(word => {
        if (Math.abs(word.length - term.length) > tolerance) return false;
        const key = `${term}\0${word}`;
        if (wordCache?.has(key)) return wordCache.get(key);
        const matched = distance(term, word) <= tolerance;
        wordCache?.set(key, matched);
        return matched;
      });
    });
  }
  // Issue #3: security is the top-priority meta-category; its queue leads the nav.
  // Batch membership is browsed through the Batches view, not a queue.
  // Issue #8: parked is a queue whose field is a reason array, so queue
  // membership is non-emptiness — an empty array must never match.
  const queues = {security: 'security_priority', all: null, candidates: 'candidate', senior: 'senior', followup: 'followup', parked: 'parked', related: 'related', revised: 'head_moved', assigned: 'assignment'};
  const queueMatch = (pr, field) => {
    if (!field) return true;
    if (field === 'assignment') return typeof pr.assignment?.member_id === 'string' && pr.assignment.member_id.length > 0;
    const value = field === 'head_moved' ? pr.activity?.head_moved : pr[field];
    return Array.isArray(value) ? value.length > 0 : Boolean(value);
  };
  const PAGE_SIZE = 30;
  function select(rows, state = {}, indexes) {
    const field = queues[state.queue || 'all'];
    const wordCache = new Map(); // Repeated corpus vocabulary pays edit distance only once.
    const filtered = rows.filter(pr => queueMatch(pr, field) &&
      (!state.batch || (pr.batches || []).some(batch => batch.id === state.batch)) &&
      (!state.category || state.category === 'all' || pr.category === state.category ||
        (pr.categories || []).includes(state.category)) &&
      matches(pr, state.q || '', indexes?.get(pr.number), wordCache));
    filtered.sort((a, b) => {
      const riskSort = state.sort === 'risk';
      const idleSort = state.sort === 'idle';
      const av = riskSort ? a.risk : Date.parse(idleSort ? a.activity?.idle_since : a.created);
      const bv = riskSort ? b.risk : Date.parse(idleSort ? b.activity?.idle_since : b.created);
      const ak = typeof av === 'number' && Number.isFinite(av);
      const bk = typeof bv === 'number' && Number.isFinite(bv);
      if (ak !== bk) return ak ? -1 : 1;
      if (!ak) return a.number - b.number;
      return (state.sort === 'oldest' || idleSort ? av - bv : bv - av) || a.number - b.number;
    });
    const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
    const page = Math.max(1, Math.min(pages, Number.isSafeInteger(state.page) ? state.page : 1));
    return {items: filtered.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), total: filtered.length, page, pages};
  }
  function positiveInteger(value) {
    return /^[1-9]\d*$/.test(String(value)) && Number.isSafeInteger(Number(value)) ? Number(value) : null;
  }
  function parseState(search, categories = ['all']) {
    const params = new URLSearchParams(search);
    const queue = params.get('queue');
    const category = params.get('category');
    const sort = params.get('sort');
    return {q: params.get('q') || '', queue: Object.hasOwn(queues, queue) ? queue : 'all',
      category: categories.includes(category) ? category : 'all',
      sort: ['newest', 'oldest', 'risk', 'idle'].includes(sort) ? sort : 'newest',
      page: positiveInteger(params.get('page')) || 1, pr: positiveInteger(params.get('pr')),
      batch: params.get('batch') || null};
  }
  function serializeState(state) {
    const params = new URLSearchParams();
    if (state.q) params.set('q', state.q);
    if (state.queue && state.queue !== 'all') params.set('queue', state.queue);
    if (state.category && state.category !== 'all') params.set('category', state.category);
    if (state.sort && state.sort !== 'newest') params.set('sort', state.sort);
    if (state.page > 1) params.set('page', state.page);
    if (positiveInteger(state.pr)) params.set('pr', state.pr);
    if (state.batch) params.set('batch', state.batch);
    return params.size ? `?${params}` : '';
  }
  const api = {matches, index, select, parseState, serializeState};
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  root.TrancheWorkbench = api;
  if (typeof document !== 'undefined') boot();

  function boot() {
    // The payload lives at data/workbench.json beside this page (docs/data/ on
    // Pages). Only a same-origin read over http(s) is accepted: file:// and
    // other origins are not the report, and fetch would refuse them anyway.
    if (location.protocol === 'file:' || location.protocol === 'about:') { fail(); return; }
    fetch('data/workbench.json', {credentials: 'same-origin'})
      .then(response => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.json();
      })
      .then(start, fail);
  }
  function fail() {
    const failed = document.getElementById('load-failure');
    if (failed) failed.hidden = false;
  }
  function start(data) {
    const rows = data.prs;
    const byNumber = new Map(rows.map(pr => [pr.number, pr]));
    const assignmentsByNumber = new Map((data.assignments?.assignments || []).map(r => [r.number, r]));
    const parkedByNumber = new Map(((data.parked || {}).members || []).map(m => [m.number, m]));
    const indexes = new Map(rows.map(pr => [pr.number, index(pr)]));
    const batchById = new Map((data.batches || []).map(batch => [batch.id, batch]));
    const categories = ['all', ...Object.keys(data.categories)];
    const $ = id => document.getElementById(id);
    const dialog = $('pr-dialog');
    let state = parseState(location.search, categories);
    let listKey = '';
    let returnFocus = null;
    let timer;
    const formatCount = n => n.toLocaleString('en-US');
    const metric = (value, ceiling, probability = false) => typeof value === 'number' && Number.isFinite(value)
      ? `${value.toFixed(probability ? 2 : 1)}${ceiling ? ` / ${ceiling}` : ''}` : 'Unknown';
    function node(tag, text, className) {
      const element = document.createElement(tag);
      if (text !== undefined) element.textContent = text;
      if (className) element.className = className;
      return element;
    }
    function button(text, action, className) {
      const element = node('button', text, className);
      element.type = 'button';
      element.addEventListener('click', action);
      return element;
    }
    function categoryLabel(cat) { return data.categories[cat] || 'Unknown'; }
    function githubLink(number) {
      const link = node('a', 'Open pull request on GitHub ↗', 'github-link');
      // Never trust captured URLs, query strings or model text as link targets.
      if (positiveInteger(number)) link.href = `https://github.com/omacom/omarchy/pull/${number}`;
      link.target = '_blank'; link.rel = 'noopener noreferrer';
      return link;
    }
    function inspect(number) {
      if (!byNumber.has(number)) return;
      if (!dialog.open) returnFocus = document.activeElement;
      update({pr: number});
    }
    function memberButton(number) {
      const pr = byNumber.get(number);
      if (!pr) return node('span', `#${number} · outside captured corpus`);
      return button(`#${number} · ${pr.title}`, () => inspect(number), 'related-member');
    }
    function pairLine(pair, label) {
      const item = node('li', undefined, 'pair-diagnostic');
      const a = Array.isArray(pair) ? pair[0] : pair.a;
      const b = Array.isArray(pair) ? pair[1] : pair.b;
      item.append(memberButton(a), node('span', ' ↔ '), memberButton(b));
      const verdict = String(pair.verdict || 'unknown').replaceAll('_', ' ');
      const detail = Array.isArray(pair) ? 'Not compared' : `${pair.classification || 'unknown'} · ${verdict} · P(same): ${metric(pair.p_same, null, true)}`;
      item.append(node('p', `${label}: ${detail}`, 'small'));
      return item;
    }
    function relationships(pr, content) {
      content.append(node('h3', 'Related PRs'));
      const section = node('div', undefined, 'relationships');
      let found = false;
      const batchmates = (pr.batches || []).flatMap(batch =>
        (batchById.get(batch.id)?.members || []).filter(n => n !== pr.number));
      if (batchmates.length) {
        found = true;
        section.append(node('h4', 'Batch members — combine into one PR'));
        const members = node('div', undefined, 'related-members');
        batchmates.forEach(n => members.append(memberButton(n)));
        section.append(members);
        section.append(node('p', 'Jev judged these PRs to be the same change as this one; verify fix coverage before combining.', 'small'));
      }
      const groups = [
        ...data.groups.confirmed_groups.map(members => ({members, consistent: true})),
        ...data.groups.review_groups,
      ];
      for (const group of groups.filter(g => g.members.includes(pr.number))) {
        found = true;
        section.append(node('h4', group.consistent ? 'Model-consistent candidate group' : 'Group needing relationship review'));
        const members = node('div', undefined, 'related-members');
        group.members.forEach(n => members.append(memberButton(n)));
        section.append(members);
        const diagnostics = node('ul', undefined, 'diagnostics');
        for (const [key, label] of [['conflicting_pairs', 'Conflict'], ['uncertain_pairs', 'Uncertain relationship'], ['missing_pairs', 'Missing pair']]) {
          (group[key] || []).forEach(pair => diagnostics.append(pairLine(pair, label)));
        }
        section.append(diagnostics);
        if (group.unbound_evidence) section.append(node('p', 'Unbound legacy evidence — revisions cannot be checked.', 'small'));
      }
      const pairs = data.groups.uncertain_pairs.filter(pair => pair.a === pr.number || pair.b === pr.number);
      if (pairs.length) {
        found = true;
        section.append(node('h4', 'Pairs needing a human comparison'));
        const list = node('ul', undefined, 'diagnostics');
        pairs.forEach(pair => list.append(pairLine(pair, 'Relationship')));
        section.append(list);
      }
      section.append(node('p', found ? 'Model-suggested relationships, not verified duplicates. No survivor selected; compare code and fix coverage.' : 'No suggested relationships in this capture.', 'small'));
      content.append(section);
    }
    function renderDetail(pr) {
      $('detail-number').textContent = `PULL REQUEST #${pr.number}`;
      const content = $('detail-content');
      content.replaceChildren();
      const title = node('h2', pr.title); title.id = 'detail-title';
      content.append(title, githubLink(pr.number));
      const properties = node('dl', undefined, 'properties');
      const fields = [
        ['Author', `@${pr.author}`], ['Category', categoryLabel(pr.category)],
        ['Status', pr.draft ? 'Draft' : 'Not draft'], ['Created', pr.created || 'Unknown'],
        ['Head activity', pr.activity?.head_moved ? 'Head revised since last judgment; review waits for a new bound judgment' : pr.activity?.idle_basis === 'judgment' ? `Head unchanged since at least ${pr.activity.idle_since}` : pr.activity?.idle_basis === 'creation' ? `No head baseline; opened ${pr.created || 'unknown'}` : 'Unknown'],
        ['Thread updated', pr.activity?.thread_updated ? `${pr.activity.thread_updated} (may be bot activity; not used for priority)` : 'Unknown'],
        ['Model evidence', pr.freshness === 'current' ? 'Bound judgment · title and description only' : pr.freshness === 'unbound' ? 'Unbound legacy judgment · revisions not checked' : 'Unjudged or stale'],
        ['Model risk', metric(pr.risk, 4)], ['Security probability', metric(pr.security, null, true)],
        ['Finished form', metric(pr.finished, 3)], ['Review effort', metric(pr.effort, 3)],
        ['Fix probability', metric(pr.is_fix, null, true)], ['Diffstat', pr.diffstat || 'Unknown'],
      ];
      for (const [label, value] of fields) {
        const field = node('div'); field.append(node('dt', label), node('dd', value)); properties.append(field);
      }
      content.append(properties, node('h3', 'Description snippet'));
      content.append(node('p', pr.body || 'No description supplied.', 'body-snippet'));
      if (pr.body_truncated) content.append(node('p', 'Captured snippet is shortened. Read the full description on GitHub.', 'small'));
      if (pr.batches?.length) {
        content.append(node('h3', 'Pre-release batches'));
        const list = node('ul', undefined, 'diagnostics');
        for (const batch of pr.batches) {
          const item = node('li', undefined, 'pair-diagnostic');
          item.append(node('span', `${batch.id} — batch of ${batch.count} PRs merged together as one tranche`, 'batch-name'));
          item.append(node('p', 'Jev determined the composition: same-change groups inside the batch combine into one pull request. Batches are disjoint, so this PR appears in at most one batch.', 'small'));
          const open = button(`Browse batch ${batch.id} →`, () => {dialog.close(); update({batch: batch.id, page: 1, pr: null});}, 'related-member');
          item.append(open);
          list.append(item);
        }
        content.append(list);
        content.append(node('p', 'Model-suggested batching for the final cumulative PRs; ordering is not a merge approval.', 'small'));
      }
      const assignment = assignmentsByNumber.get(pr.number);
      if (assignment) {
        content.append(node('h3', 'Proposed skill assignment · synthetic simulation'));
        content.append(node('p', assignment.member_id ? `Proposed owner: ${assignment.member_id}` :
          `Unassigned: ${String(assignment.reason).replaceAll('_', ' ')}`, 'small'));
        content.append(node('p', `Required skills: ${(assignment.required_skills || []).join(', ') || (assignment.evidence_known ? 'None' : 'Unknown')}`, 'small'));
        content.append(node('p', 'Synthetic people, not real GitHub handles. AI-assisted proposal only; no approval, reservation or automatic mention.', 'small'));
      }
      const parkedEntry = parkedByNumber.get(pr.number);
      if (parkedEntry) {
        content.append(node('h3', 'Parked before batching'));
        const list = node('ul', undefined, 'diagnostics');
        const item = node('li', undefined, 'pair-diagnostic');
        item.append(node('span', parkedEntry.reasons.map(reason => reason.replaceAll('_', ' ')).join(' · '), 'batch-name'));
        item.append(node('p', parkedEntry.unblock, 'small'));
        list.append(item);
        content.append(list);
        content.append(node('p', 'A hold with a named unblock path, never a close: the PR re-enters batches automatically when the reason clears. Park does not remove a security-flagged PR from the security queue; closing stays a maintainer decision.', 'small'));
      }
      relationships(pr, content);
    }
    function copyPrompt(batchId) {
      const batch = batchById.get(batchId);
      if (!batch?.review_prompt) return;
      navigator.clipboard?.writeText(batch.review_prompt);
      const card = document.querySelector(`.batch-card[data-batch="${batchId}"] .copy-prompt`);
      if (card) {card.textContent = 'Copied ✓'; setTimeout(() => {card.textContent = 'Copy agent prompt';}, 1600);}
    }
    function renderBatchOverview() {
      const section = $('batch-overview');
      const batches = data.batches || [];
      $('batches-view').querySelector('span').textContent = formatCount(batches.length);
      if (!batches.length) {section.hidden = true; return;}
      const list = $('batch-list');
      if (!list.childElementCount) {
        for (const batch of batches) {
          const card = node('article', undefined, 'batch-card');
          card.dataset.batch = batch.id;
          const head = node('div', undefined, 'batch-head');
          const open = button(`${batch.id}`, () => update({batch: batch.id, queue: 'all', category: 'all', q: '', page: 1, pr: null}), 'batch-open');
          open.setAttribute('aria-label', `Open batch ${batch.id}`);
          head.append(open, node('span', batch.security_members > 0 ? `Security first · ${batch.security_members} security PR${batch.security_members > 1 ? 's' : ''}` : 'Merge group', 'tag' + (batch.security_members > 0 ? ' security' : '')));
          const copy = button('Copy agent prompt', () => copyPrompt(batch.id), 'copy-prompt');
          copy.setAttribute('aria-label', `Copy reviewer agent prompt for ${batch.id}`);
          head.append(copy);
          const meta = node('div', undefined, 'batch-meta');
          const risk = typeof batch.average_risk === 'number' && Number.isFinite(batch.average_risk) ? batch.average_risk.toFixed(1) : 'unknown';
          meta.append(node('span', `${batch.count} PRs → one combined PR`), node('span', `avg risk ${risk}`),
            node('span', batch.created ? `oldest ${batch.created.slice(0, 10)}` : ''));
          card.append(head, meta);
          const members = node('div', undefined, 'related-members');
          batch.members.forEach(n => members.append(memberButton(n)));
          card.append(members);
          const details = node('details', undefined, 'prompt-details');
          details.append(node('summary', 'Reviewer agent prompt'));
          const pre = node('pre', batch.review_prompt, 'prompt-text');
          details.append(pre);
          card.append(details);
          list.append(card);
        }
      }
      section.hidden = !state.batch && !batchView;
    }
    let batchView = false;
    function render() {
      const result = select(rows, state, indexes);
      state.page = result.page;
      if (state.pr && !byNumber.has(state.pr)) state.pr = null;
      if (state.batch && !batchById.has(state.batch)) state.batch = null;
      $('search').value = state.q;
      $('sort').value = state.sort;
      $('category').value = state.category;
      document.querySelectorAll('[data-queue]').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.queue === state.queue)));
      document.querySelectorAll('[data-category]').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.category === state.category)));
      $('batches-view').setAttribute('aria-pressed', String(batchView || !!state.batch));
      const start = result.total ? (result.page - 1) * PAGE_SIZE + 1 : 0;
      const end = Math.min(result.page * PAGE_SIZE, result.total);
      $('result-count').textContent = state.batch
        ? `${state.batch}: ${formatCount(result.total)} PRs to merge into one pull request`
        : `${formatCount(result.total)} results · ${formatCount(start)}–${formatCount(end)} of ${formatCount(result.total)} · ${formatCount(rows.length)} captured PRs`;
      $('empty').hidden = result.total !== 0;
      $('prev').disabled = result.page <= 1;
      $('next').disabled = result.page >= result.pages;
      $('page-label').textContent = `Page ${result.page} of ${result.pages}`;
      renderBatchOverview();
      const nextKey = JSON.stringify([state.q, state.queue, state.category, state.sort, state.page, state.batch]);
      if (nextKey !== listKey) {
        listKey = nextKey;
        const fragment = document.createDocumentFragment();
        for (const pr of result.items) {
          const item = node('li', undefined, 'pr-row');
          const opener = button('', () => inspect(pr.number), 'pr-open');
          opener.dataset.number = pr.number;
          opener.setAttribute('aria-label', `Inspect PR #${pr.number}: ${pr.title}`);
          const heading = node('span', undefined, 'pr-heading');
          heading.append(node('span', `#${pr.number}`, 'pr-number'), node('span', pr.title, 'pr-title'));
          const meta = node('span', undefined, 'pr-meta');
          meta.append(node('span', `@${pr.author}`, 'pr-author'), node('span', categoryLabel(pr.category)),
            node('span', pr.created ? pr.created.slice(0, 10) : 'Date unknown'));
          if (pr.assignment?.member_id) meta.append(node('span', `Proposed: ${pr.assignment.member_id} · synthetic`, 'tag'));
          if (pr.draft) meta.append(node('span', 'Draft', 'tag draft'));
          if (pr.security_priority) meta.append(node('span', 'Security first', 'tag security'));
          if (pr.activity?.head_moved) meta.append(node('span', 'Head revised', 'tag revised'));
          if (pr.related) meta.append(node('span', 'Related', 'tag'));
          for (const batch of pr.batches || []) {
            meta.append(node('span', `${batch.id} · batch of ${batch.count}`, 'tag batch'));
          }
          if (pr.freshness !== 'current') meta.append(node('span', pr.freshness === 'unbound' ? 'Unbound evidence' : 'Unjudged / stale', 'tag'));
          if ((pr.parked || []).length) meta.append(node('span', 'Parked', 'tag parked'));
          const info = node('span', undefined, 'pr-info'); info.append(heading, meta);
          const risk = node('span', metric(pr.risk, 4), `risk ${pr.risk == null ? 'unknown' : pr.risk >= 3 ? 'high' : 'known'}`);
          risk.setAttribute('aria-label', `Model risk ${metric(pr.risk, 4)}`);
          opener.append(info, risk);
          item.append(opener); fragment.append(item);
        }
        $('results').replaceChildren(fragment);
      }
      if (state.batch) {
        document.querySelector('.batch-overview').scrollIntoView({block: 'start', behavior: 'instant'});
      }
      if (state.pr) {
        if (dialog.dataset.number !== String(state.pr)) {
          renderDetail(byNumber.get(state.pr)); dialog.dataset.number = state.pr;
          if (dialog.open) {dialog.scrollTop = 0; $('close-detail').focus();}
        }
        if (!dialog.open) {returnFocus = document.activeElement; dialog.showModal();}
      } else if (dialog.open) {
        dialog.close(); dialog.dataset.number = '';
        const fallback = document.querySelector(`.pr-open[data-number="${returnFocus?.dataset?.number || ''}"]`);
        (returnFocus?.isConnected ? returnFocus : fallback || $('search')).focus();
      }
    }
    function writeURL(replace = false) {
      const url = location.pathname + serializeState(state) + location.hash;
      if (url !== location.pathname + location.search + location.hash) history[replace ? 'replaceState' : 'pushState'](null, '', url);
    }
    function update(change, replace = false) {
      clearTimeout(timer);
      state = {...state, ...change}; render(); writeURL(replace);
    }
    function reset() { batchView = false; update({q: '', queue: 'all', category: 'all', sort: 'newest', page: 1, pr: null, batch: null}); }
    for (const category of categories) {
      const count = category === 'all' ? rows.length
        : rows.filter(pr => pr.category === category || (pr.categories || []).includes(category)).length;
      const b = button('', () => update({category, page: 1, pr: null}), 'category-button');
      b.dataset.category = category;
      b.append(node('span', category === 'all' ? 'All categories' : categoryLabel(category)), node('span', formatCount(count), 'category-count'));
      $('category-buttons').append(b);
    }
    if (!document.querySelector('[data-queue="assigned"]')) {
      const anchor = document.querySelector('[data-queue="related"]');
      if (anchor) {
        const assignmentQueue = button('Proposed owners ', () => {}, anchor.className);
        assignmentQueue.dataset.queue = 'assigned';
        assignmentQueue.append(node('span', '0'));
        anchor.after(assignmentQueue);
      }
    }
    document.querySelectorAll('[data-queue]').forEach(b => {
      const field = queues[b.dataset.queue];
      b.querySelector('span').textContent = formatCount(rows.filter(pr => queueMatch(pr, field)).length);
      b.addEventListener('click', () => update({queue: b.dataset.queue, page: 1, pr: null}));
    });
    $('search').addEventListener('input', () => {
      clearTimeout(timer);
      timer = setTimeout(() => update({q: $('search').value, page: 1, pr: null}), 50);
    });
    $('batches-view').addEventListener('click', () => {
      batchView = !batchView;
      update({batch: null, queue: 'all', category: 'all', q: '', page: 1, pr: null});
    });
    $('sort').addEventListener('change', () => update({sort: $('sort').value, page: 1, pr: null}));
    $('category').addEventListener('change', () => update({category: $('category').value, page: 1, pr: null}));
    $('prev').addEventListener('click', () => update({page: state.page - 1, pr: null}));
    $('next').addEventListener('click', () => update({page: state.page + 1, pr: null}));
    $('reset').addEventListener('click', reset); $('empty-reset').addEventListener('click', reset);
    $('close-detail').addEventListener('click', () => update({pr: null}));
    dialog.addEventListener('cancel', event => {event.preventDefault(); update({pr: null});});
    window.addEventListener('popstate', () => {clearTimeout(timer); state = parseState(location.search, categories); render(); writeURL(true);});
    document.addEventListener('keydown', event => {
      const editing = /INPUT|TEXTAREA|SELECT/.test(event.target.tagName) || event.target.isContentEditable;
      if (!dialog.open && ((event.key === '/' && !editing && !event.ctrlKey && !event.metaKey) || ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k'))) {
        event.preventDefault(); $('search').focus(); $('search').select();
      }
    });
    render(); writeURL(true);
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);
