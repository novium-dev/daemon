use crate::service::parse::FunctionArgument;
use proc_macro2::TokenStream;
use quote::quote;
use syn::LitStr;

pub(crate) fn process(
    crate_prefix: &TokenStream,
    fn_args_identified: &[FunctionArgument],
    http_args: &mut TokenStream,
    func_args: &mut Vec<TokenStream>,
    operations: &mut TokenStream,
) {
    let fn_args_path: Vec<_> = fn_args_identified
        .iter()
        .filter(|arg| matches!(arg, FunctionArgument::Path { .. }))
        .cloned()
        .map(|arg| {
            let FunctionArgument::Path {
                variable_name,
                variable_type,
            } = arg
            else {
                unreachable!()
            };
            (variable_name, variable_type)
        })
        .collect();
    if !fn_args_path.is_empty() {
        let mut path_names = TokenStream::new();
        let mut path_types = TokenStream::new();

        for (arg_name, arg_type) in fn_args_path {
            path_names.extend(quote! { #arg_name, });
            path_types.extend(quote! { #arg_type, });

            let variable_literal = LitStr::new(&arg_name.to_string(), arg_name.span());
            operations.extend(quote! {
                .parameter(
                    #crate_prefix utoipa::openapi::path::ParameterBuilder::from(
                        #crate_prefix utoipa::openapi::path::Parameter::new(#variable_literal)
                    )
                    .parameter_in(#crate_prefix utoipa::openapi::path::ParameterIn::Path)
                    .description(Some([< variable_ #arg_name:snake >]::DESCRIPTION)),
                )
            });
        }

        http_args.extend(quote! {
            #crate_prefix axum::extract::Path((#path_names)): #crate_prefix axum::extract::Path<(#path_types)>,
        });
        func_args.push(quote! { #crate_prefix axum::extract::Path<(#path_types)> });
    }
}
