#![doc = include_str!("../README.md")]

use core_strings::capitalize_first_letter;
use darling::FromDeriveInput;
use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{DeriveInput, Lit, Meta, parse_macro_input};

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(sea_orm_resource), forward_attrs(sea_orm))]
struct SeaOrmResourceInput {
    ident: syn::Ident,
    attrs: Vec<syn::Attribute>,
    #[darling(default)]
    collection: Option<String>,
    #[darling(default)]
    url: Option<String>,
    #[darling(default)]
    tag: Option<String>,
}

/// Derives the `ApiResource` trait implementation for sea-orm entities.
///
/// This macro extracts the `table_name` from the `#[sea_orm(table_name = "...")]` attribute
/// and generates REST API resource constants.
///
/// # Attributes
///
/// - `collection`: Override the collection name (default: table_name from sea_orm)
/// - `url`: Override the default URL path (default: `/table_name`)
/// - `tag`: Override the default API tag (default: capitalized table_name)
///
/// # Generated Constants
///
/// - `URL`: The base URL path for this resource (plural, e.g., "/projects")
/// - `COLLECTION`: The database collection or table name
/// - `TAG`: The API documentation tag
///
/// # Requirements
///
/// The struct must have a `#[sea_orm(table_name = "...")]` attribute.
///
/// # Examples
///
/// ```ignore
/// #[derive(DeriveEntityModel, SeaOrmResource)]
/// #[sea_orm(table_name = "users")]
/// pub struct Model {
///     #[sea_orm(primary_key)]
///     pub id: Uuid,
///     pub email: String,
/// }
///
/// assert_eq!(Model::COLLECTION, "users");
/// assert_eq!(Model::URL, "/users");
/// assert_eq!(Model::TAG, "Users");
/// ```
#[proc_macro_derive(SeaOrmResource, attributes(sea_orm_resource))]
pub fn sea_orm_resource_derive(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = parse_macro_input!(input as DeriveInput);
    let receiver = match SeaOrmResourceInput::from_derive_input(&ast) {
        Ok(receiver) => receiver,
        Err(err) => return TokenStream::from(err.write_errors()),
    };

    match impl_sea_orm_resource(receiver) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Convert underscores to hyphens for URL-friendly paths
fn underscores_to_hyphens(input: &str) -> String {
    input.replace('_', "-")
}

/// Convert snake_case to Title Case for tags
/// Example: "cloud_resources" -> "Cloud Resources"
fn snake_case_to_title_case(input: &str) -> String {
    input
        .split('_')
        .map(capitalize_first_letter)
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_table_name(attrs: &[syn::Attribute]) -> syn::Result<Option<String>> {
    for attr in attrs {
        if attr.path().is_ident("sea_orm")
            && let Meta::List(meta_list) = &attr.meta
        {
            let mut table_name = None;
            meta_list.parse_nested_meta(|meta| {
                if meta.path.is_ident("table_name") {
                    let lit: Lit = meta.value()?.parse()?;
                    if let Lit::Str(lit_str) = &lit {
                        table_name = Some(lit_str.value());
                        Ok(())
                    } else {
                        Err(syn::Error::new(
                            lit.span(),
                            "expected a string literal for `table_name`",
                        ))
                    }
                } else {
                    // Consume and ignore the value of any other sea_orm
                    // option (e.g. `schema_name = "..."`); bare flags need
                    // no consumption.
                    if let Ok(value) = meta.value() {
                        let _: syn::Expr = value.parse()?;
                    }
                    Ok(())
                }
            })?;
            if table_name.is_some() {
                return Ok(table_name);
            }
        }
    }
    Ok(None)
}

fn impl_sea_orm_resource(receiver: SeaOrmResourceInput) -> syn::Result<proc_macro2::TokenStream> {
    let ident = &receiver.ident;

    // Extract table_name from #[sea_orm(table_name = "...")]
    let Some(table_name) = extract_table_name(&receiver.attrs)? else {
        // Point the diagnostic at the sea_orm attribute when one exists,
        // otherwise at the struct name.
        let span = receiver
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("sea_orm"))
            .map_or_else(|| ident.span(), |attr| attr.span());
        return Err(syn::Error::new(
            span,
            "SeaOrmResource requires #[sea_orm(table_name = \"...\")] attribute",
        ));
    };

    // Generate defaults based on table_name
    let collection = receiver.collection.unwrap_or_else(|| table_name.clone());

    // URL uses plural with hyphens (table_name is already plural by convention)
    // Convert underscores to hyphens for URL-friendly paths
    // No /api prefix - that's added by the router layer
    let url = receiver
        .url
        .unwrap_or_else(|| format!("/{}", underscores_to_hyphens(&table_name)));

    // Convert snake_case to Title Case for better API documentation
    let tag = receiver
        .tag
        .unwrap_or_else(|| snake_case_to_title_case(&collection));

    Ok(quote! {
        impl ::core_proc_macros::ApiResource for #ident {
            const URL: &'static str = #url;
            const COLLECTION: &'static str = #collection;
            const TAG: &'static str = #tag;
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn test_extract_table_name() {
        let input = quote! {
            #[sea_orm(table_name = "projects")]
            pub struct Model {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let table_name = extract_table_name(&ast.attrs).unwrap();
        assert_eq!(table_name, Some("projects".to_string()));
    }

    #[test]
    fn test_extract_table_name_with_other_attrs() {
        let input = quote! {
            #[derive(Clone, Debug)]
            #[sea_orm(table_name = "users")]
            pub struct Model {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let table_name = extract_table_name(&ast.attrs).unwrap();
        assert_eq!(table_name, Some("users".to_string()));
    }

    #[test]
    fn test_basic_struct() {
        let input = quote! {
            #[sea_orm(table_name = "projects")]
            pub struct Model {
                id: String,
                title: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SeaOrmResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_sea_orm_resource(receiver).unwrap();
        let output_str = output.to_string();

        assert!(output_str.contains("impl :: core_proc_macros :: ApiResource for Model"));
        assert!(output_str.contains(r#"const COLLECTION : & 'static str = "projects""#));
        assert!(output_str.contains(r#"const URL : & 'static str = "/projects""#));
        assert!(output_str.contains(r#"const TAG : & 'static str = "Projects""#));
    }

    #[test]
    fn test_custom_url() {
        let input = quote! {
            #[sea_orm(table_name = "projects")]
            #[sea_orm_resource(url = "/v1/projects")]
            pub struct Model {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SeaOrmResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_sea_orm_resource(receiver).unwrap();
        let output_str = output.to_string();

        assert!(output_str.contains(r#"const URL : & 'static str = "/v1/projects""#));
    }

    #[test]
    fn test_custom_tag() {
        let input = quote! {
            #[sea_orm(table_name = "projects")]
            #[sea_orm_resource(tag = "Project Management")]
            pub struct Model {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SeaOrmResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_sea_orm_resource(receiver).unwrap();
        let output_str = output.to_string();

        assert!(output_str.contains(r#"const TAG : & 'static str = "Project Management""#));
    }

    #[test]
    fn test_all_custom_attributes() {
        let input = quote! {
            #[sea_orm(table_name = "projects")]
            #[sea_orm_resource(
                collection = "project_items",
                url = "/v1/projects",
                tag = "Project Catalog"
            )]
            pub struct Model {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SeaOrmResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_sea_orm_resource(receiver).unwrap();
        let output_str = output.to_string();

        assert!(output_str.contains(r#"const COLLECTION : & 'static str = "project_items""#));
        assert!(output_str.contains(r#"const URL : & 'static str = "/v1/projects""#));
        assert!(output_str.contains(r#"const TAG : & 'static str = "Project Catalog""#));
    }

    #[test]
    fn test_missing_table_name() {
        let input = quote! {
            pub struct Model {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SeaOrmResourceInput::from_derive_input(&ast).unwrap();
        let result = impl_sea_orm_resource(receiver);

        assert!(result.is_err());
    }

    #[test]
    fn test_underscores_to_hyphens() {
        assert_eq!(underscores_to_hyphens("projects"), "projects");
        assert_eq!(underscores_to_hyphens("cloud_resources"), "cloud-resources");
        assert_eq!(underscores_to_hyphens("user_profiles"), "user-profiles");
        assert_eq!(underscores_to_hyphens("a_b_c"), "a-b-c");
    }

    #[test]
    fn test_snake_case_to_title_case() {
        assert_eq!(snake_case_to_title_case("projects"), "Projects");
        assert_eq!(
            snake_case_to_title_case("cloud_resources"),
            "Cloud Resources"
        );
        assert_eq!(snake_case_to_title_case("user_profiles"), "User Profiles");
        assert_eq!(snake_case_to_title_case("api_keys"), "Api Keys");
    }

    #[test]
    fn test_snake_case_with_underscores() {
        let input = quote! {
            #[sea_orm(table_name = "cloud_resources")]
            pub struct Model {
                id: String,
            }
        };

        let ast: DeriveInput = syn::parse2(input).unwrap();
        let receiver = SeaOrmResourceInput::from_derive_input(&ast).unwrap();
        let output = impl_sea_orm_resource(receiver).unwrap();
        let output_str = output.to_string();

        assert!(output_str.contains(r#"const COLLECTION : & 'static str = "cloud_resources""#));
        assert!(output_str.contains(r#"const URL : & 'static str = "/cloud-resources""#));
        assert!(output_str.contains(r#"const TAG : & 'static str = "Cloud Resources""#));
    }
}
