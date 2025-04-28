pub(crate) mod define;

use either::Either;
use proc_macro2::{Delimiter, Group, Ident, Punct, Span, TokenStream, TokenTree};
use quote::quote;
use std::collections::{HashMap, HashSet};
use syn::parse::{Parse, ParseStream};
use syn::{LitStr, Token};

type ErrorAttributes = Vec<(Ident, Option<Ident>)>;
type ErrorInner = Either<(Ident, Ident, LitStr), (Ident, Ident)>;

pub(crate) struct ErrorContent {
    pub(crate) ident: Ident,
    pub(crate) errors: Vec<(ErrorInner, ErrorAttributes)>,
}

pub(crate) struct ErrorCategoryContent {
    pub(crate) vec: Vec<(Ident, ErrorAttributes)>,
}

pub(crate) struct ErrorCustomContent {
    pub(crate) vec: Vec<(Ident, Ident, LitStr, ErrorAttributes)>,
}

pub(crate) struct ErrorAttributesContent {
    pub(crate) vec: Vec<(Ident, Option<Ident>)>,
}

pub(crate) fn error0(crate_prefix: TokenStream, input: TokenStream) -> TokenStream {
    let ErrorContent { ident, mut errors } = syn::parse2(input).expect("Expected error content");

    let mut content = TokenStream::new();
    let mut response = TokenStream::new();
    let mut openapi = TokenStream::new();
    let mut impls = TokenStream::new();

    errors.insert(
        0,
        (
            Either::Left((
                Ident::new("Communication", Span::call_site()),
                Ident::new("BAD_REQUEST", Span::call_site()),
                LitStr::new("Invalid Request/Response", Span::call_site()),
            )),
            Vec::new(),
        ),
    );

    let mut attributes_parsed = HashMap::new();
    for (error, attributes) in errors {
        let entry = match error {
            Either::Left((error_ident, statuscode, msg)) => {
                let error_id = LitStr::new(&error_ident.to_string(), Span::call_site());

                if error_ident.to_string().eq("Communication") {
                    content.extend(quote! {
                        #error_ident (#crate_prefix reqwest::Error),
                    });

                    response.extend(quote! {
                        #error_ident(_err) => panic!("You tried to use the communication error entry as response"),
                    });
                } else {
                    content.extend(quote! {
                        #error_ident,
                    });

                    response.extend(quote! {
                        #error_ident => (#crate_prefix http::StatusCode::#statuscode,#crate_prefix axum::extract::Json(serde_json::json!({
                            "error": {
                                "id": #error_id,
                                "msg": #msg,
                            }
                        }))),
                    });

                    openapi.extend(quote! {
                        responses_owned = responses_owned.response(
                            #crate_prefix http::StatusCode::#statuscode.as_str(),
                            #crate_prefix utoipa::openapi::ResponseBuilder::new()
                                .description(#msg)
                                .content(
                                    "application/json",
                                    #crate_prefix utoipa::openapi::content::Content::new(Some(
                                        #crate_prefix utoipa::openapi::RefOr::T(#crate_prefix internal::error_schema(#error_id, #msg))
                                    )),
                                )
                                .build(),
                        );

                    });
                }
                quote! { [< #error_ident >] }
            }
            Either::Right((category, error_ident)) => {
                let error_id =
                    LitStr::new(&format!("{}/{}", category, error_ident), Span::call_site());

                content.extend(quote! {
                    [< #category:camel #error_ident:camel >],
                });

                response.extend(quote! {
                    [< #category:camel #error_ident:camel >] => ( [< error_ #category _ #error_ident >]::STATUSCODE, #crate_prefix axum::extract::Json(serde_json::json!({
                        "error": {
                            "id": #error_id,
                            "msg": [< error_ #category _ #error_ident >]::DESCRIPTION,
                        }
                    }))),
                });

                openapi.extend(quote! {
                    responses_owned = responses_owned.response(
                        [< error_ #category _ #error_ident >]::STATUSCODE.as_str(),
                        #crate_prefix utoipa::openapi::ResponseBuilder::new()
                            .description([< error_ #category _ #error_ident >]::DESCRIPTION)
                            .content(
                                "application/json",
                                #crate_prefix utoipa::openapi::content::Content::new(Some(
                                    #crate_prefix utoipa::openapi::RefOr::T(#crate_prefix internal::error_schema(#error_id, [< error_ #category _ #error_ident >]::DESCRIPTION))
                                )),
                            )
                            .build(),
                    );
                });

                impls.extend(quote! {
                    impl [< error_ #category _ #error_ident >]::[< From #category:camel #error_ident:camel >] for #ident {

                        fn from() -> Self {
                            Self::[< #category:camel #error_ident:camel >]
                        }

                    }
                });
                quote! { [< #category:camel #error_ident:camel >] }
            }
        };
        let attributes: Vec<_> = attributes
            .into_iter()
            .map(|(attr, key)| (attr.to_string(), key))
            .collect();

        for (attr, key) in &attributes {
            if key.is_none() {
                if attributes_parsed.contains_key(attr) {
                    panic!("Duplicate default entry for {} in error", attr);
                }
                attributes_parsed
                    .entry(attr.clone())
                    .or_insert((entry.clone(), Vec::new()));
            }
        }
        for (attr, key) in attributes {
            if key.is_none() {
                continue;
            }
            attributes_parsed
                .get_mut(&attr)
                .expect("Default implementation missing in error attribute")
                .1
                .push((key.unwrap(), entry.clone()));
        }
    }

    for (attr, (_, vec)) in &attributes_parsed {
        let mut set = HashSet::new();
        for (entry, _) in vec {
            if !set.insert(entry.to_string()) {
                panic!("Duplicate entry {} for {} in error", entry, attr);
            }
        }
    }

    for (attr, (def, other)) in attributes_parsed {
        match attr.as_str() {
            "io" => {
                let mut matches = TokenStream::new();
                for (kind, variant) in other {
                    matches.extend(quote! { ErrorKind::#kind => Self::#variant, });
                }
                impls.extend(quote! {
                    impl From<std::io::Error> for #ident {

                        fn from(err: std::io::Error) -> Self {
                            use std::io::ErrorKind;
                            match err.kind() {
                                #matches
                                _ => Self::#def,
                            }
                        }

                    }
                });
            }
            "sql" => {
                let mut matches = TokenStream::new();
                let mut database_matches = TokenStream::new();
                for (kind, variant) in other {
                    match kind.to_string().as_str() {
                        "not_found" => {
                            matches.extend(quote! {
                                if let #crate_prefix sqlx::Error::RowNotFound = err {
                                    return Self::#variant;
                                }
                            });
                        }
                        "check" => {
                            database_matches.extend(quote! {
                                    if database.is_check_violation() {
                                        return Self::#variant;
                                    }
                            });
                        }
                        "foreign_key" => {
                            database_matches.extend(quote! {
                                    if database.is_foreign_key_violation() {
                                        return Self::#variant;
                                    }
                            });
                        }
                        "unique" => {
                            database_matches.extend(quote! {
                                    if database.is_check_violation() {
                                        return Self::#variant;
                                    }
                            });
                        }
                        _ => panic!("Unknown entry {} for {} in error", kind, attr),
                    }
                }
                if !database_matches.is_empty() {
                    matches.extend(quote! {
                        if let #crate_prefix sqlx::Error::Database(ref database) = err {
                            #database_matches
                        }
                    });
                }
                impls.extend(quote! {
                    impl From<#crate_prefix sqlx::Error> for #ident {

                        fn from(err: #crate_prefix sqlx::Error) -> Self {
                            #matches
                            Self::#def
                        }

                    }
                });
            }
            _ => panic!("Unknown attribute {} in error", attr),
        }
    }

    quote! {
        #crate_prefix paste::paste! {
            #[derive(Debug)]
            pub enum #ident {
                #content
            }

            impl #crate_prefix axum::response::IntoResponse for #ident {

                fn into_response(self) -> #crate_prefix http::response::Response<#crate_prefix axum::body::Body> {
                    use #ident::*;
                    match self {
                        #response
                    }.into_response()
                }

            }

            impl #crate_prefix FrameworkDocumentedResponse for #ident {

                fn openapi(responses: &mut #crate_prefix utoipa::openapi::ResponsesBuilder, _schemas: &mut std::collections::HashMap<String, #crate_prefix utoipa::openapi::RefOr<#crate_prefix utoipa::openapi::schema::Schema>>) {
                    let mut responses_owned = #crate_prefix utoipa::openapi::ResponsesBuilder::new();
                    std::mem::swap(responses, &mut responses_owned);
                    #openapi
                    *responses = responses_owned;
                }

            }

            #impls
        }
    }
}

