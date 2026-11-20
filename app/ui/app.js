/* Safe Toolbox pages. The Rust side does the work; this file draws it with the Fluent Desktop kit. */
'use strict';
const T = window.__TAURI__;
const invoke = (cmd, args) => T.core.invoke(cmd, args);
const listen = (ev, f) => T.event.listen(ev, e => f(e.payload));

/* ---------------------------------------------------------------- helpers */
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const plural = (n, w, many) => `${n} ${n === 1 ? w : (many || w + 's')}`;
const KB = 1024, MB = KB * 1024, GB = MB * 1024;
const oneDec = v => (Math.round(v * 10) / 10).toString();
const size = b => !b ? '0 KB' : b >= GB ? (b / GB).toFixed(1) + ' GB' : b >= MB ? Math.round(b / MB) + ' MB' : Math.max(1, Math.round(b / KB)) + ' KB';
const gb = b => oneDec(b / GB) + ' GB';
const hm = d => d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
const day = d => d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });
function ago(iso) {
  if (!iso) return '';
  const d = new Date(iso), now = new Date(), start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Math.floor((start - new Date(d.getFullYear(), d.getMonth(), d.getDate())) / 864e5);
  if (days <= 0) return `today at ${hm(d)}`;
  if (days === 1) return `yesterday at ${hm(d)}`;
  if (days < 7) return `on ${d.toLocaleDateString('en-GB', { weekday: 'long' })} at ${hm(d)}`;
  return `on ${d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })} at ${hm(d)}`;
}
const G = {
  home: '\uE80F', updates: '\uE895', cleanup: '\uE74D', storage: '\uEDA2', health: '\uE95E', repair: '\uE90F', path: '\uE756',
  apps: '\uE71D', history: '\uE81C', settings: '\uE713', warn: '\uE7BA', info: '\uE946', check: '\uE73E', cancel: '\uE711',
  refresh: '\uE72C', folder: '\uE8B7', open: '\uE8A7', shield: '\uEA18', more: '\uE712', stop: '\uE71A', copy: '\uE8C8',
  undo: '\uE7A7', save: '\uE74E', download: '\uE896', bolt: '\uE945', video: '\uE714', play: '\uE768', link: '\uE71B',
  disc: '\uE958', zip: '\uF012', music: '\uE8D6', doc: '\uE8A5', chip: '\uE950', protect: '\uE72E', bell: '\uEA8F',
  task: '\uE823', palette: '\uE790', backdrop: '\uE771', clock: '\uE823', recycle: '\uE74D',
};
const ico = (g, cls = '') => `<i class="ico ${cls}" aria-hidden="true">${G[g] || g}</i>`;
const btn = (label, attrs = '', kind = '') => `<button class="fd-btn ${kind}" type="button" ${attrs}>${label}</button>`;
const iconBtn = (g, label, attrs = '') => `<button class="fd-btn" type="button" ${attrs}>${ico(g)}<span>${esc(label)}</span></button>`;
const badge = (text, tone = 'neutral') => `<span class="fd-badge fd-badge--${tone}">${esc(text)}</span>`;
const ring = (label = 'Working') => `<svg class="fd-ring fd-ring--20" viewBox="0 0 32 32" role="img" aria-label="${esc(label)}"><circle cx="16" cy="16" r="14"/></svg>`;
const progress = (v, label, cls = '') => v == null
  ? `<span class="fd-progress fd-progress--busy ${cls}" role="progressbar" aria-label="${esc(label)}"><span></span></span>`
  : `<span class="fd-progress ${cls}" role="progressbar" aria-valuenow="${Math.round(v * 100)}" aria-valuemin="0" aria-valuemax="100" aria-label="${esc(label)}" style="--v:${v}"><span></span></span>`;
const check = (on, attrs = '', label = '', disabled = false) =>
  `<label class="fd-check"><input type="checkbox" ${on ? 'checked' : ''} ${disabled ? 'disabled' : ''} ${attrs}><span></span>${label}</label>`;
const toggle = (on, attrs = '', label = '') => `<label class="fd-toggle"><input type="checkbox" role="switch" ${on ? 'checked' : ''} ${attrs}><span></span>${label}</label>`;
const ICONS = { info: 'info-mark', caution: 'warning-mark', critical: 'error', success: 'check' };
function infobar(kind, title, text = '', actions = '') {
  const k = kind === 'info' ? '' : `fd-infobar--${kind}`;
  return `<div class="fd-infobar ${k}" role="${kind === 'critical' ? 'alert' : 'status'}"><span class="fd-infobar__icon"><i class="fd-i fd-i--12 fd-i-${ICONS[kind]}" aria-hidden="true"></i></span><div class="fd-infobar__body"><strong>${esc(title)}</strong>${text ? `<span>${text}</span>` : ''}${actions}</div><button class="fd-btn fd-btn--subtle fd-btn--icon" type="button" aria-label="Close" data-fd-dismiss><i class="fd-i fd-i-close" aria-hidden="true"></i></button></div>`;
}
let menuId = 0;
/** A "More" button with a menu. items: [label, onclick-code-name, arg]. */
function more(items) {
  const id = 'm' + (++menuId);
  return `<button class="fd-btn fd-btn--icon" type="button" aria-label="More" title="More" data-fd-open="${id}">${ico('more')}</button>
  <div class="fd-menu" id="${id}" popover role="menu">${items.map(([label, act, arg]) => `<button class="fd-menu-item" type="button" role="menuitem" data-act="${act}" data-arg="${esc(arg ?? '')}"><span>${esc(label)}</span></button>`).join('')}</div>`;
}
/** A ComboBox; `name` is the setting it changes. */
function combo(name, options, value, label) {
  const id = 'c' + (++menuId), cur = options.find(o => o[0] === value) || options[0];
  return `<button class="fd-combo" type="button" aria-label="${esc(label)}" aria-haspopup="listbox" data-fd-open="${id}"><span>${esc(cur[1])}</span><i class="fd-i fd-i--12 fd-i-chevron-down" aria-hidden="true"></i></button>
  <div class="fd-menu" id="${id}" popover role="listbox">${options.map(([v, l]) => `<button class="fd-menu-item" type="button" role="option" aria-selected="${v === value}" data-combo="${name}" data-value="${esc(v)}"><span>${esc(l)}</span></button>`).join('')}</div>`;
}
const head = (title, sub, cmds = '') => `<header class="head"><div class="head__text"><h1 class="fd-title">${esc(title)}</h1>${sub ? `<p>${sub}</p>` : ''}</div><div class="cmds">${cmds}</div></header>`;

/* ---------------------------------------------------------------- dialog */
function ask(title, body, ok, cancel = 'Cancel', okKind = 'fd-btn--accent') {
  const d = $('#dlg');
  $('#dlg-t').textContent = title;
  $('#dlg-b').innerHTML = body;
  $('#dlg-a').innerHTML = (ok ? btn(esc(ok), 'data-r="1"', okKind) : '') + btn(esc(cancel), 'data-r="0"');
  return new Promise(res => {
    const done = v => { d.close(); res(v); };
    $$('#dlg-a button').forEach(b => b.onclick = () => done(b.dataset.r === '1'));
    d.oncancel = () => res(false);
    d.showModal();
    ($('#dlg-b input') || $('#dlg-a button')).focus();
  });
}
const tell = (title, body) => ask(title, body, null, 'OK');
const fail = e => tell('That didn\'t work', esc(String(e)));

/* ---------------------------------------------------------------- state */
const S = { page: 'home', settings: null, info: null, upd: null, cln: null, health: null, notes: {}, sel: {}, bars: {} };
const PAGES = [
  ['home', 'Home', 'home'], ['updates', 'Updates', 'updates'], ['cleanup', 'Clean up', 'cleanup'], ['storage', 'Big files', 'storage'],
  ['health', 'Health', 'health'], ['repair', 'Repair', 'repair'], ['path', 'PATH doctor', 'path'], ['apps', 'App list', 'apps'], ['history', 'History', 'history'],
];
const R = {};   // page renderers
const A = {};   // actions by data-act name

function drawNav() {
  const n = S.upd ? safeItems().length : 0;
  const item = ([id, label, g]) => `<button class="fd-nav-item" type="button" data-nav="${id}" ${S.page === id ? 'aria-current="page"' : ''} title="${esc(label)}">${ico(g)}<span>${esc(label)}</span>${id === 'updates' && n ? `<span class="fd-badge" aria-label="${n} safe updates">${n}</span>` : ''}</button>`;
  $('#nav').innerHTML = PAGES.map(item).join('') + '<div class="grow"></div>' + item(['settings', 'Settings', 'settings']);
}

async function go(page) {
  if (!R[page]) page = 'home';
  const changed = S.page !== page;
  S.page = page;
  drawNav();
  if (changed) $('#main').scrollTop = 0;
  await render();
  if (changed && S.settings) { S.settings.lastPage = page; invoke('set_settings', { settings: S.settings }).catch(() => {}); }
}

let renderQueued = false;
async function render(only) {
  if (only && only !== S.page) return;
  if (renderQueued) return;
  renderQueued = true;
  await Promise.resolve();
  renderQueued = false;
  const main = $('#main'), top = main.scrollTop, fid = document.activeElement?.dataset?.fid;
  const html = await R[S.page]();
  if (html == null) return;
  $('#page').innerHTML = html;
  main.scrollTop = top;
  window.FluentDesktop?.init($('#page'));
  if (fid) $(`[data-fid="${CSS.escape(fid)}"]`)?.focus();
}

document.addEventListener('click', e => {
  const nav = e.target.closest('[data-nav]');
  if (nav) { go(nav.dataset.nav); return; }
  const a = e.target.closest('[data-act]');
  if (a && A[a.dataset.act]) { e.preventDefault(); A[a.dataset.act](a.dataset.arg, a, e); }
  const c = e.target.closest('[data-combo]');
  if (c) setSetting(c.dataset.combo, c.dataset.value);
});
document.addEventListener('change', e => {
  const t = e.target.closest('[data-chg]');
  if (t && A[t.dataset.chg]) A[t.dataset.chg](t.dataset.arg, t, e);
});

