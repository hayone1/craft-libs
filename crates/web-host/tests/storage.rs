#![cfg(target_arch = "wasm32")]
#![allow(clippy::unwrap_used)]

use std::cell::Cell;
use std::rc::Rc;

use craft_web_host::storage::{self, OpenChoice, SaveChoice, SaveQueue};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

// A fake host: `open` and `save` name the answer its dialogs give ("reject" throws), `write`
// takes `delay` ms and throws when `fails`. Every call goes to a log.
#[wasm_bindgen(inline_js = r#"
const answers = {
  file: () => ({ path: "cloud:home/a.psd", bytes: new Uint8Array([1, 2, 3]) }),
  buffer: () => ({ path: "cloud:home/b.psd", bytes: new Uint8Array([4]).buffer }),
  bare: () => ({ path: "a.psd", bytes: new Uint8Array([1]) }),
  nobytes: () => ({ path: "cloud:home/a.psd" }),
  host: () => "cloud:home/out.psd",
  barename: () => "out.psd",
  device: () => "device",
  Device: () => "Device",
  null: () => null,
  number: () => 7,
};
export function host(open, save, fails, delay) {
  const log = [];
  const answer = (name) => { if (name === "reject") throw new Error("the host is offline"); return answers[name](); };
  globalThis.craftStorage = {
    async chooseOpen(exts) { log.push(["chooseOpen", exts]); return answer(open); },
    async chooseSave(name) { log.push(["chooseSave", name]); return answer(save); },
    async write(path, bytes) {
      log.push(["start", path, bytes.length]);
      await new Promise((r) => setTimeout(r, delay));
      log.push(["end", path]);
      if (fails) throw new Error("disk full");
    },
  };
  globalThis.hostLog = log;
}
export function sync_host() { globalThis.craftStorage = { chooseSave: () => "device" }; }
export function no_host() { delete globalThis.craftStorage; }
export function log() { return JSON.stringify(globalThis.hostLog); }
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
"#)]
extern "C" {
    fn host(open: &str, save: &str, fails: bool, delay: u32);
    fn sync_host();
    fn no_host();
    fn log() -> String;
    fn sleep(ms: u32) -> js_sys::Promise;
}

async fn wait(ms: u32) {
    JsFuture::from(sleep(ms)).await.unwrap();
}

#[wasm_bindgen_test]
fn without_a_host_nothing_is_offered() {
    no_host();
    assert!(!storage::available());
}

#[wasm_bindgen_test]
async fn open_answers_become_choices() {
    for (answer, want) in [
        ("file", Ok(OpenChoice::File { path: "cloud:home/a.psd".into(), bytes: vec![1, 2, 3] })),
        ("buffer", Ok(OpenChoice::File { path: "cloud:home/b.psd".into(), bytes: vec![4] })),
        ("device", Ok(OpenChoice::Device)),
        ("null", Ok(OpenChoice::Cancelled)),
    ] {
        host(answer, "null", false, 0);
        assert!(storage::available());
        assert_eq!(storage::choose_open(Some(&["psd", "png"])).await, want, "{answer}");
    }
    assert!(log().contains(r#"["chooseOpen",["psd","png"]]"#), "{}", log());
}

#[wasm_bindgen_test]
async fn only_device_means_the_device_and_a_failed_host_chooses_nothing() {
    for answer in ["reject", "bare", "nobytes", "Device", "number"] {
        host(answer, "null", false, 0);
        assert!(storage::choose_open(None).await.is_err(), "open {answer}");
    }
    for answer in ["reject", "barename", "Device", "number"] {
        host("null", answer, false, 0);
        assert!(storage::choose_save("x.psd").await.is_err(), "save {answer}");
    }
    host("null", "reject", false, 0);
    assert_eq!(storage::choose_save("x.psd").await, Err("the host is offline".into()));
    no_host();
    assert!(storage::choose_save("x.psd").await.is_err());
}

#[wasm_bindgen_test]
async fn save_answers_become_choices_and_writes_report_back() {
    for (answer, want) in [("host", SaveChoice::Host("cloud:home/out.psd".into())), ("device", SaveChoice::Device), ("null", SaveChoice::Cancelled)] {
        host("null", answer, false, 0);
        assert_eq!(storage::choose_save("out.psd").await, Ok(want), "{answer}");
    }
    host("null", "null", false, 0);
    assert_eq!(storage::write("cloud:home/out.psd", &[1, 2]).await, Ok(()));
    host("null", "null", true, 0);
    assert_eq!(storage::write("cloud:home/out.psd", &[1, 2]).await, Err("disk full".into()));
    // A host may answer without a promise.
    sync_host();
    assert_eq!(storage::choose_save("out.psd").await, Ok(SaveChoice::Device));
}

#[wasm_bindgen_test]
async fn the_queue_saves_one_at_a_time_in_order() {
    host("null", "null", false, 20);
    let queue: SaveQueue<Result<(), String>> = SaveQueue::new();
    let woken = Rc::new(Cell::new(0));
    let w = woken.clone();
    queue.on_done(move || w.set(w.get() + 1));
    assert_eq!((queue.busy(), queue.latest()), (false, None));
    let ids: Vec<u64> =
        ["a", "b", "c"].into_iter().map(|n| queue.push(async move { storage::write(&format!("cloud:home/{n}.psd"), &[0]).await })).collect();
    assert!(queue.busy());
    assert_eq!(queue.latest(), Some(ids[2]));
    while queue.busy() {
        wait(10).await;
    }
    let done = queue.take_done();
    assert_eq!(done.iter().map(|d| d.0).collect::<Vec<_>>(), ids);
    assert!(done.iter().all(|d| d.1.is_ok()));
    assert_eq!(woken.get(), 3);
    assert!(queue.take_done().is_empty());
    // Each write ended before the next one started.
    let order = r#"["start","cloud:home/a.psd",1],["end","cloud:home/a.psd"],["start","cloud:home/b.psd",1],["end","cloud:home/b.psd"],["start","cloud:home/c.psd",1],["end","cloud:home/c.psd"]"#;
    assert!(log().contains(order), "{}", log());
}

#[wasm_bindgen_test]
async fn a_save_pushed_when_one_finishes_still_waits_its_turn() {
    host("null", "null", true, 10);
    let queue: SaveQueue<Result<(), String>> = SaveQueue::new();
    let q = queue.clone();
    let pushed = Rc::new(Cell::new(false));
    let p = pushed.clone();
    queue.on_done(move || {
        if !p.replace(true) {
            q.push(async { storage::write("cloud:home/again.psd", &[0]).await });
        }
    });
    queue.push(async { storage::write("cloud:home/first.psd", &[0]).await });
    queue.push(async { storage::write("cloud:home/second.psd", &[0]).await });
    while queue.busy() {
        wait(10).await;
    }
    let done = queue.take_done();
    assert_eq!(done.len(), 3);
    assert!(done.iter().all(|d| d.1 == Err("disk full".to_string())));
    // Never two writes at once.
    let log = log();
    assert!(
        !log.contains(r#"["start","cloud:home/second.psd",1],["start""#) && !log.contains(r#"["start","cloud:home/first.psd",1],["start""#),
        "{log}"
    );
}
