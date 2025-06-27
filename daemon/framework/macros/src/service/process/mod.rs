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
    #[cfg(feature = "ts")]
    ts_funcs_impl: TokenStream,
    #[cfg(feature = "ts")]
    ts_types_impl: TokenStream,
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
        operations.extend(quote! { .description(Some(#desc)) });
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
        fn_args_out.extend(quote! { #fn_arg, });

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

            if HttpMethod::Ws == method {
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
        assert_eq!(
            punct.as_char(),
            ':',
            "Expected : after function argument, got {}",
            punct.as_char()
        );
        let fn_arg_type: TokenStream = fn_arg.collect();
        let (fn_arg_type_opt, fn_arg_type_opt_used) = type_option(fn_arg_type.clone());

        if path.contains(&format!("{{{fn_arg_name}}}")) {
            fn_args_identified.push(FunctionArgument::Path {
                variable_name: fn_arg_name,
                variable_type: fn_arg_type,
            });
        } else if let Some(desc) = query.get(&fn_arg_name.to_string()) {
            fn_args_identified.push(FunctionArgument::Query {
                variable_name: fn_arg_name,
                #[cfg(feature = "ts")]
                variable_type: fn_arg_type_opt,
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
            | FunctionArgument::Payload { variable_name, .. }) = argument;
            quote! { #variable_name, }
        })
        .collect();

    let mut schema_extra = TokenStream::new();

    let mut client_impl_body = TokenStream::new();
    let mut client_def_body = TokenStream::new();
    let mut fn_def_args = quote! { &self, };
    let mut fn_def_call = TokenStream::new();
    #[cfg(feature = "ts")]
    let mut ts_types_impl = TokenStream::new();
    #[cfg(feature = "ts")]
    let mut ts_funcs_impl = TokenStream::new();

    #[cfg(feature = "ts")]
    let mut ts_replace: std::collections::HashMap<String, TokenStream> =
        std::collections::HashMap::new();
    #[cfg(feature = "ts")]
    let mut ts_options_body = String::new();

    #[cfg(feature = "ts")]
    let mut ts_func_body_url = String::new();
    #[cfg(feature = "ts")]
    let mut ts_func_args_url: indexmap::IndexMap<String, TokenStream> = indexmap::IndexMap::new();

    path::process(
        crate_prefix,
        #[cfg(feature = "ts")]
        trait_ident,
        #[cfg(feature = "ts")]
        &fn_ident,
        &path,
        &fn_args_identified,
        &mut http_args,
        &mut func_args,
        &mut operations,
        &mut client_def_body,
        &mut fn_def_args,
        &mut fn_def_call,
        #[cfg(feature = "ts")]
        &mut ts_types_impl,
        #[cfg(feature = "ts")]
        &mut ts_func_body_url,
        #[cfg(feature = "ts")]
        &mut ts_func_args_url,
    );

    query::process(
        crate_prefix,
        #[cfg(feature = "ts")]
        trait_ident,
        &fn_ident,
        &fn_args_identified,
        routes,
        &mut http_args,
        &mut func_args,
        &mut operations,
        &mut client_def_body,
        &mut fn_def_args,
        &mut fn_def_call,
        #[cfg(feature = "ts")]
        &mut ts_types_impl,
        #[cfg(feature = "ts")]
        &mut ts_func_body_url,
        #[cfg(feature = "ts")]
        &mut ts_func_args_url,
    );

    header::process(
        crate_prefix,
        #[cfg(feature = "ts")]
        &fn_ident,
        &fn_args_identified,
        &mut http_args,
        &mut func_args,
        &mut operations,
        user,
        &mut client_impl_body,
        #[cfg(feature = "ts")]
        &mut ts_options_body,
        #[cfg(feature = "ts")]
        &mut ts_replace,
    );

    payload::process(
        crate_prefix,
        #[cfg(feature = "ts")]
        trait_ident,
        &fn_ident,
        &fn_args_identified,
        routes,
        &mut http_args,
        &mut func_args,
        &mut operations,
        &mut schema_extra,
        example,
        &mut client_impl_body,
        #[cfg(feature = "ts")]
        &mut ts_types_impl,
        #[cfg(feature = "ts")]
        &mut ts_options_body,
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
        client_impl_body = quote! { panic!("Websocket client is not implemented (and very likely will never be)"); };
    } else if raw {
        #[cfg(feature = "ts")]
        {
            let ts_crate_literal = LitStr::new(&format!("{crate_prefix}ts_rs"), Span::call_site());
            let trait_name = LitStr::new(&trait_ident.to_string(), trait_ident.span());
            let fn_name = LitStr::new(&fn_ident.to_string(), fn_ident.span());

            ts_types_impl.extend(quote! {

                {
                    println!("{}::{} Result (raw)", #trait_name, #fn_name);
                    #[derive(#crate_prefix ts_rs::TS)]
                    #[ts(crate = #ts_crate_literal)]
                    #[allow(dead_code)]
                    struct TypescriptResultStruct {
                        result: #crate_prefix ts::TSResponse<#result>,
                    }

                    visitor.allowed.extend(<TypescriptResultStruct as #crate_prefix ts_rs::TS>::dependencies().into_iter().map(|dep| dep.type_id));
                    <TypescriptResultStruct as #crate_prefix ts_rs::TS>::visit_dependencies(visitor);
                }
            });
        };

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
        #[cfg(feature = "ts")]
        {
            let ts_crate_literal = LitStr::new(&format!("{crate_prefix}ts_rs"), Span::call_site());
            let trait_name = LitStr::new(&trait_ident.to_string(), trait_ident.span());
            let fn_name = LitStr::new(&fn_ident.to_string(), fn_ident.span());

            ts_types_impl.extend(quote! {

                {
                    println!("{}::{} Result", #trait_name, #fn_name);
                    #[derive(#crate_prefix ts_rs::TS)]
                    #[ts(crate = #ts_crate_literal)]
                    #[allow(dead_code)]
                    struct TypescriptResultStruct {
                        result: #result,
                    }

                    visitor.allowed.extend(<TypescriptResultStruct as #crate_prefix ts_rs::TS>::dependencies().into_iter().map(|dep| dep.type_id));
                    <TypescriptResultStruct as #crate_prefix ts_rs::TS>::visit_dependencies(visitor);
                }
            });
        };

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
            daemon_result_url
        }

    };

    #[cfg(feature = "ts")]
    {
        use convert_case::{Case, Casing as _};
        use itertools::Itertools as _;
        ts_func_body_url.push_str("return daemonResultUrl;\n");

        #[expect(unstable_name_collisions)]
        let ts_func_url_args: String = ts_func_args_url
            .keys()
            .map(String::as_str)
            .intersperse(", ")
            .collect();
        let ts_func_body = format!(
            r#"const daemonRequestUrl = this.{}({});
const daemonRequest: DaemonSDKRequest = {{
  path: `${{daemonRequestUrl.pathname}}${{daemonRequestUrl.search}}`,
  method: '{}',
  headers: Object.assign({{}}, this.headers),
}};
const client = new UndiciClient(daemonRequestUrl.origin, {{
  connect: this.connect,
  bodyTimeout: 0,
  headersTimeout: 0,
}});
{}
try {{
  const response = await client.request(daemonRequest);
  if (response.statusCode >= 200 && response.statusCode <= 299) {{
    daemon_return_response
  }} else {{
    const errorResponse = await response.body.json() as {{ error: {{ id: string, msg: string }} }};
    const errorId = errorResponse.error.id;
    switch (errorId) {{
daemon_return_error_response
      default:
        throw new Error(`Unknown error: ${{errorId}}`);
    }}
  }}

}} catch (err) {{
  return {{ Err: {{ "Communication": err }} }};
}}
            "#,
            format!("{fn_ident}_url").to_case(Case::Camel),
            ts_func_url_args,
            method,
            ts_options_body,
        );

        ts_replace.insert("daemon_return_error_response".to_owned(), quote! {
            &{
                <#result as #crate_prefix FrameworkDocumentedResponse>::error_values()
                    .iter()
                    .map(|str| format!("      case {}:\n        return {{ Err: errorId }};\n", #crate_prefix serde_json::to_string(str).unwrap() ))
                    .collect::<String>()
            }
        });
        ts_replace.insert(
            "daemon_return_response".to_owned(),
            quote! {
               &(
                    <#result as #crate_prefix ts::FrameworkTSResponse>::visit_ts_response()
                )
            },
        );

        let func_name_url = LitStr::new(&format!("{fn_ident}_url"), Span::call_site());
        let func_name = LitStr::new(&fn_ident.to_string(), Span::call_site());
        let func_body_url = LitStr::new(&ts_func_body_url, Span::call_site());
        let func_body = LitStr::new(&ts_func_body, Span::call_site());
        let mut func_args_url = TokenStream::new();
        let mut func_args = TokenStream::new();

        for (name, typ) in ts_func_args_url {
            let name = LitStr::new(&name, Span::call_site());
            func_args_url.extend(quote! {
                visitor.visit_argument::<#typ>(#crate_prefix ts::to_camel(#name));
            });
        }

        for arg in &fn_args_identified {
            match arg {
                FunctionArgument::Path {
                    variable_name,
                    variable_type,
                }
                | FunctionArgument::Query {
                    variable_name,
                    variable_type,
                    ..
                }
                | FunctionArgument::Payload {
                    variable_name,
                    variable_type,
                } => {
                    let name = LitStr::new(&variable_name.to_string(), Span::call_site());
                    func_args.extend(quote! {
                        visitor.visit_argument::<#variable_type>(#crate_prefix ts::to_camel(#name));
                    });
                }
                FunctionArgument::Header {
                    variable_name,
                    required,
                    ..
                } => {
                    let name = LitStr::new(&variable_name.to_string(), Span::call_site());
                    if *required {
                        func_args.extend(quote! {
                            visitor.visit_argument::<String>(#crate_prefix ts::to_camel(#name));
                        });
                    } else {
                        func_args.extend(quote! {
                            visitor.visit_argument::<Option<String>>(#crate_prefix ts::to_camel(#name));
                        });
                    }
                }
            }
        }

        ts_funcs_impl.extend(quote! {

            visitor.visit_start(false, #crate_prefix ts::to_camel(#func_name_url));

            #func_args_url

            visitor.visit_end_result_custom(#func_body_url, "URL");
        });

        let mut replace_chain = TokenStream::new();
        for (replace_key, replace_value) in ts_replace {
            let replace_key = LitStr::new(&replace_key, Span::call_site());
            replace_chain.extend(quote! {
                .replace(#replace_key, #replace_value)
            });
        }

        if method == HttpMethod::Ws {
            // client_impl_body = quote! { panic!("Websocket client is not implemented (and very likely will never be)"); };
        } else if raw {
            if let Some(doc) = doc {
                ts_funcs_impl.extend(quote! {
                    visitor.visit_doc(#doc);
                });
            }

            ts_funcs_impl.extend(quote! {
                visitor.visit_start(true, #crate_prefix ts::to_camel(#func_name));

                #func_args

                visitor.visit_end_result::<#crate_prefix ts::TSResponse<#result>>(#func_body #replace_chain);
            });
        } else {
            if let Some(doc) = doc {
                ts_funcs_impl.extend(quote! {
                    visitor.visit_doc(#doc);
                });
            }

            ts_funcs_impl.extend(quote! {
                visitor.visit_start(true, #crate_prefix ts::to_camel(#func_name));

                #func_args

                visitor.visit_end_result::<#result>(#func_body #replace_chain);
            });
        }
    }

    functions.push(ProcessedFunction {
        client_def,
        client_impl,
        fn_ident,
        method,
        operations,
        path,
        result,
        schema_extra,
        #[cfg(feature = "ts")]
        ts_funcs_impl,
        #[cfg(feature = "ts")]
        ts_types_impl,
        user,
    });

    func
}

pub fn finish(
    crate_prefix: &TokenStream,
    trait_ident: &Ident,
    trait_body_output: &mut Vec<TokenTree>,
    output_extra: &mut Vec<TokenTree>,
    functions: Vec<ProcessedFunction>,
) {
    let mut routes = TokenStream::new();
    let mut routes_user = TokenStream::new();

    let mut client_impl_merged = TokenStream::new();
    let mut client_def_merged = TokenStream::new();

    #[cfg(feature = "ts")]
    let mut ts_types_impl_merged = TokenStream::new();
    #[cfg(feature = "ts")]
    let mut ts_funcs_impl_merged = TokenStream::new();

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
        #[cfg(feature = "ts")]
        ts_types_impl,
        #[cfg(feature = "ts")]
        ts_funcs_impl,
    } in functions
    {
        let token = if user { &mut routes_user } else { &mut routes };

        client_impl_merged.extend(client_impl);
        client_def_merged.extend(client_def);
        #[cfg(feature = "ts")]
        ts_types_impl_merged.extend(ts_types_impl);
        #[cfg(feature = "ts")]
        ts_funcs_impl_merged.extend(ts_funcs_impl);

        let method = method.to_utoipa();

        let path = LitStr::new(&path, Span::call_site());

        let operation_id = LitStr::new(&fn_ident.to_string(), Span::call_site());

        token.extend(quote! {

            .routes(
                {
                    let mut schemas = std::collections::HashMap::new();
                    #schema_extra
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
                    }, MethodRouter::new().on(#crate_prefix utoipa::openapi::HttpMethod::#method.to_method_filter(), __daemon_framework_routes::#fn_ident(self.clone())))
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

    #[cfg(feature = "ts")]
    {
        trait_body_output.extend(quote! {

            #crate_prefix paste::paste! {

                fn visit_types(visitor: &mut #crate_prefix ts::TSTypeVisitor) where Self: Sized {
                    #ts_types_impl_merged
                }

                fn visit_funcs(visitor: &mut #crate_prefix ts::TSClassFunctionVisitor) where Self: Sized {
                    #ts_funcs_impl_merged
                }

            }

        });
    };

    output_extra.extend(quote! {

        #crate_prefix paste::paste! {

            pub trait [< #trait_ident URL >]: #crate_prefix FrameworkClientImpl {
                #client_def_merged
            }

            pub trait [< #trait_ident ClientImpl >]: #crate_prefix FrameworkClientImpl {

            }

            impl<T: #trait_ident + #crate_prefix FrameworkClientImpl> [< #trait_ident URL >] for T {

            }

            #[#crate_prefix async_trait]
            #[diagnostic::do_not_recommend]
            #[allow(unused_variables)] // TODO: Why is this needed?
            impl<T: [< #trait_ident ClientImpl >] + Send + Sync> #trait_ident for T {
                #client_impl_merged
            }

        }

    });
}