/* ---------------------------------------------------------------- appearance */
const NEUTRAL = { accent: '#808080', 'accent-light-1': '#a8a8a8', 'accent-light-2': '#d6d6d6', 'accent-light-3': '#eeeeee', 'accent-dark-1': '#3a3a3a', 'accent-dark-2': '#2b2b2b', 'accent-dark-3': '#1c1c1c' };
function decl(r, dark) {
  const fill = dark ? r['accent-light-2'] : r['accent-dark-1'];
  return Object.keys(NEUTRAL).map(k => `--${k}:${r[k]}`).concat([`--accent-fill-secondary:${fill}e6`, `--accent-fill-tertiary:${fill}cc`, `--mica-wash:${r.accent}${dark ? '1f' : '17'}`]).join(';');
}
async function applyAppearance() {
  const s = S.settings, root = document.documentElement;
  if (s.theme === 'System') root.removeAttribute('data-theme'); else root.dataset.theme = s.theme.toLowerCase();
  root.dataset.backdrop = s.backdrop;
  S.accent = await invoke('accent').catch(() => null);
  const use = s.accent === 'Always' || (s.accent === 'FollowWindows' && S.accent?.windowsShows);
  const r = use && S.accent?.ramp?.accent ? S.accent.ramp : NEUTRAL;
  let el = $('#accent');
  if (!el) { el = document.createElement('style'); el.id = 'accent'; document.head.appendChild(el); }
  el.textContent = `:root{${decl(r, false)}}@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){${decl(r, true)}}}:root[data-theme="dark"]{${decl(r, true)}}:root[data-theme="light"]{${decl(r, false)}}`;
}
async function setSetting(name, value) {
  const s = S.settings;
  if (name === 'reminderThresholdMb') value = +value;
  if (typeof value === 'string' && (value === 'true' || value === 'false')) value = value === 'true';
  s[name] = value;
  try {
    const problem = await invoke('set_settings', { settings: s });
    if (problem) tell('Task Scheduler didn\'t take the change', esc(problem));
  } catch (e) { fail(e); }
  if (['theme', 'accent', 'backdrop'].includes(name)) await applyAppearance();
  render('settings');
}

/* ================================================================ Home */
R.home = async () => {
  const h = await invoke('home_state').catch(() => null);
  const u = S.upd, c = S.cln, sum = h?.summary;
  // Updates card
  let updNum = 'Not checked yet', updSub = 'Open Updates to check.', names = [];
  if (u?.checking) { updNum = 'Checking…'; updSub = 'Every source is checked at once. This takes about a minute.'; }
  else if (u?.checked) {
    names = safeItems().map(i => i.displayName);
    updNum = names.length ? plural(names.length, 'safe update') : 'Up to date';
    updSub = `Checked ${ago(u.lastChecked)}` + (leftOut().length ? `, ${leftOut().length} left out for safety` : '') + (wuItems().length ? `. ${wuItems().length} more waiting in Windows Update.` : '');
  } else if (sum?.updatesChecked) {
    names = sum.updates.map(x => x.name);
    updNum = names.length ? plural(names.length, 'safe update') : 'Up to date';
    updSub = `From the background check ${ago(sum.updatesWhen || sum.when)}`;
  }
  const list = names.slice(0, 4).map(n => `<span class="break">${esc(n)}</span>`).join('') + (names.length > 4 ? `<span class="muted small">and ${names.length - 4} more</span>` : '');
  // Clean up card
  let clnNum = 'Scanning…', clnSub = '';
  if (c?.scanned && !c.scanning) {
    const t = tickedTotal();
    clnNum = t >= 50 * MB ? size(t) + ' to free' : 'Nothing much to clean';
    clnSub = t >= 50 * MB ? 'Ticked by default, nothing you\'d miss.' : 'Ticked by default, nothing you\'d miss.';
  }
  const dr = h?.drive || c?.drive, used = dr ? 1 - dr.free / dr.total : null;
  // Health card
  const hl = healthLines();
  // Background card
  const bg = h?.background;
  const bgText = !bg ? '' : !bg.on ? 'Off. Turn it on to get a notification when safe updates are ready or there is a lot to clean.'
    : `On: updates ${bg.updates}, cleanup reminder ${bg.cleanup}.` + (sum?.when ? ` Last check ${ago(sum.when)}` + (sum.updatesChecked ? `: ${plural(sum.updates.length, 'safe update')}` : '') + (sum.cleanupChecked ? `, ${size(sum.cleanable)} to free` : '') + '.' : '');
  const recent = (h?.recent || []).map(r => `<div class="row">${ico(r.kind === 'Updates' ? 'updates' : 'cleanup', r.problems ? 't-caution' : 't-muted')}<span class="grow"><span>${r.kind === 'Updates' ? 'Update run' : 'Clean'} ${esc(ago(r.when))}</span><br><small class="muted">${esc(r.summary)}</small></span></div>`).join('')
    || '<span class="muted">Nothing yet. Update runs and cleans show up here.</span>';
  const card = (g, title, sub, body, foot) => `<section class="fd-card card"><div class="card__head">${ico(g, 'ico--20')}<div><h2 class="fd-body-strong">${esc(title)}</h2><p class="muted small">${esc(sub)}</p></div></div><div class="card__body">${body}</div>${foot ? `<div class="card__foot">${foot}</div>` : ''}</section>`;
  const tool = (g, title, what, page) => `<button class="tool" type="button" data-nav="${page}">${ico(g, 'ico--20')}<span><span class="fd-body-strong">${esc(title)}</span><small>${esc(what)}</small></span></button>`;
  return head('Home', 'Your PC at a glance. Each card opens its tool.') + `<div class="cards">
    ${card('updates', 'Updates', 'Apps, Windows, drivers and dev tools', `<div class="big">${esc(updNum)}</div><p class="muted">${esc(updSub)}</p><div class="stack">${list}</div>`, btn('Review updates', 'data-nav="updates"', 'fd-btn--accent'))}
    ${card('cleanup', 'Clean up', 'Caches and leftovers, checked by the guard', `<div class="big">${esc(clnNum)}</div>${clnSub ? `<p class="muted">${esc(clnSub)}</p>` : ''}${dr ? `<div class="meter">${progress(used, 'System drive space used', used > .9 ? 'fd-progress--error' : '')}<span class="muted small">${S.info?.drive || 'C:'} ${gb(dr.free)} free of ${gb(dr.total)}</span></div>` : ''}`, btn('Clean up', 'data-nav="cleanup"'))}
    ${card('health', 'Health', 'Crashes, drive, battery and security', hl, btn('Open Health', 'data-nav="health"'))}
    ${card('bell', 'Background checks', 'Notifications when something needs you', `<p>${esc(bgText)}</p>${bg?.problem ? `<p class="t-caution">${esc(bg.problem)}</p>` : ''}`, btn('Change in Settings', 'data-nav="settings"'))}
    ${card('history', 'Recent activity', 'Every update run and clean, from the logs', `<div class="stack stack--12">${recent}</div>`, btn('Open History', 'data-nav="history"'))}
    ${card('repair', 'More tools', 'Fixes and checks you run when you need them', `<div class="tools">
      ${tool('storage', 'Big files', 'Your largest files, to decide what to keep.', 'storage')}
      ${tool('repair', 'Repair', 'Windows file repair, network reset, DNS flush, Explorer restart.', 'repair')}
      ${tool('path', 'PATH doctor', 'Dead or repeated PATH entries and which python or node runs.', 'path')}
      ${tool('apps', 'App list', 'Save what\'s installed, to set up a new PC fast.', 'apps')}</div>`, '')}
  </div>`;
};

function healthLines() {
  const h = S.health;
  if (!h) { loadHealth(); return `<div class="loading">${ring('Reading')}<span>Reading Windows' records…</span></div>`; }
  if (h.error) return '<p class="muted">Windows\' records couldn\'t be read. Open Health to try again.</p>';
  const line = (g, tone, text) => `<div class="row">${ico(g, tone)}<span class="grow">${esc(text)}</span></div>`;
  const recent = h.crashes.filter(c => Date.now() - new Date(c.when) < 30 * 864e5).length;
  const out = [recent ? line('warn', 't-caution', `${plural(recent, 'crash', 'crashes')} in the last 30 days`) : line('check', 't-success', 'No crashes in the last 30 days')];
  h.drives.forEach(d => out.push(d.health === 'Healthy' ? line('check', 't-success', `${d.name} is healthy`) : line('warn', 't-critical', `${d.name}: ${d.health}`)));
  h.batteries.forEach(b => out.push(line(b.percent >= 80 ? 'check' : 'warn', b.percent >= 80 ? 't-success' : 't-caution', `Battery holds ${b.percent}% of its new charge`)));
  const av = h.antivirus.find(a => a.on);
  out.push(av ? line('shield', av.upToDate ? 't-success' : 't-caution', `${av.name} is on${av.upToDate ? '' : ', out of date'}`) : line('shield', 't-critical', 'No antivirus is on'));
  if (h.restartPending) out.push(line('warn', 't-caution', 'Windows is waiting for a restart'));
  return `<div class="stack stack--12">${out.join('')}</div>`;
}
let healthLoading = null;
function loadHealth(force) {
  if (healthLoading && !force) return healthLoading;
  healthLoading = invoke('health_snapshot').then(h => { S.health = h; }, e => { S.health = { error: String(e) }; }).finally(() => { healthLoading = null; render('home'); render('health'); });
  return healthLoading;
}

/* ================================================================ Updates */
const isPending = i => i.selectable && !i.hidden && i.status !== 'Done';
const safeItems = () => (S.upd?.items || []).filter(isPending);
const leftOut = () => (S.upd?.items || []).filter(i => i.hidden);
const selected = () => safeItems().filter(i => i.selected);
const STATUS = { Queued: 'Waiting', Running: 'Installing', Done: 'Updated', Failed: 'Failed', Skipped: 'Kept back' };
const BADGE_TONE = { 'Major': 'caution', 'Held': 'caution', 'Old driver': 'critical', 'Same family': 'success' };
const target = i => i.targetVersion || i.available;
const wuItems = () => (S.upd?.items || []).filter(i => i.source === 'Windows' && !i.hidden);
const ver = i => i.source === 'Windows' ? (i.available || '') : `${i.current || '?'}  →  ${target(i)}`;
const wuButtons = items => btn(`${ico('updates')}<span>Open Windows Update</span>`, 'data-act="open" data-arg="ms-settings:windowsupdate-action" title="Opens Windows Update and checks for updates"')
  + (items.some(i => i.kind === 'Driver') ? btn('Optional updates', 'data-act="open" data-arg="ms-settings:windowsupdate-optionalupdates" title="Drivers are listed under Optional updates"') : '');
function defaultWhy(i) {
  if (i.note) return i.note;
  if (i.kind === 'WindowsUpdate') return 'An update offered by Windows Update.';
  if (i.kind === 'Driver') return 'An optional driver offered by Windows Update.';
  return i.recommended ? 'Picked: a routine update within the same version family.' : 'Not picked by default.';
}

