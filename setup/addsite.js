const { invoke } = window.__TAURI__.core;
const form = document.querySelector('#form');
const address = document.querySelector('#address');
const error = document.querySelector('#error');
const add = document.querySelector('#add');

invoke('site_error').then((message) => { if (message) error.textContent = message; });

form.onsubmit = async (event) => {
  event.preventDefault();
  error.textContent = '';
  add.disabled = true;
  try {
    await invoke('add_site', { address: address.value });
  } catch (failure) {
    error.textContent = String(failure);
  } finally {
    add.disabled = false;
  }
};
