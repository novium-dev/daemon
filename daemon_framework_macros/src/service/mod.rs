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
pub mod args;
pub mod parse;
pub mod process;

use crate::service::args::ServiceArgs;
use crate::service::parse::ParseExpectation;
use core::mem::take;
use either::Either;
use proc_macro2::{Delimiter, Group, Ident, Span, TokenStream, TokenTree};
use quote::quote;
use std::collections::VecDeque;

pub fn service0(crate_prefix: &TokenStream, args: &ServiceArgs, input: TokenStream) -> TokenStream {
    let mut output = Vec::new();

    output.extend(quote! {
        #[allow(clippy::too_many_arguments)]
        #[#crate_prefix async_trait]
    });

    let mut input: VecDeque<_> = input
        .into_iter()
        .skip_while(|item| match item {
            TokenTree::Ident(ident)
                if ident.to_string().eq("trait") || ident.to_string().eq("impl") =>
            {
                false
            }
            TokenTree::Group(_)
            | TokenTree::Ident(_)
            | TokenTree::Punct(_)
            | TokenTree::Literal(_) => {
                output.push(item.clone());
                true
            }
        })
        .collect();
    let Some(TokenTree::Ident(trait_keyword)) = input.pop_front() else {
        panic!("Expected trait or impl keyword");
    };
    output.push(TokenTree::Ident(if trait_keyword.to_string().eq("impl") {
        Ident::new("trait", Span::call_site())
    } else {
        trait_keyword.clone()
    }));

    let Some(TokenTree::Ident(trait_ident)) = input.pop_front() else {
        panic!("Expected identifier after trait keyword");
    };
    output.push(TokenTree::Ident(trait_ident.clone()));

    let mut for_ident = None;
    let mut struct_ident = None;

    input = input
        .into_iter()
        .enumerate()
        .skip_while(|(index, item)| match item {
            TokenTree::Ident(ident) if *index == 1 && for_ident.is_some() => {
                struct_ident = Some(ident.clone());
                true
            }
            TokenTree::Group(_)
            | TokenTree::Ident(_)
            | TokenTree::Punct(_)
            | TokenTree::Literal(_)
                if *index == 1 && for_ident.is_some() =>
            {
                panic!("Expected struct after impl <trait> for");
            }
            TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => false,
            TokenTree::Ident(ident)
                if ident.to_string().eq("for")
                    && *index == 0
                    && trait_keyword.to_string().eq("impl") =>
            {
                for_ident = Some(ident.clone());
                true
            }
            TokenTree::Group(_)
            | TokenTree::Ident(_)
            | TokenTree::Punct(_)
            | TokenTree::Literal(_) => {
                output.push(item.clone());
                true
            }
        })
        .map(|(_index, item)| item)
        .collect();

    assert!(
        !(struct_ident.is_none() && trait_keyword.to_string().eq("impl")),
        "Expected for <struct> after impl <trait>"
    );
    let impl_data = for_ident.and_then(|for_ident| {
        struct_ident.map(|struct_ident| (trait_keyword, for_ident, struct_ident))
    });

    let Some(TokenTree::Group(trait_body)) = input.pop_front() else {
        panic!("Expected trait body");
    };
    let mut trait_body_output = Vec::new();

    let mut expect_fn = ParseExpectation::FnKeyword;
    let mut result = Vec::new();
    let mut pre_fn = Vec::new();
    let mut post_fn = Vec::new();

    let input_left = input;
    let mut input: VecDeque<_> = trait_body.stream().into_iter().collect();

    let mut errors = TokenStream::new();
    let mut routes = TokenStream::new();

    let mut functions = Vec::new();

    while let Some(tree) = input.pop_front() {
        match (tree, expect_fn, &impl_data) {
            (TokenTree::Ident(ident), ParseExpectation::FnKeyword, _)
                if ident.to_string().eq("fn") =>
            {
                expect_fn = ParseExpectation::FunctionIdentifier { fn_keyword: ident };
            }
            (item, ParseExpectation::FnKeyword, _) => {
                expect_fn = ParseExpectation::FnKeyword;
                pre_fn.push(item);
            }
            (TokenTree::Ident(ident), ParseExpectation::FunctionIdentifier { fn_keyword }, _) => {
                expect_fn = ParseExpectation::ArgumentsTuple {
                    fn_keyword,
                    fn_ident: ident,
                };
            }
            (_, ParseExpectation::FunctionIdentifier { .. }, _) => {
                panic!("Expected function identifier after fn keyword")
            }
            (
                TokenTree::Group(group),
                ParseExpectation::ArgumentsTuple {
                    fn_keyword,
                    fn_ident,
                },
                _,
            ) if group.delimiter() == Delimiter::Parenthesis => {
                expect_fn = ParseExpectation::FunctionOrResult {
                    fn_keyword,
                    fn_ident,
                    fn_args: group,
                };
            }
            (
                item,
                ParseExpectation::ArgumentsTuple {
                    fn_ident,
                    fn_keyword,
                },
                _,
            ) => {
                expect_fn = ParseExpectation::ArgumentsTuple {
                    fn_ident,
                    fn_keyword,
                };
                post_fn.push(item);
            } // Ignore type parameter if any
            (
                TokenTree::Punct(punct),
                ParseExpectation::FunctionOrResult {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                },
                _,
            ) if punct.as_char() == '-' => {
                expect_fn = ParseExpectation::Arrow {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                };
                post_fn.push(TokenTree::Punct(punct.clone()));
            }
            (
                TokenTree::Punct(punct),
                ParseExpectation::Arrow {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                },
                _,
            ) if punct.as_char() == '>' => {
                expect_fn = ParseExpectation::Result {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                };
                post_fn.push(TokenTree::Punct(punct.clone()));
            }
            (_, ParseExpectation::Arrow { .. }, _) => {
                panic!("Expected > after -, expecting the function result after that.")
            }
            (
                TokenTree::Group(group),
                ParseExpectation::FunctionOrResult {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                }
                | ParseExpectation::Result {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                },
                Some(_),
            ) if group.delimiter() == Delimiter::Brace => {
                expect_fn = ParseExpectation::FnKeyword; // for next function

                trait_body_output.extend(process::process(
                    crate_prefix,
                    args,
                    &trait_ident,
                    &fn_keyword,
                    fn_ident,
                    Either::Right(group),
                    #[expect(
                        clippy::iter_with_drain,
                        reason = "We want to reuse the pre_fn later, it should be cleared"
                    )]
                    pre_fn.drain(..).collect(),
                    take(&mut post_fn),
                    fn_args,
                    &mut errors,
                    &mut routes,
                    take(&mut result),
                    &mut functions,
                ));
            }
            (
                TokenTree::Punct(punct),
                ParseExpectation::FunctionOrResult {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                }
                | ParseExpectation::Result {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                },
                None,
            ) if punct.as_char() == ';' => {
                expect_fn = ParseExpectation::FnKeyword; // for next function

                trait_body_output.extend(process::process(
                    crate_prefix,
                    args,
                    &trait_ident,
                    &fn_keyword,
                    fn_ident,
                    Either::Left(punct.clone()),
                    #[expect(
                        clippy::iter_with_drain,
                        reason = "We want to reuse the pre_fn later, it should be cleared"
                    )]
                    pre_fn.drain(..).collect(),
                    take(&mut post_fn),
                    fn_args,
                    &mut errors,
                    &mut routes,
                    take(&mut result),
                    &mut functions,
                ));
            }
            (_, ParseExpectation::FunctionOrResult { .. }, None) => {
                panic!("Expected semicolon after function")
            }
            (_, ParseExpectation::FunctionOrResult { .. }, Some(_)) => {
                panic!("Expected method body after function")
            }
            (
                tree,
                ParseExpectation::Result {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                },
                _,
            ) => {
                expect_fn = ParseExpectation::Result {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                };
                result.push(tree);
            }
        }
    }

    let mut output_extra = Vec::new();

    process::finish(
        crate_prefix,
        args,
        &trait_ident,
        &mut trait_body_output,
        &mut output_extra,
        functions,
        impl_data,
    );

    output.push(TokenTree::Group(Group::new(
        Delimiter::Brace,
        TokenStream::from_iter(trait_body_output),
    )));
    output.extend(input_left);
    output.extend(output_extra);

    output.extend(quote! {

        #crate_prefix pastey::paste! {
            pub mod [< __ #trait_ident _error >] {
                use super::*;

                #errors
            }
        }

    });
    output.extend(quote! {

        #crate_prefix pastey::paste! {
            mod [< __ #trait_ident _framework_routes >] {
                use super::*;

                #routes
            }
        }

    });

    TokenStream::from_iter(output)
}
