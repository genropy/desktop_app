const { invoke } = window.__TAURI__.core;
const message = document.querySelector('#message');

const VERIFICATION = { biometry: 'Touch ID', user_presence: 'Touch ID o password del Mac' };

function say(text, error = false) {
  message.textContent = text;
  message.className = error ? 'error' : '';
}

function cell(row, ...content) {
  const td = document.createElement('td');
  td.append(...content);
  row.append(td);
  return td;
}

function small(text) {
  const element = document.createElement('small');
  element.textContent = text;
  return element;
}

function button(label, action) {
  const element = document.createElement('button');
  element.textContent = label;
  element.onclick = async () => {
    element.disabled = true;
    say('');
    try {
      const result = await action();
      if (typeof result === 'string') say(result);
    } catch (failure) {
      say(String(failure), true);
    } finally {
      element.disabled = false;
    }
  };
  return element;
}

async function loadSites() {
  const body = document.querySelector('#sites');
  body.replaceChildren();
  const sites = await invoke('manage_sites');
  if (!sites.length) {
    const row = document.createElement('tr');
    cell(row, 'Nessun sito aggiunto.').colSpan = 4;
    body.append(row);
  }
  for (const site of sites) {
    const row = document.createElement('tr');
    const name = cell(row, site.name, small(site.origin));
    if (site.primary) {
      const badge = document.createElement('span');
      badge.className = 'primary';
      badge.textContent = 'primario';
      name.insertBefore(badge, name.childNodes[1]);
      name.insertBefore(document.createTextNode(' '), badge);
    }
    cell(row, site.registered ? site.user : '—');
    cell(row, site.registered ? (VERIFICATION[site.verification] || site.verification || '') : 'non registrato');
    const actions = cell(row);
    actions.className = 'actions';
    if (site.registered) {
      actions.append(button('Prova chiave', () => invoke('manage_test_key', { origin: site.origin })));
      actions.append(button('Dimentica', async () => {
        if (!confirm(`Cancellare la chiave di questo computer per ${site.name}?\nIl sito conserva la registrazione finché un amministratore non la revoca.`)) return;
        await invoke('manage_forget', { origin: site.origin });
        await loadSites();
        return 'Chiave cancellata.';
      }));
    }
    actions.append(button(site.primary ? 'Togli primario' : 'Primario', async () => {
      await invoke('manage_set_primary', { origin: site.origin });
      await loadSites();
    }));
    actions.append(button('Rimuovi', async () => {
      if (!confirm(`Rimuovere ${site.name} da GenroApp? Anche la chiave di questo computer viene cancellata.`)) return;
      await invoke('manage_remove', { origin: site.origin });
      await loadSites();
      return 'Sito rimosso.';
    }));
    body.append(row);
  }
}

(async () => {
  try {
    await loadSites();
  } catch (failure) {
    say(String(failure), true);
  }
})();
