mod header;
mod macros;
mod path;
mod payload;
mod query;
mod route;

use crate::route::RouteArgs;
use crate::service::args::ServiceArgs;
use crate::service::parse::FunctionArgument;
use crate::utils::{TokenStreamArray, type_option};
use proc_macro2::{Delimiter, Group, Ident, Punct, Span, TokenStream, TokenTree};
use quote::quote;
use std::collections::VecDeque;
use std::str::FromStr;
use syn::LitStr;

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
#[allow(clippy::upper_case_acronyms)]
pub(crate) enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    OPTIONS,
    HEAD,
    PATCH,
    TRACE,
    WS,
}

impl FromStr for HttpMethod {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "get" => Ok(HttpMethod::GET),
            "post" => Ok(HttpMethod::POST),
            "put" => Ok(HttpMethod::PUT),
            "delete" => Ok(HttpMethod::DELETE),
            "options" => Ok(HttpMethod::OPTIONS),
            "head" => Ok(HttpMethod::HEAD),
            "patch" => Ok(HttpMethod::PATCH),
            "trace" => Ok(HttpMethod::TRACE),
            "ws" => Ok(HttpMethod::WS),
            _ => Err(()),
        }
    }
}

impl HttpMethod {
    pub(crate) fn to_utoipa(self) -> Ident {
        Ident::new(
            match self {
                HttpMethod::GET | HttpMethod::WS => "Get",
                HttpMethod::POST => "Post",
                HttpMethod::PUT => "Put",
                HttpMethod::DELETE => "Delete",
                HttpMethod::OPTIONS => "Options",
                HttpMethod::HEAD => "Head",
                HttpMethod::PATCH => "Patch",
                HttpMethod::TRACE => "Trace",
            },
            Span::call_site(),
        )
    }

