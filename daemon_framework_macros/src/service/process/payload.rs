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
    trait_ident: &Ident,
    fn_ident: &Ident,
    fn_args_identified: &[FunctionArgument],
    routes: &mut TokenStream,
    http_args: &mut TokenStream,
    func_args: &mut Vec<TokenStream>,
    operations: &mut TokenStream,
    schema_extra: &mut TokenStream,
    example: Option<LitStr>,
    client_impl_body: &mut TokenStream,
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
                operations = operations.request_body(Some(
                    #crate_prefix utoipa::openapi::request_body::RequestBodyBuilder::new()
                        .content("application/octet-stream", #crate_prefix utoipa::openapi::content::Content::builder()
                            .build()
                        )
                        .required(Some(#crate_prefix utoipa::openapi::Required::True))
                        .build()
                ));
            });

            client_impl_body.extend(quote! {
                request = request.body(#arg_name.into_reqwest());
            });
        } else {
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
                operations = operations.request_body(Some(
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
                ));
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

        routes.extend(quote! {
            #[derive(Clone, #crate_prefix serde::Serialize, #crate_prefix serde::Deserialize, Debug, #crate_prefix utoipa::ToSchema)]
            pub(crate) struct [< #fn_ident:camel Payload >] {
                #payload_fields
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
            operations = operations.request_body(Some(
                #crate_prefix utoipa::openapi::request_body::RequestBodyBuilder::new()
                    .content("application/json", #crate_prefix utoipa::openapi::content::Content::builder()
                        .schema(Some(
                            #crate_prefix utoipa::openapi::schema::RefBuilder::new()
                                .ref_location_from_schema_name(< [< __ #trait_ident:snake _framework_routes >]::[< #fn_ident:camel Payload >] as #crate_prefix utoipa::ToSchema >::name())
                                .build()
                        ))
                        #content_extra
                        .build()
                    )
                    .required(Some(#crate_prefix utoipa::openapi::Required::True))
                    .build()
            ));
        });
        schema_extra.extend(quote! {
            schemas.insert(< [< __ #trait_ident:snake _framework_routes >]::[< #fn_ident:camel Payload >] as #crate_prefix utoipa::ToSchema >::name().to_string(), < [< __ #trait_ident:snake _framework_routes >]::[< #fn_ident:camel Payload >] as #crate_prefix utoipa::PartialSchema >::schema());
            let mut schemas_vec = Vec::new();
            < [< __ #trait_ident:snake _framework_routes >]::[< #fn_ident:camel Payload >] as #crate_prefix utoipa::ToSchema >::schemas(&mut schemas_vec);
            schemas.extend(schemas_vec);
        });

        client_impl_body.extend(quote! {
            request = request.json(&[< __ #trait_ident:snake _framework_routes >]::[< #fn_ident:camel Payload >] { #payload_names });
        });
    } else {
        // If there are no payload arguments, there is no payload.
    }
}
