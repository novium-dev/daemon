/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use proc_macro2::{Delimiter, Group, Ident, Span, TokenStream, TokenTree};
use quote::quote;
use std::collections::HashMap;
use syn::parse::{Parse, ParseStream};
use syn::{Error, LitStr, Token};

pub struct ErrorDefinitionContent {
    pub map: HashMap<Ident, ErrorDefinitionInner>,
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
        Ok(Self { map })
    }
}

pub struct ErrorDefinitionInner {
    pub description: LitStr,
    pub statuscode: Ident,
}

impl Parse for ErrorDefinitionInner {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let description: LitStr = input.parse()?;

        let statuscode = input.parse()?;

        Ok(Self {
            description,
            statuscode,
        })
    }
}

pub fn define_error0(crate_prefix: &TokenStream, attr: TokenStream) -> TokenStream {
    let mut result = Vec::new();

    let mut attr = attr.into_iter();

    let Some(TokenTree::Ident(category)) = attr.next() else {
        panic!("Expected identifier");
    };

    let Some(TokenTree::Group(content)) = attr.next() else {
        panic!("Expected braces");
    };
    assert_eq!(content.delimiter(), Delimiter::Brace, "Expected braces");
    let ErrorDefinitionContent { map } =
        syn::parse2(content.stream()).expect("Expected error content");

    for (error, inner) in map {
        let statuscode = inner.statuscode;
        let description = inner.description;

        result.extend(quote! {
            #crate_prefix pastey::paste! {
                #[automatically_derived]
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