R.updates = () => {
  const u = S.upd;
  if (!u) return '';
  const safe = safeItems(), sel = selected(), hidden = leftOut();
  let summary = 'Not checked yet';
  if (u.checking) summary = `Checking… ${u.total - u.pending} of ${u.total} done`;
  else if (u.checked) summary = safe.length ? `${plural(safe.length, 'safe update')}${hidden.length ? `, ${hidden.length} left out for safety` : ''}` : 'Everything safe is up to date' + (hidden.length ? `, ${hidden.length} left out for safety` : '');
  const run = u.running;
  const cmds = [
    btn(`${ico('refresh')}<span>Check again</span>`, `data-act="updCheck" ${u.checking || run ? 'disabled' : ''}`),
    toggle(u.includeMajors, `data-chg="updMajors" ${run ? 'disabled' : ''}`, 'Include major versions'),
    run ? btn(`${ico('stop')}<span>${S.stopping ? 'Stopping after this update…' : 'Stop after this update'}</span>`, `data-act="updStop" ${S.stopping ? 'disabled' : ''} title="Stop after the update that is installing now"`)
      : btn(sel.length ? `Update selected (${sel.length})` : 'Update selected', `data-act="updRun" ${sel.length && !u.checking ? '' : 'disabled'}`, 'fd-btn--accent'),
    more([['Roll back the last Python update', 'updRollback'], ['Open the NVIDIA app', 'updNvidia'], ['Windows Update settings', 'open', 'ms-settings:windowsupdate'], ['Open the update logs', 'tool', 'update-logs']]),
  ].join('');
  const notes = [];
  if (!u.isAdmin && sel.some(i => i.needsAdmin) && !run) notes.push(infobar('info', 'One admin prompt', 'Windows will ask for admin once, for the updates that need it. Everything else runs as you.'));
  if (u.checked && u.warnings.length && !run) notes.push(infobar('caution', 'Before you update', u.warnings.map(esc).join(' ')));
  if (S.bars.updates) notes.push(S.bars.updates);
  const groups = u.sources.filter(s => !s.absent && (s.done || u.checking)).map(s => updGroup(s, u)).join('');
  const hiddenHtml = hidden.length ? `<details class="fd-expander" style="margin-top:12px"><summary><span>Left out for safety (${hidden.length}). These aren't listed above, so they can't be ticked by accident.</span><span class="fd-expander__chev"><i class="fd-i fd-i--12 fd-i-chevron-down" aria-hidden="true"></i></span></summary><div class="fd-expander__content"><div class="stack stack--12">${hidden.map(i => `<div class="row row--top"><span class="grow"><span class="fd-body-strong">${esc(i.displayName)}</span> <span class="muted mono">${esc(ver(i))}</span><br><small class="muted">${esc(i.note || i.unsafeReason || i.badge || '')}</small></span>${i.ignoredByUser ? btn('Show again', `data-act="updShow" data-arg="${esc(i.key)}"`) : ''}</div>`).join('')}</div></div></details>` : '';
  const body = u.checked || u.checking
    ? `<div class="stack stack--12">${groups || `<div class="loading">${ring()}<span>Checking apps, Windows, drivers and dev tools…</span></div>`}</div>${hiddenHtml}`
    : `<div class="empty">${ico('info')}<span>Press Check again to look for updates.</span></div>`;
  return head('Updates', 'Apps, Windows, drivers and your dev tools, checked in one place. Only updates that are safe to install are listed; anything that could break something is left out, and the end of the list says why.', cmds)
    + `<div class="status"><span class="fd-subtitle">${esc(summary)}</span><span class="muted">${u.lastChecked ? `Checked ${esc(ago(u.lastChecked))}` : ''}${u.lastUpdated ? `, last updated ${esc(ago(u.lastUpdated))}` : ''}</span>${run && S.updStatus ? `<span class="muted">${esc(S.updStatus)}</span>` : ''}</div>`
    + (run || u.checking ? `<div style="margin:-4px 0 12px">${progress(u.checking ? (u.total - u.pending) / u.total : null, 'Progress')}</div>` : '')
    + `<div class="notes">${notes.join('')}</div><div class="split"><div>${body}</div><aside class="fd-card side" aria-label="Details">${updDetails()}</aside></div>`;
};

function updGroup(s, u) {
  const items = u.items.filter(i => i.source === s.source && (!i.hidden || i.status === 'Done'));
  const pend = items.filter(isPending), hiddenN = u.items.filter(i => i.source === s.source && i.hidden).length, wu = s.source === 'Windows';
  let status;
  if (!s.done) status = 'Checking…';
  else if (s.failed) status = s.message || 'The check failed.';
  else if (!items.length) status = hiddenN ? `Nothing safe to update. ${hiddenN} left out for safety, listed at the end.` : (s.message || 'Up to date');
  else if (wu) status = `${plural(items.length, 'update')} worth installing. Windows Update installs ${items.length === 1 ? 'it' : 'them'}, not this app.`;
  else status = plural(items.length, 'update') + (s.message ? ` · ${s.message}` : '');
  const allOn = pend.length && pend.every(i => i.selected);
  const rows = items.map(i => {
    const st = i.status !== 'Idle' ? (i.result || STATUS[i.status]) : '';
    const tone = i.status === 'Failed' ? 't-critical' : i.status === 'Done' ? 't-success' : 't-muted';
    const icon = i.status === 'Done' ? ico('check', 't-success') : i.status === 'Failed' ? ico('warn', 't-critical') : i.status === 'Running' ? ring('Installing') : '';
    return `<div class="item" role="option" aria-selected="${S.sel.updates === i.key}">
      ${i.selectable ? check(i.selected && isPending(i), `data-chg="updTick" data-arg="${esc(i.key)}" data-fid="t-${esc(i.key)}" aria-label="Update ${esc(i.displayName)}"`, '', !isPending(i) || u.running) : '<span class="slot" aria-hidden="true"></span>'}
      <button class="item__main" type="button" data-act="updPick" data-arg="${esc(i.key)}"><span class="break">${esc(i.displayName)}</span><small class="mono">${esc(ver(i))}</small></button>
      <span class="item__end">${i.badge ? badge(i.badge, BADGE_TONE[i.badge] || 'neutral') : ''}${st ? `<span class="small ${tone}">${esc(st)}</span>` : ''}${icon}</span></div>`;
  }).join('');
  return `<section class="fd-card group"><div class="group__head">${pend.length ? check(allOn, `data-chg="updGroup" data-arg="${s.source}" aria-label="Select every update in ${esc(s.title)}"`, '', u.running) : '<span class="slot" aria-hidden="true"></span>'}
    <div class="group__title"><h2 class="fd-body-strong">${esc(s.title)}</h2><small class="${s.failed ? 't-caution' : ''}">${esc(status)}</small></div>${wu && items.length ? `<span class="cmds">${wuButtons(items)}</span>` : ''}${!s.done ? ring('Checking') : ''}</div>${rows}</section>`;
}

