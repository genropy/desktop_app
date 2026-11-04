const { invoke } = window.__TAURI__.core;

const form = document.getElementById('form');
const input = document.getElementById('url');
const error = document.getElementById('error');

invoke('get_url').then((url) => {
  if (url) {
    input.value = url;
  }
});

form.addEventListener('submit', (event) => {
  event.preventDefault();
  error.textContent = '';
  invoke('save_url', { url: input.value }).catch((message) => {
    error.textContent = message;
  });
});
