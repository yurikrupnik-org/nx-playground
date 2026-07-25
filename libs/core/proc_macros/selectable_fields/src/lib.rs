//! SelectableFields derive macro for automatic field selection support.
//!
//! This crate provides the [`SelectableFields`](macro@SelectableFields) derive macro that enables
//! automatic implementation of field selection with role-based access control.
//!
//! # Examples
//!
//! Basic usage:
//!
//! ```ignore
//! use selectable_fields::SelectableFields;
//!
//! #[derive(SelectableFields)]
//! pub struct User {
//!     id: String,
//!     username: String,
//!     email: String,
//! }
//!
//! // Auto-generated:
//! // - User::available_fields() -> ["id", "username", "email"]
//! // - All fields accessible to anonymous users
//! ```
//!
//! Restricting sensitive fields:
//!
//! ```ignore
//! #[derive(SelectableFields)]
//! pub struct User {
//!     id: String,
//!     username: String,
//!     #[field(skip)]
//!     password_hash: String,  // Never exposed via field selection
//! }
//! ```
//!
//! Role-based field access:
//!
//! ```ignore
//! #[derive(SelectableFields)]
//! pub struct User {
//!     id: String,
//!     username: String,
//!
//!     #[field(role = "user")]
//!     email: String,  // Requires authenticated user
//!
//!     #[field(role = "admin")]
//!     internal_notes: String,  // Admin only
//! }
//! ```

use darling::{FromDeriveInput, FromField, FromMeta};
use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[derive(FromDeriveInput)]
#[darling(attributes(selectable), supports(struct_named))]
struct SelectableInput {
    ident: syn::Ident,
    data: darling::ast::Data<(), SelectableField>,
}

#[derive(FromField)]
#[darling(attributes(field))]
struct SelectableField {
    ident: Option<syn::Ident>,
    /// Skip this field entirely (restricted field)
    #[darling(default)]
    skip: bool,
    /// Role required to access this field: "anonymous", "user", or "admin"
    #[darling(default)]
    role: Option<Role>,
    /// Rename the field in the API
    #[darling(default)]
    rename: Option<String>,
}

/// Minimum role required to access a field.
#[derive(Debug, Clone, Copy, Default, FromMeta)]
enum Role {
    #[default]
    Anonymous,
    User,
    Admin,
}

impl Role {
    fn to_tokens(self) -> proc_macro2::TokenStream {
        match self {
            Role::Anonymous => quote! { ::field_selector::UserRole::Anonymous },
            Role::User => quote! { ::field_selector::UserRole::User },
            Role::Admin => quote! { ::field_selector::UserRole::Admin },
        }
    }
}

/// Derives the `SelectableFields` trait for dynamic field selection with security.
///
/// This macro generates an implementation that allows runtime field selection
/// with role-based access control, field validation, and security restrictions.
///
/// # Attributes
///
/// - `skip`: Exclude a field from selection (restricted field, never accessible)
/// - `role`: Minimum role required to access this field ("anonymous", "user", "admin")
/// - `rename`: Use a different name for the field in API responses
///
/// # Generated Trait Implementation
///
/// Implements the `SelectableFields` trait with:
/// - `available_fields()`: Returns all non-skipped field names
/// - `restricted_fields()`: Returns fields marked with `skip`
/// - `field_access()`: Returns role requirements for each field
///
/// # Requirements
///
/// The struct must implement or derive `Serialize`.
///
/// # Examples
///
/// Basic field selection:
///
/// ```ignore
/// #[derive(SelectableFields, Serialize)]
/// pub struct Product {
///     id: Uuid,
///     name: String,
///     price: f64,
/// }
///
/// // All fields accessible to everyone by default
/// assert_eq!(Product::available_fields(), ["id", "name", "price"]);
/// ```
///
/// With security attributes:
///
/// ```ignore
/// #[derive(SelectableFields, Serialize)]
/// pub struct User {
///     id: Uuid,
///     username: String,
///
///     #[field(role = "user")]
///     email: String,  // Requires authentication
///
///     #[field(role = "admin")]
///     internal_id: i64,  // Admin only
///
///     #[field(skip)]
///     password_hash: String,  // Never exposed
/// }
/// ```
///
/// With field renaming:
///
/// ```ignore
/// #[derive(SelectableFields, Serialize)]
/// pub struct ApiResponse {
///     #[field(rename = "response_id")]
///     id: Uuid,
///
///     #[field(rename = "data")]
///     payload: String,
/// }
/// ```
#[proc_macro_derive(SelectableFields, attributes(field, selectable))]
pub fn selectable_fields_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    match SelectableInput::from_derive_input(&ast) {
        Ok(receiver) => impl_selectable_fields(receiver).into(),
        Err(err) => err.write_errors().into(),
    }
}