function updDetails() {
  const i = (S.upd?.items || []).find(x => x.key === S.sel.updates);
  if (!i) return `<h2 class="fd-subtitle">Details</h2><p class="muted" style="margin-top:8px">Pick an update to see what it is, why it is or isn't selected, what's new, and the exact command it runs.</p>`;
  const wu = i.source === 'Windows';
  const f = wu ? [['Download', i.available || 'Unknown']] : [['Installed', i.current || 'Unknown'], ['Newest', i.available]];
  if (i.targetVersion) f.push(['Will install', i.targetVersion]);
  if (i.driverDate) f.push(['Driver date', day(new Date(i.driverDate))]);
  if (i.detail) f.push([i.kind === 'WindowsUpdate' ? 'Article' : 'About', i.detail]);
  if (!wu) f.push(['Runs', `<span class="mono">${esc(i.command)}</span>`]);
  if (i.needsAdmin) f.push(['Admin', 'Yes, in the one admin step of the run']);
  if (i.result) f.push(['Result', i.result]);
  const notes = S.notes[i.key];
  if (notes === undefined) { S.notes[i.key] = null; invoke('upd_notes', { key: i.key }).then(n => { S.notes[i.key] = n; render('updates'); }); }
  const links = notes == null ? '<span class="muted">Looking up the release notes…</span>' : notes.length ? notes.map(l => `<a class="fd-link" data-act="open" data-arg="${esc(l.url)}">${esc(l.label)}</a>`).join('<br>') : '<span class="muted">No release notes found.</span>';
  return `<h2 class="fd-subtitle break">${esc(i.displayName)}</h2><p class="muted" style="margin-top:4px">${esc(defaultWhy(i))}</p>
    <dl class="facts">${f.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${k === 'Runs' ? v : esc(v)}</dd>`).join('')}<dt>What's new</dt><dd>${links}</dd></dl>
    ${wu && !i.hidden ? `<div class="cmds">${wuButtons([i])}</div>` : ''}
    ${i.output?.length ? `<h3 class="fd-body-strong">Last lines of output</h3><pre class="out mono" style="min-height:0;margin-top:8px">${esc(i.output.join('\n'))}</pre>` : ''}
    ${isPending(i) ? `<div class="stack" style="margin-top:16px">${btn('Skip this version', `data-act="updSkip" data-arg="${esc(i.key)}" title="Hides ${esc(target(i))} only. The next version after it is listed again."`)}<small class="muted">Hides ${esc(target(i))} only. The next version after it is listed again.</small>
    ${btn('Don\'t show this update', `data-act="updHide" data-arg="${esc(i.key)}"`)}<small class="muted">Hides every version of it until you choose Show again in the left-out list.</small></div>` : ''}`;
}

A.updCheck = () => { S.bars.updates = ''; invoke('upd_check'); };
A.updMajors = (_, el) => invoke('upd_majors', { on: el.checked });
A.updTick = (key, el) => invoke('upd_select', { keys: [key], on: el.checked });
A.updGroup = (src, el) => invoke('upd_select', { keys: safeItems().filter(i => i.source === src).map(i => i.key), on: el.checked });
A.updPick = key => { S.sel.updates = key; render('updates'); };
A.updSkip = key => invoke('upd_skip', { key });
A.updHide = key => invoke('upd_ignore', { key, ignored: true });
A.updShow = key => invoke('upd_ignore', { key, ignored: false });
A.updStop = () => { S.stopping = true; invoke('upd_stop'); render('updates'); };
A.updNvidia = async () => { if (!await invoke('upd_open_nvidia')) tell('NVIDIA app not found', 'It isn\'t in the usual folder. Open it from the Start menu.'); };
A.updRun = async () => {
  const keys = selected().map(i => i.key);
  const open = await invoke('upd_running_apps', { keys }).catch(() => []);
  if (open.length && !await ask(open.length === 1 ? 'One of these apps is open' : 'Some of these apps are open',
    `${esc(open.join(', '))} ${open.length === 1 ? 'is' : 'are'} running. The installer may close ${open.length === 1 ? 'it' : 'them'}, so save your work first.`, 'Update anyway', 'Wait')) return;
  S.stopping = false; S.updStatus = 'Starting…'; S.bars.updates = '';
  invoke('upd_run', { keys }).catch(fail);
};
A.updRollback = async () => {
  if (!S.upd?.canRollback) return tell('Nothing to roll back', 'No Python update has run from here yet, so there is no snapshot.');
  if (!await ask('Roll back the last Python update?', 'Every package in your user site goes back to the version it had before the last update run.', 'Roll back')) return;
  S.bars.updates = infobar('info', 'Rolling back Python packages…'); render('updates');
  invoke('upd_rollback').then(m => { S.bars.updates = infobar('success', m); }, e => { S.bars.updates = infobar('critical', 'The rollback didn\'t finish', esc(e)); }).finally(() => render('updates'));
};

/* ================================================================ Clean up */
const tickedRules = () => (S.cln?.rules || []).filter(r => r.canClean && r.ticked);
const tickedTotal = () => tickedRules().reduce((a, r) => a + r.total, 0);
const GROUP_ICON = { Leftovers: 'cleanup', Apps: 'apps', YourCall: 'folder', Windows: 'shield' };

R.cleanup = () => {
  const c = S.cln;
  if (!c) return '';
  const dr = c.drive, used = dr ? 1 - dr.free / dr.total : 0;
  const sel = tickedRules(), total = tickedTotal(), unknown = sel.some(r => r.unknownSize);
  const busy = c.scanning || c.cleaning;
  const cmds = [
    btn(`${ico('refresh')}<span>Rescan</span>`, `data-act="clnScan" ${busy ? 'disabled' : ''}`),
    btn(`${ico('protect')}<span>Protect a folder</span>`, `data-act="clnProtect" ${busy ? 'disabled' : ''}`),
    c.cleaning ? btn(`${ico('stop')}<span>${S.clnStopping ? 'Stopping…' : 'Stop'}</span>`, `data-act="clnStop" title="Stop cleaning after the item in progress" ${S.clnStopping ? 'disabled' : ''}`)
      : btn(sel.length ? `Clean selected · ${size(total)}${unknown ? '+' : ''}` : 'Clean selected', `data-act="clnClean" ${sel.length && !busy ? '' : 'disabled'}`, 'fd-btn--accent'),
    more([['Open the cleanup log', 'tool', 'clean-log'], ['Edit protected folders (protect.txt)', 'tool', 'protect'], ['Windows storage settings', 'open', 'ms-settings:storagesense']]),
  ].join('');
  const last = c.lastCleaned ? `Last cleaned ${ago(c.lastCleaned.when)}${c.lastCleaned.freed > 0 ? `, freed ${size(c.lastCleaned.freed)}` : ''}` : 'Not cleaned with this app yet';
  const prog = S.clnProgress;
  let status = '';
  if (c.scanning) status = `Scanning… ${prog?.current || ''}`;
  else if (c.cleaning) status = `${S.clnPhase || 'Cleaning…'} ${prog?.current || ''}`;
  else if (c.scanned) status = `Found ${size(c.rules.filter(r => r.canClean).reduce((a, r) => a + r.total, 0))} in ${c.elapsed.toFixed(1)} s.`;
  const groups = c.groups.map(g => clnGroup(g, c.rules.filter(r => r.group === g.id), c)).join('');
  return head('Clean up', 'Covers what Windows Disk Cleanup does, plus app caches it never sees. Installed tools, models and your files are never touched.', cmds)
    + `<div class="status"><div class="meter">${progress(used, 'System drive space used', used > .9 ? 'fd-progress--error' : '')}<span class="muted small">Drive ${esc(S.info?.drive || 'C:')} ${Math.round(used * 100)} percent used, ${dr ? gb(dr.free) : '?'} free · ${esc(last)}</span></div><span class="muted">${esc(status)}</span></div>`
    + (busy ? `<div style="margin:0 0 12px">${progress(prog && prog.total ? prog.done / prog.total : null, 'Progress')}</div>` : '')
    + `<div class="notes">${S.bars.cleanup || ''}</div>`
    + `<div class="split"><div class="stack stack--12">${c.scanned ? groups + clnKept(c) : `<div class="loading">${ring()}<span>Scanning…</span></div>`}</div><aside class="fd-card side" aria-label="Details">${clnDetails()}</aside></div>`;
};

function clnGroup(g, rules, c) {
  const live = rules.filter(r => r.canClean), allOn = live.length && live.every(r => r.ticked);
  const adminScan = g.id === 'Windows' && !c.admin && !rules.some(r => r.scannedElevated);
  const rows = rules.filter(r => !(adminScan && r.needsAdmin)).map(r => {
    const sz = r.canClean ? (r.total ? size(r.total) : r.unknownSize ? 'Size shows after the run' : '') : '';
    const why = r.blocked ? (r.blocked === 'needs admin' ? 'Needs admin. Use Scan with admin on this group.' : r.blocked) : r.why;
    return `<div class="item ${r.canClean ? '' : 'item--off'}" role="option" aria-selected="${S.sel.cleanup === r.id}">
      ${check(r.ticked && r.canClean, `data-chg="clnTick" data-arg="${esc(r.id)}" data-fid="c-${esc(r.id)}" aria-label="${esc(r.title)}"`, '', !r.canClean || c.cleaning)}
      <button class="item__main" type="button" data-act="clnPick" data-arg="${esc(r.id)}"><span>${esc(r.title)}</span><small>${esc(why)}${r.skippedCount ? `  (${r.skippedCount} left alone)` : ''}</small></button>
      <span class="item__end small nowrap">${esc(sz)}</span></div>`;
  }).join('');
  const admin = adminScan ? `<div class="item"><span class="item__main"><span>What Disk Cleanup's system-file mode clears. Scanning it needs admin.</span><small>Windows asks for admin once. Only Windows' own folders are scanned this way.</small></span><span class="item__end">${btn(`${ico('shield')}<span>Scan with admin</span>`, `data-act="clnAdmin" ${c.scanning || c.cleaning || S.clnAdminBusy ? 'disabled' : ''} aria-label="Scan the Windows group with admin"`)}</span></div>` : '';
  if (!rows && !admin) return '';
  return `<section class="fd-card group"><div class="group__head">${live.length ? check(allOn, `data-chg="clnGroup" data-arg="${g.id}" aria-label="Select everything in ${esc(g.title)}"`, '', c.cleaning) : ico(GROUP_ICON[g.id], 't-muted')}
    <div class="group__title"><h2 class="fd-body-strong">${esc(g.title)}</h2><small>${esc(g.blurb)}</small></div><span class="small nowrap muted">${live.length ? size(live.reduce((a, r) => a + r.total, 0)) : ''}</span></div>${rows}${admin}</section>`;
}

function clnKept(c) {
  return `<details class="fd-expander"><summary><span>Kept on purpose <span class="muted">Never cleaned. Every delete is also checked against ${c.guardFolders} install and PATH folders on this PC.</span></span><span class="fd-expander__chev"><i class="fd-i fd-i--12 fd-i-chevron-down" aria-hidden="true"></i></span></summary>
    <div class="fd-expander__content"><dl class="facts">${c.kept.map(k => `<dt>${esc(k.what)}</dt><dd class="muted">${esc(k.why)}</dd>`).join('')}</dl></div></details>`;
}

function clnDetails() {
  const r = (S.cln?.rules || []).find(x => x.id === S.sel.cleanup);
  if (!r) return `<h2 class="fd-subtitle">Details</h2><p class="muted" style="margin-top:8px">Select a row to see exactly which folders it covers, and anything the safety check refused.</p><p class="muted" style="margin-top:8px">Every run is logged to runs.csv, and the History page shows each one.</p>`;
  const items = r.items.map(i => `<div class="row row--top"><span class="grow break"><span>${esc(i.label || i.path)}</span>${i.path && i.label && i.label !== i.path ? `<br><small class="muted mono">${esc(i.path)}</small>` : ''}${i.result ? `<br><small class="${/^(deleted|done)/.test(i.result) ? 't-success' : 't-caution'}">${esc(i.result)}</small>` : ''}</span><span class="small nowrap muted">${i.size == null ? 'after the run' : size(i.size)}</span></div>`).join('');
  const skipped = r.skipped.map(k => `<div class="break"><span class="mono small">${esc(k.what)}</span><br><small class="muted">${esc(k.why)}</small></div>`).join('');
  return `<h2 class="fd-subtitle">${esc(r.title)}</h2><p class="muted" style="margin-top:4px">${esc(r.why)}${r.needsAdmin && !r.scannedElevated && !S.cln.admin ? ' Needs admin: use Scan with admin on the Windows group.' : ''}</p>
    <div class="lines" style="margin-top:12px">${items || '<span class="muted">Nothing found.</span>'}${r.itemCount > r.items.length ? `<span class="muted">…and ${r.itemCount - r.items.length} more.</span>` : ''}</div>
    ${skipped ? `<h3 class="fd-body-strong" style="margin-top:16px">Left alone</h3><div class="lines" style="margin-top:8px">${skipped}${r.skippedCount > r.skipped.length ? `<span class="muted">…and ${r.skippedCount - r.skipped.length} more.</span>` : ''}</div>` : ''}`;
}

A.clnScan = () => { S.bars.cleanup = ''; invoke('cln_scan'); };
A.clnPick = id => { S.sel.cleanup = id; render('cleanup'); };
A.clnTick = (id, el) => invoke('cln_tick', { ids: [id], on: el.checked });
A.clnGroup = (g, el) => invoke('cln_tick', { ids: S.cln.rules.filter(r => r.group === g && r.canClean).map(r => r.id), on: el.checked });
A.clnStop = () => { S.clnStopping = true; invoke('cln_stop'); render('cleanup'); };
A.clnAdmin = async () => {
  S.clnAdminBusy = true; S.bars.cleanup = infobar('info', 'Waiting for you to allow admin access…'); render('cleanup');
  try { S.bars.cleanup = infobar('success', await invoke('cln_scan_admin')); }
  catch (e) { S.bars.cleanup = String(e) === 'declined' ? infobar('caution', 'Admin access was declined, so the Windows group wasn\'t scanned.') : infobar('critical', 'The Windows scan failed', esc(e)); }
  S.clnAdminBusy = false; render('cleanup');
};
A.clnClean = async () => {
  const sel = tickedRules(), n = sel.reduce((a, r) => a + r.itemCount, 0), bytes = tickedTotal();
  let body = `Delete ${plural(n, 'item')}, about ${size(bytes)}? This can't be undone.`;
  if (!S.cln.admin && sel.some(r => r.needsAdmin)) body += '<br><br>Windows will ask for admin once, for the Windows items.';
  if (sel.some(r => r.handlers)) body += '<br><br>Windows\' own Disk Cleanup runs too and can take 10+ minutes.';
  if (!await ask('Clean selected?', body, 'Delete', 'Keep these files')) return;
  S.clnStopping = false; S.bars.cleanup = ''; S.clnProgress = null;
  invoke('cln_clean', { ids: sel.map(r => r.id) }).catch(fail);
};
A.clnProtect = async () => {
  const body = `<div class="fd-field"><label for="pf">Folder to protect</label><div class="row"><input class="fd-textbox grow" id="pf" placeholder="For example D:\\Games\\Mods or %USERPROFILE%\\Mods" spellcheck="false">${btn('Browse…', 'id="pf-b"')}</div></div>`;
  const p = ask('Protect a folder', body, 'Protect');
  $('#pf-b').onclick = async () => { const f = await invoke('pick_folder'); if (f) $('#pf').value = f; };
  if (!await p) return;
  const v = $('#pf').value.trim();
  if (!v) return;
  try {
    const f = await invoke('cln_protect', { folder: v });
    S.bars.cleanup = infobar('success', 'Folder protected', `${esc(f)} and everything in it are never cleaned. Rescan to see the change.`);
  } catch (e) { S.bars.cleanup = infobar('critical', 'Folder not found', esc(e)); }
  render('cleanup');
};

