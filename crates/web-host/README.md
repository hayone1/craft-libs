# craft-web-host

What a web page that hosts a Craft app offers it, in one place for every app. Today that is the
host's own file storage, `window.craftStorage`: a page that serves an app (a cloud drive, a team
folder) can offer its storage, and the app's File › Open and Save offer it next to this device.

```js
window.craftStorage = {
  // File › Open: {path, bytes} for a file in the host's storage, "device" for the browser's
  // own file picker, or null when the user cancelled.
  async chooseOpen(extensions /* ["psd", …] or null */) { … },
  // A save or export: a path in the host's storage, "device" for a download, or null.
  async chooseSave(suggested /* a file name, or the document's host path */) { … },
  // Writes a file the user saved to a host path. The save counts once it resolves; a
  // rejection leaves the document unsaved.
  async write(path, bytes /* Uint8Array */) { … },
};
```

| Part | What it does |
| --- | --- |
| `is_host_path` (every target) | A host path has a scheme of its own, `cloud:home/art/photo.psd`. A one-letter scheme is a Windows drive. Documents opened from a host path keep it, so File › Save writes back there. |
| `storage::available` | Whether the page offers `craftStorage`. Without it the app works as it does on its own. |
| `storage::choose_open`, `storage::choose_save` | The host's dialogs as `OpenChoice` / `SaveChoice`. Only `"device"` means the device; a host that rejects or answers something else is an error, which the app treats as nothing chosen. |
| `storage::write` | Writes to a host path; `Ok` once the host has the file. |
| `storage::SaveQueue` | Saves one at a time, in order, so an earlier save of a file never lands after a later one. Each result comes back with the id `push` gave it. |

Rules for a host:

- Answer `"device"` right from the user's click in your own dialog: browsers open a file picker
  only during a click.
- Make every `write` settle. Saves run one at a time, so one that never settles holds the rest. A
  host whose storage can hang should reject after its own timeout (only the host can tell a slow
  upload from a stuck one).

What a save means for a document (its path, its saved state, when it may close) stays in each
app: the app builds each save's future, pushes it, and applies the results it takes back.

The `storage` module is for `wasm32` only. Its tests run in Node (`wasm-bindgen-test-runner`) with
a fake host, since the module reads `globalThis.craftStorage`, which is `window` in a page.