impl Parse for ErrorContent {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut errors = Vec::new();
        let ident: Ident = input.parse()?;

        while !input.is_empty() {
            if input.peek(Token![,]) {
                let _: Punct = input.parse()?;
            }
            if input.peek(syn::Ident) {
                let category: Ident = input.parse()?;
                let content: Group = input.parse()?;
                if content.delimiter() != Delimiter::Parenthesis {
                    return Err(syn::Error::new(content.span(), "Expected parenthesis"));
                }
                let content: ErrorCategoryContent = syn::parse2(content.stream())?;
                for (error, attributes) in content.vec {
                    errors.push((Either::Right((category.clone(), error)), attributes));
                }
            } else if input.peek(syn::token::Brace) {
                let custom: Group = input.parse()?;
                let content: ErrorCustomContent = syn::parse2(custom.stream())?;
                for (error, statuscode, description, attributes) in content.vec {
                    errors.push((Either::Left((error, statuscode, description)), attributes));
                }
            } else {
                let token: TokenTree = input.parse()?;
                return Err(syn::Error::new(
                    token.span(),
                    "Expected identifier or braces",
                ));
            }

            if input.peek(Token![,]) {
                let _: Punct = input.parse()?;
            }
        }

        Ok(ErrorContent { ident, errors })
    }
}

