//! Per-user identity for feature-flag evaluation — the Rust twin of
//! `apps/todo/web/src/lib/identity.ts`.
//!
//! todo-api is the single flag evaluation point: it resolves the caller's
//! identity (`X-Todo-Identity` header, `todo_identity` cookie, `?identity=`
//! query param) and asks Flagsmith for that user. This app only has to pick an
//! identity, persist it and attach it to everything it sends.
//!
//! Storage is `localStorage['todo_identity']`, with an in-memory fallback so a
//! blocked or absent storage (private mode, embedded webview) degrades instead
//! of failing.

use std::cell::RefCell;

/// App name reported to todo-api; becomes the Flagsmith trait `app`.
///
/// Deliberately `"web"` and not a fourth value: the flag catalogue has one flag
/// per delivery surface (`todo_app_web`/`_htmx`/`_astro`) and this arm exists to
/// be measured against `todo-web` under *identical* server-side evaluation. A
/// new app name would be evaluated against a flag todo-api does not know.
pub const TODO_APP: &str = "web";

const STORAGE_KEY: &str = "todo_identity";

thread_local! {
    /// Survives a `localStorage` that throws or is missing.
    static IN_MEMORY: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The identity grammar todo-api accepts: 1..=64 chars of `[A-Za-z0-9_.@-]`.
pub fn is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'@' | b'-'))
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn read_stored() -> Option<String> {
    storage()?.get_item(STORAGE_KEY).ok().flatten()
}

fn write_stored(value: &str) {
    if let Some(store) = storage() {
        // Storage is unavailable or full; `IN_MEMORY` already holds the value.
        let _ = store.set_item(STORAGE_KEY, value);
    }
}

/// `anon-<8 lowercase hex>`, the default identity for a first-time visitor.
/// Same source of randomness as the Solid app: `crypto.getRandomValues`.
fn generate() -> String {
    let mut bytes = [0u8; 4];
    let filled = web_sys::window()
        .and_then(|w| w.crypto().ok())
        .is_some_and(|c| c.get_random_values_with_u8_array(&mut bytes).is_ok());
    if !filled {
        // No crypto (non-secure context): a fixed identity is still a valid one,
        // and flags fail open, so the app stays usable.
        bytes = [0, 0, 0, 0];
    }
    let mut hex = String::with_capacity(13);
    hex.push_str("anon-");
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// The current identity, generating and persisting an anonymous one on first
/// use. A stored value wins over the in-memory one so another tab's switch is
/// picked up.
pub fn get() -> String {
    if let Some(stored) = read_stored()
        && is_valid(&stored)
    {
        IN_MEMORY.with(|cell| *cell.borrow_mut() = Some(stored.clone()));
        return stored;
    }
    if let Some(cached) = IN_MEMORY.with(|cell| cell.borrow().clone()) {
        return cached;
    }

    let generated = generate();
    IN_MEMORY.with(|cell| *cell.borrow_mut() = Some(generated.clone()));
    write_stored(&generated);
    generated
}

/// Persist `value` as the identity. Returns the stored (trimmed) value, or
/// `None` when it does not match the identity grammar — in which case nothing
/// is written.
pub fn set(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if !is_valid(trimmed) {
        return None;
    }
    IN_MEMORY.with(|cell| *cell.borrow_mut() = Some(trimmed.to_owned()));
    write_stored(trimmed);
    Some(trimmed.to_owned())
}