/* ================================================================ Big files */
const KINDS = [['All', 'All files'], ['Video', 'Videos'], ['DiskImage', 'Disk images'], ['Archive', 'Archives'], ['Installer', 'Installers'], ['Audio', 'Audio'], ['AiModel', 'AI models'], ['Other', 'Other']];
const KIND_ICON = { Video: 'video', DiskImage: 'disc', Archive: 'zip', Installer: 'download', Audio: 'music', AiModel: 'chip', Other: 'doc' };
S.big = { min: 100, kind: 'All', files: null, busy: false, where: '' };
R.storage = () => {
  const b = S.big;
  if (b.files === null && !b.busy) A.bigFind();
  const sizes = [[50, '50 MB'], [100, '100 MB'], [500, '500 MB'], [1024, '1 GB']];
  const cmds = `<span class="muted">At least</span>${combo('bigMin', sizes.map(([v, l]) => [String(v), l]), String(b.min), 'Smallest size to list')}
    ${btn(`${ico('refresh')}<span>Find big files</span>`, `data-act="bigFind" ${b.busy ? 'disabled' : ''}`, 'fd-btn--accent')}
    ${more([['Storage settings', 'open', 'ms-settings:storagesense'], ['Open the Recycle Bin', 'open', 'shell:RecycleBinFolder']])}`;
  const shown = (b.files || []).filter(f => b.kind === 'All' || f.kind === b.kind);
  const total = shown.reduce((a, f) => a + f.size, 0);
  const year = Date.now() - 365 * 864e5;
  const rows = shown.map(f => {
    const hints = [];
    if (f.kind === 'Installer' && f.folder.toLowerCase().startsWith((b.downloads || '').toLowerCase())) hints.push(badge('Installer in Downloads: you can usually download it again', 'success'));
    if (new Date(f.modified) < year) hints.push(badge('Not changed for over a year', 'neutral'));
    if (f.kind === 'AiModel') hints.push(badge('A model an AI tool may still use', 'caution'));
    return `<div class="item">${ico(KIND_ICON[f.kind] || 'doc', 'ico--20 t-muted')}<span class="item__main"><span class="break">${esc(f.name)}</span><small class="break">${esc(f.folder)} · ${esc(f.kindName)} · changed ${day(new Date(f.modified))}</small>${hints.length ? `<span class="row wrap" style="gap:6px;margin-top:4px">${hints.join('')}</span>` : ''}</span>
      <span class="item__end"><span class="fd-body-strong nowrap">${size(f.size)}</span>
      <button class="fd-btn fd-btn--subtle fd-btn--icon" type="button" data-act="reveal" data-arg="${esc(f.path)}" aria-label="Show ${esc(f.name)} in its folder" title="Show in folder">${ico('folder')}</button>
      <button class="fd-btn fd-btn--subtle fd-btn--icon" type="button" data-act="bigRecycle" data-arg="${esc(f.path)}" aria-label="Move ${esc(f.name)} to the Recycle Bin" title="Move to the Recycle Bin">${ico('recycle')}</button></span></div>`;
  }).join('');
  const tabs = `<div class="fd-selector" role="tablist" aria-label="Kind of file" style="margin-bottom:12px">${KINDS.map(([k, l]) => `<button class="fd-selector-item" type="button" role="tab" aria-selected="${b.kind === k}" ${b.kind === k ? '' : 'tabindex="-1"'} data-act="bigKind" data-arg="${k}">${esc(l)}</button>`).join('')}</div>`;
  let list;
  if (b.busy) list = `<div class="loading">${ring()}<span class="break">Looking in ${esc(b.where || 'your profile')}…</span></div>`;
  else if (!shown.length) list = `<div class="empty">${ico('info')}<span>${b.files?.length ? 'None of this kind.' : 'Nothing that size in your profile.'}</span></div>`;
  else list = `<section class="fd-card group"><div class="group__head"><div class="group__title"><h2 class="fd-body-strong">${plural(shown.length, 'file')}, ${size(total)}</h2><small>${b.files.length >= 300 ? 'The 300 largest. ' : ''}Looked through your profile at ${b.taken ? hm(new Date(b.taken)) : ''}.</small></div></div>${rows}</section>`;
  return head('Big files', 'Your largest files across Desktop, Documents, Downloads, Videos and the rest of your profile, so you can decide what to keep. Nothing is deleted: a file you choose goes to the Recycle Bin, so it can be restored.', cmds)
    + `<div class="notes">${S.bars.storage || ''}</div>${tabs}${list}`;
};
A.bigKind = k => { S.big.kind = k; render('storage'); };
A.bigFind = async () => {
  const b = S.big; b.busy = true; b.where = ''; render('storage');
  try { const r = await invoke('big_find', { minMb: b.min }); b.files = r.files; b.downloads = r.downloads; b.taken = r.taken; }
  catch (e) { b.files = []; S.bars.storage = infobar('critical', 'The search stopped', esc(e)); }
  b.busy = false; render('storage');
};
A.bigRecycle = async path => {
  const f = S.big.files.find(x => x.path === path);
  if (!f) return;
  let body = `${esc(f.name)} (${size(f.size)}) goes to the Recycle Bin, so you can restore it.`;
  if (f.size > 4 * GB) body += '<br><br>It is very large, so Windows may say it is too big for the Recycle Bin and ask whether to delete it for good. Say No if you\'re not sure.';
  if (!await ask('Move to the Recycle Bin?', body, 'Move to Recycle Bin', 'Keep the file')) return;
  const why = await invoke('big_recycle', { path }).catch(e => String(e));
  if (why) S.bars.storage = infobar('caution', `${f.name} was left in place`, esc(why));
  else { S.big.files = S.big.files.filter(x => x.path !== path); S.bars.storage = infobar('success', 'Moved to the Recycle Bin', esc(f.name)); }
  render('storage');
};

/* ================================================================ Health */
R.health = () => {
  const h = S.health;
  const cmds = btn(`${ico('refresh')}<span>Check again</span>`, `data-act="healthAgain" ${healthLoading ? 'disabled' : ''}`)
    + more([['Reliability Monitor', 'tool', 'reliability'], ['Windows Security', 'open', 'windowsdefender:'], ['System information', 'tool', 'sysinfo']]);
  const top = head('Health', 'Crashes and blue screens, drive health, battery wear and security, from what Windows already records. Nothing here changes your PC.', cmds);
  if (!h || healthLoading) { if (!h) loadHealth(); return top + `<div class="loading">${ring()}<span>Reading Windows' records…</span></div>`; }
  if (h.error) return top + infobar('critical', 'Windows\' records couldn\'t be read', 'Press Check again to retry. Details are in error.log.');
  const recent = h.crashes.filter(c => Date.now() - new Date(c.when) < 30 * 864e5).length;
  const issues = (recent ? 1 : 0) + h.drives.filter(d => d.health !== 'Healthy').length + h.batteries.filter(b => b.percent < 60).length + (h.antivirus.some(a => a.on) ? 0 : 1) + (h.restartPending ? 1 : 0);
  const up = h.uptimeSecs, upText = up >= 86400 ? `${plural(Math.floor(up / 86400), 'day')} ${Math.floor(up % 86400 / 3600)} h` : `${Math.floor(up / 3600)} h ${Math.floor(up % 3600 / 60)} min`;
  const card = (title, sub, body) => `<section class="fd-card card"><div><h2 class="fd-subtitle">${esc(title)}</h2><p class="muted small">${esc(sub)}</p></div><div class="card__body">${body}</div></section>`;
  // Stability
  const crashRow = c => {
    const title = c.kind === 'BlueScreen' ? (c.stopCodeHex ? `Blue screen ${c.stopCodeHex}${c.stopName ? ' ' + c.stopName : ''}` : 'Blue screen') : c.kind === 'PowerLoss' ? 'Lost power or was forced off (no stop code)' : 'Shut down unexpectedly';
    return `<div class="row row--top">${ico(c.kind === 'BlueScreen' ? 'warn' : 'bolt', c.kind === 'BlueScreen' ? 't-critical' : 't-caution')}<span class="grow"><span class="break">${esc(title)}</span><br><small class="muted">${esc(ago(c.when))} · ${esc(c.source)}</small>${c.hint ? `<br><small>${esc(c.hint)}</small>` : ''}${c.link ? `<br><a class="fd-link small" data-act="open" data-arg="${esc(c.link)}">What Microsoft says about this stop code</a>` : ''}</span></div>`;
  };
  const crashes = h.crashes.length ? h.crashes.slice(0, 12).map(crashRow).join('') + (h.crashes.length > 12 ? `<p class="muted small">and ${h.crashes.length - 12} earlier ones. Reliability Monitor shows them all.</p>` : '') : `<div class="row">${ico('check', 't-success')}<span>No crashes in the last 90 days</span></div>`;
  const apps = h.appCrashes.length ? `<h3 class="fd-body-strong" style="margin-top:8px">Apps that crashed in the last 14 days</h3>${h.appCrashes.slice(0, 8).map(a => `<div class="row"><span class="grow break">${esc(a.app)}</span><span class="small muted nowrap">${plural(a.count, 'time')}, last ${esc(ago(a.last))}</span></div>`).join('')}` : '';
  // Drives
  const wear = S.wear || [];
  const drives = h.drives.length ? h.drives.map(d => {
    const w = wear.find(x => x.deviceId?.trim() === d.name) || (wear.length === 1 && h.drives.length === 1 ? wear[0] : null);
    const tone = d.health === 'Healthy' ? 'success' : d.health === 'Warning' ? 'caution' : d.health === 'Unknown' ? 'neutral' : 'critical';
    const wearText = w ? [w.wear != null ? `${w.wear}% worn` : 'wear not reported', w.temperature != null ? `${w.temperature} °C now${w.temperatureMax ? `, rated up to ${w.temperatureMax} °C` : ''}` : '', w.powerOnHours != null ? `${w.powerOnHours} hours on` : '', w.readErrors ? `${w.readErrors} read errors` : ''].filter(Boolean).join(' · ') : '';
    return `<div class="row row--top">${ico('storage', 'ico--20 t-muted')}<span class="grow"><span class="row wrap" style="gap:8px"><span class="fd-body-strong break">${esc(d.name)}</span>${badge(d.health, tone)}</span><small class="muted">${esc([d.media, d.bus].filter(Boolean).join(' · '))} · ${gb(d.size)} · status ${esc(d.status)}</small>${wearText ? `<br><small>${esc(wearText)}</small>` : ''}</span></div>`;
  }).join('') : '<p class="muted">Windows didn\'t report any drives.</p>';
  const wearBtn = `<div class="stack" style="margin-top:4px">${btn(`${ico('shield')}<span>Read wear and temperature</span>`, `data-act="healthWear" ${S.wearBusy ? 'disabled' : ''} title="Windows gives these counters to administrators only, so it asks for admin once."`)}<small class="muted">SSD wear and temperature need admin. Windows asks once, and only these counters are read.</small></div>`;
  // Battery
  const bat = h.batteries.map(b => `<div class="stack"><div class="big">${b.percent}%</div><span>Holds ${b.percent}% of the charge it held new${b.cycles ? ` · ${b.cycles} charge cycles` : ''}</span>${progress(b.percent / 100, `Battery holds ${b.percent} percent of its new charge`, b.percent < 60 ? 'fd-progress--error' : b.percent < 80 ? 'fd-progress--paused' : '')}<small class="muted">${b.percent >= 80 ? 'Normal wear.' : b.percent >= 60 ? 'Worn. It runs for less time than new; that is normal after a few years, and nothing needs doing yet.' : 'Heavily worn. If it drains fast or the PC shuts down on battery, a replacement battery helps.'}</small></div>`).join('');
  // Security and system
  const av = h.antivirus.length ? h.antivirus.map(a => `<div class="row">${ico('shield', a.on ? (a.upToDate ? 't-success' : 't-caution') : 't-muted')}<span class="grow">${esc(a.name)}: ${a.on ? 'on' : 'off'}${a.on && !a.upToDate ? ', out of date' : ''}</span></div>`).join('') : `<div class="row">${ico('shield', 't-critical')}<span>Windows Security reports no antivirus.</span></div>`;
  const sys = `${av}<div class="row">${ico(h.restartPending ? 'warn' : 'check', h.restartPending ? 't-caution' : 't-success')}<span>${h.restartPending ? 'Windows is waiting for a restart to finish installing something.' : 'No restart pending.'}</span></div><div class="row">${ico('clock', 't-muted')}<span>Up for ${esc(upText)}</span></div>
    <div class="stack" style="margin-top:4px">${btn(`${ico('save')}<span>Make a restore point now</span>`, `data-act="healthRestore" ${S.rpBusy ? 'disabled' : ''}`)}<small class="muted">Asks for admin once. Windows skips it if it made one in the last 24 hours.</small></div>`;
  return top + `<div class="status"><span class="fd-subtitle">${issues ? `${plural(issues, 'thing')} to look at` : 'Looks healthy'}</span><span class="muted">Read ${esc(ago(h.taken))}. Up for ${esc(upText)}${h.restartPending ? ', restart pending' : ''}.</span></div><div class="notes">${S.bars.health || ''}</div>
    <div class="cards">${card('Stability', 'Blue screens (stop codes), power cuts and unexpected shutdowns from the System log.', `<div class="stack stack--12">${crashes}${apps}</div>`)}
    ${card('Drives', 'Health as the drive reports it to Windows.', `<div class="stack stack--12">${drives}${wearBtn}</div>`)}
    ${h.batteries.length ? card('Battery', 'From Windows\' battery report (powercfg).', `${bat}<div class="card__foot">${btn('Open the full battery report', `data-act="healthBattery" ${S.batBusy ? 'disabled' : ''}`)}</div>`) : ''}
    ${card('Security and system', 'Antivirus as Windows Security reports it, pending restarts and restore points.', `<div class="stack stack--12">${sys}</div>`)}</div>`;
};
A.healthAgain = () => { S.bars.health = ''; loadHealth(true); render('health'); };
A.healthWear = async () => {
  S.wearBusy = true; S.bars.health = infobar('info', 'Waiting for you to allow admin access…'); render('health');
  try { S.wear = await invoke('health_wear'); S.bars.health = S.wear.length ? '' : infobar('info', 'No counters', 'The drive didn\'t report wear or temperature to Windows. Many USB and some SATA drives don\'t.'); }
  catch (e) { S.bars.health = String(e) === 'declined' ? infobar('caution', 'Admin access was declined.') : infobar('critical', 'The counters couldn\'t be read', esc(e)); }
  S.wearBusy = false; render('health');
};
A.healthBattery = async () => {
  S.batBusy = true; S.bars.health = infobar('info', 'Making the battery report…'); render('health');
  try { await invoke('health_battery_report'); S.bars.health = infobar('success', 'Opened the battery report.'); } catch (e) { S.bars.health = infobar('critical', 'Windows couldn\'t make the report.', esc(e)); }
  S.batBusy = false; render('health');
};
A.healthRestore = async () => {
  S.rpBusy = true; S.bars.health = infobar('info', 'Waiting for you to allow admin access…'); render('health');
  try { S.bars.health = infobar('success', await invoke('health_restore_point')); }
  catch (e) { S.bars.health = String(e) === 'declined' ? infobar('caution', 'Admin access was declined.') : infobar('critical', 'No restore point was made', esc(e)); }
  S.rpBusy = false; render('health');
};

