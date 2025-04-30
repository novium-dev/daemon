pub(crate) mod args;
pub(crate) mod parse;
pub(crate) mod process;

use crate::service::args::ServiceArgs;
use crate::service::parse::ParseExpectation;
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::quote;
use std::collections::VecDeque;
use std::mem::take;

pub(crate) fn service0(
    crate_prefix: TokenStream,
    args: ServiceArgs,
    input: TokenStream,
) -> TokenStream {
    let mut output = Vec::new();

    output.extend(quote! {
        #[allow(clippy::too_many_arguments)]
        #[#crate_prefix async_trait]
    });

    let mut input: VecDeque<_> = input
        .into_iter()
        .skip_while(|item| match item {
            TokenTree::Ident(ident) if ident.to_string().eq("trait") => false,
            _ => {
                output.push(item.clone());
                true
            }
        })
        .collect();
    let Some(trait_keyword) = input.pop_front() else {
        panic!("Expected trait keyword");
    };
    output.push(trait_keyword);
    let Some(TokenTree::Ident(trait_ident)) = input.pop_front() else {
        panic!("Expected identifier after trait keyword");
    };
    output.push(TokenTree::Ident(trait_ident.clone()));

    input = input
        .into_iter()
        .skip_while(|item| match item {
            TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => false,
            _ => {
                output.push(item.clone());
                true
            }
        })
        .collect();
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
        match (tree, expect_fn) {
            (TokenTree::Ident(ident), ParseExpectation::FnKeyword)
                if ident.to_string().eq("fn") =>
            {
                expect_fn = ParseExpectation::FunctionIdentifier { fn_keyword: ident };
            }
            (item, ParseExpectation::FnKeyword) => {
                expect_fn = ParseExpectation::FnKeyword;
                pre_fn.push(item);
            }
            (TokenTree::Ident(ident), ParseExpectation::FunctionIdentifier { fn_keyword }) => {
                expect_fn = ParseExpectation::ArgumentsTuple {
                    fn_keyword,
                    fn_ident: ident,
                };
            }
            (_, ParseExpectation::FunctionIdentifier { .. }) => {
                panic!("Expected function identifier after fn keyword")
            }
            (
                TokenTree::Group(group),
                ParseExpectation::ArgumentsTuple {
                    fn_keyword,
                    fn_ident,
                },
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
            ) if punct.as_char() == '>' => {
                expect_fn = ParseExpectation::Result {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                };
                post_fn.push(TokenTree::Punct(punct.clone()));
            }
            (_, ParseExpectation::Arrow { .. }) => {
                panic!("Expected > after -, expecting the function result after that.")
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
            ) if punct.as_char() == ';' => {
                expect_fn = ParseExpectation::FnKeyword; // for next function

                trait_body_output.extend(process::process(
                    &crate_prefix,
                    &args,
                    &trait_ident,
                    fn_keyword,
                    fn_ident,
                    punct.clone(),
                    pre_fn.drain(..).collect(),
                    take(&mut post_fn),
                    fn_args,
                    &mut errors,
                    &mut routes,
                    take(&mut result),
                    &mut functions,
                ));
            }
            (_, ParseExpectation::FunctionOrResult { .. }) => {
                panic!("Expected semicolon after function")
            }
            (
                tree,
                ParseExpectation::Result {
                    fn_keyword,
                    fn_ident,
                    fn_args,
                },
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
        &crate_prefix,
        &trait_ident,
        &mut trait_body_output,
        &mut output_extra,
        functions,
    );

    output.push(TokenTree::Group(Group::new(
        Delimiter::Brace,
        TokenStream::from_iter(trait_body_output),
    )));
    output.extend(input_left);
    output.extend(output_extra);

    output.extend(quote! {

        pub mod error {
            use super::*;

            #errors
        }

    });
    output.extend(quote! {

        mod __daemon_framework_routes {
            use super::*;

            #routes
        }

    });

    TokenStream::from_iter(output)
}
