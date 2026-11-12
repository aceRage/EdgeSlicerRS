// "Save Project" [Ctrl+S] and "Save Project as..." [Ctrl+Shift+S], from
// the keys and from a project tile's menu ("Save", "Save as..."), and
// what is asked before a project or the window closes with unsaved
// changes. The reference's words: MainFrame.cpp `init_menubar_as_editor`,
// Plater.cpp `save_project`, `close_with_confirm`, ProjectStrip.cpp
// `close_project`, `confirm_close_all`.
//
// Uses app.js's `call`, `pick`, `applyListing`, `refreshProject`,
// `activeId`, `state`, `say` and `attempt`, which load after it.

/* global call, pick, applyListing, refreshProject, activeId, state, say, attempt, tauri, $, el */

// A question with up to three answers, each a button: answers the value
// of the one pressed ('cancel' for Escape). With `ticks`, a list of
// labels to tick, all ticked to start with; which are ticked comes back
// in `ticked`.
function askThree({ title, text, answers, ticks = null }) {
  const dialog = $('save-ask');
  $('save-ask-title').textContent = title;
  $('save-ask-text').textContent = text;
  $('save-ask-list').replaceChildren(...(ticks || []).map((label, index) => el('li', {},
    el('label', {}, el('input', { type: 'checkbox', value: index, checked: true }), label))));
  $('save-ask-buttons').replaceChildren(...answers.map(([value, label], at) =>
    el('button', { type: 'submit', value, textContent: label, className: at === 0 ? 'primary' : '' })));
  return new Promise((resolve) => {
    dialog.returnValue = 'cancel';
    dialog.addEventListener('close', () => {
      const ticked = [...$('save-ask-list').querySelectorAll('input:checked')].map((box) => Number(box.value));
      resolve({ answer: dialog.returnValue || 'cancel', ticked });
    }, { once: true });
    dialog.showModal();
    dialog.querySelector('button.primary')?.focus();
  });
}

// Saves a project (the one shown when none is named): to the project
// file it was opened from, else (and always for "Save as...") where the
// save dialog says. True once it is saved; false when the person changed
// their mind.
async function saveProject(id = activeId(), saveAs = false) {
  const args = { project: id };
  if (saveAs) {
    // The name the session would offer.
    const asked = await call('project_save', { project: id, suggest: true });
    const path = await pick('project', asked && asked.suggested ? asked.suggested : '');
    if (!path) return false;
    args.path = path;
  }
  for (;;) {
    let answer;
    try {
      answer = await call('project_save', args);
    } catch (error) {
      await askThree({
        title: t('Save project'),
        text: `${t('Failed to save the project.\nPlease check whether the folder exists online or if other programs open the project file.')}\n\n${error.message}`,
        answers: [['ok', t('OK')]],
      });
      return false;
    }
    if (answer.saved) {
      await applyListing(answer);
      if (id === activeId()) await refreshProject();
      say(`Saved ${answer.path}`);
      return true;
    }
    if (answer.ask === 'path') {
      const path = await pick('project', answer.suggested);
      if (!path) return false;
      args.path = path;
    } else if (answer.ask === 'losses') {
      // This app's own question: the reference keeps everything it reads.
      const { answer: sure } = await askThree({
        title: t('Save project'),
        text: `"${answer.name}" would be saved without:\n\n${answer.losses.map((loss) => `• ${loss}`).join('\n')}\n\nSave it all the same?`,
        answers: [['yes', t('Save')], ['cancel', t('Cancel')]],
      });
      if (sure !== 'yes') return false;
      args.accept = true;
    } else {
      return false;
    }
  }
}

// Closing a project with unsaved changes: "Save them before closing it?"
// Answers whether to go on closing it (it was saved, or is not to be).
async function saveBeforeClosing(id, name) {
  const { answer } = await askThree({
    title: t('Close project'),
    text: fmt(t('"%s" has unsaved changes. Save them before closing it?'), name),
    answers: [['yes', t('Yes')], ['no', t('No')], ['cancel', t('Cancel')]],
  });
  if (answer === 'cancel') return false;
  if (answer === 'yes') return saveProject(id);
  return true;
}

// Closing the window: with one project, "The current project has unsaved
// changes, save it before continue?"; with several, one question for
// every unsaved one ("Unsaved projects"). Answers whether to close.
async function saveBeforeQuitting() {
  const { unsaved, projects } = await call('unsaved');
  if (!unsaved.length) return true;
  if (projects < 2) {
    const { answer } = await askThree({
      title: `EdgeSlicerRS - ${t('Save')}`,
      text: t('The current project has unsaved changes, save it before continue?'),
      answers: [['yes', t('Yes')], ['no', t('No')], ['cancel', t('Cancel')]],
    });
    if (answer === 'cancel') return false;
    return answer === 'no' || saveProject(unsaved[0].id);
  }
  const { answer, ticked } = await askThree({
    title: t('Unsaved projects'),
    text: t('These projects have unsaved changes. Save the ticked ones before closing?'),
    ticks: unsaved.map((project) => project.name + (project.path ? `  (${project.path})` : '')),
    answers: [['yes', t('Save')], ['no', t("Don't save")], ['cancel', t('Cancel')]],
  });
  if (answer === 'cancel') return false;
  if (answer === 'yes') {
    for (const at of ticked) {
      if (!(await saveProject(unsaved[at].id))) return false;
    }
  }
  return true;
}

document.addEventListener('keydown', (event) => {
  if (document.querySelector('dialog[open]')) return;
  const command = event.ctrlKey || event.metaKey;
  if (!command || event.altKey || event.key.toLowerCase() !== 's') return;
  event.preventDefault();
  attempt(() => saveProject(activeId(), event.shiftKey));
});

// The window asks before it closes (main.rs holds the close back while a
// project has unsaved changes).
if (tauri) {
  tauri.event.listen('close-requested', () => attempt(async () => {
    if (await saveBeforeQuitting()) await tauri.core.invoke('close_window');
  }));
}
