const { invoke } = window.__TAURI__.core;
const file = document.querySelector('#file');
const status = document.querySelector('#status');
const confirm = document.querySelector('#confirm');
let documentText;
let selection = 0;

async function preview(read) {
  const current = ++selection;
  documentText = undefined;
  confirm.hidden = true;
  status.textContent = '';
  try {
    const text = await read();
    if (text === null) return;
    const link = await invoke('inspect_connection', { text });
    if (current !== selection) return;
    documentText = text;
    status.textContent = link.name + '\n' + link.url;
    confirm.hidden = false;
  } catch (error) {
    if (current === selection) status.textContent = String(error);
  }
}

file.onchange = () => preview(() => {
  const selected = file.files[0];
  if (!selected) return null;
  if (selected.size > 16384) throw Error('Collegamento troppo grande.');
  return selected.text();
});
confirm.onclick = async () => {
  confirm.disabled = true;
  try { await invoke('activate_connection', { text: documentText }); }
  catch (error) { status.textContent = String(error); }
  finally { confirm.disabled = false; }
};
const pending = () => preview(() => invoke('pending_connection'));
window.__TAURI__.event.listen('connection-opened', pending).then(pending);
