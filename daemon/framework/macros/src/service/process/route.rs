use crate::service::process::HttpMethod;
use proc_macro2::{Ident, TokenStream};
use quote::quote;

#[allow(clippy::too_many_arguments)]
pub(crate) fn process(
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

    if let HttpMethod::WS = method {
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
            if index == func_args.len() - 1 {
                generic_type_conditions.extend(
                    quote! { #arg: #crate_prefix axum::extract::FromRequest<(), M> + Send, },
                );
            }
        }
    }

    let result_map = if let HttpMethod::WS = method {
        TokenStream::new()
    } else if raw {
        TokenStream::new()
    } else {
        quote! { .map(#crate_prefix internal::FrameworkJsonResponse) }
    };

    routes.extend(quote! {

        #crate_prefix paste::paste! {

            pub(crate) fn #fn_ident<T: super::#trait_ident + Clone + Send + Sync + 'static, #generic_types>(service: T) -> impl #crate_prefix axum::handler::Handler<(#target_generic_types), ()> #generic_type_conditions {
                |#http_args| async move {
                    service.#fn_ident(#call_args).await
                    #result_map
                }
            }

        }

    });
}
