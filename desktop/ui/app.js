'use strict';
const T = window.__TAURI__;
const invoke = T.core.invoke;
const $ = (id) => document.getElementById(id);
const esc = (v) => String(v ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

let overview = null;        // configuration (from Rust)
let status = null;          // server status, refreshed every few seconds
let page = 'home';
let phones = null;          // { devices: [...] }
let invite = null;          // newest valid invitation, with its QR code
let reqs = null;            // { requests, shares }
const rec = { busy: false, result: null, showFiles: false, err: '' };
const gb = { step: 1, code: '', found: null, days: 7, link: null, svg: null, busy: false, err: '' };
const inv = { hours: 48, max: 50, busy: false };

// ------------------------------------------------------------------ small helpers

let toastTimer = null;
function toast(msg) {
  const el = $('toast');
  el.innerHTML = icon('circle-check') + esc(msg);
  el.classList.remove('hidden');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.add('hidden'), 2200);
}

function closeModal() { $('modal').classList.add('hidden'); $('modal').innerHTML = ''; }

/** Dialog box. `body` is trusted markup. Resolves with the first field's value (input mode), true, or null. */
function dialog({ title, text = '', body = '', ok = t('ok'), danger = false, input = false }) {
  return new Promise((resolve) => {
    const m = $('modal');
    m.innerHTML = `<form class="dialog" id="dlg">
      <h2>${esc(title)}</h2>
      ${text ? `<p class="muted">${esc(text)}</p>` : ''}
      ${body}
      <div class="actions">
        <button type="button" class="btn ghost" id="dlg-cancel">${esc(t('cancel'))}</button>
        <button type="submit" class="btn ${danger ? 'danger' : ''}">${esc(ok)}</button>
      </div></form>`;
    m.classList.remove('hidden');
    const field = m.querySelector('input');
    if (field) { field.focus(); field.select(); }
    bindEyes(m);
    const done = (v) => { document.removeEventListener('keydown', onKey); closeModal(); resolve(v); };
    const onKey = (e) => { if (e.key === 'Escape') done(null); };
    document.addEventListener('keydown', onKey);
    m.onclick = (e) => { if (e.target === m) done(null); };
    $('dlg-cancel').onclick = () => done(null);
    $('dlg').onsubmit = (e) => { e.preventDefault(); done(input ? (field ? field.value : '') : true); };
  });
}

const passField = (id, ph = '') => `<div class="input-wrap">
  <input class="input" id="${id}" type="password" autocomplete="off" spellcheck="false" placeholder="${esc(ph)}">
  <button type="button" class="btn ghost icon-only eye" data-eye="${id}" title="${esc(t('show'))}" aria-label="${esc(t('show'))}">${icon('eye')}</button></div>`;

function bindEyes(root) {
  root.querySelectorAll('[data-eye]').forEach((b) => b.onclick = () => {
    const f = $(b.dataset.eye);
    const show = f.type === 'password';
    f.type = show ? 'text' : 'password';
    b.innerHTML = icon(show ? 'eye-off' : 'eye');
    b.title = show ? t('hide') : t('show');
    f.focus();
  });
}

/** Runs `action(passphrase)`, asking again (error shown) while the passphrase is wrong. Null if cancelled. */
async function withPassphrase(action) {
  let error = '';
  for (;;) {
    const pass = await dialog({
      title: t('unlockTitle'),
      text: t('unlockSub'),
      body: `${passField('dlg-pass', t('passphrase'))}<p class="error">${esc(error)}</p>`,
      input: true,
    });
    if (pass == null) return null;
    try { return await action(pass); } catch (e) {
      if (String(e) === '@wrong_passphrase' || String(e) === '@no_passphrase') { error = errText(e); continue; }
      throw e;
    }
  }
}

function serverState(s) {
  if (!s || !s.running) return 'stopped';
  if (s.public_url) return 'online';
  if (s.tunnel && s.tunnel.state === 'down') return 'down';
  return 'connecting';
}

async function startServer() {
  try { await invoke('start'); } catch (e) { toast(errText(e)); }
  await refreshStatus();
}
async function stopServer() {
  try { await invoke('stop'); } catch (e) { toast(errText(e)); }
  phones = invite = reqs = null;
  await refreshStatus();
}

// ------------------------------------------------------------------ shell

const PAGES = [
  ['home', 'house', 'navHome'],
  ['phones', 'smartphone', 'navPhones'],
  ['recordings', 'film', 'navRecordings'],
  ['giveback', 'send', 'navGiveBack'],
  ['settings', 'settings', 'navSettings'],
];

function renderNav() {
  const s = status || {};
  $('nav').innerHTML = PAGES.map(([id, ic, label]) => {
    let badge = '';
    if (id === 'recordings' && s.new_objects > 0) badge = '<span class="badge dot"></span>';
    if (id === 'giveback' && s.pending_requests > 0) badge = `<span class="badge">${s.pending_requests}</span>`;
    return `<button class="nav" data-page="${id}" ${page === id ? 'aria-current="page"' : ''}>${icon(ic)}<span class="label">${esc(t(label))}</span>${badge}</button>`;
  }).join('');
  $('nav').querySelectorAll('[data-page]').forEach((b) => b.onclick = () => go(b.dataset.page));

  const st = serverState(s);
  const led = { online: 'on', connecting: 'wait', down: 'bad', stopped: 'off' }[st];
  const label = { online: t('stOnline'), connecting: t('stConnecting'), down: t('stUnreachable'), stopped: t('stStopped') }[st];
  $('server-box').innerHTML = `<div class="server-line"><span class="led ${led}"></span><span class="label">${esc(label)}</span></div>
    <button class="btn ${s.running ? 'secondary' : ''} block" id="srv-toggle">${icon('power')}<span class="label">${esc(s.running ? t('stop') : t('start'))}</span></button>`;
  $('srv-toggle').onclick = () => (s.running ? stopServer() : startServer());
}

