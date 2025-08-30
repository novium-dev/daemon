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
use crate::service::process::HttpMethod;
use proc_macro2::{Ident, TokenStream};
use quote::quote;

#[expect(clippy::too_many_arguments)]
pub fn process(
    crate_prefix: &TokenStream,
    method: HttpMethod,
    raw: bool,
    trait_ident: &Ident,
    fn_ident: &Ident,
    mut func_args: Vec<TokenStream>,
    mut http_args: TokenStream,
    mut call_args: TokenStream,
    routes: &mut TokenStream,
) {
    let mut generic_types = TokenStream::new();
    let mut target_generic_types = TokenStream::new();
    let mut generic_type_conditions = TokenStream::new();

    if HttpMethod::Ws == method {
        func_args.insert(
            0,
            quote! { #crate_prefix axum::extract::ws::WebSocketUpgrade },
        );
        http_args = quote! { ws: #crate_prefix axum::extract::ws::WebSocketUpgrade, #http_args };
        call_args = quote! { ws, #call_args };
    }

    if func_args.is_empty() {
        target_generic_types.extend(quote! { (), });
    } else {
        generic_types.extend(quote! { M, });
        generic_type_conditions.extend(quote! { where });
        target_generic_types.extend(quote! { M, });
        for (index, arg) in func_args.iter().enumerate() {
            target_generic_types.extend(quote! { #arg, });
            if index == func_args.len().wrapping_sub(1) {
                generic_type_conditions.extend(
                    quote! { #arg: #crate_prefix axum::extract::FromRequest<(), M> + Send, },
                );
            }
        }
    }

    let result_map = if HttpMethod::Ws == method || raw {
        TokenStream::new()
    } else {
        quote! { .map(#crate_prefix internal::FrameworkJsonResponse) }
    };

    routes.extend(quote! {

        #crate_prefix pastey::paste! {

            pub(crate) fn #fn_ident<T: super::#trait_ident + Clone + Send + Sync + 'static, #generic_types>(service: T) -> impl #crate_prefix axum::handler::Handler<(#target_generic_types), ()> #generic_type_conditions {
                |#http_args| async move {
                    service.#fn_ident(#call_args).await
                    #result_map
                }
            }

        }

    });
}
