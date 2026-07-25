//! ApiResource derive macro for automatic REST API resource trait implementation.
//!
//! This crate provides the [`ApiResource`](macro@ApiResource) derive macro that automatically
//! implements resource metadata traits for API entities. It handles URL generation,
//! collection naming, and API tagging with sensible defaults and customization options.
//!
//! # Examples
//!
//! Basic usage with automatic pluralization and URL generation:
//!
//! ```ignore
//! use core_proc_macros::ApiResource;
//!
//! #[derive(ApiResource)]
//! pub struct User {
//!     id: String,
//!     email: String,
//! }
//!
//! // Auto-generated constants:
//! assert_eq!(User::COLLECTION, "users");
//! assert_eq!(User::URL, "/user");
//! assert_eq!(User::TAG, "Users");
//! ```
//!
//! Customizing resource configuration:
//!
//! ```ignore
//! use core_proc_macros::ApiResource;
//!
//! #[derive(ApiResource)]
//! #[api_resource(
//!     collection = "people",
//!     url = "/api/users",
//!     tag = "User Management"
//! )]
//! pub struct User {
//!     id: String,
//! }
//!
//! assert_eq!(User::COLLECTION, "people");
//! assert_eq!(User::URL, "/api/users");
//! assert_eq!(User::TAG, "User Management");
//! ```

use darling::FromDeriveInput;
use pluralizer::pluralize;
use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(api_resource))]
struct ApiResourceInput {
    ident: syn::Ident,
    #[darling(default)]
    collection: Option<String>,
    #[darling(default)]
    url: Option<String>,
    #[darling(default)]
    tag: Option<String>,
}

/// Derives the `ApiResource` trait implementation with automatic defaults.
///
/// This macro generates an implementation of the `ApiResource` trait for your struct,
/// providing constants for collection name, URL paths, and API tags.
///
/// # Attributes
///
/// - `collection`: Override the default pluralized collection name (default: pluralized struct name)
/// - `url`: Override the default URL path (default: `/lowercase_struct_name`)
/// - `tag`: Override the default API tag (default: capitalized collection name)
///
/// # Generated Constants
///
/// - `URL`: The base URL path for this resource
/// - `COLLECTION`: The database collection or table name
/// - `TAG`: The API documentation tag
///
/// # Requirements
///
/// The struct must be a named struct (not a tuple struct or unit struct).
///
/// # Examples
///
/// Default behavior with automatic pluralization:
///
/// ```ignore
/// #[derive(ApiResource)]
/// pub struct Product {
///     id: Uuid,
///     name: String,
/// }
///
/// assert_eq!(Product::COLLECTION, "products");
/// assert_eq!(Product::URL, "/product");
/// assert_eq!(Product::TAG, "Products");
/// ```
///
/// Custom configuration:
///
/// ```ignore
/// #[derive(ApiResource)]
/// #[api_resource(
///     collection = "product_items",
///     url = "/api/v1/products",
///     tag = "Product Catalog"
/// )]
/// pub struct Product {
///     id: Uuid,
/// }
///
/// assert_eq!(Product::COLLECTION, "product_items");
/// assert_eq!(Product::URL, "/api/v1/products");
/// assert_eq!(Product::TAG, "Product Catalog");
/// ```
///
/// Handles irregular pluralization:
///
/// ```ignore
/// #[derive(ApiResource)]
/// pub struct Story {
///     id: Uuid,
/// }
///
/// assert_eq!(Story::COLLECTION, "stories");
/// assert_eq!(Story::TAG, "Stories");
/// ```
#[proc_macro_derive(ApiResource, attributes(api_resource))]
pub fn api_resource_derive(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = parse_macro_input!(input as DeriveInput);
    let receiver = match ApiResourceInput::from_derive_input(&ast) {
        Ok(receiver) => receiver,
        Err(err) => return TokenStream::from(err.write_errors()),
    };
    impl_api_resource(receiver).into()
}