function go(p) {
  page = p;
  renderNav();
  renderPage();
  $('content').scrollTop = 0;
  loadPageData();
}

function renderPage() {
  const c = $('content');
  ({ home: renderHome, phones: renderPhones, recordings: renderRecordings, giveback: renderGiveBack, settings: renderSettings })[page](c);
}

async function loadPageData() {
  if (!status?.running) return;
  try {
    if (page === 'phones') {
      const [d, i] = await Promise.all([invoke('devices'), invoke('current_invite')]);
      phones = d; invite = i;
      if (page === 'phones' && !modalOpen()) renderPhones($('content'));
    }
    if (page === 'giveback') {
      reqs = await invoke('requests');
      if (page === 'giveback') renderLinks();
    }
  } catch (_) { /* server still starting */ }
}

const modalOpen = () => !$('modal').classList.contains('hidden');

// ------------------------------------------------------------------ home

function renderHome(c) {
  const s = status || {};
  const st = serverState(s);
  const hero = {
    online: ['on', 'circle-check', t('heroOnline'), s.devices === 0 ? t('todoFirstSub') : t('heroOnlineSub'),`<button class="btn lg" id="hero-btn">${icon('qr-code')}${esc(t('invitePhone'))}</button>`],
    connecting: ['wait', 'loader-circle', t('heroConnecting'), t('heroConnectingSub'), ''],
    down: ['bad', 'triangle-alert', t('heroDown'), s.tunnel?.error || t('heroDownSub'), `<button class="btn lg secondary" id="hero-btn">${icon('refresh-cw')}${esc(t('retry'))}</button>`],
    stopped: ['', 'power', t('heroStopped'), s.error || t('heroStoppedSub'), `<button class="btn lg" id="hero-btn">${icon('power')}${esc(t('startServer'))}</button>`],
  }[st];
  const todos = [];
  if (s.new_objects > 0) todos.push(['download', t('todoNew'), t('todoNewSub'), t('recover'), 'recordings']);
  if (s.pending_requests > 0) todos.push(['send', t('todoRequests', s.pending_requests), t('todoRequestsSub'), t('giveBack'), 'giveback']);
  const dash = (v) => (s.running ? esc(v) : '–');

  c.innerHTML = `<div class="page">
    <div class="page-head"><h1>${esc(overview.config.name)}</h1></div>
    <div class="card hero">
      <div class="hero-mark ${hero[0]}">${icon(hero[1], 'xl' + (st === 'connecting' ? ' spin' : ''))}</div>
      <div class="grow">
        <h2>${esc(hero[2])}</h2>
        <p class="muted">${esc(hero[3])}</p>
        ${st === 'online' ? `<div class="address">${icon('globe')}<span class="mono selectable">${esc(s.public_url)}</span></div>` : ''}
      </div>
      ${hero[4]}
    </div>
    ${todos.map(([ic, title, sub, label, target]) => `<div class="card todo">
      <div class="todo-icon">${icon(ic, 'lg')}</div>
      <div class="grow"><b>${esc(title)}</b><span class="muted">${esc(sub)}</span></div>
      <button class="btn secondary" data-go="${target}">${esc(label)}${icon('chevron-right')}</button></div>`).join('')}
    <div class="stats">
      <div class="stat"><b>${dash(s.devices)}</b><span>${esc(t('statPhones'))}</span></div>
      <div class="stat"><b>${dash(s.captures)}</b><span>${esc(t('statCaptures'))}</span></div>
      <div class="stat"><b>${dash(s.last_received_at ? relative(s.last_received_at, 'short') : t('never'))}</b><span>${esc(t('statLast'))}</span></div>
      <div class="stat"><b>${dash(fmtBytes(s.free_bytes))}</b><span>${esc(t('statFree'))}</span></div>
    </div>
  </div>`;
  const hb = $('hero-btn');
  if (hb) {
    hb.onclick = st === 'online' ? () => go('phones')
      : st === 'down' ? async () => { await stopServer(); await startServer(); }
      : startServer;
  }
  c.querySelectorAll('[data-go]').forEach((b) => b.onclick = () => go(b.dataset.go));
}

// ------------------------------------------------------------------ phones

