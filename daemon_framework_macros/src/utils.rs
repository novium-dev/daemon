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
use proc_macro2::{Ident, Punct, TokenStream, TokenTree};
use quote::TokenStreamExt as _;
use syn::Token;
use syn::ext::IdentExt as _;
use syn::parse::{Parse, ParseStream};

pub struct TokenStreamArray(pub Vec<TokenStream>);

impl Parse for TokenStreamArray {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input
            .parse_terminated(parse_tokenstream_array, Token![,])
            .map(|result| Self(result.into_iter().collect()))
    }
}

struct OptionTypeParseResult(Option<TokenStream>);

impl Parse for OptionTypeParseResult {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if !input.peek(syn::Ident::peek_any) {
            while !input.is_empty() {
                let _: TokenTree = input.parse()?;
            }
            return Ok(Self(None));
        }
        let ident: Ident = input.parse()?;
        if !ident.to_string().eq("Option") {
            while !input.is_empty() {
                let _: TokenTree = input.parse()?;
            }
            return Ok(Self(None));
        }
        let punct: Punct = input.parse()?;
        if punct.as_char() != '<' {
            return Err(syn::Error::new(punct.span(), "Expected < after Option"));
        }
        let mut stream = TokenStream::new();

        let mut layer = 1u8;
        while !input.is_empty() {
            let token: TokenTree = input.parse()?;
            if let TokenTree::Punct(ref punct) = token {
                if punct.as_char() == '<' {
                    layer = layer
                        .checked_add(1)
                        .expect("Overflow occurred adding layer");
                }
                if punct.as_char() == '>' {
                    layer = layer
                        .checked_sub(1)
                        .expect("Underflow occurred removing layer");
                }
            }
            if layer == 0 {
                break;
            }
            stream.extend([token]);
        }

        if layer != 0 || !input.is_empty() {
            return Err(syn::Error::new(punct.span(), "Unable to parse Option"));
        }
        Ok(Self(Some(stream)))
    }
}

fn parse_tokenstream_array(stream: ParseStream) -> Result<TokenStream, syn::Error> {
    let mut result = TokenStream::new();
    let mut level = 0u8;
    while !stream.is_empty() {
        if stream.peek(Token![,]) && level == 0 {
            break;
        }
        if stream.peek(Token![<]) {
            level = level
                .checked_add(1)
                .expect("Overflow occurred adding layer");
        }
        if stream.peek(Token![>]) {
            level = level
                .checked_sub(1)
                .expect("Underflow occurred removing layer");
        }
        result.append(stream.parse::<TokenTree>()?);
    }
    Ok(result)
}

pub fn type_option(input: TokenStream) -> (TokenStream, bool) {
    let OptionTypeParseResult(result) = syn::parse2(input.clone()).expect("Unable to parse type");
    result.map_or((input, false), |result| (result, true))
}
