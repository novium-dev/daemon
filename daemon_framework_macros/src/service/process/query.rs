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
use quote::{ToTokens as _, quote};
use syn::LitStr;

#[expect(clippy::too_many_arguments)]
pub fn process(
    crate_prefix: &TokenStream,
    fn_ident: &Ident,
    fn_args_identified: &[FunctionArgument],
    routes: &mut TokenStream,
    http_args: &mut TokenStream,
    func_args: &mut Vec<TokenStream>,
    operations: &mut TokenStream,
    client_def_body: &mut TokenStream,
    fn_def_args: &mut TokenStream,
    fn_def_call: &mut TokenStream,
) {
    let fn_args_query: Vec<_> = fn_args_identified
        .iter()
        .filter_map(|arg| {
            if let FunctionArgument::Query {
                variable_name,
                variable_type_wopt,
                required,
                desc,
                ..
            } = arg
            {
                Some((
                    variable_name.clone(),
                    variable_type_wopt.clone(),
                    *required,
                    desc.clone(),
                ))
            } else {
                None
            }
        })
        .collect();
    if !fn_args_query.is_empty() {
        let mut client_def_body_pre = TokenStream::new();
        let mut client_def_body_post = quote! {
            daemon_result_url.query_pairs_mut()
        };

        let mut query_fields = TokenStream::new();
        let mut query_names = TokenStream::new();

        for (arg_name, arg_type, required, desc) in fn_args_query {
            query_fields.extend(quote! {
                #arg_name: #arg_type,
            });
            query_names.extend(quote! {
                #arg_name,
            });

            let variable_literal = LitStr::new(&arg_name.to_string(), arg_name.span());
            let required_indent =
                Ident::new(if required { "True" } else { "False" }, Span::call_site());

            let desc = desc.map_or_else(
                || quote! { [< variable_query_ #arg_name:snake >]::DESCRIPTION },
                LitStr::into_token_stream,
            );
            operations.extend(quote! {
                .parameter(
                    #crate_prefix utoipa::openapi::path::ParameterBuilder::from(
                        #crate_prefix utoipa::openapi::path::Parameter::new(#variable_literal)
                    )
                    .parameter_in(#crate_prefix utoipa::openapi::path::ParameterIn::Query)
                    .description(Some(#desc))
                    .required(#crate_prefix utoipa::openapi::Required::#required_indent),
                )
            });

            fn_def_args.extend(quote! { #arg_name: #arg_type, });
            fn_def_call.extend(quote! { #arg_name, });

            client_def_body_pre.extend(quote! {
                let #arg_name: Option<String> = #arg_name.serialize(#crate_prefix internal::StringSerializer).unwrap();
            });
            client_def_body_post.extend(quote! {
                .extend_pairs(#arg_name.map(|val| (#variable_literal, val)).iter())
            });
        }

        client_def_body_post.extend(quote! { ; });

        client_def_body.extend(client_def_body_pre);
        client_def_body.extend(client_def_body_post);

        routes.extend(quote! {
            #crate_prefix pastey::paste! {
                #[derive(Debug, Clone, #crate_prefix serde::Serialize, #crate_prefix serde::Deserialize)]
                pub(crate) struct [< #fn_ident:camel Query >] {
                    #query_fields
                }
            }
        });
        http_args.extend(quote! {
                #crate_prefix axum::extract::Query(
                    [< #fn_ident:camel Query >] { #query_names }
                ):
                #crate_prefix axum::extract::Query<
                    [< #fn_ident:camel Query >]
                >,
        });
        func_args
            .push(quote! { #crate_prefix axum::extract::Query< [< #fn_ident:camel Query >] > });
    }
}