function renderPhones(c) {
  const s = status || {};
  const online = serverState(s) === 'online';
  let inviteHtml;
  if (!online) {
    inviteHtml = `<div class="note">${icon('info')}<span>${esc(t('startToInvite'))}</span></div>`;
  } else if (invite) {
    const meta = invite.expires_at ? t('inviteValid', fmtDate(invite.expires_at), invite.max_uses) : t('inviteForever');
    inviteHtml = `<div class="invite">
      <div class="qr">${invite.svg}</div>
      <div class="stack">
        <ol class="steps"><li>${esc(t('step1'))}</li><li>${esc(t('step2'))}</li><li>${esc(t('step3'))}</li></ol>
        <p class="muted small">${esc(meta)}</p>
        <p class="small"><span class="muted">${esc(t('manualCode'))}</span> <b class="mono selectable">${esc(invite.code.replace(/(.{4})(?=.)/g, '$1 '))}</b></p>
        <div><button class="btn secondary" id="new-code">${icon('refresh-cw')}${esc(t('newCode'))}</button></div>
        ${s.tunnel?.mode === 'cloudflare' ? `<p class="muted small">${esc(t('tunnelHint'))}</p>` : ''}
      </div></div>`;
  } else {
    inviteHtml = inviteForm();
  }

  const devices = phones?.devices || [];
  const list = devices.length ? devices.map((d) => {
    const pct = d.quota_bytes ? Math.min(100, (100 * d.used_bytes) / d.quota_bytes) : 0;
    return `<div class="list-item">
      <div class="avatar">${icon('smartphone')}</div>
      <div class="grow">
        <div class="row"><b>${esc(d.name)}</b>${d.banned ? `<span class="tag bad">${icon('ban')}${esc(t('blocked'))}</span>` : ''}</div>
        <span class="muted small">${esc(d.last_upload_at ? t('lastUpload', relative(d.last_upload_at)) : t('noUpload'))}</span>
      </div>
      <div class="stack" style="gap:4px;align-items:flex-end">
        <span class="muted small">${esc(fmtBytes(d.used_bytes))} / ${esc(fmtBytes(d.quota_bytes))}</span>
        <div class="meter"><i style="width:${pct}%"></i></div>
      </div>
      <button class="btn ghost icon-only" title="${esc(t('rename'))}" aria-label="${esc(t('rename'))}" data-rename="${esc(d.id)}">${icon('pencil')}</button>
      <button class="btn ${d.banned ? 'secondary' : 'ghost'}" data-ban="${esc(d.id)}" data-banned="${d.banned}">${esc(d.banned ? t('unblock') : t('block'))}</button>
    </div>`;
  }).join('') : `<div class="empty">${icon('smartphone')}<p>${esc(t('noPhones'))}</p></div>`;

  const inviteCard = `<div class="card"><div class="card-head"><h2>${esc(t('inviteTitle'))}</h2></div>${inviteHtml}</div>`;
  const listCard = `<div class="card"><div class="card-head"><h2>${esc(t('registered', devices.length))}</h2></div><div class="list">${list}</div></div>`;
  c.innerHTML = `<div class="page">
    <div class="page-head"><h1>${esc(t('phonesTitle'))}</h1><p>${esc(t('phonesSub'))}</p></div>
    ${devices.length ? listCard + inviteCard : inviteCard + listCard}
  </div>`;

  if ($('new-code')) $('new-code').onclick = () => { invite = null; renderPhones(c); };
  bindInviteForm();
  c.querySelectorAll('[data-rename]').forEach((b) => b.onclick = async () => {
    const d = devices.find((x) => x.id === b.dataset.rename);
    const name = await dialog({ title: t('renameTitle'), text: t('renameHint'), body: `<input class="input" value="${esc(d.name)}" maxlength="60">`, input: true, ok: t('save') });
    if (!name || !name.trim() || name.trim() === d.name) return;
    try { await invoke('update_device', { id: d.id, name: name.trim(), banned: null }); toast(t('saved')); } catch (e) { toast(errText(e)); }
    loadPageData();
  });
  c.querySelectorAll('[data-ban]').forEach((b) => b.onclick = async () => {
    const d = devices.find((x) => x.id === b.dataset.ban);
    const ban = b.dataset.banned !== 'true';
    if (ban && !(await dialog({ title: t('blockConfirmTitle'), text: t('blockConfirm', d.name), ok: t('block'), danger: true }))) return;
    try { await invoke('update_device', { id: d.id, banned: ban, name: null }); } catch (e) { toast(errText(e)); }
    loadPageData();
  });
}

function inviteForm() {
  const chip = (h, label) => `<button class="chip" data-hours="${h}" aria-pressed="${inv.hours === h}">${esc(label)}</button>`;
  return `<div class="stack">
    <div class="field"><span>${esc(t('validFor'))}</span>
      <div class="chips">${chip(6, t('h6'))}${chip(24, t('h24'))}${chip(48, t('h48'))}${chip(168, t('d7'))}${chip(0, t('unlimited'))}</div></div>
    <div><button class="btn lg" id="create-invite" ${inv.busy ? 'disabled' : ''}>${icon('qr-code')}${esc(t('createCode'))}</button></div>
  </div>`;
}

function bindInviteForm() {
  document.querySelectorAll('[data-hours]').forEach((b) => b.onclick = () => {
    inv.hours = Number(b.dataset.hours);
    document.querySelectorAll('[data-hours]').forEach((x) => x.setAttribute('aria-pressed', String(Number(x.dataset.hours) === inv.hours)));
  });
  const btn = $('create-invite');
  if (!btn) return;
  btn.onclick = async () => {
    inv.busy = true; btn.disabled = true;
    try {
      await invoke('invite', { label: overview.config.name, hours: inv.hours, maxUses: inv.max });
      invite = await invoke('current_invite');
    } catch (e) { toast(errText(e)); }
    inv.busy = false;
    renderPhones($('content'));
  };
}

// ------------------------------------------------------------------ recordings