    pub(crate) fn to_http(self) -> Ident {
        Ident::new(
            match self {
                HttpMethod::GET | HttpMethod::WS => "GET",
                HttpMethod::POST => "POST",
                HttpMethod::PUT => "PUT",
                HttpMethod::DELETE => "DELETE",
                HttpMethod::OPTIONS => "OPTIONS",
                HttpMethod::HEAD => "HEAD",
                HttpMethod::PATCH => "PATCH",
                HttpMethod::TRACE => "TRACE",
            },
            Span::call_site(),
        )
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub(crate) fn process(
    crate_prefix: &TokenStream,
    args: &ServiceArgs,

    trait_ident: &Ident,
    fn_keyword: Ident,
    fn_ident: Ident,
    end_punct: Punct,

    pre_fn: VecDeque<TokenTree>,
    mut post_fn: Vec<TokenTree>,

    mut fn_args: Group,
    errors: &mut TokenStream,
    routes: &mut TokenStream,
    mut result: Vec<TokenTree>,
    functions: &mut Vec<(
        Ident,
        String,
        HttpMethod,
        bool,
        TokenStream,
        TokenStream,
        TokenStream,
        TokenStream,
        TokenStream,
    )>,
) -> Vec<TokenTree> {
    let mut operations = TokenStream::new();

    if let Some(tag) = &args.tag {
        operations.extend(quote! { .tag(#tag) });
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
    ) = macros::process(
        crate_prefix,
        &fn_ident,
        pre_fn,
        errors,
        &mut post_fn,
        &mut result,
        &mut operations,
    );

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
        fn_args_out.extend(quote! { #fn_arg, });

        let mut fn_arg = fn_arg.into_iter();
        if fn_index == 0 {
            let Some(TokenTree::Punct(punct)) = fn_arg.next() else {
                panic!("Your function needs to have &self as the first argument");
            };
            if punct.as_char() != '&' {
                panic!("Your function needs to have &self as the first argument");
            }
            let Some(TokenTree::Ident(ident)) = fn_arg.next() else {
                panic!("Your function needs to have &self as the first argument");
            };
            if !ident.to_string().eq("self") {
                panic!("Your function needs to have &self as the first argument");
            }

            if let HttpMethod::WS = method {
                fn_args_out
                    .extend(quote! { ws: #crate_prefix axum::extract::ws::WebSocketUpgrade, });
            }

            continue;
        }
        let Some(TokenTree::Ident(fn_arg_name)) = fn_arg.next() else {
            continue;
        };
        let Some(TokenTree::Punct(punct)) = fn_arg.next() else {
            panic!("Expected : after function argument");
        };
        if punct.as_char() != ':' {
            panic!("Expected : after function argument");
        }
        let fn_arg_type: TokenStream = fn_arg.collect();
        let (fn_arg_type_opt, fn_arg_type_opt_used) = type_option(fn_arg_type.clone());

        if path.contains(&format!("{{{}}}", fn_arg_name)) {
            fn_args_identified.push(FunctionArgument::Path {
                variable_name: fn_arg_name,
                variable_type: fn_arg_type,
            });
        } else if query.contains(&fn_arg_name.to_string()) {
            fn_args_identified.push(FunctionArgument::Query {
                variable_name: fn_arg_name,
                variable_type: fn_arg_type_opt,
                variable_type_wopt: fn_arg_type,
                required: !fn_arg_type_opt_used,
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
            | FunctionArgument::Payload { variable_name, .. }) = argument;
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
        user,
        &mut client_impl_body,
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
    if method == HttpMethod::WS {
        client_impl_body = quote! { panic!("Websocket client is not implemented (and very likely will never be)"); };
    } else if raw {
        client_impl_body = quote! { panic!("Raw data responses are not implemented (and very likely will never be)"); };
    } else {
        client_impl_body = quote! {
            let url = self.[< #fn_ident _url >] #fn_def_call;
            let mut request = self.client().request(#crate_prefix http::Method::#method_http, url)
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

        #fn_keyword [< #fn_ident _url >] #fn_def_args -> #crate_prefix reqwest::Url {
            #client_def_body
            url
        }

    };

    functions.push((
        fn_ident,
        path,
        method,
        user,
        result,
        operations,
        schema_extra,
        client_impl,
        client_def,
    ));

    func
}

#[allow(clippy::type_complexity)]
pub(crate) fn finish(
    crate_prefix: &TokenStream,
    trait_ident: &Ident,
    trait_body_output: &mut Vec<TokenTree>,
    output_extra: &mut Vec<TokenTree>,
    functions: Vec<(
        Ident,
        String,
        HttpMethod,
        bool,
        TokenStream,
        TokenStream,
        TokenStream,
        TokenStream,
        TokenStream,
    )>,
) {
    let mut routes = TokenStream::new();
    let mut routes_user = TokenStream::new();

    let mut client_impl = TokenStream::new();
    let mut client_definition = TokenStream::new();

    for (ident, path, method, user, result, operations, schema, client, client_def) in functions {
        let token = if user { &mut routes_user } else { &mut routes };

        client_impl.extend(client);
        client_definition.extend(client_def);

        let method = method.to_utoipa();

        let path = LitStr::new(&path, Span::call_site());

        let operation_id = LitStr::new(&ident.to_string(), Span::call_site());

        token.extend(quote! {

            .routes(
                {
                    let mut schemas = std::collections::HashMap::new();
                    #schema
                    let mut builder = #crate_prefix utoipa::openapi::ResponsesBuilder::new();
                    <#result as #crate_prefix FrameworkDocumentedResponse>::openapi(&mut builder, &mut schemas);
                    (schemas.into_iter().collect(), {
                        use #crate_prefix utoipa::PartialSchema;
                        let mut paths = #crate_prefix utoipa::openapi::path::Paths::new();
                        paths.add_path_operation(#path, vec![#crate_prefix utoipa::openapi::HttpMethod::#method],

                            OperationBuilder::new()
                                .responses(builder)
                                .operation_id(Some(#operation_id))
                                #operations
                            .build()
                        );
                        paths
                    }, MethodRouter::new().on(#crate_prefix utoipa::openapi::HttpMethod::#method.to_method_filter(), __daemon_framework_routes::#ident(self.clone())))
                }
            )

        });
    }

    trait_body_output.extend(quote! {

        #crate_prefix paste::paste! {

            fn routes(&self) -> #crate_prefix utoipa_axum::router::OpenApiRouter where Self: Clone + Send + Sync + Sized + 'static {
                use #crate_prefix utoipa_axum::router::OpenApiRouter;
                use #crate_prefix utoipa::openapi::path::OperationBuilder;
                use #crate_prefix utoipa_axum::PathItemExt;
                use #crate_prefix axum::routing::method_routing::MethodRouter;

                OpenApiRouter::new()
                    #routes
            }

            fn routes_user(&self) -> #crate_prefix utoipa_axum::router::OpenApiRouter where Self: Clone + Send + Sync + Sized + 'static {
                use #crate_prefix utoipa_axum::router::OpenApiRouter;
                use #crate_prefix utoipa::openapi::path::OperationBuilder;
                use #crate_prefix utoipa_axum::PathItemExt;
                use #crate_prefix axum::routing::method_routing::MethodRouter;

                OpenApiRouter::new()
                    #routes_user
            }

        }

    });

    output_extra.extend(quote! {

        #crate_prefix paste::paste! {

            pub trait [< #trait_ident ClientImpl >]: #crate_prefix FrameworkClientImpl {
                #client_definition
            }

            #[#crate_prefix async_trait]
            #[allow(unused_variables)] // TODO: Remove when implementation is done
            impl<T: [< #trait_ident ClientImpl >] + Send + Sync> #trait_ident for T {
                #client_impl
            }

        }

    });
}
