const { invoke } = window.__TAURI__.core;
const form = document.querySelector('#form');
const port = document.querySelector('#port');
const error = document.querySelector('#error');
const open = document.querySelector('#open');
const KEY = 'last-port';

try { port.value = localStorage.getItem(KEY) || ''; } catch {}

form.onsubmit = async (event) => {
  event.preventDefault();
  error.textContent = '';
  open.disabled = true;
  try {
    try { localStorage.setItem(KEY, port.value); } catch {}
    await invoke('open_local', { port: Number(port.value) });
  } catch (failure) {
    error.textContent = String(failure);
  } finally {
    open.disabled = false;
  }
};
