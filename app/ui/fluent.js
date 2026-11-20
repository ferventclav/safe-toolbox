/* Fluent Desktop behaviour. No dependencies. window.FluentDesktop.init() wires any markup added later. */
(function () {
  'use strict';
  var RAMP = {"accent-light-1":[0.078,1.134],"accent-light-2":[0.2532,0.88],"accent-light-3":[0.3526,0.849],"accent-dark-1":[-0.0846,0.889],"accent-dark-2":[-0.1981,0.694],"accent-dark-3":[-0.3398,0.538]};
  var done = new WeakSet();

  function once(el) { if (done.has(el)) return false; done.add(el); return true; }
  function all(root, sel) { return Array.prototype.slice.call(root.querySelectorAll(sel)); }

  /* Colour maths: sRGB to OKLCH and back, used to build an accent ramp from one colour. */
  function hexRgb(h) { h = h.replace('#', ''); if (h.length === 3) h = h.replace(/./g, '$&$&'); return [0, 2, 4].map(function (i) { return parseInt(h.substr(i, 2), 16) / 255; }); }
  function lin(c) { return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); }
  function gam(c) { return c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(c, 1 / 2.4) - 0.055; }
  function toLch(hex) {
    var c = hexRgb(hex).map(lin), r = c[0], g = c[1], b = c[2];
    var l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b), m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b), s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
    var L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s, A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s, B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    return [L, Math.sqrt(A * A + B * B), Math.atan2(B, A)];
  }
  function rawRgb(L, C, H) {
    var A = C * Math.cos(H), B = C * Math.sin(H), l = Math.pow(L + 0.3963377774 * A + 0.2158037573 * B, 3), m = Math.pow(L - 0.1055613458 * A - 0.0638541728 * B, 3), s = Math.pow(L - 0.0894841775 * A - 1.291485548 * B, 3);
    return [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s];
  }
  function toHex(L, C, H) {
    L = Math.min(1, Math.max(0, L));
    for (var i = 0; i < 24; i++) { var t = rawRgb(L, C, H); if (t.every(function (v) { return v >= -0.0005 && v <= 1.0005; })) break; C *= 0.94; }
    return '#' + rawRgb(L, C, H).map(function (v) { return ('0' + Math.round(Math.min(1, Math.max(0, gam(Math.max(0, v)))) * 255).toString(16)).slice(-2); }).join('');
  }
  function lum(hex) { var c = hexRgb(hex).map(lin); return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]; }
  function contrast(a, b) { var x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); }

  function ramp(hex) {
    var b = toLch(hex), out = { accent: hex.toLowerCase() };
    Object.keys(RAMP).forEach(function (k) { var d = RAMP[k]; out[k] = toHex(b[0] + d[0], b[1] * d[1], b[2]); });
    var t = toLch(out['accent-dark-1']), i = 0;
    while (contrast(out['accent-dark-1'], '#ffffff') < 4.5 && i++ < 40) { t[0] -= 0.01; out['accent-dark-1'] = toHex(t[0], t[1], t[2]); }
    t = toLch(out['accent-light-2']); i = 0;
    while (contrast(out['accent-light-2'], '#000000') < 7 && i++ < 40) { t[0] += 0.01; out['accent-light-2'] = toHex(t[0], t[1], t[2]); }
    return out;
  }
  function decl(r, dark) {
    var fill = dark ? r['accent-light-2'] : r['accent-dark-1'];
    var keys = ['accent', 'accent-light-1', 'accent-light-2', 'accent-light-3', 'accent-dark-1', 'accent-dark-2', 'accent-dark-3'];
    return keys.map(function (k) { return '--' + k + ':' + r[k]; }).concat([
      '--accent-fill-secondary:' + fill + 'e6', '--accent-fill-tertiary:' + fill + 'cc', '--mica-wash:' + r.accent + (dark ? '1f' : '17')
    ]).join(';');
  }
  function setAccent(hex) {
    var r = ramp(hex), id = 'fd-accent', el = document.getElementById(id);
    if (!el) { el = document.createElement('style'); el.id = id; document.head.appendChild(el); }
    el.textContent = ':root{' + decl(r, false) + '}@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){' + decl(r, true) + '}}:root[data-theme="dark"]{' + decl(r, true) + '}:root[data-theme="light"]{' + decl(r, false) + '}';
    return r;
  }
  function resetAccent() { var el = document.getElementById('fd-accent'); if (el) el.remove(); }

  /* Places a popover under its trigger, flipping up and clamping to the window. */
  function place(pop, btn) {
    var a = btn.getBoundingClientRect(), w = pop.offsetWidth, h = pop.offsetHeight, gap = 8;
    var x = Math.max(8, Math.min(a.left, innerWidth - w - 8)), y = a.bottom + gap;
    if (y + h > innerHeight - 8 && a.top - gap - h > 8) y = a.top - gap - h;
    pop.style.left = x + 'px'; pop.style.top = Math.max(8, y) + 'px';
  }
  function wirePopovers(root) {
    all(root, '[data-fd-open]').forEach(function (btn) {
      if (!once(btn)) return;
      btn.addEventListener('click', function () {
        var pop = document.getElementById(btn.getAttribute('data-fd-open'));
        if (!pop) return;
        if (pop.tagName === 'DIALOG') { pop.showModal(); return; }
        if (!pop.showPopover) { pop.hidden = !pop.hidden; return; }
        if (pop.matches(':popover-open')) { pop.hidePopover(); return; }
        pop.showPopover(); place(pop, btn);
        if (btn.classList.contains('fd-combo')) pop.style.minWidth = btn.offsetWidth + 'px';
        var first = pop.querySelector('[aria-selected="true"],.fd-menu-item'); if (first) first.focus();
        pop._fdBtn = btn;
      });
    });
    all(root, 'dialog.fd-dialog [data-fd-close]').forEach(function (b) { if (once(b)) b.addEventListener('click', function () { b.closest('dialog').close(); }); });
    all(root, '.fd-menu').forEach(function (menu) {
      if (!once(menu)) return;
      menu.addEventListener('keydown', function (e) {
        var items = all(menu, '.fd-menu-item'), i = items.indexOf(document.activeElement), n = items.length;
        if (e.key === 'ArrowDown') { items[(i + 1) % n].focus(); e.preventDefault(); }
        if (e.key === 'ArrowUp') { items[(i - 1 + n) % n].focus(); e.preventDefault(); }
        if (e.key === 'Home') { items[0].focus(); e.preventDefault(); }
        if (e.key === 'End') { items[n - 1].focus(); e.preventDefault(); }
      });
      menu.addEventListener('click', function (e) {
        var it = e.target.closest('.fd-menu-item'); if (!it) return;
        var btn = menu._fdBtn;
        if (btn && btn.classList.contains('fd-combo')) {
          all(menu, '.fd-menu-item').forEach(function (x) { x.setAttribute('aria-selected', x === it ? 'true' : 'false'); });
          var label = btn.querySelector('span'); if (label) label.textContent = it.textContent.trim();
        }
        if (menu.hidePopover && menu.matches(':popover-open')) { menu.hidePopover(); if (btn) btn.focus(); }
      });
    });
  }
  /* One selected item per group, with arrow keys, for tabs, selector bars and lists. */
  function wireGroups(root) {
    [['[role="tablist"]', '[role="tab"]'], ['[role="listbox"]:not(.fd-menu)', '[role="option"]']].forEach(function (p) {
      all(root, p[0]).forEach(function (g) {
        if (!once(g)) return;
        g.addEventListener('click', function (e) {
          if (e.target.closest('[data-fd-noselect]')) return;
          var it = e.target.closest(p[1]); if (it) pick(g, p[1], it);
        });
        g.addEventListener('keydown', function (e) {
          var keys = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }, d = keys[e.key]; if (!d) return;
          var items = all(g, p[1]), i = items.indexOf(document.activeElement); if (i < 0) return;
          var n = items[(i + d + items.length) % items.length]; n.focus(); pick(g, p[1], n); e.preventDefault();
        });
      });
    });
  }
  function pick(g, sel, it) {
    all(g, sel).forEach(function (x) {
      var on = x === it; x.setAttribute('aria-selected', on ? 'true' : 'false');
      if (x.getAttribute('role') === 'tab') x.tabIndex = on ? 0 : -1;
      var id = x.getAttribute('aria-controls'), panel = id && document.getElementById(id); if (panel) panel.hidden = !on;
    });
  }
  function wireControls(root) {
    all(root, '.fd-slider').forEach(function (s) {
      if (!once(s)) return;
      var f = function () { var a = +s.min || 0, b = s.max === '' ? 100 : +s.max; s.style.setProperty('--v', ((s.value - a) / (b - a) * 100) + '%'); };
      s.addEventListener('input', f); f();
    });
    all(root, 'input[data-fd-indeterminate]').forEach(function (c) { c.indeterminate = true; });
    all(root, '[data-fd-toggle-press]').forEach(function (b) { if (once(b)) b.addEventListener('click', function () { b.setAttribute('aria-pressed', b.getAttribute('aria-pressed') === 'true' ? 'false' : 'true'); }); });
    all(root, '[data-fd-reveal]').forEach(function (b) { if (once(b)) b.addEventListener('click', function () { var i = b.parentNode.querySelector('input'); var show = i.type === 'password'; i.type = show ? 'text' : 'password'; b.setAttribute('aria-pressed', show ? 'true' : 'false'); }); });
    all(root, '[data-fd-step]').forEach(function (b) { if (once(b)) b.addEventListener('click', function () { var i = b.parentNode.querySelector('input'); if (b.getAttribute('data-fd-step') === 'up') i.stepUp(); else i.stepDown(); i.dispatchEvent(new Event('input', { bubbles: true })); }); });
    all(root, '[data-fd-clear]').forEach(function (b) { if (once(b)) b.addEventListener('click', function () { var i = b.parentNode.querySelector('input'); i.value = ''; i.focus(); }); });
    all(root, '[data-fd-dismiss]').forEach(function (b) { if (once(b)) b.addEventListener('click', function () { var t = b.closest('.fd-infobar,.fd-teach'); if (t) t.hidden = true; }); });
    all(root, '.fd-tree-item[aria-expanded]').forEach(function (b) { if (once(b)) b.addEventListener('click', function () { b.setAttribute('aria-expanded', b.getAttribute('aria-expanded') === 'true' ? 'false' : 'true'); }); });
    all(root, '.fd-tree').forEach(function (t) { if (once(t)) t.addEventListener('click', function (e) { var it = e.target.closest('.fd-tree-item'); if (!it) return; all(t, '.fd-tree-item').forEach(function (x) { x.setAttribute('aria-selected', x === it ? 'true' : 'false'); }); }); });
  }
  function init(root) { root = root || document; wirePopovers(root); wireGroups(root); wireControls(root); return api; }

  var api = { init: init, setAccent: setAccent, resetAccent: resetAccent, accentRamp: ramp, contrast: contrast };
  window.FluentDesktop = api;
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', function () { init(); }); else init();
})();
