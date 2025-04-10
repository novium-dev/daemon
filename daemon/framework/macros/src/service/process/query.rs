use crate::service::parse::FunctionArgument;
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::LitStr;

pub(crate) fn process(
    crate_prefix: &TokenStream,
    fn_ident: &Ident,
    fn_args_identified: &[FunctionArgument],
    routes: &mut TokenStream,
    http_args: &mut TokenStream,
    func_args: &mut Vec<TokenStream>,
    operations: &mut TokenStream,
) {
    let fn_args_query: Vec<_> = fn_args_identified
        .iter()
        .filter(|arg| matches!(arg, FunctionArgument::Query { .. }))
        .cloned()
        .map(|arg| {
            let FunctionArgument::Query {
                variable_name,
                variable_type_wopt,
                required,
                ..
            } = arg
            else {
                unreachable!()
            };
            (variable_name, variable_type_wopt, required)
        })
        .collect();
    if !fn_args_query.is_empty() {
        let mut query_fields = TokenStream::new();
        let mut query_names = TokenStream::new();

        for (arg_name, arg_type, required) in fn_args_query {
            query_fields.extend(quote! {
                #arg_name: #arg_type,
            });
            query_names.extend(quote! {
                #arg_name,
            });

            let variable_literal = LitStr::new(&arg_name.to_string(), arg_name.span());
            let required = Ident::new(if required { "True" } else { "False" }, Span::call_site());

            operations.extend(quote! {
                .parameter(
                    #crate_prefix utoipa::openapi::path::ParameterBuilder::from(
                        #crate_prefix utoipa::openapi::path::Parameter::new(#variable_literal)
                    )
                    .parameter_in(#crate_prefix utoipa::openapi::path::ParameterIn::Query)
                    .description(Some([< variable_query_ #arg_name:snake >]::DESCRIPTION))
                    .required(#crate_prefix utoipa::openapi::Required::#required),
                )
            });
        }

        routes.extend(quote! {
            #crate_prefix paste::paste! {
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
