use crate::service::parse::FunctionArgument;
use proc_macro2::{Ident, TokenStream};
use quote::quote;
use syn::LitStr;

#[allow(clippy::too_many_arguments)]
pub(crate) fn process(
    crate_prefix: &TokenStream,
    fn_ident: &Ident,
    fn_args_identified: &[FunctionArgument],
    routes: &mut TokenStream,
    http_args: &mut TokenStream,
    func_args: &mut Vec<TokenStream>,
    operations: &mut TokenStream,
    example: Option<LitStr>,
) {
    let fn_args_payload: Vec<_> = fn_args_identified
        .iter()
        .filter(|arg| matches!(arg, FunctionArgument::Payload { .. }))
        .cloned()
        .map(|arg| {
            let FunctionArgument::Payload {
                variable_name,
                variable_type,
            } = arg
            else {
                unreachable!()
            };
            (variable_name, variable_type)
        })
        .collect();
    let content_extra = if let Some(_example) = example {
        // TODO
        quote! {
            .example(Some(serde_json::json!({})))
        }
    } else {
        TokenStream::new()
    };

    if fn_args_payload.len() == 1 {
        let (arg_name, arg_type) = fn_args_payload[0].clone();

        http_args.extend(quote! {
                #crate_prefix axum::extract::Json(
                    #arg_name
                ):
                #crate_prefix axum::extract::Json<
                    #arg_type
                >,
        });
        func_args.push(quote! { #crate_prefix axum::extract::Json< #arg_type > });

        operations.extend(quote! {
            .request_body(Some(
                #crate_prefix utoipa::openapi::request_body::RequestBodyBuilder::new()
                    .content("application/json", #crate_prefix utoipa::openapi::content::Content::builder()
                        .schema(Some(< #arg_type >::schema()))
                        #content_extra
                        .build()
                    )
                    .required(Some(#crate_prefix utoipa::openapi::Required::True))
                    .build()
            ))
        });
    } else if !fn_args_payload.is_empty() {
        let mut payload_fields = TokenStream::new();
        let mut payload_names = TokenStream::new();

        for (arg_name, arg_type) in &fn_args_payload {
            payload_fields.extend(quote! {
                #arg_name: #arg_type,
            });
            payload_names.extend(quote! {
                #arg_name,
            });
        }

        routes.extend(quote! {
            #crate_prefix paste::paste! {
                #[derive(Clone, #crate_prefix serde::Serialize, #crate_prefix serde::Deserialize, Debug, #crate_prefix utoipa::ToSchema)]
                pub(crate) struct [< #fn_ident:camel Payload >] {
                    #payload_fields
                }
            }
        });
        http_args.extend(quote! {
                #crate_prefix axum::extract::Json(
                    [< #fn_ident:camel Payload >] { #payload_names }
                ):
                #crate_prefix axum::extract::Json<
                    [< #fn_ident:camel Payload >]
                >,
        });
        func_args
            .push(quote! { #crate_prefix axum::extract::Json< [< #fn_ident:camel Payload >] > });

        operations.extend(quote! {
            .request_body(Some(
                #crate_prefix utoipa::openapi::request_body::RequestBodyBuilder::new()
                    .content("application/json", #crate_prefix utoipa::openapi::content::Content::builder()
                        .schema(Some(< __daemon_framework_routes::[< #fn_ident:camel Payload >] >::schema()))
                        #content_extra
                        .build()
                    )
                    .required(Some(#crate_prefix utoipa::openapi::Required::True))
                    .build()
            ))
        });
    }

    if !fn_args_payload.is_empty() {}
}