function renderRecordings(c) {
  const s = status || {};
  const fresh = s.new_objects > 0;
  const r = rec.result;
  c.innerHTML = `<div class="page">
    <div class="page-head"><h1>${esc(t('recTitle'))}</h1><p>${esc(t('recSub'))}</p></div>
    <div class="card stack">
      <div class="hero">
        <div class="hero-mark ${fresh ? 'wait' : 'on'}">${icon(fresh ? 'inbox' : 'circle-check', 'xl')}</div>
        <div class="grow"><h2>${esc(fresh ? t('recNew') : t('recUpToDate'))}</h2><p class="muted">${esc(fresh ? t('recNewSub') : t('recUpToDateSub'))}</p></div>
      </div>
      <div class="field"><span>${esc(t('saveTo'))}</span>
        <div class="row"><div class="path grow">${icon('folder-open')}<span class="mono">${esc(overview.config.recovery_dir)}</span></div>
        <button class="btn secondary" id="rec-dir">${esc(t('change'))}</button></div></div>
      <div class="row between">
        <button class="link-btn" id="rec-all">${esc(t('recoverAgain'))}</button>
        <button class="btn lg ${fresh ? '' : 'secondary'}" id="rec-go" ${rec.busy ? 'disabled' : ''}>${rec.busy ? icon('loader-circle', 'spin') + esc(t('decrypting')) : icon('download') + esc(t('recover'))}</button>
      </div>
      <p class="error">${esc(rec.err)}</p>
    </div>
    ${r ? `<div class="card stack">
      <div class="row">
        <div class="hero-mark on sm">${icon('check', 'lg')}</div>
        <div class="grow"><b>${esc(r.nothing_new ? t('recNothing') : t('recDone', r.sessions))}</b>
          ${r.unreadable ? `<p class="error">${esc(t('recUnreadable', r.unreadable))}</p>` : ''}</div>
        ${r.folder ? `<button class="btn secondary" id="rec-open">${icon('folder-open')}${esc(t('openFolder'))}</button>` : ''}
      </div>
      ${r.lines?.length ? `<div><button class="link-btn" id="rec-files">${esc(rec.showFiles ? t('hideFiles') : t('showFiles'))}</button></div>
        ${rec.showFiles ? `<pre class="files mono small selectable">${esc(r.lines.join('\n'))}</pre>` : ''}` : ''}
    </div>` : ''}
    <p class="muted small row">${icon('shield-check')}${esc(t('neverDeleted'))}</p>
  </div>`;
  $('rec-dir').onclick = async () => {
    const dir = await T.dialog.open({ directory: true, defaultPath: overview.config.recovery_dir });
    if (!dir) return;
    const cfg = overview.config;
    try {
      await invoke('save_settings', { name: cfg.name, reach: cfg.reach, acmeDomain: cfg.acme_domain, acmeEmail: cfg.acme_email, autostart: null, recoveryDir: dir });
      overview = await invoke('overview');
    } catch (e) { toast(errText(e)); }
    renderRecordings(c);
  };
  const run = async (all) => {
    rec.err = '';
    const result = await withPassphrase(async (pass) => {
      rec.busy = true; renderRecordings(c);
      try { return await invoke('recover', { passphrase: pass, out: overview.config.recovery_dir, all }); } finally { rec.busy = false; if (page === 'recordings') renderRecordings(c); }
    }).catch((e) => { rec.err = errText(e); return null; });
    if (result) { rec.result = result; rec.showFiles = false; }
    await refreshStatus();
    if (page === 'recordings') renderRecordings(c);
  };
  $('rec-go').onclick = () => run(false);
  $('rec-all').onclick = () => run(true);
  if ($('rec-open')) $('rec-open').onclick = () => T.opener.openPath(r.folder);
  if ($('rec-files')) $('rec-files').onclick = () => { rec.showFiles = !rec.showFiles; renderRecordings(c); };
}

// ------------------------------------------------------------------ give back

