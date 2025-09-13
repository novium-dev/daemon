use crate::service::parse::FunctionArgument;
use proc_macro2::TokenStream;
use quote::quote;

pub fn process(
    crate_prefix: &TokenStream,
    fn_args_identified: &[FunctionArgument],
    http_args: &mut TokenStream,
    func_args: &mut Vec<TokenStream>,
    operations: &mut TokenStream,
    client_impl_body: &mut TokenStream,
) {
    let fn_args_path: Vec<_> = fn_args_identified
        .iter()
        .filter_map(|arg| {
            if let FunctionArgument::Special {
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

    for (arg_name, arg_type) in &fn_args_path {
        operations.extend(quote! {
            operations = <#arg_type as #crate_prefix types::DocumentedType>::openapi(operations);
        });

        http_args.extend(quote! {
            #arg_name: #arg_type,
        });
        func_args.push(quote! { #arg_type });
        client_impl_body.extend(quote! {
            request = <#arg_type as #crate_prefix sdk::IntoRequestParts>::into_request(#arg_name, request).await;
        });
    }
}
