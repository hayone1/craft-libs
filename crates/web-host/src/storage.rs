//! The host's file storage, `window.craftStorage` (see the crate docs for the contract).
//!
//! The calls answer in Rust terms: [`choose_open`] and [`choose_save`] turn the host's answer
//! into a choice, or an error for a host that failed or answered something else, which the app
//! treats as nothing chosen. [`SaveQueue`] runs saves one at a time and reports each back.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::is_host_path;

/// The page's `craftStorage` object, when it offers one.
fn host() -> Option<JsValue> {
    let storage = js_sys::Reflect::get(&js_sys::global(), &"craftStorage".into()).ok()?;
    storage.is_object().then_some(storage)
}

/// Whether the page offers its own storage. Without it the app works as it does on its own.
pub fn available() -> bool {
    host().is_some()
}

/// What a JavaScript failure says: an `Error`'s message, a thrown string, or a plain note.
fn message(e: &JsValue) -> String {
    if let Some(e) = e.dyn_ref::<js_sys::Error>() {
        return e.message().into();
    }
    e.as_string().unwrap_or_else(|| "the host's storage failed".to_string())
}

/// Call `craftStorage.<method>(args…)` and wait for its answer.
async fn call(method: &str, args: &[JsValue]) -> Result<JsValue, String> {
    let storage = host().ok_or("the page has no craftStorage")?;
    let f = js_sys::Reflect::get(&storage, &method.into()).map_err(|e| message(&e))?;
    let f: js_sys::Function = f.dyn_into().map_err(|_| format!("craftStorage.{method} is not a function"))?;
    let answer = f.apply(&storage, &args.iter().collect::<js_sys::Array>()).map_err(|e| message(&e))?;
    // A host may answer at once instead of with a promise.
    match answer.dyn_into::<js_sys::Promise>() {
        Ok(promise) => JsFuture::from(promise).await.map_err(|e| message(&e)),
        Err(answer) => Ok(answer),
    }
}

/// What the user picked in the host's Open dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenChoice {
    /// A file in the host's storage: its host path and its bytes.
    File { path: String, bytes: Vec<u8> },
    /// The browser's own file picker (the host answered `"device"`).
    Device,
    /// Nothing (the host answered `null`).
    Cancelled,
}

/// What the user picked in the host's Save dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveChoice {
    /// A path in the host's storage, to [`write`] to.
    Host(String),
    /// A browser download (the host answered `"device"`).
    Device,
    /// Nothing (the host answered `null`).
    Cancelled,
}

fn is_device(answer: &JsValue) -> bool {
    answer.as_string().as_deref() == Some("device")
}

fn unexpected(method: &str, answer: &JsValue) -> String {
    format!("craftStorage.{method} answered something unexpected: {answer:?}")
}

/// Ask the host which file to open; `extensions` (without dots) narrows its list, `None` shows all.
/// An error means the host failed or answered something else: nothing was chosen.
pub async fn choose_open(extensions: Option<&[&str]>) -> Result<OpenChoice, String> {
    let exts = extensions.map_or(JsValue::NULL, |e| e.iter().map(|x| JsValue::from(*x)).collect::<js_sys::Array>().into());
    let answer = call("chooseOpen", &[exts]).await?;
    if answer.is_null() {
        return Ok(OpenChoice::Cancelled);
    }
    if is_device(&answer) {
        return Ok(OpenChoice::Device);
    }
    let path = js_sys::Reflect::get(&answer, &"path".into()).ok().and_then(|p| p.as_string()).filter(|p| is_host_path(p));
    let bytes = js_sys::Reflect::get(&answer, &"bytes".into())
        .ok()
        .filter(|b| b.is_instance_of::<js_sys::Uint8Array>() || b.is_instance_of::<js_sys::ArrayBuffer>())
        .map(|b| js_sys::Uint8Array::new(&b).to_vec());
    match (path, bytes) {
        (Some(path), Some(bytes)) => Ok(OpenChoice::File { path, bytes }),
        _ => Err(unexpected("chooseOpen", &answer)),
    }
}

