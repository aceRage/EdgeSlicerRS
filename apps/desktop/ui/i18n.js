'use strict';

// The page's words in the language chosen: gettext's behaviour over the
// table the session hands the page (`language`, src/api/i18n.rs), which
// tools/make_translations.py makes from the reference's catalogs.
//
// Every string the page shows that is the C++ application's is written
// as its message id, the source's own words, and passed through `t`; the
// C++ shows its message ids the same way, English included (English is
// the `en` catalog: "Advance" reads "Advanced"). A message id the table
// has no translation for is shown as it is.
//
// The table comes first: this script asks for it, puts the static page
// (index.html) into the language, and only then loads the page's other
// scripts, in their order, so nothing is drawn before the words are
// there. The page loads nothing from the network: the table comes
// through the session, as every call does.

// The page's theme: dark or light as Preferences' "Enable Dark mode"
// says (prefs.js), else as the system is. Set before anything is drawn,
// from what was last in force (kept in the browser only to draw the
// first frame right; the setting is the session's). `light` on <html>
// takes the light tokens (style.css); `themechange` tells what draws.
const EdgeTheme = (() => {
  const system = window.matchMedia('(prefers-color-scheme: light)');
  let chosen = null;
  try { chosen = localStorage.getItem('edge.theme'); } catch { /* none */ }
  const light = () => (chosen ? chosen === 'light' : system.matches);
  const show = () => {
    const was = document.documentElement.classList.contains('light');
    document.documentElement.classList.toggle('light', light());
    if (was !== light()) document.dispatchEvent(new Event('themechange'));
  };
  system.addEventListener('change', show);
  show();
  return {
    light,
    // 'dark', 'light', or null to follow the system.
    set(mode) {
      chosen = mode;
      try {
        if (mode) localStorage.setItem('edge.theme', mode); else localStorage.removeItem('edge.theme');
      } catch { /* not kept here */ }
      show();
    },
  };
})();

