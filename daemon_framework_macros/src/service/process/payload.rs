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
use crate::service::parse::FunctionArgument;
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::LitStr;

#[expect(clippy::too_many_arguments)]
pub fn process(
    crate_prefix: &TokenStream,
    #[cfg(feature = "ts")] trait_ident: &Ident,
    fn_ident: &Ident,
    fn_args_identified: &[FunctionArgument],
    routes: &mut TokenStream,
    http_args: &mut TokenStream,
    func_args: &mut Vec<TokenStream>,
    operations: &mut TokenStream,
    schema_extra: &mut TokenStream,
    example: Option<LitStr>,
    client_impl_body: &mut TokenStream,
    #[cfg(feature = "ts")] types_impl: &mut TokenStream,
    #[cfg(feature = "ts")] ts_options_body: &mut String,
) {
    let fn_args_payload: Vec<_> = fn_args_identified
        .iter()
        .filter_map(|arg| {
            if let FunctionArgument::Payload {
                variable_name,
                variable_type,
            } = arg
            {
                Some((variable_name.clone(), variable_type.clone()))
            } else {
                None
            }
        })
        .collect();
    let content_extra = example.map_or_else(TokenStream::new, |example| {
        let err = LitStr::new(
            &format!("Failed to parse example {:?}", example.value()),
            Span::call_site(),
        );
        quote! {
            .example(Some(#crate_prefix serde_json::from_str::<'static, #crate_prefix serde_json::Value>(include_str!(#example)).expect(#err)))
        }
    });

    let stream = fn_args_payload
        .iter()
        .map(|(_, typ)| typ.to_string())
        .any(|typ| typ.eq("DataStream"));

    if fn_args_payload.len() == 1 {
        #[expect(clippy::indexing_slicing, reason = "Bounds check before indexing")]
        let (arg_name, arg_type) = fn_args_payload[0].clone();

        if stream {
            http_args.extend(quote! {
                #arg_name:
                #arg_type,
            });
            func_args.push(quote! { #arg_type });

            operations.extend(quote! {
                .request_body(Some(
                    #crate_prefix utoipa::openapi::request_body::RequestBodyBuilder::new()
                        .content("application/octet-stream", #crate_prefix utoipa::openapi::content::Content::builder()
                            .build()
                        )
                        .required(Some(#crate_prefix utoipa::openapi::Required::True))
                        .build()
                ))
            });

            client_impl_body.extend(quote! {
                request = request.body(#arg_name.into_reqwest());
            });

            #[cfg(feature = "ts")]
            {
                use convert_case::{Case, Casing as _};
                ts_options_body.push_str("daemonRequest.body = ");
                ts_options_body.push_str(&arg_name.to_string().to_case(Case::Camel));
                ts_options_body.push_str(";\n");
                ts_options_body.push_str(
                    "daemonRequest.headers['content-type'] = 'application/octet-stream';\n",
                );
            };
        } else {
            #[cfg(feature = "ts")]
            {
                use convert_case::{Case, Casing as _};
                let ts_crate_literal =
                    LitStr::new(&format!("{crate_prefix}ts_rs"), Span::call_site());
                let trait_name = LitStr::new(&trait_ident.to_string(), trait_ident.span());
                let fn_name = LitStr::new(&fn_ident.to_string(), fn_ident.span());

                types_impl.extend(quote! {
                    {
                        println!("{}::{} Payload", #trait_name, #fn_name);

                        #[derive(#crate_prefix ts_rs::TS)]
                        #[ts(crate = #ts_crate_literal)]
                        #[allow(dead_code)]
                        struct TypescriptPayloadStruct {
                            payload: #arg_type,
                        }

                        visitor.allowed.extend(<TypescriptPayloadStruct as #crate_prefix ts_rs::TS>::dependencies().into_iter().map(|dep| dep.type_id));
                        <TypescriptPayloadStruct as #crate_prefix ts_rs::TS>::visit_dependencies(visitor);
                    }
                });

                ts_options_body.push_str("daemonRequest.body = JSON.stringify(");
                ts_options_body.push_str(&arg_name.to_string().to_case(Case::Camel));
                ts_options_body.push_str(");\n");
                ts_options_body
                    .push_str("daemonRequest.headers['content-type'] = 'application/json';\n");
            };

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
                            .schema(Some(
                                #crate_prefix utoipa::openapi::schema::RefBuilder::new()
                                    .ref_location_from_schema_name(< #arg_type as #crate_prefix utoipa::ToSchema >::name())
                                    .build()
                            ))
                            #content_extra
                            .build()
                        )
                        .required(Some(#crate_prefix utoipa::openapi::Required::True))
                        .build()
                ))
            });
            schema_extra.extend(quote! {
                schemas.insert(< #arg_type as #crate_prefix utoipa::ToSchema >::name().to_string(), < #arg_type as #crate_prefix utoipa::PartialSchema >::schema());
                let mut schemas_vec = Vec::new();
                < #arg_type as #crate_prefix utoipa::ToSchema >::schemas(&mut schemas_vec);
                schemas.extend(schemas_vec);
            });

            client_impl_body.extend(quote! {
                request = request.json(&#arg_name);
            });
        }
    } else if !fn_args_payload.is_empty() {
        assert!(
            !stream,
            "You can't mix a DataStream with multiple payload fields"
        );
        let mut payload_fields = TokenStream::new();
        let mut payload_names = TokenStream::new();

        for (arg_name, arg_type) in &fn_args_payload {
            payload_fields.extend(quote! {
                pub(crate) #arg_name: #arg_type,
            });
            payload_names.extend(quote! {
                #arg_name,
            });
        }

        #[cfg(not(feature = "ts"))]
        let (ts_extra, ts_attr_extra) = { (TokenStream::new(), TokenStream::new()) };
        #[cfg(feature = "ts")]
        let (ts_extra, ts_attr_extra) = {
            let ts_crate_literal = LitStr::new(&format!("{crate_prefix}ts_rs"), Span::call_site());

            let ts_extra = quote! { #crate_prefix ts_rs::TS };
            #[cfg(feature = "ts")]
            let ts_attr_extra = quote! { #[ts(crate = #ts_crate_literal )]};
            (ts_extra, ts_attr_extra)
        };

        routes.extend(quote! {
            #crate_prefix pastey::paste! {
                #[derive(Clone, #crate_prefix serde::Serialize, #crate_prefix serde::Deserialize, Debug, #crate_prefix utoipa::ToSchema, #ts_extra)]
                #ts_attr_extra
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
                        .schema(Some(
                            #crate_prefix utoipa::openapi::schema::RefBuilder::new()
                                .ref_location_from_schema_name(< __daemon_framework_routes::[< #fn_ident:camel Payload >] as #crate_prefix utoipa::ToSchema >::name())
                                .build()
                        ))
                        #content_extra
                        .build()
                    )
                    .required(Some(#crate_prefix utoipa::openapi::Required::True))
                    .build()
            ))
        });
        schema_extra.extend(quote! {
            schemas.insert(< __daemon_framework_routes::[< #fn_ident:camel Payload >] as #crate_prefix utoipa::ToSchema >::name().to_string(), < __daemon_framework_routes::[< #fn_ident:camel Payload >] as #crate_prefix utoipa::PartialSchema >::schema());
            let mut schemas_vec = Vec::new();
            < __daemon_framework_routes::[< #fn_ident:camel Payload >] as #crate_prefix utoipa::ToSchema >::schemas(&mut schemas_vec);
            schemas.extend(schemas_vec);
        });

        client_impl_body.extend(quote! {
            request = request.json(&__daemon_framework_routes::[< #fn_ident:camel Payload >] { #payload_names });
        });

        #[cfg(feature = "ts")]
        {
            use convert_case::{Case, Casing as _};
            use itertools::Itertools as _;
            let trait_name = LitStr::new(&trait_ident.to_string(), trait_ident.span());
            let fn_name = LitStr::new(&fn_ident.to_string(), fn_ident.span());
            types_impl.extend(quote! {
                {
                    println!("{}::{} Payload", #trait_name, #fn_name);

                    visitor.allowed.extend(<__daemon_framework_routes::[< #fn_ident:camel Payload >] as #crate_prefix ts_rs::TS>::dependencies().into_iter().map(|dep| dep.type_id));
                    <__daemon_framework_routes::[< #fn_ident:camel Payload >] as #crate_prefix ts_rs::TS>::visit_dependencies(visitor);
                }
            });

            ts_options_body.push_str("daemonRequest.body = JSON.stringify({ ");
            #[expect(unstable_name_collisions)]
            let arg_names: String = fn_args_payload
                .iter()
                .map(|(ident, _)| format!("{}: {}", ident, ident.to_string().to_case(Case::Camel)))
                .intersperse(", ".to_owned())
                .collect();
            ts_options_body.push_str(&arg_names);
            ts_options_body.push_str(" });\n");
            ts_options_body
                .push_str("daemonRequest.headers['content-type'] = 'application/json';\n");
        }
    } else {
        // If there are no payload arguments, there is no payload.
    }
}
