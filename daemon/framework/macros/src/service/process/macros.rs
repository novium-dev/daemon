use crate::route::RouteArgs;
use crate::service::process::HttpMethod;
use proc_macro2::{Delimiter, Ident, TokenStream, TokenTree};
use quote::quote;
use std::collections::VecDeque;
use syn::LitStr;

pub(crate) fn process(
    crate_prefix: &TokenStream,
    fn_ident: &Ident,
    mut pre_fn: VecDeque<TokenTree>,
    errors: &mut TokenStream,
    post_fn: &mut Vec<TokenTree>,
    result: &mut Vec<TokenTree>,
    operations: &mut TokenStream,
) -> (Vec<TokenTree>, RouteArgs, Option<LitStr>) {
    let mut route_args = None;
    let mut pre_fn_result0 = VecDeque::new();
    let mut pre_fn_result1 = Vec::new();
    let mut description = None;
    let mut example = None;

    let mut result_edited = false;
    if result.is_empty() {
        post_fn.extend(quote! { -> });
        *result = quote! { std::result::Result<(), _> }.into_iter().collect();
        result_edited = true;
    }

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
                if group.delimiter() != Delimiter::Parenthesis {
                    panic!("Expected route macro arguments in parenthesis");
                }
                match syn::parse2::<RouteArgs>(group.stream()) {
                    Ok(args) => {
                        route_args = Some(args);
                    }
                    Err(err) => panic!("Failed to parse route macro arguments: {}", err),
                }
            }
            "doc" => {
                let Some(TokenTree::Punct(punct)) = macro_inner.pop_front() else {
                    panic!("Expected =");
                };
                if punct.as_char() != '=' {
                    panic!("Expected =");
                }
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
                if group.delimiter() != Delimiter::Parenthesis {
                    panic!("Expected example macro arguments in parenthesis");
                }
                example = Some(
                    syn::parse2(group.stream()).expect("Expected literal string as example path"),
                );
            }
            _ => {
                pre_fn_result0.push_back(pre_fn_tree);
                pre_fn_result0.push_back(TokenTree::Group(group));
                continue;
            }
        }
    }

    let route_args = route_args.take().expect("Expected route attribute");

    if let HttpMethod::WS = route_args.method {
        if result_edited {
            *result = quote! { std::result::Result<#crate_prefix WebsocketResponse, _> }
                .into_iter()
                .collect();
        }
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
        match ident.to_string().as_str() {
            "error" => {
                let Some(TokenTree::Group(group)) = macro_inner.pop_front() else {
                    panic!("Expected error macro arguments");
                };
                if group.delimiter() != Delimiter::Parenthesis {
                    panic!("Expected error macro arguments in parenthesis");
                }

                let mut args = TokenStream::new();
                if !route_args.user {
                    args.extend(quote! { , auth(invalid) });
                }
                if !group.stream().is_empty() {
                    args.extend(quote! { , });
                    args.extend(group.stream());
                }
                errors.extend(quote! {
                        #crate_prefix paste::paste! {
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
                                    #crate_prefix paste::paste! { error::[< #fn_ident:camel Error >] }
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
                continue;
            }
        }
    }

    if let Some(desc) = description {
        operations.extend(quote! { .description(Some(#desc)) });
    }

    (pre_fn_result1, route_args, example)
}
