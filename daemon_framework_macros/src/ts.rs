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
use proc_macro2::TokenStream;

#[cfg_attr(feature = "ts", expect(clippy::needless_pass_by_value))]
pub fn ts0(crate_prefix: &TokenStream, attr: &TokenStream, input: TokenStream) -> TokenStream {
    #[cfg(feature = "ts")]
    {
        use proc_macro2::Span;
        use quote::quote;
        use syn::LitStr;

        let extra = if attr.is_empty() {
            TokenStream::new()
        } else {
            quote! {
                #[ts( #attr )]
            }
        };

        let crate_literal = LitStr::new(&format!("{crate_prefix}ts_rs"), Span::call_site());
        quote! {
            #[derive(#crate_prefix ts_rs::TS)]
            #[ts(crate = #crate_literal)]
            #extra
            #input
        }
    }
    #[cfg(not(feature = "ts"))]
    {
        filter_stream(crate_prefix, attr, input)
    }
}

#[cfg(not(feature = "ts"))]
fn filter_stream(
    _crate_prefix: &TokenStream,
    _attr: &TokenStream,
    input: TokenStream,
) -> TokenStream {
    use proc_macro2::{Group, TokenTree};

    let mut iter = input.into_iter();
    let mut out = Vec::new();
    while let Some(token) = iter.next() {
        if let TokenTree::Group(group) = token {
            out.push(TokenTree::Group(Group::new(
                group.delimiter(),
                filter_stream(_crate_prefix, _attr, group.stream()),
            )));
            continue;
        }
        if let TokenTree::Punct(punct) = &token {
            if punct.as_char() == '#' {
                let next = iter.next();
                if let Some(next) = next {
                    if let TokenTree::Group(group) = &next {
                        if let Some(TokenTree::Ident(i)) = group.stream().into_iter().next() {
                            if i.to_string().eq("ts") {
                                continue;
                            }
                        }
                    }
                    out.push(token);
                    out.push(next);
                    continue;
                }
            }
        }
        out.push(token);
    }
    TokenStream::from_iter(out)
}