/* ================================================================ Repair */
S.rep = { list: null, out: [], running: null, phase: '' };
R.repair = async () => {
  const r = S.rep;
  if (!r.list) r.list = await invoke('repair_list');
  const cmds = btn(`${ico('copy')}<span>Copy the output</span>`, `data-act="repCopy" ${r.out.length ? '' : 'disabled'}`) + more([['Windows troubleshooters', 'open', 'ms-settings:troubleshoot'], ['Recovery options', 'open', 'ms-settings:recovery']]);
  const cards = r.list.map(x => `<section class="fd-card" style="display:flex;flex-direction:column;gap:8px"><div class="row wrap" style="gap:8px"><h2 class="fd-body-strong grow">${esc(x.title)}</h2>${x.needsAdmin ? badge('Asks for admin', 'neutral') : ''}${x.restartAfter ? badge('Restart after', 'caution') : ''}</div>
    <p>${esc(x.what)}</p><p class="muted small">${esc(x.heads)}</p><p class="muted small mono break">${x.steps.map(s => esc(s.command)).join('<br>')}</p>
    <div>${btn(r.running === x.id ? `${ring('Running')}<span>Running…</span>` : `${ico('play')}<span>Run</span>`, `data-act="repRun" data-arg="${x.id}" ${r.running ? 'disabled' : ''}`)}</div></section>`).join('');
  const out = r.out.length ? r.out.map(l => l.startsWith('## ') ? `<span class="h">${esc(l.slice(3))}</span>` : l.startsWith('== ') ? `<span class="ok">${esc(l.slice(3))}</span>` : l.startsWith('!! ') ? `<span class="bad">${esc(l.slice(3))}</span>` : esc(l)).join('\n') : '<span class="muted">Pick a repair. Its output shows here as it runs.</span>';
  return head('Repair', 'Fixes for common Windows problems. Each one is a documented Windows command, and you see everything it prints. For damaged system files, Microsoft\'s order is used: DISM repairs the Windows image first, then SFC checks the files.', cmds)
    + `<div class="notes">${S.bars.repair || ''}</div><div class="split"><div class="cards" style="grid-template-columns:repeat(auto-fill,minmax(min(100%,320px),1fr))">${cards}</div>
    <aside class="fd-card side" aria-label="Output"><div class="row" style="margin-bottom:8px"><h2 class="fd-subtitle grow">Output</h2>${r.running ? `<span class="muted small">${esc(r.phase)}</span>` : ''}</div><pre class="out mono" id="rep-out">${out}</pre></aside></div>`;
};
A.repRun = async id => {
  const x = S.rep.list.find(r => r.id === id);
  if (!await ask(`${x.title}?`, esc(x.heads) + (x.needsAdmin ? ' Windows asks for admin once.' : ''), 'Run')) return;
  S.rep.out = []; S.rep.running = id; S.rep.phase = 'Starting…'; S.bars.repair = '';
  invoke('repair_run', { id }).catch(e => { S.rep.running = null; fail(e); });
  render('repair');
};
A.repCopy = () => navigator.clipboard.writeText(S.rep.out.join('\n')).then(() => { S.bars.repair = infobar('success', 'Copied the output.'); render('repair'); });

/* ================================================================ PATH doctor */
const PROBLEM = { Missing: 'The folder doesn\'t exist', Duplicate: 'Repeats', Empty: 'Empty entry (two semicolons in a row)', NotAFolder: 'Points to a file, not a folder' };
R.path = async () => {
  if (!S.pathState) S.pathState = await invoke('path_state').catch(e => ({ error: String(e) }));
  const p = S.pathState;
  const fixable = (p.entries || []).filter(e => e.fixable);
  const cmds = btn(`${ico('refresh')}<span>Check again</span>`, 'data-act="pathAgain"')
    + btn(fixable.length ? `Fix my user PATH (${fixable.length})` : 'Fix my user PATH', `data-act="pathFix" ${fixable.length ? '' : 'disabled'}`, 'fd-btn--accent')
    + more([['Undo the last fix', 'pathUndo'], ['Edit environment variables', 'tool', 'env']]);
  const top = head('PATH doctor', 'Finds PATH entries that point to folders that no longer exist or repeat an earlier one, and command links an update broke. It edits only your user PATH, and saves the old value first so you can put it back.', cmds);
  if (p.error) return top + infobar('critical', 'PATH couldn\'t be read', esc(p.error));
  const problems = p.entries.filter(e => e.problem !== 'None');
  const prow = e => `<div class="item">${ico('warn', e.fixable ? 't-caution' : 't-muted')}<span class="item__main"><span class="mono break">${esc(e.raw || '(empty)')}</span><small>${esc(e.problem === 'Duplicate' ? `Repeats ${e.duplicateOf}` : PROBLEM[e.problem])}${e.scope === 'Machine' ? '. In the system PATH, which needs admin to change: use Edit environment variables in the More menu.' : ''}</small></span><span class="item__end">${badge(e.scope === 'Machine' ? 'System' : 'User', 'neutral')}</span></div>`;
  const crow = c => `<div class="item">${ico(c.storeAlias ? 'warn' : c.others.length ? 'info' : 'check', c.storeAlias ? 't-caution' : c.others.length ? 't-muted' : 't-success')}<span class="item__main"><span><span class="fd-body-strong">${esc(c.command)}</span> runs <span class="mono break">${esc(c.runs)}</span></span>${c.storeAlias ? `<small class="t-caution">This is Windows' shortcut that opens the Microsoft Store, not a real Python. Turn it off in Settings, Apps, Advanced app settings, App execution aliases.</small>` : ''}${c.others.length ? `<small class="mono break">Also on PATH, but not used: ${c.others.map(esc).join('; ')}</small>` : ''}</span></div>`;
  const brow = l => `<div class="item">${ico('link', 't-caution')}<span class="item__main"><span class="mono break">${esc(l.link)}</span><small class="break">Points to ${esc(l.target)}, which is gone. Reinstall the package with winget (winget install --id &lt;id&gt; --force) to bring the link back.</small></span></div>`;
  const group = (title, sub, rows, empty) => `<section class="fd-card group"><div class="group__head"><div class="group__title"><h2 class="fd-body-strong">${esc(title)}</h2><small>${esc(sub)}</small></div></div>${rows || `<div class="item"><span class="muted">${esc(empty)}</span></div>`}</section>`;
  const all = p.entries.map(e => `<div class="row"><span class="small muted nowrap" style="width:72px">${e.scope === 'Machine' ? 'System' : 'User'} ${e.index + 1}</span><span class="mono break grow ${e.problem !== 'None' ? 't-caution' : ''}">${esc(e.raw || '(empty)')}</span></div>`).join('');
  return top + `<div class="status"><span class="fd-subtitle">${problems.length ? plural(problems.length, 'problem') : 'PATH looks clean'}</span>${p.backup ? `<span class="muted">Last fix saved the old PATH to ${esc(p.backup)}.</span>` : ''}</div><div class="notes">${S.bars.path || ''}</div>
    <div class="stack stack--12">${group('Problems', 'Missing folders slow down every command lookup a little; repeats just add noise.', problems.map(prow).join(''), 'No problems found.')}
    ${group('Which copy runs', 'When two folders hold the same command, the first one in PATH wins.', p.commands.map(crow).join(''), 'None of the common developer commands are on PATH.')}
    ${p.broken.length ? group('winget command links', 'Links in WinGet\\Links that make tools like ffmpeg runnable from any terminal.', p.broken.map(brow).join(''), '') : ''}
    <details class="fd-expander"><summary><span>Every PATH entry, in the order Windows searches them</span><span class="fd-expander__chev"><i class="fd-i fd-i--12 fd-i-chevron-down" aria-hidden="true"></i></span></summary><div class="fd-expander__content"><div class="stack">${all}</div></div></details></div>`;
};
A.pathAgain = () => { S.pathState = null; S.bars.path = ''; render('path'); };
A.pathFix = async () => {
  const fix = S.pathState.entries.filter(e => e.fixable);
  if (!await ask('Remove these from your user PATH?', `<div class="stack mono small break" style="margin-bottom:12px">${fix.map(x => esc(x.raw || '(empty entry)')).join('<br>')}</div>The old PATH is saved first, and Undo the last fix puts it back. Open terminals keep the old PATH until you reopen them.`, 'Remove', 'Keep them')) return;
  try { await invoke('path_fix'); S.bars.path = infobar('success', 'User PATH fixed', `Removed ${plural(fix.length, 'entry', 'entries')}. Reopen terminals to pick it up.`); }
  catch (e) { S.bars.path = infobar('critical', 'PATH wasn\'t changed', esc(e)); }
  S.pathState = null; render('path');
};
A.pathUndo = async () => {
  const b = S.pathState?.backup;
  if (!b) return tell('Nothing to undo', 'No PATH fix has been made from here yet.');
  if (!await ask('Put the old user PATH back?', `Restores the user PATH saved in ${esc(b)}.`, 'Restore')) return;
  try { await invoke('path_undo'); S.bars.path = infobar('success', 'Old user PATH restored', 'Reopen terminals to pick it up.'); } catch (e) { S.bars.path = infobar('critical', 'PATH wasn\'t changed', esc(e)); }
  S.pathState = null; render('path');
};