fn capitalize_first_letter(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn impl_api_resource(receiver: ApiResourceInput) -> proc_macro2::TokenStream {
    let ident = &receiver.ident;
    let name = ident.to_string().to_lowercase();

    // Generate defaults with sensible fallbacks
    let collection = receiver
        .collection
        .unwrap_or_else(|| pluralize(&name, 2, false));

    let url = receiver.url.unwrap_or_else(|| format!("/{name}"));

    let tag = receiver
        .tag
        .unwrap_or_else(|| capitalize_first_letter(&collection));

    quote! {
        impl ::core_proc_macros::ApiResource for #ident {
            const URL: &'static str = #url;
            const COLLECTION: &'static str = #collection;
            const TAG: &'static str = #tag;
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
        let receiver = ApiResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_api_resource(receiver);
        let output_str = output.to_string();

        assert!(output_str.contains("impl :: core_proc_macros :: ApiResource for User"));
        assert!(output_str.contains(r#"const COLLECTION : & 'static str = "users""#));
        assert!(output_str.contains(r#"const URL : & 'static str = "/user""#));
        assert!(output_str.contains(r#"const TAG : & 'static str = "Users""#));
    }

    #[test]
    fn test_custom_collection() {
        let input = quote! {
            #[api_resource(collection = "people")]
            pub struct User {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = ApiResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_api_resource(receiver);
        let output_str = output.to_string();

        assert!(output_str.contains(r#"const COLLECTION : & 'static str = "people""#));
        assert!(output_str.contains(r#"const TAG : & 'static str = "People""#));
    }

    #[test]
    fn test_custom_url() {
        let input = quote! {
            #[api_resource(url = "/api/users")]
            pub struct User {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = ApiResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_api_resource(receiver);
        let output_str = output.to_string();

        assert!(output_str.contains(r#"const URL : & 'static str = "/api/users""#));
    }

    #[test]
    fn test_custom_tag() {
        let input = quote! {
            #[api_resource(tag = "User Management")]
            pub struct User {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = ApiResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_api_resource(receiver);
        let output_str = output.to_string();

        assert!(output_str.contains(r#"const TAG : & 'static str = "User Management""#));
    }

    #[test]
    fn test_all_custom_attributes() {
        let input = quote! {
            #[api_resource(
                collection = "product_items",
                url = "/api/v1/products",
                tag = "Product Catalog"
            )]
            pub struct Product {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = ApiResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_api_resource(receiver);
        let output_str = output.to_string();

        assert!(output_str.contains(r#"const COLLECTION : & 'static str = "product_items""#));
        assert!(output_str.contains(r#"const URL : & 'static str = "/api/v1/products""#));
        assert!(output_str.contains(r#"const TAG : & 'static str = "Product Catalog""#));
    }

    #[test]
    fn test_story_pluralization() {
        let input = quote! {
            pub struct Story {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = ApiResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_api_resource(receiver);
        let output_str = output.to_string();

        assert!(output_str.contains(r#"const COLLECTION : & 'static str = "stories""#));
        assert!(output_str.contains(r#"const URL : & 'static str = "/story""#));
        assert!(output_str.contains(r#"const TAG : & 'static str = "Stories""#));
    }

    #[test]
    fn test_capitalize_first_letter() {
        assert_eq!(capitalize_first_letter(""), "");
        assert_eq!(capitalize_first_letter("a"), "A");
        assert_eq!(capitalize_first_letter("hello"), "Hello");
        assert_eq!(capitalize_first_letter("users"), "Users");
        assert_eq!(capitalize_first_letter("API"), "API");
    }

    #[test]
    fn test_product_defaults() {
        let input = quote! {
            pub struct Product {
                id: String,
                name: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = ApiResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_api_resource(receiver);
        let output_str = output.to_string();

        assert!(output_str.contains(r#"const COLLECTION : & 'static str = "products""#));
        assert!(output_str.contains(r#"const TAG : & 'static str = "Products""#));
    }
}
