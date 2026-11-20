const { invoke } = window.__TAURI__.core;
const form = document.querySelector('#form');
const list = document.querySelector('#list');
const error = document.querySelector('#error');
const open = document.querySelector('#open');

invoke('saved_sites').then((sites) => {
  for (const site of sites) {
    const row = document.createElement('label');
    row.className = 'site';
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.value = site.origin;
    row.append(box, ` ${site.name} `);
    const origin = document.createElement('small');
    origin.textContent = site.origin;
    row.append(origin);
    list.append(row);
  }
}).catch((failure) => { error.textContent = String(failure); });

form.onsubmit = async (event) => {
  event.preventDefault();
  error.textContent = '';
  open.disabled = true;
  try {
    const origins = [...list.querySelectorAll('input:checked')].map((box) => box.value);
    await invoke('open_sites', { origins });
  } catch (failure) {
    error.textContent = String(failure);
  } finally {
    open.disabled = false;
  }
};