const I18n = (() => {
  let table = { messages: {}, plurals: {}, plural: '' };
  let current = { language: 'en_GB', languages: [] };

  // The same two ways `call` has (app.js), which is not loaded yet.
  async function ask(name, args) {
    const tauri = window.__TAURI__;
    if (tauri) return tauri.core.invoke('call', { name, args });
    const token = document.querySelector('meta[name="edge-token"]').content;
    const response = await fetch(`/api/${name}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Edge-Token': token },
      body: JSON.stringify(args),
    });
    if (!response.ok) throw new Error(await response.text());
    const answer = await response.json();
    if ('error' in answer) throw new Error(answer.error);
    return answer.ok;
  }

  function take(answer) {
    table = answer.table;
    current = { language: answer.language, languages: answer.languages, search: answer.search };
    // The page's own language, for the system's choice of fonts (CJK)
    // and of hyphenation.
    const tag = answer.language.replace('_', '-');
    document.documentElement.lang = tag.startsWith('en') ? 'en' : tag;
  }

  // The static page: every text and every title, label and placeholder
  // that is a message id is put in the language. A title of the form
  // "Words [Key]" has its words put in, the key kept, as the reference
  // builds such a tooltip.
  const ATTRIBUTES = ['title', 'aria-label', 'placeholder'];
  function translated(text) {
    const words = text.replace(/\s+/g, ' ').trim();
    if (!words) return text;
    const done = t(words);
    if (done !== words) return done;
    const keyed = /^(.*?) \[([^\]]+)\]$/.exec(words);
    if (keyed && t(keyed[1]) !== keyed[1]) return `${t(keyed[1])} [${keyed[2]}]`;
    return text;
  }
  function translatePage(root = document.body) {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const parent = node.parentElement;
      if (!parent || parent.closest('script, style, svg, [data-no-t]')) continue;
      if (!/[A-Za-z]/.test(node.nodeValue)) continue;
      const words = node.nodeValue.replace(/\s+/g, ' ').trim();
      const done = translated(words);
      // The spaces round the words are kept.
      if (done !== words) node.nodeValue = node.nodeValue.replace(/\S(?:[\s\S]*\S)?/, () => done);
    }
    for (const name of ATTRIBUTES) {
      for (const element of root.querySelectorAll(`[${name}]`)) {
        element.setAttribute(name, translated(element.getAttribute(name)));
      }
    }
  }

  // The settings pages' search fields (`data-search-words`): "Search for
  // setting" in English, the catalog's words for "Search in preset"
  // elsewhere, as the session picks them (`search_words`, src/api/i18n.rs).
  // A change of language loads the page again, which puts them in anew.
  const searchWords = () => current.search || 'Search for setting';
  function wordSearches(root = document.body) {
    for (const field of root.querySelectorAll('[data-search-words]')) {
      field.placeholder = searchWords();
      field.setAttribute('aria-label', searchWords());
    }
  }

  const SCRIPTS = [
    'gl/math.js', 'gl/camera.js', 'gl/shaders.js', 'gl/renderer.js', 'gl/scene.js',
    'gl/toolpath.js', 'gl/controls.js', 'gl/gcode.js', 'gl/stage.js',
    'paint.js', 'cut.js', 'connectors.js', 'curved.js', 'drawn.js', 'joints.js', 'boolean.js', 'vlh.js', 'scene-edit.js', 'plates.js', 'objects.js', 'printers.js', 'settings.js',
    'objsettings.js', 'presets.js', 'setup.js', 'filaments.js', 'prefs.js', 'app.js', 'save.js', 'hosts.js', 'menu.js',
  ];
  // Once the last has run, `scripts-loaded` is sent where the page's
  // scripts used to wait for DOMContentLoaded.
  function loadScripts() {
    for (const src of SCRIPTS) {
      const script = document.createElement('script');
      script.src = src;
      // In order, as the tags they were.
      script.async = false;
      if (src === SCRIPTS[SCRIPTS.length - 1]) {
        script.addEventListener('load', () => document.dispatchEvent(new Event('scripts-loaded')));
      }
      document.body.append(script);
    }
  }

  // The page's layout as it was left (`layout`, src/api/layout.rs): asked
  // for beside the words, and the sidebar's width put on the page before
  // it is first shown, so it does not jump once app.js has run.
  let layout = null;
  function takeLayout() {
    const plater = document.getElementById('plater');
    if (!layout || !plater || layout.sidebar_width === null) return;
    plater.classList.add('side-sized');
    plater.style.setProperty('--side-width', `${layout.sidebar_width}px`);
  }

  async function start() {
    // Nothing is shown until it is in the language (style.css).
    document.documentElement.classList.add('words-wait');
    const parsed = document.readyState === 'loading'
      ? new Promise((done) => document.addEventListener('DOMContentLoaded', done, { once: true }))
      : null;
    const laidOut = ask('layout', {}).then((answer) => { layout = answer; }, (error) => console.error(error));
    try {
      // The system's languages, the first of which is the system's; the
      // session chooses from it at the first run and keeps the choice.
      take(await ask('language', { system: [...(navigator.languages || [navigator.language])] }));
    } catch (error) {
      // No table: the message ids, as the C++ shows them without a catalog.
      console.error(error);
    }
    await parsed;
    await laidOut;
    takeLayout();
    translatePage();
    wordSearches();
    document.documentElement.classList.remove('words-wait');
    loadScripts();
  }

  // Another language, kept by the session. The reference asks to restart
  // the application ("Switching the language requires application
  // restart."); the page has nothing to restart but itself, and loads
  // itself again, with every project kept where it is in the session.
  async function choose(language) {
    take(await ask('language_set', { language }));
    window.location.reload();
  }

  return {
    get table() { return table; },
    get language() { return current.language; },
    get languages() { return current.languages; },
    get layout() { return layout; },
    searchWords,
    translatePage,
    choose,
    start,
  };
})();

// The translation of `msgid`, or `msgid` itself.
function t(msgid) {
  const found = I18n.table.messages[msgid];
  return found === undefined ? msgid : found;
}

// The same for a message id with a context (`L_CONTEXT`).
function tc(context, msgid) {
  const found = I18n.table.messages[`${context}\u0004${msgid}`];
  return found === undefined ? msgid : found;
}

// The form of a message id with a plural (`_L_PLURAL`) for `n`; without a
// translation, the singular for one and the plural for any other number.
function tn(singular, plural, n) {
  const forms = I18n.table.plurals[singular];
  if (forms) {
    const at = n < 200 ? n : 100 + (n % 100);
    const form = forms[Number(I18n.table.plural[at] || 0)];
    if (form !== undefined) return form;
  }
  return n === 1 ? singular : plural;
}

// `text` with `values` in its placeholders, as the C++ formats it:
// boost::format's `%1%`, printf's `%1$d` by position, `%s`, `%d` and
// `%.2f` in order (a precision kept for a number), `%%` a percent sign.
// A placeholder with no value is left as it is.
function fmt(text, ...values) {
  let next = 0;
  return text.replace(/%%|%(\d+)%|%(?:(\d+)\$)?[-+ #0']*\d*(?:\.(\d+))?(?:hh|h|ll|l|L|q|j|z|t)?([diouxXeEfFgGsc])/g,
    (whole, boost, position, precision, conversion) => {
      if (whole === '%%') return '%';
      const index = boost ? Number(boost) : position ? Number(position) : (next += 1);
      if (index < 1 || index > values.length) return whole;
      const value = values[index - 1];
      if (precision !== undefined && /[eEfFgG]/.test(conversion) && Number.isFinite(Number(value))) {
        return Number(value).toFixed(Number(precision));
      }
      return String(value);
    });
}

I18n.start();