function renderGiveBack(c) {
  const labels = [t('gbStep1'), t('gbStep2'), t('gbStep3')];
  const stepper = `<div class="stepper">${labels.map((s, i) => {
    const n = i + 1;
    const finished = n < gb.step || (n === 3 && gb.link);
    const cls = finished ? 'done' : n === gb.step ? 'current' : '';
    return `<div class="${cls}"><span>${finished ? icon('check') : n}</span>${esc(s)}</div>`;
  }).join('')}</div>`;
  let body = '';
  if (!status?.running) {
    body = `<div class="note">${icon('info')}<span>${esc(t('heroStoppedSub'))}</span></div>`;
  } else if (gb.step === 1) {
    body = `<form class="stack" id="gb-form">
      <div><b>${esc(t('gbAsk'))}</b><p class="muted small">${esc(t('gbAskSub'))}</p></div>
      <input class="input big mono" id="gb-code" maxlength="9" placeholder="K7F3-QX9M" value="${esc(gb.code)}" autocomplete="off" spellcheck="false">
      <p class="error">${esc(gb.err)}</p>
      <div class="row between"><span></span><button class="btn lg" type="submit" id="gb-next" ${gb.code.length < 9 || gb.busy ? 'disabled' : ''}>${esc(t('continue'))}${icon('chevron-right')}</button></div>
    </form>`;
  } else if (gb.step === 2) {
    const f = gb.found;
    const chip = (d, label) => `<button type="button" class="chip" data-days="${d}" aria-pressed="${gb.days === d}">${esc(label)}</button>`;
    body = gb.busy
      ? `<div class="hero"><div class="hero-mark wait">${icon('loader-circle', 'xl spin')}</div><div><h2>${esc(t('preparing'))}</h2><p class="muted">${esc(t('preparingSub'))}</p></div></div>`
      : `<div class="stack">
      <div class="row"><div class="avatar">${icon('smartphone')}</div>
        <div class="grow"><b>${esc(f.device_name || '?')}</b><p class="muted small">${esc(t('gbFound', f.objects, fmtBytes(f.bytes)))}</p></div>
        <span class="tag mono">${esc(f.code)}</span></div>
      <div class="note">${icon('info')}<span>${esc(t('gbCheckHint'))}</span></div>
      ${f.cancelled ? `<div class="note warn">${icon('triangle-alert')}<span>${esc(t('gbCancelled'))}</span></div>` : `
      <div class="field"><span>${esc(t('gbLinkValid'))}</span><div class="chips">${chip(1, t('d1'))}${chip(3, t('d3'))}${chip(7, t('d7'))}${chip(30, t('d30'))}</div></div>`}
      <p class="error">${esc(gb.err)}</p>
      <div class="row between"><button class="btn ghost" id="gb-back">${icon('arrow-left')}${esc(t('back'))}</button>
        ${f.cancelled ? '' : `<button class="btn lg" id="gb-create">${icon('link')}${esc(t('createLink'))}</button>`}</div>
    </div>`;
  } else {
    body = `<div class="result">
      <div class="qr">${gb.svg}</div>
      <div class="stack">
        <h2 class="row ok-text">${icon('circle-check')}${esc(t('gbReady'))}</h2>
        <div class="linkbox"><span class="mono selectable">${esc(gb.link)}</span><button class="btn" id="gb-copy">${icon('copy')}${esc(t('copy'))}</button></div>
        <ul class="tips muted small"><li>${esc(t('gbReadyTip1'))}</li><li>${esc(t('gbReadyTip2'))}</li></ul>
        <div><button class="btn secondary" id="gb-again">${esc(t('another'))}</button></div>
      </div></div>`;
  }
  c.innerHTML = `<div class="page">
    <div class="page-head"><h1>${esc(t('gbTitle'))}</h1><p>${esc(t('gbSub'))}</p></div>
    <div class="card stack">${stepper}${body}</div>
    <div class="card"><div class="card-head"><h2>${esc(t('linksTitle'))}</h2></div><div class="list" id="links"></div></div>
  </div>`;
  renderLinks();

  const code = $('gb-code');
  if (code) {
    code.focus();
    code.oninput = () => {
      const raw = code.value.toUpperCase().replace(/[^A-Z0-9]/g, '').slice(0, 8);
      code.value = raw.length > 4 ? raw.slice(0, 4) + '-' + raw.slice(4) : raw;
      gb.code = code.value;
      $('gb-next').disabled = gb.code.length < 9;
    };
    $('gb-form').onsubmit = async (e) => {
      e.preventDefault();
      gb.err = ''; gb.busy = true;
      try { gb.found = await invoke('lookup_request', { code: gb.code }); gb.step = 2; } catch (er) { gb.err = errText(er); }
      gb.busy = false;
      renderGiveBack(c);
    };
  }
  c.querySelectorAll('[data-days]').forEach((b) => b.onclick = () => { gb.days = Number(b.dataset.days); renderGiveBack(c); });
  if ($('gb-back')) $('gb-back').onclick = () => { gb.step = 1; gb.err = ''; renderGiveBack(c); };
  if ($('gb-create')) $('gb-create').onclick = async () => {
    gb.err = '';
    const r = await withPassphrase(async (pass) => {
      gb.busy = true; renderGiveBack(c);
      try { return await invoke('hand_back', { code: gb.code, passphrase: pass, days: gb.days }); } finally { gb.busy = false; if (page === 'giveback') renderGiveBack(c); }
    }).catch((e) => { gb.err = errText(e); return null; });
    if (r) { gb.link = r.link; gb.svg = r.svg; gb.step = 3; }
    if (page === 'giveback') renderGiveBack(c);
    loadPageData();
  };
  if ($('gb-copy')) $('gb-copy').onclick = async () => { await navigator.clipboard.writeText(gb.link); toast(t('copied')); };
  if ($('gb-again')) $('gb-again').onclick = () => { Object.assign(gb, { step: 1, code: '', found: null, link: null, svg: null, err: '' }); renderGiveBack(c); };
}

const LINK_STATE = { uploading: ['lkUploading', 'warn'], ready: ['lkReady', 'ok'], opened: ['lkOpened', 'ok'], done: ['lkDone', ''], expired: ['lkExpired', 'bad'], revoked: ['lkRevoked', 'bad'] };

function renderLinks() {
  const el = $('links');
  if (!el) return;
  const items = (reqs?.requests || []).filter((r) => !r.cancelled);
  if (!items.length) { el.innerHTML = `<div class="empty">${icon('send')}<p>${esc(t('noLinks'))}</p></div>`; return; }
  el.innerHTML = items.map((r) => {
    const sh = r.share;
    const [k, cls] = sh ? LINK_STATE[sh.state] || [sh.state, ''] : ['lkWaiting', 'warn'];
    const live = sh && ['uploading', 'ready', 'opened'].includes(sh.state);
    const when = sh?.state === 'ready' ? t('expiresOn', fmtDate(sh.expires_at)) : fmtDate(sh?.closed_at || sh?.created_at || r.created_at);
    return `<div class="list-item">
      <div class="avatar">${icon('smartphone')}</div>
      <div class="grow"><b>${esc(r.device_name || '?')}</b> <span class="muted small mono">${esc(r.code)}</span>
        <div class="muted small">${esc(when)}</div></div>
      <span class="tag ${cls}">${esc(t(k))}</span>
      ${live ? `<button class="btn ghost" data-revoke="${esc(sh.id)}">${esc(t('revoke'))}</button>` : ''}
      ${!sh ? `<button class="btn secondary" data-answer="${esc(r.code)}">${esc(t('answer'))}${icon('chevron-right')}</button>` : ''}
    </div>`;
  }).join('');
  el.querySelectorAll('[data-answer]').forEach((b) => b.onclick = async () => {
    gb.code = b.dataset.answer; gb.err = '';
    try { gb.found = await invoke('lookup_request', { code: gb.code }); gb.step = 2; } catch (e) { gb.err = errText(e); gb.step = 1; }
    renderGiveBack($('content'));
    $('content').scrollTop = 0;
  });
  el.querySelectorAll('[data-revoke]').forEach((b) => b.onclick = async () => {
    if (!(await dialog({ title: t('revokeTitle'), text: t('revokeText'), ok: t('revoke'), danger: true }))) return;
    try { await invoke('revoke_share', { id: b.dataset.revoke }); } catch (e) { toast(errText(e)); }
    loadPageData();
  });
}

