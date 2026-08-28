# sea_orm_resource

SeaOrmResource derive macro for automatic REST API resource trait implementation.

This crate provides the `SeaOrmResource` derive macro that automatically
implements resource metadata traits for sea-orm entities by extracting the table name.

This README is the crate-level rustdoc (`#![doc = include_str!("../README.md")]`).

## Examples

Basic usage — extracts `table_name` from the `sea_orm` attribute:

```rust,ignore
use sea_orm::entity::prelude::*;
use core_proc_macros::SeaOrmResource;

#[derive(Clone, Debug, DeriveEntityModel, SeaOrmResource)]
#[sea_orm(table_name = "projects")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub title: String,
}

// Auto-generated constants (using table_name):
// - Underscores in table names are converted to hyphens in URLs
// - Snake_case is converted to Title Case for tags
assert_eq!(Model::COLLECTION, "projects");
assert_eq!(Model::URL, "/projects");
assert_eq!(Model::TAG, "Projects");
```

With underscores in table name:

```rust,ignore
#[derive(Clone, Debug, DeriveEntityModel, SeaOrmResource)]
#[sea_orm(table_name = "cloud_resources")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
}

assert_eq!(Model::COLLECTION, "cloud_resources");
assert_eq!(Model::URL, "/cloud-resources");  // Hyphen for URL
assert_eq!(Model::TAG, "Cloud Resources");  // Title Case
```

Customizing resource configuration:

```rust,ignore
use sea_orm::entity::prelude::*;
use core_proc_macros::SeaOrmResource;

#[derive(Clone, Debug, DeriveEntityModel, SeaOrmResource)]
#[sea_orm(table_name = "projects")]
#[sea_orm_resource(
    url = "/v1/projects",
    tag = "Project Management"
)]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
}

assert_eq!(Model::COLLECTION, "projects");
assert_eq!(Model::URL, "/v1/projects");
assert_eq!(Model::TAG, "Project Management");
```

Title Case tags are built with `core_strings::capitalize_first_letter`.
