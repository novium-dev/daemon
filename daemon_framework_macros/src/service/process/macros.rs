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
use crate::route::RouteArgs;
use crate::service::process::HttpMethod;
use proc_macro2::{Delimiter, Ident, Literal, TokenStream, TokenTree};
use quote::quote;
use std::collections::VecDeque;
use syn::LitStr;

pub fn process(
    crate_prefix: &TokenStream,
    fn_ident: &Ident,
    mut pre_fn: VecDeque<TokenTree>,
    errors: &mut TokenStream,
    post_fn: &mut Vec<TokenTree>,
    result: &mut Vec<TokenTree>,
    #[cfg(feature = "ts")] ts_types_impl: &mut TokenStream,
) -> (Vec<TokenTree>, RouteArgs, Option<LitStr>, Option<Literal>) {
    let mut route_args = None;
    let mut pre_fn_result0 = VecDeque::new();
    let mut pre_fn_result1 = Vec::new();
    let mut description = None;
    let mut example = None;

    let result_edited = if result.is_empty() {
        post_fn.extend(quote! { -> });
        *result = quote! { std::result::Result<(), _> }.into_iter().collect();
        true
    } else {
        false
    };

    while let Some(pre_fn_tree) = pre_fn.pop_front() {
        let TokenTree::Punct(punct) = pre_fn_tree.clone() else {
            pre_fn_result0.push_back(pre_fn_tree);
            continue;
        };
        if punct.as_char() != '#' {
            pre_fn_result0.push_back(pre_fn_tree);
            continue;
        }
        let Some(TokenTree::Group(group)) = pre_fn.pop_front() else {
            pre_fn_result0.push_back(pre_fn_tree);
            continue;
        };
        if group.delimiter() != Delimiter::Bracket {
            pre_fn_result0.push_back(pre_fn_tree);
            pre_fn_result0.push_back(TokenTree::Group(group));
            continue;
        }
        let mut macro_inner: VecDeque<_> = group.stream().into_iter().collect();
        let Some(TokenTree::Ident(ident)) = macro_inner.pop_front() else {
            pre_fn_result0.push_back(pre_fn_tree);
            pre_fn_result0.push_back(TokenTree::Group(group));
            continue;
        };
        match ident.to_string().as_str() {
            "route" => {
                let Some(TokenTree::Group(group)) = macro_inner.pop_front() else {
                    panic!("Expected route macro arguments");
                };
                assert_eq!(
                    group.delimiter(),
                    Delimiter::Parenthesis,
                    "Expected route macro arguments in parenthesis"
                );
                match syn::parse2::<RouteArgs>(group.stream()) {
                    Ok(args) => {
                        route_args = Some(args);
                    }
                    Err(err) => panic!("Failed to parse route macro arguments: {err}"),
                }
            }
            "ts" => {
                let Some(TokenTree::Group(group)) = macro_inner.pop_front() else {
                    panic!("Expected ts macro arguments");
                };
                assert_eq!(
                    group.delimiter(),
                    Delimiter::Parenthesis,
                    "Expected ts macro arguments in parenthesis"
                );
                for token in group.stream() {
                    let TokenTree::Ident(tstype) = token else {
                        panic!("Only identifiers are allowed as ts macro argument");
                    };
                    #[cfg(feature = "ts")]
                    {
                        use proc_macro2::Span;
                        let ts_crate_literal =
                            LitStr::new(&format!("{crate_prefix}ts_rs"), Span::call_site());
                        ts_types_impl.extend(quote! {

                            {
                                #[derive(#crate_prefix ts_rs::TS)]
                                #[ts(crate = #ts_crate_literal)]
                                #[allow(dead_code)]
                                struct ExtraStruct {
                                    inner: #tstype,
                                }

                                visitor.allowed.extend(<ExtraStruct as #crate_prefix ts_rs::TS>::dependencies().into_iter().map(|dep| dep.type_id));
                                <ExtraStruct as #crate_prefix ts_rs::TS>::visit_dependencies(visitor);
                            }
                        });
                    }

                    #[cfg(not(feature = "ts"))]
                    {
                        drop(tstype);
                    }
                }
            }
            "doc" => {
                let Some(TokenTree::Punct(punct)) = macro_inner.pop_front() else {
                    panic!("Expected =");
                };
                assert_eq!(punct.as_char(), '=', "Expected =");
                let Some(TokenTree::Literal(lit)) = macro_inner.pop_front() else {
                    panic!("Expected documentation");
                };

                description = Some(lit);

                pre_fn_result0.push_back(pre_fn_tree);
                pre_fn_result0.push_back(TokenTree::Group(group));
            }
            "example" => {
                let Some(TokenTree::Group(group)) = macro_inner.pop_front() else {
                    panic!("Expected example macro arguments");
                };
                assert_eq!(
                    group.delimiter(),
                    Delimiter::Parenthesis,
                    "Expected example macro arguments in parenthesis"
                );
                example = Some(
                    syn::parse2(group.stream()).expect("Expected literal string as example path"),
                );
            }
            _ => {
                pre_fn_result0.push_back(pre_fn_tree);
                pre_fn_result0.push_back(TokenTree::Group(group));
            }
        }
    }

    let route_args = route_args.take().expect("Expected route attribute");

    if HttpMethod::Ws == route_args.method && result_edited {
        *result = quote! { std::result::Result<#crate_prefix WebsocketResponse, _> }
            .into_iter()
            .collect();
    }

    while let Some(pre_fn_tree) = pre_fn_result0.pop_front() {
        let TokenTree::Punct(punct) = pre_fn_tree.clone() else {
            pre_fn_result1.push(pre_fn_tree);
            continue;
        };
        if punct.as_char() != '#' {
            pre_fn_result1.push(pre_fn_tree);
            continue;
        }
        let Some(TokenTree::Group(group)) = pre_fn_result0.pop_front() else {
            pre_fn_result1.push(pre_fn_tree);
            continue;
        };
        if group.delimiter() != Delimiter::Bracket {
            pre_fn_result1.push(pre_fn_tree);
            pre_fn_result1.push(TokenTree::Group(group));
            continue;
        }
        let mut macro_inner: VecDeque<_> = group.stream().into_iter().collect();
        let Some(TokenTree::Ident(ident)) = macro_inner.pop_front() else {
            pre_fn_result1.push(pre_fn_tree);
            pre_fn_result1.push(TokenTree::Group(group));
            continue;
        };

        let fn_ident = fn_ident.clone();
        #[expect(clippy::single_match_else, reason = "Could be extended in the future")]
        match ident.to_string().as_str() {
            "error" => {
                let Some(TokenTree::Group(group)) = macro_inner.pop_front() else {
                    panic!("Expected error macro arguments");
                };
                assert_eq!(
                    group.delimiter(),
                    Delimiter::Parenthesis,
                    "Expected error macro arguments in parenthesis"
                );

                let mut args = TokenStream::new();
                if !route_args.user {
                    args.extend(quote! { , auth(invalid) });
                }
                if !group.stream().is_empty() {
                    args.extend(quote! { , });
                    args.extend(group.stream());
                }
                errors.extend(quote! {
                        #crate_prefix pastey::paste! {
                            #crate_prefix error!([< #fn_ident:camel Error >] #args);
                        }
                });

                *result = result
                    .clone()
                    .into_iter()
                    .flat_map(|tree| {
                        if let TokenTree::Ident(ident) = tree {
                            if ident.to_string().eq("_") {
                                quote! {
                                    #crate_prefix pastey::paste! { error::[< #fn_ident:camel Error >] }
                                }
                                    .into_iter()
                                    .collect()
                            } else {
                                vec![TokenTree::Ident(ident)]
                            }
                        } else {
                            vec![tree]
                        }
                    })
                    .collect();
            }
            _ => {
                pre_fn_result1.push(pre_fn_tree);
                pre_fn_result1.push(TokenTree::Group(group));
            }
        }
    }

    (pre_fn_result1, route_args, example, description)
}