// ------------------------------------------------------------------ settings

function renderSettings(c) {
  const cfg = overview.config;
  const s = status || {};
  c.innerHTML = `<div class="page">
    <div class="page-head"><h1>${esc(t('setTitle'))}</h1></div>

    <div class="card"><div class="card-head"><h2>${esc(t('setGeneral'))}</h2></div>
      <div class="setting"><div class="grow"><b>${esc(t('setName'))}</b><p>${esc(t('setNameSub'))}</p></div>
        <input class="input" id="set-name" value="${esc(cfg.name)}" maxlength="60" style="width:220px">
        <button class="btn secondary" id="set-name-save">${esc(t('save'))}</button></div>
      <div class="setting"><div class="grow"><b>${esc(t('setAutostart'))}</b><p>${esc(t('setAutostartSub'))}</p></div>
        <label class="switch"><input type="checkbox" id="set-auto" ${cfg.autostart ? 'checked' : ''}><span></span></label></div>
    </div>

    <div class="card stack"><h2>${esc(t('setAccess'))}</h2>
      <label class="choice"><input type="radio" name="reach" value="tunnel" ${cfg.reach !== 'https' ? 'checked' : ''}>
        <div><b>${esc(t('accAuto'))}</b><p>${esc(t('accAutoSub'))}</p></div></label>
      <label class="choice"><input type="radio" name="reach" value="https" ${cfg.reach === 'https' ? 'checked' : ''}>
        <div class="grow"><b>${esc(t('accDomain'))}</b><p>${esc(t('accDomainSub'))}</p></div></label>
      <div class="row wrap ${cfg.reach === 'https' ? '' : 'hidden'}" id="domain-fields">
        <label class="field grow"><span>${esc(t('domain'))}</span><input class="input" id="set-domain" placeholder="${esc(t('domainPh'))}" value="${esc(cfg.acme_domain || '')}"></label>
        <label class="field grow"><span>${esc(t('email'))}</span><input class="input" id="set-email" value="${esc(cfg.acme_email || '')}"></label>
      </div>
      <div class="row between hidden" id="reach-save-row"><p class="muted small">${s.running ? esc(t('restartNote')) : ''}</p>
        <button class="btn secondary" id="set-reach-save">${esc(t('save'))}</button></div>
      <p class="error" id="set-err"></p>
    </div>

    <div class="card"><div class="card-head"><h2>${esc(t('setStorage'))}</h2></div>
      <div class="setting"><div class="grow"><b>${esc(t('setData'))}</b><p class="mono small">${esc(cfg.data_dir)}</p>${s.free_bytes != null ? `<p>${esc(t('freeSpace', fmtBytes(s.free_bytes)))}</p>` : ''}</div>
        <button class="btn secondary" id="open-data">${icon('folder-open')}${esc(t('open'))}</button></div>
      <div class="setting"><div class="grow"><b>${esc(t('setRecovered'))}</b><p class="mono small">${esc(cfg.recovery_dir)}</p></div>
        <button class="btn secondary" id="open-rec">${icon('folder-open')}${esc(t('open'))}</button></div>
    </div>

    <div class="card"><div class="card-head"><h2>${esc(t('setSecurity'))}</h2></div>
      <div class="setting"><div class="grow"><b>${esc(t('setKey'))}</b><p>${esc(t('setKeySub'))}</p><p class="mono small">${esc(cfg.fingerprint)}</p></div>
        <button class="btn secondary" id="save-key">${icon('key-round')}${esc(t('saveKeyCopy'))}</button></div>
    </div>

    <div class="card"><div class="card-head"><h2>${esc(t('setAdvanced'))}</h2></div>
      <div class="setting"><div class="grow"><b>${esc(t('setAdmin'))}</b><p>${esc(t('setAdminSub'))}</p></div>
        <button class="btn secondary" id="open-admin" ${s.running ? '' : 'disabled'}>${icon('external-link')}${esc(t('openAdmin'))}</button></div>
      <div class="setting"><div class="grow"><b>${esc(t('setAbout'))}</b><p>${esc(t('aboutText'))}</p></div>
        <button class="btn ghost" id="source">${icon('external-link')}${esc(t('sourceCode'))}</button></div>
    </div>
  </div>`;

  const saveAll = async (extra = {}) => {
    const args = {
      name: $('set-name').value.trim() || cfg.name,
      reach: document.querySelector('input[name=reach]:checked').value,
      acmeDomain: $('set-domain').value, acmeEmail: $('set-email').value,
      autostart: null, recoveryDir: null, ...extra,
    };
    try {
      await invoke('save_settings', args);
    } catch (e) {
      if (String(e) !== '@stop_first') throw e;
      // The name and the Internet access only change on a restart.
      await invoke('stop');
      await invoke('save_settings', args);
      await invoke('start');
    }
    overview = await invoke('overview');
    $('brand-name').textContent = overview.config.name;
    await refreshStatus();
  };
  const dirty = () => $('reach-save-row').classList.remove('hidden');
  document.querySelectorAll('input[name=reach]').forEach((r) => r.onchange = () => {
    $('domain-fields').classList.toggle('hidden', document.querySelector('input[name=reach]:checked').value !== 'https');
    dirty();
  });
  $('set-domain').oninput = dirty;
  $('set-email').oninput = dirty;
  $('set-name-save').onclick = () => saveAll().then(() => toast(t('saved'))).catch((e) => toast(errText(e)));
  $('set-reach-save').onclick = () => saveAll()
    .then(() => { toast(t('saved')); renderSettings(c); })
    .catch((e) => { $('set-err').textContent = errText(e); });
  $('set-auto').onchange = (e) => saveAll({ autostart: e.target.checked }).then(() => toast(t('saved'))).catch((er) => toast(errText(er)));
  $('open-data').onclick = () => T.opener.openPath(cfg.data_dir).catch((e) => toast(errText(e)));
  $('open-rec').onclick = () => T.opener.openPath(cfg.recovery_dir).catch(() => T.opener.openPath(cfg.data_dir));
  $('save-key').onclick = () => exportKey().then((ok) => ok && toast(t('keySaved')));
  $('open-admin').onclick = () => invoke('open_admin').catch((e) => toast(errText(e)));
  $('source').onclick = () => T.opener.openUrl('https://github.com/temoin-app/temoin');
}

