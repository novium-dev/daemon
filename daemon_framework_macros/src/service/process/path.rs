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
#[cfg(feature = "ts")]
use proc_macro2::Ident;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::LitStr;

#[expect(clippy::too_many_arguments)]
pub fn process(
    crate_prefix: &TokenStream,
    #[cfg(feature = "ts")] trait_ident: &Ident,
    #[cfg(feature = "ts")] fn_ident: &Ident,
    path: &str,
    fn_args_identified: &[FunctionArgument],
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
    let fn_args_path: Vec<_> = fn_args_identified
        .iter()
        .filter_map(|arg| {
            if let FunctionArgument::Path {
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

    let mut urlencode = TokenStream::new();

    if !fn_args_path.is_empty() {
        let mut path_names = TokenStream::new();
        let mut path_types = TokenStream::new();

        let mut ts_struct = TokenStream::new();

        for (arg_name, arg_type) in &fn_args_path {
            path_names.extend(quote! { #arg_name, });
            path_types.extend(quote! { #arg_type, });

            let variable_literal = LitStr::new(&arg_name.to_string(), arg_name.span());
            operations.extend(quote! {
                .parameter(
                    #crate_prefix utoipa::openapi::path::ParameterBuilder::from(
                        #crate_prefix utoipa::openapi::path::Parameter::new(#variable_literal)
                    )
                    .parameter_in(#crate_prefix utoipa::openapi::path::ParameterIn::Path)
                    .description(Some([< variable_path_ #arg_name:snake >]::DESCRIPTION)),
                )
            });

            fn_def_args.extend(quote! { #arg_name: #arg_type, });
            fn_def_call.extend(quote! { #arg_name, });

            #[cfg(feature = "ts")]
            {
                use convert_case::{Case, Casing as _};
                ts_func_args_url
                    .insert(arg_name.to_string().to_case(Case::Camel), arg_type.clone());

                ts_func_body_url.push_str("const ");
                ts_func_body_url.push_str(&format!("{arg_name}_encoded").to_case(Case::Camel));
                ts_func_body_url.push_str(" = ");
                ts_func_body_url.push_str("encodeURIComponent(String(");
                ts_func_body_url.push_str(&arg_name.to_string().to_case(Case::Camel));
                ts_func_body_url.push_str("));\n");
            };

            urlencode.extend(quote! {
                let #arg_name: String = #arg_name.serialize(#crate_prefix internal::StringSerializer).unwrap().unwrap_or_default();
                let #arg_name = #crate_prefix urlencoding::encode(& #arg_name);
            });

            ts_struct.extend(quote! { #arg_name: #arg_type, });
        }

        #[cfg(feature = "ts")]
        {
            let ts_crate_literal = LitStr::new(&format!("{crate_prefix}ts_rs"), Span::call_site());
            let trait_name = LitStr::new(&trait_ident.to_string(), trait_ident.span());
            let fn_name = LitStr::new(&fn_ident.to_string(), fn_ident.span());

            types_impl.extend(quote! {

                {
                    println!("{}::{} Path", #trait_name, #fn_name);

                    #[derive(#crate_prefix ts_rs::TS)]
                    #[ts(crate = #ts_crate_literal)]
                    #[allow(dead_code)]
                    struct TypescriptPathStruct {
                        #ts_struct
                    }

                    visitor.allowed.extend(<TypescriptPathStruct as #crate_prefix ts_rs::TS>::dependencies().into_iter().map(|dep| dep.type_id));
                    <TypescriptPathStruct as #crate_prefix ts_rs::TS>::visit_dependencies(visitor);
                }
            });
        };

        http_args.extend(quote! {
            #crate_prefix axum::extract::Path((#path_names)): #crate_prefix axum::extract::Path<(#path_types)>,
        });
        func_args.push(quote! { #crate_prefix axum::extract::Path<(#path_types)> });
    }

    let Some(path) = path.strip_prefix("/") else {
        panic!("Path must start with /");
    };

    #[cfg(feature = "ts")]
    {
        use convert_case::{Case, Casing as _};
        ts_func_body_url.push_str("const daemonResultUrl = new URL(`${this.baseUrl}");
        let mut ts_path = path.to_owned();
        for (name, _) in fn_args_path {
            ts_path = ts_path.replace(
                &format!("{{{name}}}"),
                &format!("${{{}}}", format!("{name}_encoded").to_case(Case::Camel)),
            );
        }
        ts_func_body_url.push_str(&ts_path);
        ts_func_body_url.push_str("`);\n");
    };

    let path = LitStr::new(path, Span::call_site());
    client_def_body.extend(quote! {
        use #crate_prefix serde::Serialize;
        #urlencode
        let daemon_result_url: String = format!(#path);
        let mut daemon_result_url = <Self as #crate_prefix FrameworkClientImpl>::base_url(self).join(&daemon_result_url).expect("Expected url");
    });
}