/// Ask the host where a save of `suggested` (a file name, or the document's host path) goes.
/// An error means the host failed or answered something else: nothing was chosen.
pub async fn choose_save(suggested: &str) -> Result<SaveChoice, String> {
    let answer = call("chooseSave", &[suggested.into()]).await?;
    if answer.is_null() {
        return Ok(SaveChoice::Cancelled);
    }
    if is_device(&answer) {
        return Ok(SaveChoice::Device);
    }
    match answer.as_string().filter(|p| is_host_path(p)) {
        Some(path) => Ok(SaveChoice::Host(path)),
        None => Err(unexpected("chooseSave", &answer)),
    }
}

/// Write `bytes` to `path` in the host's storage. `Ok` once the host has the file.
pub async fn write(path: &str, bytes: &[u8]) -> Result<(), String> {
    call("write", &[path.into(), js_sys::Uint8Array::from(bytes).into()]).await.map(|_| ())
}

/// A save waiting in a [`SaveQueue`].
type Save<R> = Pin<Box<dyn Future<Output = R>>>;

struct Queue<R> {
    waiting: VecDeque<(u64, Save<R>)>,
    running: Option<u64>,
    /// The worker that works through `waiting` is alive (it may be between two saves).
    worker: bool,
    last_id: u64,
    done: Vec<(u64, R)>,
    wake: Option<Rc<dyn Fn()>>,
}

/// Saves to the host, one at a time in the order they were pushed, so an earlier save of a file
/// never lands after a later one. Each save is a future the app builds (asking the host where it
/// goes, then [`write`]); its result comes back through [`Self::take_done`] with the id
/// [`Self::push`] gave it. Cloning shares the queue.
pub struct SaveQueue<R> {
    queue: Rc<RefCell<Queue<R>>>,
}

impl<R> Clone for SaveQueue<R> {
    fn clone(&self) -> Self {
        Self { queue: self.queue.clone() }
    }
}

impl<R: 'static> Default for SaveQueue<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: 'static> SaveQueue<R> {
    pub fn new() -> Self {
        Self {
            queue: Rc::new(RefCell::new(Queue { waiting: VecDeque::new(), running: None, worker: false, last_id: 0, done: Vec::new(), wake: None })),
        }
    }

    /// Called after each save finishes, for example to ask the UI for a frame that reads
    /// [`Self::take_done`].
    pub fn on_done(&self, wake: impl Fn() + 'static) {
        self.queue.borrow_mut().wake = Some(Rc::new(wake));
    }

    /// Queue a save; it starts once the saves before it have finished. Returns its id.
    pub fn push(&self, save: impl Future<Output = R> + 'static) -> u64 {
        let (id, start) = {
            let mut q = self.queue.borrow_mut();
            q.last_id += 1;
            let id = q.last_id;
            q.waiting.push_back((id, Box::pin(save)));
            let start = !q.worker;
            q.worker = true;
            (id, start)
        };
        if start {
            self.run();
        }
        id
    }

    /// The id of the latest save that hasn't finished, if any.
    pub fn latest(&self) -> Option<u64> {
        let q = self.queue.borrow();
        q.waiting.back().map(|w| w.0).or(q.running)
    }

    /// Whether a save is waiting or running.
    pub fn busy(&self) -> bool {
        let q = self.queue.borrow();
        q.running.is_some() || !q.waiting.is_empty()
    }

    /// The saves that finished since the last call, in order: (id, result).
    pub fn take_done(&self) -> Vec<(u64, R)> {
        std::mem::take(&mut self.queue.borrow_mut().done)
    }

    fn run(&self) {
        let queue = self.queue.clone();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let next = {
                    let mut q = queue.borrow_mut();
                    let next = q.waiting.pop_front();
                    q.running = next.as_ref().map(|n| n.0);
                    q.worker = next.is_some();
                    next
                };
                let Some((id, save)) = next else { break };
                // No borrow is held while the save runs: it may push more saves.
                let result = save.await;
                let wake = {
                    let mut q = queue.borrow_mut();
                    q.running = None;
                    q.done.push((id, result));
                    q.wake.clone()
                };
                if let Some(wake) = wake {
                    wake();
                }
            }
        });
    }
}
