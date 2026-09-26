//! Leptos CSR entrypoint — the Rust twin of `apps/todo/web/src/index.tsx`.
//!
//! One route and no router: the Solid app's `/xstate` and `/effect` routes are
//! `lazy()` chunks that `/` never downloads, so shipping a router here would
//! add bytes the measured Solid baseline does not carry. See README.md.

mod api;
mod app;
mod dto;
mod event_feed;
mod flags;
mod identity;
mod realtime;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

fn main() {
    // A wasm panic is otherwise an opaque "unreachable executed"; this prints
    // the Rust panic message and a stack trace to the browser console.
    console_error_panic_hook::set_once();

    let root = document()
        .get_element_by_id("root")
        .expect("Root element not found. Did you forget to add it to index.html?")
        .unchecked_into::<web_sys::HtmlElement>();

    // The app owns the page for its whole lifetime, so the unmount handle is
    // deliberately dropped-with-leak rather than held in a static.
    leptos::mount::mount_to(root, app::App).forget();
}
