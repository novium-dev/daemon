use proc_macro2::{Ident, TokenStream};
use quote::quote;
use syn::LitStr;
use syn::parse::{Parse, ParseStream};

pub(crate) struct VariableDefinitionContent {
    pub(crate) name: Ident,
    pub(crate) description: LitStr,
}

pub(crate) fn define_variable0(crate_prefix: TokenStream, attr: TokenStream) -> TokenStream {
    let VariableDefinitionContent { name, description } =
        syn::parse2(attr).expect("Expected variable content");

    let mut result = Vec::new();

    result.extend(quote! {
        #crate_prefix paste::paste! {
            pub mod [< variable_ #name:snake >] {

                pub const DESCRIPTION: &str = #description;

            }
        }
    });

    TokenStream::from_iter(result)
}

impl Parse for VariableDefinitionContent {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        let description: LitStr = input.parse()?;

        Ok(VariableDefinitionContent { name, description })
    }
}