async function exportKey() {
  const dest = await T.dialog.save({ defaultPath: LANG === 'fr' ? 'temoin-cle.age' : 'temoin-key.age', filters: [{ name: 'Témoin', extensions: ['age'] }] });
  if (!dest) return false;
  try { await invoke('export_key', { dest }); return true; } catch (e) { toast(errText(e)); return false; }
}

// ------------------------------------------------------------------ first run

const ob = { step: 0, name: '', dir: '', pass: '', pass2: '', noted: false, saved: false, err: '', busy: false, free: null };
const OB_STEPS = 6;

/** 1 too short, 2 weak, 3 good, 4 strong (0 when empty). */
function strength(p) {
  if (!p) return 0;
  if (p.length < 10) return 1;
  if (/^(.)\1+$/.test(p) || /^(0123|1234|abcd|azer|qwer|password|motdepasse)/i.test(p)) return 2;
  if (p.length >= 24 || (p.trim().split(/\s+/).length >= 4 && p.length >= 18)) return 4;
  return p.length >= 16 ? 3 : 2;
}

function renderOnboarding() {
  const card = $('ob-card');
  const dots = `<div class="ob-dots">${Array.from({ length: OB_STEPS - 1 }, (_, i) => `<i class="${i < ob.step ? 'on' : ''}"></i>`).join('')}</div>`;
  const back = ob.step > 0 && ob.step < 4 ? `<button class="btn ghost" id="ob-back">${icon('arrow-left')}${esc(t('back'))}</button>` : '<span></span>';
  const next = (label, ic = 'chevron-right', extra = '') => `<button class="btn lg" id="ob-next" ${extra}>${esc(label)}${icon(ic)}</button>`;
  let html = '';
  if (ob.step === 0) {
    html = `<img class="ob-logo" src="logo.png" alt="">
      <h1>${esc(t('obWelcome'))}</h1><p class="lead">${esc(t('obWelcomeSub'))}</p>
      <div class="ob-body features">
        <div>${icon('shield-check')}<div><b>${esc(t('obF1'))}</b><p class="muted">${esc(t('obF1Sub'))}</p></div></div>
        <div>${icon('globe')}<div><b>${esc(t('obF2'))}</b><p class="muted">${esc(t('obF2Sub'))}</p></div></div>
        <div>${icon('send')}<div><b>${esc(t('obF3'))}</b><p class="muted">${esc(t('obF3Sub'))}</p></div></div>
      </div>
      <div class="ob-foot"><span></span>${next(t('getStarted'))}</div>`;
  } else if (ob.step === 1) {
    html = `${dots}<h1>${esc(t('obName'))}</h1><p class="lead">${esc(t('obNameSub'))}</p>
      <div class="ob-body"><input class="input" id="ob-name" maxlength="60" placeholder="${esc(t('obNamePh'))}" value="${esc(ob.name)}"></div>
      <div class="ob-foot">${back}${next(t('continue'))}</div>`;
  } else if (ob.step === 2) {
    html = `${dots}<h1>${esc(t('obFolder'))}</h1><p class="lead">${esc(t('obFolderSub'))}</p>
      <div class="ob-body"><div class="row"><div class="path grow">${icon('hard-drive')}<span class="mono">${esc(ob.dir)}</span></div>
        <button class="btn secondary" id="ob-dir">${esc(t('change'))}</button></div>
        ${ob.free != null ? `<p class="muted small">${esc(t('freeSpace', fmtBytes(ob.free)))}</p>` : ''}</div>
      <div class="ob-foot">${back}${next(t('continue'))}</div>`;
  } else if (ob.step === 3) {
    const s = strength(ob.pass);
    html = `${dots}<h1>${esc(t('obPass'))}</h1><p class="lead">${esc(t('obPassSub'))}</p>
      <div class="ob-body">
        ${passField('ob-pass', t('obPassExample'))}
        <div class="row"><div class="strength s${s} grow"><i></i><i></i><i></i><i></i></div>
          <span class="muted small strength-label" id="ob-str">${s ? esc(t('strength')[s - 1]) : ''}</span></div>
        ${passField('ob-pass2', t('obPassAgain'))}
        <label class="check" id="ob-noted-row"><input type="checkbox" id="ob-noted" ${ob.noted ? 'checked' : ''}><span>${esc(t('obPassNoted'))}</span></label>
        <p class="error">${esc(ob.err)}</p>
      </div>
      <div class="ob-foot">${back}<button class="btn lg" id="ob-next" ${ob.busy ? 'disabled' : ''}>${ob.busy ? icon('loader-circle', 'spin') + esc(t('creatingKey')) : icon('key-round') + esc(t('createKey'))}</button></div>`;
  } else if (ob.step === 4) {
    html = `${dots}<h1>${esc(t('obBackup'))}</h1><p class="lead">${esc(t('obBackupSub'))}</p>
      ${ob.saved ? `<div class="ob-body"><div class="note ok">${icon('circle-check')}<span>${esc(t('keySaved'))}</span></div></div>` : ''}
      <div class="ob-foot">${ob.saved
        ? `<span></span>${next(t('continue'))}`
        : `<button class="btn ghost" id="ob-later">${esc(t('later'))}</button><button class="btn lg" id="ob-save">${icon('key-round')}${esc(t('saveKeyCopy'))}</button>`}</div>`;
  } else {
    html = `${dots}<div class="hero-mark on ob-done">${icon('circle-check', 'xl')}</div>
      <h1>${esc(t('obReady'))}</h1><p class="lead">${esc(t('obReadySub'))}</p>
      <div class="ob-foot"><span></span>${next(t('openApp'))}</div>`;
  }
  card.innerHTML = html;
  bindEyes(card);
  if ($('ob-back')) $('ob-back').onclick = () => { ob.step--; ob.err = ''; renderOnboarding(); };
  const nextBtn = $('ob-next');

  if (ob.step === 1) {
    const f = $('ob-name'); f.focus();
    f.oninput = () => { ob.name = f.value; };
    f.onkeydown = (e) => { if (e.key === 'Enter') nextBtn.click(); };
  }
  if (ob.step === 2) {
    $('ob-dir').onclick = async () => {
      const d = await T.dialog.open({ directory: true, defaultPath: ob.dir });
      if (d) { ob.dir = d; ob.free = await invoke('free_space', { path: d }); renderOnboarding(); }
    };
  }
  if (ob.step === 3) {
    const p = $('ob-pass'), p2 = $('ob-pass2');
    p.value = ob.pass; p2.value = ob.pass2; (ob.pass ? p2 : p).focus();
    p.oninput = () => {
      ob.pass = p.value;
      const s = strength(ob.pass);
      card.querySelector('.strength').className = `strength s${s} grow`;
      $('ob-str').textContent = s ? t('strength')[s - 1] : '';
    };
    p2.oninput = () => { ob.pass2 = p2.value; };
    p2.onkeydown = (e) => { if (e.key === 'Enter') nextBtn.click(); };
    $('ob-noted').onchange = (e) => { ob.noted = e.target.checked; $('ob-noted-row').classList.remove('invalid'); };
  }
  if (ob.step === 4) {
    if ($('ob-save')) $('ob-save').onclick = async () => { if (await exportKey()) { ob.saved = true; renderOnboarding(); } };
    if ($('ob-later')) $('ob-later').onclick = async () => {
      if (await dialog({ title: t('backupLaterTitle'), text: t('backupLaterText'), ok: t('continue') })) { ob.step = 5; startServer(); renderOnboarding(); }
    };
  }

  if (nextBtn) nextBtn.onclick = async () => {
    ob.err = '';
    if (ob.step === 1 && !ob.name.trim()) { ob.name = t('obNamePh'); }
    if (ob.step === 3) {
      if (ob.pass.length < 10) { ob.err = errText('@passphrase_short'); return renderOnboarding(); }
      if (ob.pass !== ob.pass2) { ob.err = t('passMismatch'); return renderOnboarding(); }
      if (!ob.noted) { $('ob-noted-row').classList.add('invalid'); return; }
      ob.busy = true; renderOnboarding();
      try {
        await invoke('setup', { name: ob.name.trim(), dataDir: ob.dir, passphrase: ob.pass });
        ob.pass = ob.pass2 = '';
        overview = await invoke('overview');
        ob.step = 4;
      } catch (e) { ob.err = errText(e); }
      ob.busy = false;
      return renderOnboarding();
    }
    if (ob.step === 4) { ob.step = 5; startServer(); return renderOnboarding(); }
    if (ob.step === 5) return showApp();
    ob.step++;
    renderOnboarding();
  };
}

// ------------------------------------------------------------------ boot

async function refreshStatus() {
  try { status = await invoke('status'); } catch (_) { return; }
  if ($('app').classList.contains('hidden')) return;
  renderNav();
  // Pages without text fields follow the server live; the others only refresh their lists.
  if (page === 'home' || (page === 'recordings' && !rec.busy)) renderPage();
  if (page === 'phones' && !modalOpen()) renderPhones($('content'));
}

let pollTimer = null;
async function showApp() {
  $('onboarding').classList.add('hidden');
  $('app').classList.remove('hidden');
  $('brand-name').textContent = overview.config.name;
  status = await invoke('status').catch(() => null);
  if (overview.config.autostart && !status?.running) await startServer();
  go('home');
  clearInterval(pollTimer);
  pollTimer = setInterval(async () => {
    await refreshStatus();
    loadPageData();
  }, 2500);
}

async function boot() {
  document.documentElement.lang = LANG;
  document.title = t('appTitle');
  overview = await invoke('overview');
  if (overview.configured) return showApp();
  ob.dir = overview.default_data_dir;
  ob.free = await invoke('free_space', { path: ob.dir }).catch(() => null);
  $('onboarding').classList.remove('hidden');
  renderOnboarding();
}
boot();