impl Parse for ErrorCustomContent {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut vec = Vec::new();

        while !input.is_empty() {
            if input.peek(Token![,]) {
                let _: Punct = input.parse()?;
            }
            let error: Ident = input.parse()?;
            let description: LitStr = input.parse()?;
            let statuscode: Ident = input.parse()?;
            let attributes = if input.peek(syn::token::Bracket) {
                let group: Group = input.parse()?;
                let attributes: ErrorAttributesContent = syn::parse2(group.stream())?;
                attributes.vec
            } else {
                Vec::new()
            };
            vec.push((error, statuscode, description, attributes));

            if input.peek(Token![,]) {
                let _: Punct = input.parse()?;
            }
        }

        Ok(ErrorCustomContent { vec })
    }
}

impl Parse for ErrorCategoryContent {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut vec = Vec::new();

        while !input.is_empty() {
            let error: Ident = input.parse()?;
            let attributes = if input.peek(syn::token::Bracket) {
                let group: Group = input.parse()?;
                let attributes: ErrorAttributesContent = syn::parse2(group.stream())?;
                attributes.vec
            } else {
                Vec::new()
            };
            vec.push((error, attributes));
        }

        Ok(ErrorCategoryContent { vec })
    }
}

impl Parse for ErrorAttributesContent {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut vec = Vec::new();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let p: Punct = input.parse()?;
            if p.as_char() != ':' {
                return Err(syn::Error::new(p.span(), "Expected :"));
            }
            if input.peek(Token![*]) {
                let _: Punct = input.parse()?;
                vec.push((key, None));
            } else {
                let value: Ident = input.parse()?;
                vec.push((key, Some(value)));
            }
        }

        Ok(ErrorAttributesContent { vec })
    }
}