/* ================================================================ App list */
S.al = { versions: false, saving: false, result: null, pick: null };
R.apps = () => {
  const a = S.al, r = a.result, p = a.pick;
  const saved = r ? `<div class="stack stack--12" style="margin-top:12px">${infobar('success', 'Saved', `to <span class="mono break">${esc(r.folder)}</span>`, `<span>${btn('Open the folder', `data-act="open" data-arg="${esc(r.folder)}"`)}</span>`)}
    ${r.parts.map(x => `<div class="row row--top">${ico('check', 't-success')}<span class="grow"><span class="fd-body-strong">${esc(x.name)}</span>: ${plural(x.count, 'item')}${x.note ? `<br><small class="muted">${esc(x.note)}</small>` : ''}</span></div>`).join('')}
    ${r.notInWinget.length ? `<div class="row row--top">${ico('warn', 't-caution')}<span class="grow"><span class="fd-body-strong">${plural(r.notInWinget.length, 'app')} winget can't reinstall</span><br><small class="muted">Reinstall these by hand on the new PC. The README lists them too.</small><br><small>${r.notInWinget.slice(0, 40).map(esc).join(', ')}${r.notInWinget.length > 40 ? `, and ${r.notInWinget.length - 40} more in the README` : ''}</small></span></div>` : ''}</div>` : '';
  const picked = p ? `<div class="stack stack--12" style="margin-top:12px"><p><span class="fd-body-strong">${plural(p.ids.length, 'app')}</span> in ${esc(p.name)}</p><p class="mono small break">${p.ids.slice(0, 60).map(esc).join(', ')}${p.ids.length > 60 ? `, and ${p.ids.length - 60} more` : ''}</p><p class="muted small">Apps you already have are skipped by winget. Some installers ask for admin themselves.</p><div>${btn('Install them with winget', 'data-act="alImport"', 'fd-btn--accent')}</div></div>` : '';
  return head('App list', 'Saves the list of what\'s installed, to set up a new PC or get back to normal after a reset. Saving only reads. Restoring runs winget import in a window you can watch.')
    + `<div class="notes">${S.bars.apps || ''}</div><div class="cards" style="grid-template-columns:repeat(auto-fill,minmax(min(100%,460px),1fr))">
    <section class="fd-card card"><div class="card__head">${ico('save', 'ico--20')}<div><h2 class="fd-subtitle">Save my app list</h2></div></div><div class="card__body"><p>winget apps as winget-apps.json (the file winget import reads), plus plain lists of pip user packages, global npm packages and VS Code extensions, and a README with what to do.</p>
    ${check(a.versions, 'data-chg="alVersions"', 'Include the exact versions (import then installs those, not the newest)')}${saved}</div>
    <div class="card__foot">${btn(a.saving ? `${ring('Saving')}<span>Saving…</span>` : 'Save my app list', `data-act="alSave" ${a.saving ? 'disabled' : ''}`, 'fd-btn--accent')}</div>${a.saving ? '<p class="muted small">Asking winget, pip, npm and VS Code what\'s installed. This takes about a minute.</p>' : ''}</section>
    <section class="fd-card card"><div class="card__head">${ico('download', 'ico--20')}<div><h2 class="fd-subtitle">Restore</h2></div></div><div class="card__body"><p>Pick a winget-apps.json saved here or by winget export. You see the list before anything installs.</p>${picked}</div><div class="card__foot">${btn(`${ico('folder')}<span>Pick a saved list</span>`, 'data-act="alPick"')}</div></section></div>`;
};
A.alVersions = (_, el) => { S.al.versions = el.checked; };
A.alSave = async () => {
  S.al.saving = true; S.al.result = null; S.bars.apps = ''; render('apps');
  try { S.al.result = await invoke('applist_save', { includeVersions: S.al.versions }); } catch (e) { S.bars.apps = infobar('critical', 'The list wasn\'t saved', esc(e)); }
  S.al.saving = false; render('apps');
};
A.alPick = async () => {
  try { const p = await invoke('applist_pick'); if (p) S.al.pick = p; S.bars.apps = ''; } catch (e) { S.bars.apps = infobar('critical', 'That file couldn\'t be used', esc(e)); }
  render('apps');
};
A.alImport = async () => {
  const p = S.al.pick;
  if (!await ask('Install these apps?', `A terminal window opens and runs winget import for ${plural(p.ids.length, 'app')}. You can watch it and close it to stop.`, 'Open the terminal')) return;
  if (!await invoke('applist_import', { file: p.file })) fail('The terminal didn\'t open.');
};

/* ================================================================ History */
S.hist = { filter: 'All', runs: null };
R.history = async () => {
  const h = S.hist;
  if (!h.runs) h.runs = await invoke('history').catch(() => []);
  const runs = h.runs.filter(r => h.filter === 'All' || r.kind === h.filter);
  const cmds = btn(`${ico('refresh')}<span>Refresh</span>`, 'data-act="histRefresh"') + more([['Open the update log (runs.csv)', 'tool', 'update-runs'], ['Open the cleanup log (runs.csv)', 'tool', 'clean-log']]);
  const tabs = `<div class="fd-selector" role="tablist" aria-label="Show" style="margin-bottom:12px">${[['All', 'Everything'], ['Updates', 'Update runs'], ['Cleanup', 'Cleans']].map(([k, l]) => `<button class="fd-selector-item" type="button" role="tab" aria-selected="${h.filter === k}" ${h.filter === k ? '' : 'tabindex="-1"'} data-act="histFilter" data-arg="${k}">${esc(l)}</button>`).join('')}</div>`;
  const run = r => `<details class="fd-expander"><summary>${ico(r.kind === 'Updates' ? 'updates' : 'cleanup', r.problems ? 't-caution' : 't-muted')}<span><span class="fd-body-strong">${r.kind === 'Updates' ? 'Update run' : 'Clean'} ${esc(ago(r.when))}</span><br><small class="muted">${esc(r.summary)}</small></span>${r.problems ? badge(plural(r.problems, 'problem'), 'caution') : ''}<span class="fd-expander__chev"><i class="fd-i fd-i--12 fd-i-chevron-down" aria-hidden="true"></i></span></summary>
    <div class="fd-expander__content"><div class="stack stack--12">${r.lines.map(l => `<div class="row row--top">${ico(l.problem ? 'warn' : 'check', l.problem ? 't-caution' : 't-success')}<span class="grow"><span class="break">${esc(l.title)}</span><br><small class="muted">${esc(l.detail)}</small></span><span class="small ${l.problem ? 't-caution' : 'muted'}" style="max-width:45%;text-align:right">${esc(l.outcome)}</span></div>`).join('')}</div></div></details>`;
  return head('History', 'What each update run and each clean did, newest first. Open a run to see every item.', cmds) + tabs
    + (runs.length ? `<div class="stack">${runs.map(run).join('')}</div>` : `<div class="empty">${ico('info')}<span>Nothing here yet. Runs from Updates and Clean up show up as soon as they finish.</span></div>`);
};
A.histFilter = k => { S.hist.filter = k; render('history'); };
A.histRefresh = () => { S.hist.runs = null; render('history'); };

