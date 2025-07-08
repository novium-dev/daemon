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

#[cfg_attr(feature = "ts", expect(clippy::too_many_arguments))]
pub fn process(
    crate_prefix: &TokenStream,
    #[cfg(feature = "ts")] fn_ident: &Ident,
    fn_args_identified: &[FunctionArgument],
    http_args: &mut TokenStream,
    func_args: &mut Vec<TokenStream>,
    operations: &mut TokenStream,
    user: bool,
    client_impl_body: &mut TokenStream,
    #[cfg(feature = "ts")] ts_options_body: &mut String,
    #[cfg(feature = "ts")] ts_replace: &mut std::collections::HashMap<String, TokenStream>,
) {
    if !user {
        operations.extend(quote! {
            .parameter(
                #crate_prefix utoipa::openapi::path::ParameterBuilder::from(
                    #crate_prefix utoipa::openapi::path::Parameter::new("Authorization")
                )
                .parameter_in(#crate_prefix utoipa::openapi::path::ParameterIn::Header)
                .description(Some(variable_header_auth::DESCRIPTION))
                .required(#crate_prefix utoipa::openapi::Required::True),
            )
        });
    }

    if !fn_args_identified.is_empty() {
        client_impl_body.extend(quote! {
            let mut headers = #crate_prefix http::header::HeaderMap::new();
        });

        for arg in fn_args_identified {
            let FunctionArgument::Header {
                variable_name,
                variable_type,
                variable_type_wopt,
                required,
            } = arg
            else {
                continue;
            };
            http_args.extend(quote! {
                #variable_name: #variable_type_wopt,
            });
            func_args.push(variable_type_wopt.clone());

            client_impl_body.extend(if *required {
                quote! { #crate_prefix internal::header_insert::<#variable_type, _>(#variable_name, &mut headers); }
            } else {
                quote! {
                    if let Some(daemon_header_value) = #variable_name {
                        #crate_prefix internal::header_insert::<#variable_type, _>(daemon_header_value, &mut headers);
                    }
                }
            });

            #[cfg(feature = "ts")]
            {
                use convert_case::{Case, Casing as _};
                let key = format!("daemon_header_{fn_ident}_{variable_name}");

                if *required {
                    ts_options_body.push_str("daemonRequest.headers[");
                    ts_options_body.push_str(&key);
                    ts_options_body.push_str("] = ");
                    ts_options_body.push_str(&variable_name.to_string().to_case(Case::Camel));
                    ts_options_body.push_str(";\n");
                } else {
                    ts_options_body.push_str("if (");
                    ts_options_body.push_str(&variable_name.to_string().to_case(Case::Camel));
                    ts_options_body.push_str(" != null) { daemonRequest.headers[");
                    ts_options_body.push_str(&key);
                    ts_options_body.push_str("] = ");
                    ts_options_body.push_str(&variable_name.to_string().to_case(Case::Camel));
                    ts_options_body.push_str("; }\n");
                }
                ts_replace.insert(key, quote! {
                    &#crate_prefix serde_json::to_string(#crate_prefix internal::header_name::<#variable_type, _>().as_str()).unwrap()
                });
            };

            let required = Ident::new(if *required { "True" } else { "False" }, Span::call_site());

            operations.extend(quote! {
                            .parameter(
                                #crate_prefix utoipa::openapi::path::ParameterBuilder::from(
                                    #crate_prefix utoipa::openapi::path::Parameter::new(#crate_prefix internal::header_name::<#variable_type, _>().as_str())
                                )
                                .parameter_in(#crate_prefix utoipa::openapi::path::ParameterIn::Header)
                                .description(Some([< variable_header_ #variable_name:snake >]::DESCRIPTION))
                                .required(#crate_prefix utoipa::openapi::Required::#required),
                            )
                        });
        }

        client_impl_body.extend(quote! {
            request = request.headers(headers);
        });
    }
}
