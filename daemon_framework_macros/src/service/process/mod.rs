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
mod header;
mod macros;
mod path;
mod payload;
mod query;
mod route;
mod special;

use crate::route::RouteArgs;
use crate::service::args::ServiceArgs;
use crate::service::parse::FunctionArgument;
use crate::utils::{TokenStreamArray, type_option};
use core::fmt::{Display, Formatter};
use core::str::FromStr;
use proc_macro2::{Delimiter, Group, Ident, Punct, Span, TokenStream, TokenTree};
use quote::quote;
use std::collections::VecDeque;
use syn::LitStr;

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
#[expect(
    clippy::arbitrary_source_item_ordering,
    reason = "common http methods order"
)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
    Trace,
    Ws,
}

impl Display for HttpMethod {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Get | Self::Ws => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
            Self::Trace => "TRACE",
        })
    }
}

impl FromStr for HttpMethod {
    type Err = ();

    fn from_str(str: &str) -> Result<Self, Self::Err> {
        match str.to_lowercase().as_str() {
            "get" => Ok(Self::Get),
            "post" => Ok(Self::Post),
            "put" => Ok(Self::Put),
            "patch" => Ok(Self::Patch),
            "delete" => Ok(Self::Delete),
            "head" => Ok(Self::Head),
            "options" => Ok(Self::Options),
            "trace" => Ok(Self::Trace),
            "ws" => Ok(Self::Ws),
            _ => Err(()),
        }
    }
}

impl HttpMethod {
    pub(crate) fn to_http(self) -> Ident {
        Ident::new(&self.to_string(), Span::call_site())
    }

    pub(crate) fn to_utoipa(self) -> Ident {
        Ident::new(
            match self {
                Self::Get | Self::Ws => "Get",
                Self::Post => "Post",
                Self::Put => "Put",
                Self::Patch => "Patch",
                Self::Delete => "Delete",
                Self::Head => "Head",
                Self::Options => "Options",
                Self::Trace => "Trace",
            },
            Span::call_site(),
        )
    }
}

pub struct ProcessedFunction {
    client_def: TokenStream,
    client_impl: TokenStream,

    fn_ident: Ident,
    method: HttpMethod,
    operations: TokenStream,
    path: String,
    result: TokenStream,
    schema_extra: TokenStream,
    user: bool,
}