/* ================================================================ Settings */
R.settings = async () => {
  const s = S.settings, us = await invoke('upd_settings'), task = S.task;
  if (!task) invoke('task_status').then(t => { S.task = t; render('settings'); });
  const row = (g, title, desc, control) => `<div class="fd-setting">${ico(g, 'ico--20')}<div class="fd-setting__text">${esc(title)}<small>${desc}</small></div>${control}</div>`;
  const freq = [['Off', 'Off'], ['Daily', 'Every day'], ['Weekly', 'Every week']];
  const ws = S.accent?.windowsShows;
  const accentNote = `${ws ? 'Windows shows the accent colour on title bars and window borders' : 'Windows is set not to show the accent colour on title bars and window borders'}, so Safe Toolbox ${ws ? 'uses it' : 'uses neutral greys'} when set to Follow Windows.`;
  let taskText = '';
  if (task) {
    if (!task.wanted) taskText = task.exists ? 'Both checks are off, but the task still exists. Change a setting above to remove it.' : 'Off. No task is set up.';
    else if (!task.exists) taskText = 'The task is missing. Change a setting above to create it again.';
    else taskText = `On: the task "${task.taskName}" runs ${task.wanted === 'daily' ? 'every day' : 'every Sunday'} at 12:30, or at the next chance after. ${task.lastWhen ? `Last check ${ago(task.lastWhen)}.` : 'No background check has run yet.'}`;
  }
  const sec = t => `<h2 class="fd-body-strong sec">${esc(t)}</h2>`;
  const differ = Object.keys(s.cleanupChoices || {}).length;
  return head('Settings', 'How Safe Toolbox looks, and what it checks by itself.') + `<div class="notes">${S.bars.settings || ''}</div><div class="settings">
    ${sec('Appearance')}
    ${row('palette', 'Theme', 'Light, dark, or the same as Windows.', combo('theme', [['System', 'Use my Windows setting'], ['Light', 'Light'], ['Dark', 'Dark']], s.theme, 'Theme'))}
    ${row('palette', 'Accent colour', `Colour on buttons, ticks, switches and progress bars. ${esc(accentNote)}`, combo('accent', [['FollowWindows', 'Follow Windows'], ['Always', 'Always'], ['Never', 'Never']], s.accent, 'Accent colour'))}
    ${row('backdrop', 'Background', 'Mica Alt is tinted by your wallpaper, Mica is subtler, Solid has no transparency.', combo('backdrop', [['MicaAlt', 'Mica Alt'], ['Mica', 'Mica'], ['Solid', 'Solid']], s.backdrop, 'Background'))}
    ${sec('When Safe Toolbox opens')}
    ${row('updates', 'Check for updates', 'Starts the update check as soon as the window opens.', toggle(s.checkUpdatesOnOpen, 'data-chg="setToggle" data-arg="checkUpdatesOnOpen" aria-label="Check for updates when Safe Toolbox opens"'))}
    ${sec('Background checks')}
    <p class="muted" style="padding:0 0 8px">A Task Scheduler task runs Safe Toolbox without a window, as you, never as admin. It only looks and tells you. Nothing is installed or deleted in the background.</p>
    ${row('updates', 'Check for updates', 'Notifies you when safe updates are ready, the same list the Updates page shows.', combo('updateCheck', freq, s.updateCheck, 'Background update check'))}
    ${row('cleanup', 'Remind me to clean up', 'Scans like the Clean up page and notifies you when there is a lot to free, or when C: is nearly full.', combo('cleanupReminder', freq, s.cleanupReminder, 'Cleanup reminder'))}
    ${row('storage', 'Remind me from', 'The cleanup reminder waits until at least this much can be freed.', combo('reminderThresholdMb', [['500', '500 MB'], ['1024', '1 GB'], ['2048', '2 GB'], ['5120', '5 GB']], String(s.reminderThresholdMb), 'Remind me from'))}
    ${row('bell', 'Notifications', 'Show a Windows notification when a background check finds something.', `${btn('Send a test notification', 'data-act="setTest"')}${toggle(s.notify, 'data-chg="setToggle" data-arg="notify" aria-label="Notifications"')}`)}
    ${row('task', 'Check now in the background', esc(taskText || 'Reading the task…'), btn(S.bgBusy ? `${ring('Checking')}<span>Checking…</span>` : 'Check now', `data-act="setBgNow" ${S.bgBusy ? 'disabled' : ''}`))}
    ${task?.problem ? `<div class="note t-caution">${esc(task.problem)}</div>` : ''}
    ${sec('Updates')}
    ${row('save', 'Make a restore point before admin updates', 'Skipped when Windows made one in the last 24 hours.', toggle(us.restorePoint, 'data-chg="setUpd" data-arg="restorePoint" aria-label="Make a restore point before admin updates"'))}
    ${row('check', 'Test my tools after a Python update', 'Runs the tool check before and after; a check that passed before and fails after rolls the update back.', toggle(us.testAfterUpdate, 'data-chg="setUpd" data-arg="testAfterUpdate" aria-label="Test my tools after a Python update"'))}
    ${row('undo', 'Hidden and skipped updates', `${plural(us.ignored + us.skipped, 'update')} hidden by you. Each one can also be shown again from the Updates page.`, btn('Show them all again', `data-act="setUnhide" ${us.ignored + us.skipped ? '' : 'disabled'}`))}
    ${sec('Clean up')}
    ${row('check', 'Remember my ticks', 'Keeps the rows you tick or untick for the next scan.', toggle(s.rememberCleanupTicks, 'data-chg="setToggle" data-arg="rememberCleanupTicks" aria-label="Remember my ticks"'))}
    ${row('undo', 'Forget my ticks', `Goes back to what each scan ticks by default. ${plural(differ, 'rule')} differ from the defaults.`, btn('Reset to defaults', `data-act="setForget" ${differ ? '' : 'disabled'}`))}
    ${sec('About')}
    <div class="fd-card stack stack--12"><span class="fd-body-strong">Safe Toolbox ${esc(S.info?.version || '')}</span><p>Updates and cleanup in one app, plus health checks, repairs, a PATH doctor, a big-files view and an app-list backup.</p>
    <p class="muted">What it never does: install or delete anything in the background, change Windows' services or privacy settings, clean the registry, or touch your documents without you asking for that file.</p>
    <div class="row wrap" style="gap:8px">${btn('Open the settings folder', 'data-act="tool" data-arg="settings-folder"')}${btn('Open the update logs', 'data-act="tool" data-arg="update-logs"')}${btn('Notification settings', 'data-act="open" data-arg="ms-settings:notifications"')}</div></div></div>`;
};
A.setToggle = (name, el) => setSetting(name, el.checked);
A.setUpd = (name, el) => invoke('set_upd_setting', { name, on: el.checked });
A.setUnhide = async () => { const n = await invoke('upd_unhide_all'); S.bars.settings = infobar('success', `${plural(n, 'update')} will be listed again after the next check.`); render('settings'); };
A.setForget = async () => { await invoke('cln_forget_ticks'); S.settings = await invoke('get_settings'); S.bars.settings = infobar('success', 'Back to the default ticks', 'Rescan the Clean up page to see them.'); render('settings'); };
A.setTest = async () => {
  try { await invoke('test_notification'); S.bars.settings = infobar('success', 'Notification sent', 'If nothing appeared, check that notifications are on for Safe Toolbox in Windows Settings, System, Notifications.'); }
  catch (e) { S.bars.settings = infobar('caution', 'Notifications aren\'t available', `Windows didn't accept it (${esc(e)}). Background checks still save their result, and the Home page shows it.`); }
  render('settings');
};
A.setBgNow = async () => {
  S.bgBusy = true; render('settings');
  try {
    const r = await invoke('background_now');
    S.bars.settings = r.notification ? infobar('success', r.notification[0], esc(r.notification[1])) : infobar('info', 'Nothing to report', 'No safe updates and not much to clean, so a background check would stay quiet.');
  } catch (e) { S.bars.settings = infobar('critical', 'The check didn\'t finish', esc(e)); }
  S.bgBusy = false; S.task = null; render('settings');
};

/* ---------------------------------------------------------------- shared actions */
A.open = target => invoke('open', { target }).catch(fail);
A.tool = name => invoke('open_tool', { name }).catch(e => tell('Couldn\'t open it', esc(e)));
A.reveal = path => invoke('reveal', { path });
document.addEventListener('click', e => { const c = e.target.closest('[data-combo="bigMin"]'); if (c) { S.big.min = +c.dataset.value; A.bigFind(); } }, true);

/* ---------------------------------------------------------------- events from Rust */
listen('upd', v => { const was = S.upd?.checking; S.upd = v; drawNav(); render('updates'); render('home'); if (was && !v.checking) S.notes = {}; });
listen('upd-status', s => { S.updStatus = s; render('updates'); });
listen('upd-done', sum => {
  S.stopping = false; S.updStatus = '';
  const bits = [`${sum.updated} updated`].concat(sum.failed ? [`${sum.failed} didn't`] : [], sum.skipped ? [`${sum.skipped} not started`] : []).join(', ');
  const log = sum.logFile ? `<span>${btn('Open the log', `data-act="open" data-arg="${esc(sum.logFile)}"`)}</span>` : '';
  S.bars.updates = sum.rebootRecommended ? infobar('caution', 'Restart to finish', `Some updates finish installing when Windows restarts. ${esc(bits)}.`, log)
    : infobar(sum.failed ? 'caution' : 'success', sum.skipped && !sum.updated ? 'The update run stopped' : `Finished at ${hm(new Date())}`, `${esc(bits)}. Press Check again to see what's left.`, log);
  S.hist.runs = null; render('updates');
});
listen('cln', v => { S.cln = v; if (!v.cleaning) S.clnProgress = null; render('cleanup'); render('home'); });
listen('cln-progress', p => { S.clnProgress = p; if (S.page === 'cleanup') render('cleanup'); });
listen('cln-phase', p => { S.clnPhase = p; });
listen('cln-done', r => {
  S.clnStopping = false; S.clnPhase = ''; S.hist.runs = null;
  const parts = [`${plural(r.items, 'item')} cleaned`].concat(r.stopped ? [`Stopped: ${r.stopped} not started`] : [], r.problems ? [`${r.problems} left alone`] : []);
  const log = `<span>${btn('Open the log', 'data-act="tool" data-arg="clean-log"')}</span>`;
  let bar = r.freed > 0 ? infobar('success', `Freed ${size(r.freed)} (measured on the drive)`, esc(parts.join('. ')) + '.' + (r.helper ? ' The Windows group needs another admin scan to show what\'s left.' : ''), log)
    : infobar('info', 'Done. The drive\'s free space did not grow measurably.', esc(parts.join('. ')) + '.', log);
  if (r.declined) bar += infobar('caution', 'Admin access was declined, so the Windows items were left alone.');
  if (r.error) bar += infobar('critical', 'The Windows items weren\'t cleaned', esc(r.error));
  S.bars.cleanup = bar; render('cleanup');
});
listen('repair-line', l => { S.rep.out.push(l); const o = $('#rep-out'); if (S.page === 'repair' && o) { render('repair').then(() => { const n = $('#rep-out'); if (n) n.scrollTop = n.scrollHeight; }); } });
listen('repair-phase', p => { S.rep.phase = p; render('repair'); });
listen('repair-done', r => {
  S.rep.running = null;
  S.bars.repair = r.declined ? infobar('caution', 'Admin access was declined, so nothing ran.')
    : r.ok ? infobar(r.restart ? 'caution' : 'success', `Finished at ${hm(new Date())}.`, r.restart ? 'Restart Windows to finish.' : 'The output is on the right. Copy the output keeps it for later.')
      : infobar('critical', 'The repair didn\'t finish', 'The output on the right says which step stopped and why.');
  render('repair');
});
listen('big-progress', d => { S.big.where = d; if (S.page === 'storage') { const l = $('.loading span'); if (l) l.textContent = `Looking in ${d}…`; } });
listen('navigate', p => go(p));
listen('close-blocked', async reason => {
  if (await ask('Close Safe Toolbox?', esc(reason), 'Close anyway', 'Keep it open')) invoke('force_close');
});
window.addEventListener('focus', () => { if (S.settings?.accent === 'FollowWindows') applyAppearance(); });

/* ---------------------------------------------------------------- start */
(async () => {
  S.info = await invoke('app_info');
  S.settings = await invoke('get_settings');
  await applyAppearance();
  S.upd = await invoke('upd_state');
  S.cln = await invoke('cln_state');
  const start = S.info.startPage || 'home';
  await go(start);
  if (!S.cln.scanned) invoke('cln_scan');
  if (S.settings.checkUpdatesOnOpen && !S.upd.checked && !S.upd.checking) invoke('upd_check');
})();
