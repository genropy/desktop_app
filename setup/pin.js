const { invoke } = window.__TAURI__.core;

const form = document.getElementById('form');
const input = document.getElementById('pin');
const error = document.getElementById('error');
const ok = document.getElementById('ok');

invoke('pin_request').then((request) => {
  document.getElementById('holder').textContent = request.holder;
  input.minLength = request.min_length;
  input.maxLength = request.max_length;
  if (request.wrong_pin_before) {
    error.textContent = 'A wrong PIN was entered since the last right one.';
  }
}).catch((message) => {
  // Last attempt or locked PIN: the PIN is not tried, only Cancel is left.
  error.textContent = message;
  input.disabled = true;
  ok.disabled = true;
});

form.addEventListener('submit', (event) => {
  event.preventDefault();
  error.textContent = '';
  ok.disabled = true;
  invoke('pin_submit', { pin: input.value }).catch((message) => {
    input.value = '';
    error.textContent = message;
    ok.disabled = false;
  });
});

document.getElementById('cancel').addEventListener('click', () => invoke('pin_cancel'));
