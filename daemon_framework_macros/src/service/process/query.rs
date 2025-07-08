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
    #[cfg(feature = "ts")] trait_ident: &Ident,
    fn_ident: &Ident,
    fn_args_identified: &[FunctionArgument],
    routes: &mut TokenStream,
    http_args: &mut TokenStream,
    func_args: &mut Vec<TokenStream>,
    operations: &mut TokenStream,
    client_def_body: &mut TokenStream,
    fn_def_args: &mut TokenStream,
    fn_def_call: &mut TokenStream,
    #[cfg(feature = "ts")] types_impl: &mut TokenStream,
    #[cfg(feature = "ts")] ts_func_body_url: &mut String,
    #[cfg(feature = "ts")] ts_func_args_url: &mut indexmap::IndexMap<String, TokenStream>,
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

            #[cfg(feature = "ts")]
            {
                use convert_case::{Case, Casing as _};
                ts_func_args_url
                    .insert(arg_name.to_string().to_case(Case::Camel), arg_type.clone());

                if !required {
                    ts_func_body_url.push_str("if (");
                    ts_func_body_url.push_str(&arg_name.to_string().to_case(Case::Camel));
                    ts_func_body_url.push_str(" != null) { ");
                }
                ts_func_body_url.push_str("daemonResultUrl.searchParams.append('");
                ts_func_body_url.push_str(&arg_name.to_string());
                ts_func_body_url.push_str("', String(");
                ts_func_body_url.push_str(&arg_name.to_string().to_case(Case::Camel));
                ts_func_body_url.push_str("));");

                if !required {
                    ts_func_body_url.push_str(" }");
                }

                ts_func_body_url.push('\n');
            };

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
            #crate_prefix paste::paste! {
                #[derive(Debug, Clone, #crate_prefix serde::Serialize, #crate_prefix serde::Deserialize, #ts_extra)]
                #ts_attr_extra
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

        #[cfg(feature = "ts")]
        {
            let trait_name = LitStr::new(&trait_ident.to_string(), trait_ident.span());
            let fn_name = LitStr::new(&fn_ident.to_string(), fn_ident.span());

            types_impl.extend(quote! {
                {
                    println!("{}::{} Query", #trait_name, #fn_name);
                    use __daemon_framework_routes::*;
                    visitor.allowed.extend(<[< #fn_ident:camel Query >] as #crate_prefix ts_rs::TS>::dependencies().into_iter().map(|dep| dep.type_id));
                    <[< #fn_ident:camel Query >] as #crate_prefix ts_rs::TS>::visit_dependencies(visitor);
                }
            });
        }
    }
}