#[expect(clippy::too_many_arguments)]
pub fn process(
    crate_prefix: &TokenStream,
    args: &ServiceArgs,

    trait_ident: &Ident,
    fn_keyword: &Ident,
    fn_ident: Ident,
    end_punct: Punct,

    pre_fn: VecDeque<TokenTree>,
    mut post_fn: Vec<TokenTree>,

    mut fn_args: Group,
    errors: &mut TokenStream,
    routes: &mut TokenStream,
    mut result: Vec<TokenTree>,
    functions: &mut Vec<ProcessedFunction>,
) -> Vec<TokenTree> {
    let mut operations = TokenStream::new();

    if let Some(tag) = &args.tag {
        operations.extend(quote! { operations = operations.tag(#tag); });
    }

    let (
        pre_fn_result,
        RouteArgs {
            absolute,
            path,
            method,
            query,
            user,
            raw,
        },
        example,
        doc,
    ) = macros::process(
        crate_prefix,
        &fn_ident,
        pre_fn,
        errors,
        &mut post_fn,
        &mut result,
    );

    if let Some(desc) = &doc {
        operations.extend(quote! { operations = operations.description(Some(#desc)); });
    }

    let path = if absolute {
        path
    } else {
        format!("{}{}", args.path, path)
    };

    let mut fn_args_identified = Vec::new();

    let fn_args_array: TokenStreamArray =
        syn::parse2(fn_args.stream()).expect("Expected function arguments");

    let mut fn_args_out = TokenStream::new();

    for (fn_index, fn_arg) in fn_args_array.0.into_iter().enumerate() {
        let fn_arg_unmodified = fn_arg.clone();
        let mut fn_arg = fn_arg.into_iter();
        if fn_index == 0 {
            let Some(TokenTree::Punct(punct)) = fn_arg.next() else {
                panic!("Your function needs to have &self as the first argument");
            };
            assert_eq!(
                punct.as_char(),
                '&',
                "Your function needs to have &self as the first argument"
            );
            let Some(TokenTree::Ident(ident)) = fn_arg.next() else {
                panic!("Your function needs to have &self as the first argument");
            };
            assert_eq!(
                ident.to_string(),
                "self",
                "Your function needs to have &self as the first argument"
            );

            fn_args_out.extend(quote! { #fn_arg_unmodified, });
            if HttpMethod::Ws == method {
                fn_args_out
                    .extend(quote! { ws: #crate_prefix axum::extract::ws::WebSocketUpgrade, });
            }

            continue;
        }

        let (fn_arg_name, special) = match fn_arg.next() {
            Some(TokenTree::Ident(fn_arg_name)) => {
                fn_args_out.extend(quote! { #fn_arg_unmodified, });
                (fn_arg_name, false)
            }
            Some(TokenTree::Punct(ch)) if ch.as_char() == '#' => {
                let Some(TokenTree::Group(group)) = fn_arg.next() else {
                    panic!("Expected brackets after # in function argument");
                };
                assert_eq!(
                    group.delimiter(),
                    Delimiter::Bracket,
                    "Expected brackets after # in function argument"
                );
                assert_eq!(
                    group.stream().to_string(),
                    "special",
                    "Only #[special] attribute is allowed in function arguments"
                );

                let Some(TokenTree::Ident(fn_arg_name)) = fn_arg.next() else {
                    panic!("Expected parameter name in function");
                };

                let fn_arg_out_stripped: TokenStream =
                    fn_arg_unmodified.clone().into_iter().skip(2).collect();
                fn_args_out.extend(quote! { #fn_arg_out_stripped, });

                (fn_arg_name, true)
            }
            _ => {
                fn_args_out.extend(quote! { #fn_arg_unmodified, });
                continue;
            }
        };

        let Some(TokenTree::Punct(punct)) = fn_arg.next() else {
            panic!("Expected : after function argument");
        };
        assert_eq!(
            punct.as_char(),
            ':',
            "Expected : after function argument, got {}",
            punct.as_char()
        );
        let fn_arg_type: TokenStream = fn_arg.collect();
        let (fn_arg_type_opt, fn_arg_type_opt_used) = type_option(fn_arg_type.clone());

        if special {
            fn_args_identified.push(FunctionArgument::Special {
                variable_name: fn_arg_name,
                variable_type: fn_arg_type,
            });
        } else if path.contains(&format!("{{{fn_arg_name}}}")) {
            fn_args_identified.push(FunctionArgument::Path {
                variable_name: fn_arg_name,
                variable_type: fn_arg_type,
            });
        } else if let Some(desc) = query.get(&fn_arg_name.to_string()) {
            fn_args_identified.push(FunctionArgument::Query {
                variable_name: fn_arg_name,
                variable_type_wopt: fn_arg_type,
                required: !fn_arg_type_opt_used,
                desc: desc.clone(),
            });
        } else if fn_arg_type_opt
            .clone()
            .into_iter()
            .next()
            .is_some_and(|token| {
                if let TokenTree::Ident(i) = token {
                    i.to_string().eq_ignore_ascii_case("TypedHeader")
                } else {
                    false
                }
            })
        {
            fn_args_identified.push(FunctionArgument::Header {
                variable_name: fn_arg_name,
                variable_type: fn_arg_type_opt,
                variable_type_wopt: fn_arg_type,
                required: !fn_arg_type_opt_used,
            });
        } else {
            fn_args_identified.push(FunctionArgument::Payload {
                variable_name: fn_arg_name,
                variable_type: fn_arg_type,
            });
        }
    }

    let mut group = Group::new(fn_args.delimiter(), fn_args_out);
    group.set_span(fn_args.span());
    fn_args = group;

    let mut func_args = Vec::new();

    let mut http_args = TokenStream::new();
    let call_args: TokenStream = fn_args_identified
        .iter()
        .flat_map(|argument| {
            let (FunctionArgument::Path { variable_name, .. }
            | FunctionArgument::Query { variable_name, .. }
            | FunctionArgument::Header { variable_name, .. }
            | FunctionArgument::Payload { variable_name, .. }
            | FunctionArgument::Special { variable_name, .. }) = argument;
            quote! { #variable_name, }
        })
        .collect();

    let mut schema_extra = TokenStream::new();

    let mut client_impl_body = TokenStream::new();
    let mut client_def_body = TokenStream::new();
    let mut fn_def_args = quote! { &self, };
    let mut fn_def_call = TokenStream::new();

    path::process(
        crate_prefix,
        &path,
        &fn_args_identified,
        &mut http_args,
        &mut func_args,
        &mut operations,
        &mut client_def_body,
        &mut fn_def_args,
        &mut fn_def_call,
    );

    query::process(
        crate_prefix,
        &fn_ident,
        &fn_args_identified,
        routes,
        &mut http_args,
        &mut func_args,
        &mut operations,
        &mut client_def_body,
        &mut fn_def_args,
        &mut fn_def_call,
    );

    header::process(
        crate_prefix,
        &fn_args_identified,
        &mut http_args,
        &mut func_args,
        &mut operations,
        &mut client_impl_body,
    );

    special::process(
        crate_prefix,
        &fn_args_identified,
        &mut http_args,
        &mut func_args,
        &mut operations,
    );

    payload::process(
        crate_prefix,
        &fn_ident,
        &fn_args_identified,
        routes,
        &mut http_args,
        &mut func_args,
        &mut operations,
        &mut schema_extra,
        example,
        &mut client_impl_body,
    );

    route::process(
        crate_prefix,
        method,
        raw,
        trait_ident,
        &fn_ident,
        func_args,
        http_args,
        call_args,
        routes,
    );

    let mut func = Vec::new();
    func.extend(pre_fn_result);
    func.push(TokenTree::Ident(fn_keyword.clone()));
    func.push(TokenTree::Ident(fn_ident.clone()));
    func.push(TokenTree::Group(fn_args.clone()));
    func.append(&mut post_fn.clone());
    func.append(&mut result.clone());
    func.push(TokenTree::Punct(end_punct));

    let post_fn = TokenStream::from_iter(post_fn);
    let result = TokenStream::from_iter(result);
    let fn_def_args = Group::new(Delimiter::Parenthesis, fn_def_args);
    let fn_args = TokenTree::Group(fn_args);

    let fn_def_call = Group::new(Delimiter::Parenthesis, fn_def_call);
    let method_http = method.to_http();
    if method == HttpMethod::Ws {
        client_impl_body =
            quote! { panic!("Websocket client is not implemented (and will maybe never be)"); };
    } else if raw {
        client_impl_body = quote! {
            let mut request = self.client().request(#crate_prefix http::Method::#method_http, self.[< #fn_ident _url >] #fn_def_call);
            #client_impl_body
            let response = request
                .send()
                .await?;
            match response.error_for_status_ref() {
                Ok(_) => {
                    Ok(#crate_prefix internal::from_raw(response))
                },
                Err(_err) => {
                    Err(#crate_prefix internal::error_by_response(response).await)
                }
            }
        };
    } else {
        client_impl_body = quote! {
            let mut request = self.client().request(#crate_prefix http::Method::#method_http, self.[< #fn_ident _url >] #fn_def_call)
                .header(#crate_prefix http::header::ACCEPT, "application/json");
            #client_impl_body
            let response = request
                .send()
                .await?;
            match response.error_for_status_ref() {
                Ok(_) => {
                    Ok(#crate_prefix internal::FrameworkJsonResponse::from_reqwest(response)
                        .await?.0)
                },
                Err(_err) => {
                    Err(#crate_prefix internal::error_by_response(response).await)
                }
            }
        };
    }

    let client_impl = quote! { async #fn_keyword #fn_ident #fn_args #post_fn #result {
        #client_impl_body
    } };

    let client_def = quote! {

        #fn_keyword [< #fn_ident _url >] #fn_def_args -> #crate_prefix url::Url {
            #client_def_body
            framework_result_url
        }

    };

    functions.push(ProcessedFunction {
        client_def,
        client_impl,
        fn_ident,
        method,
        operations,
        path,
        result,
        schema_extra,
        user,
    });

    func
}

pub fn finish(
    crate_prefix: &TokenStream,
    args: &ServiceArgs,
    trait_ident: &Ident,
    trait_body_output: &mut Vec<TokenTree>,
    output_extra: &mut Vec<TokenTree>,
    functions: Vec<ProcessedFunction>,
) {
    let mut routes = TokenStream::new();
    let mut routes_user = TokenStream::new();

    let mut client_impl_merged = TokenStream::new();
    let mut client_def_merged = TokenStream::new();

    for ProcessedFunction {
        fn_ident,
        path,
        method,
        user,
        result,
        operations,
        schema_extra,
        client_impl,
        client_def,
    } in functions
    {
        let token = if user { &mut routes_user } else { &mut routes };

        client_impl_merged.extend(client_impl);
        client_def_merged.extend(client_def);

        let method = method.to_utoipa();

        let path = LitStr::new(&path, Span::call_site());

        let operation_id = LitStr::new(&fn_ident.to_string(), Span::call_site());

        token.extend(quote! {

            .route(
                #path,
                #crate_prefix utoipa::openapi::HttpMethod::#method,
                {
                    let mut schemas = std::collections::HashMap::new();
                    #schema_extra

                    let responses = <#result as #crate_prefix responses::DocumentedResponse>::openapi(
                        #crate_prefix utoipa::openapi::ResponsesBuilder::new(),
                        &mut schemas
                    );
                    let mut operations = OperationBuilder::new()
                        .operation_id(Some(#operation_id));
                    #operations

                    (responses, operations, schemas.into_iter().collect())
                },
                MethodRouter::new()
                    .on(#crate_prefix utoipa::openapi::HttpMethod::#method.to_method_filter(), __framework_routes::#fn_ident(self.clone())),
            )
        });
    }

    trait_body_output.extend(quote! {

        #crate_prefix pastey::paste! {

            fn routes(&self) -> #crate_prefix router::FrameworkRouter where Self: Clone + Send + Sync + Sized + 'static {
                use #crate_prefix router::FrameworkRouter;
                use #crate_prefix utoipa::openapi::path::OperationBuilder;
                use #crate_prefix utoipa_axum::PathItemExt;
                use #crate_prefix axum::routing::method_routing::MethodRouter;

                FrameworkRouter::new()
                    #routes
            }

            fn routes_user(&self) -> #crate_prefix router::FrameworkRouter where Self: Clone + Send + Sync + Sized + 'static {
                use #crate_prefix router::FrameworkRouter;
                use #crate_prefix utoipa::openapi::path::OperationBuilder;
                use #crate_prefix utoipa_axum::PathItemExt;
                use #crate_prefix axum::routing::method_routing::MethodRouter;

                FrameworkRouter::new()
                    #routes_user
            }

        }

    });

    if !args.no_sdk {
        output_extra.extend(quote! {

        #crate_prefix pastey::paste! {

                pub trait [< #trait_ident URL >]: #crate_prefix sdk::FrameworkClientImpl {
                    #client_def_merged
                }

                pub trait [< #trait_ident ClientImpl >]: #crate_prefix sdk::FrameworkClientImpl {}

                impl<T: #trait_ident + #crate_prefix sdk::FrameworkClientImpl> [< #trait_ident URL >] for T {}

                #[#crate_prefix async_trait]
                #[diagnostic::do_not_recommend]
                #[allow(unused_variables)] // TODO: Why is this needed?
                impl<T: [< #trait_ident ClientImpl >] + Send + Sync> #trait_ident for T {
                    #client_impl_merged
                }

            }

        });
    }
}
