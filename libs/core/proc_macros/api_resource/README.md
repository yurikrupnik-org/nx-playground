# api_resource

ApiResource derive macro for automatic REST API resource trait implementation.

This crate provides the `ApiResource` derive macro that automatically
implements resource metadata traits for API entities. It handles URL generation,
collection naming, and API tagging with sensible defaults and customization options.

This README is the crate-level rustdoc (`#![doc = include_str!("../README.md")]`).

## Examples

Basic usage with automatic pluralization and URL generation:

```rust,ignore
use core_proc_macros::ApiResource;

#[derive(ApiResource)]
pub struct User {
    id: String,
    email: String,
}

// Auto-generated constants:
assert_eq!(User::COLLECTION, "users");
assert_eq!(User::URL, "/user");
assert_eq!(User::TAG, "Users");
```

Customizing resource configuration:

```rust,ignore
use core_proc_macros::ApiResource;

#[derive(ApiResource)]
#[api_resource(
    collection = "people",
    url = "/api/users",
    tag = "User Management"
)]
pub struct User {
    id: String,
}

assert_eq!(User::COLLECTION, "people");
assert_eq!(User::URL, "/api/users");
assert_eq!(User::TAG, "User Management");
```

Pluralization uses the `pluralizer` crate (e.g. `Story` → `stories`); default
tags are capitalized via `core_strings::capitalize_first_letter`.
