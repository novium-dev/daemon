use crate::service::parse::FunctionArgument;
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;

pub(crate) fn process(
    crate_prefix: &TokenStream,
    fn_args_identified: &[FunctionArgument],
    http_args: &mut TokenStream,
    func_args: &mut Vec<TokenStream>,
    operations: &mut TokenStream,
    user: bool,
    client_impl_body: &mut TokenStream,
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