fn impl_selectable_fields(receiver: SelectableInput) -> proc_macro2::TokenStream {
    let ident = &receiver.ident;

    // `supports(struct_named)` guarantees named-struct data; keep a spanned
    // error rather than a panic if that invariant ever breaks.
    let Some(fields) = receiver.data.take_struct() else {
        return darling::Error::unsupported_shape("enum")
            .with_span(ident)
            .write_errors();
    };

    // Separate fields into different categories
    let mut available_fields = Vec::new();
    let mut restricted_fields = Vec::new();
    let mut field_access_items = Vec::new();

    for field in fields {
        // Named fields are guaranteed by `supports(struct_named)`.
        let Some(field_ident) = &field.ident else {
            continue;
        };
        let field_name = field.rename.unwrap_or_else(|| field_ident.to_string());

        if field.skip {
            // Restricted field - never accessible
            restricted_fields.push(field_name);
        } else {
            // Available field with its role requirement
            let role = field.role.unwrap_or_default().to_tokens();
            field_access_items.push(quote! {
                ::field_selector::FieldAccess {
                    field: #field_name,
                    required_role: #role,
                }
            });
            available_fields.push(field_name);
        }
    }

    quote! {
        impl ::field_selector::SelectableFields for #ident {
            fn available_fields() -> &'static [&'static str] {
                &[#(#available_fields),*]
            }

            fn restricted_fields() -> &'static [&'static str] {
                &[#(#restricted_fields),*]
            }

            fn field_access() -> &'static [::field_selector::FieldAccess] {
                &[
                    #(#field_access_items),*
                ]
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn test_basic_struct() {
        let input = quote! {
            pub struct User {
                id: String,
                email: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SelectableInput::from_derive_input(&ast).unwrap();
        let output = impl_selectable_fields(receiver);
        let output_str = output.to_string();

        assert!(output_str.contains("impl :: field_selector :: SelectableFields for User"));
        assert!(output_str.contains(r#"& ["id" , "email"]"#));
        assert!(output_str.contains("UserRole :: Anonymous"));
    }

    #[test]
    fn test_skip_field() {
        let input = quote! {
            pub struct User {
                id: String,
                email: String,
                #[field(skip)]
                password: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SelectableInput::from_derive_input(&ast).unwrap();
        let output = impl_selectable_fields(receiver);
        let output_str = output.to_string();

        // Available fields should not include password
        assert!(output_str.contains(r#"& ["id" , "email"]"#));

        // Restricted fields should include password
        assert!(output_str.contains(r#"fn restricted_fields"#));
        assert!(output_str.contains(r#""password""#));
    }

    #[test]
    fn test_role_based_access() {
        let input = quote! {
            pub struct User {
                id: String,
                #[field(role = "user")]
                email: String,
                #[field(role = "admin")]
                internal_id: i64,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SelectableInput::from_derive_input(&ast).unwrap();
        let output = impl_selectable_fields(receiver);
        let output_str = output.to_string();

        assert!(output_str.contains("UserRole :: Anonymous")); // for id
        assert!(output_str.contains("UserRole :: User")); // for email
        assert!(output_str.contains("UserRole :: Admin")); // for internal_id
    }

    #[test]
    fn test_invalid_role_is_error() {
        let input = quote! {
            pub struct User {
                id: String,
                #[field(role = "superuser")]
                email: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        assert!(SelectableInput::from_derive_input(&ast).is_err());
    }

    #[test]
    fn test_enum_is_error() {
        let input = quote! {
            pub enum Status {
                Active,
                Inactive,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        assert!(SelectableInput::from_derive_input(&ast).is_err());
    }

    #[test]
    fn test_rename_field() {
        let input = quote! {
            pub struct User {
                id: String,
                #[field(rename = "user_email")]
                email: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SelectableInput::from_derive_input(&ast).unwrap();
        let output = impl_selectable_fields(receiver);
        let output_str = output.to_string();

        assert!(output_str.contains(r#"& ["id" , "user_email"]"#));
        assert!(!output_str.contains(r#""email""#));
    }

    #[test]
    fn test_combined_attributes() {
        let input = quote! {
            pub struct ApiRequest {
                #[field(rename = "request_id")]
                id: String,
                endpoint: String,
                #[field(skip)]
                auth_token: String,
                #[field(role = "admin", rename = "admin_notes")]
                notes: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SelectableInput::from_derive_input(&ast).unwrap();
        let output = impl_selectable_fields(receiver);
        let output_str = output.to_string();

        // Check available fields
        assert!(output_str.contains(r#""request_id""#));
        assert!(output_str.contains(r#""endpoint""#));
        assert!(output_str.contains(r#""admin_notes""#));

        // Check restricted fields
        assert!(output_str.contains(r#""auth_token""#));

        // Check roles
        assert!(output_str.contains("UserRole :: Admin"));
    }

    #[test]
    fn test_all_fields_public() {
        let input = quote! {
            pub struct Product {
                name: String,
                price: f64,
                sku: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SelectableInput::from_derive_input(&ast).unwrap();
        let output = impl_selectable_fields(receiver);
        let output_str = output.to_string();

        // All fields should be available
        assert!(output_str.contains(r#"& ["name" , "price" , "sku"]"#));

        // No restricted fields
        assert!(output_str.contains(r#"fn restricted_fields"#));
        assert!(output_str.contains(r#"& []"#));
    }
}
