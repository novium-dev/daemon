use proc_macro2::{Delimiter, Group, Ident, Span, TokenStream, TokenTree};
use quote::quote;
use std::collections::HashMap;
use syn::parse::{Parse, ParseStream};
use syn::{Error, LitStr, Token};

pub(crate) struct ErrorDefinitionContent {
    pub(crate) map: HashMap<Ident, ErrorDefinitionInner>,
}

pub(crate) struct ErrorDefinitionInner {
    pub(crate) description: LitStr,
    pub(crate) statuscode: Ident,
}

pub(crate) fn define_error0(crate_prefix: TokenStream, attr: TokenStream) -> TokenStream {
    let mut result = Vec::new();

    let mut attr = attr.into_iter();

    let Some(TokenTree::Ident(category)) = attr.next() else {
        panic!("Expected identifier");
    };

    let Some(TokenTree::Group(content)) = attr.next() else {
        panic!("Expected braces");
    };
    if content.delimiter() != Delimiter::Brace {
        panic!("Expected braces");
    }
    let ErrorDefinitionContent { map } =
        syn::parse2(content.stream()).expect("Expected error content");

    for (error, inner) in map {
        let statuscode = inner.statuscode;
        let description = inner.description;

        result.extend(quote! {
            #crate_prefix paste::paste! {
                pub mod [< error_ #category:snake _ #error:snake >] {

                    pub const STATUSCODE: #crate_prefix http::StatusCode = #crate_prefix http::StatusCode::#statuscode;
                    pub const DESCRIPTION: &str = #description;

                    pub trait [< From #category:camel #error:camel >] {

                        fn from() -> Self where Self: Sized;

                    }

                }
            }
        });
    }

    TokenStream::from_iter(result)
}

impl Parse for ErrorDefinitionContent {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut map = HashMap::new();
        for (name, description, statuscode) in
            input.parse_terminated(parse_error_content, Token![,])?
        {
            map.insert(
                name,
                ErrorDefinitionInner {
                    description,
                    statuscode,
                },
            );
        }
        Ok(ErrorDefinitionContent { map })
    }
}

fn parse_error_content(stream: ParseStream) -> Result<(Ident, LitStr, Ident), syn::Error> {
    let name: Ident = stream.parse()?;
    let group: Group = stream.parse()?;
    if group.delimiter() != Delimiter::Parenthesis {
        return Err(Error::new(
            Span::call_site(),
            "Expected parenthesis (error content)",
        ));
    }
    let ErrorDefinitionInner {
        description,
        statuscode,
    } = syn::parse2(group.stream())?;

    Ok((name, description, statuscode))
}

impl Parse for ErrorDefinitionInner {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let description: LitStr = input.parse()?;

        let statuscode = input.parse()?;

        Ok(ErrorDefinitionInner {
            description,
            statuscode,
        })
    }
}
