use proc_macro2::{Group, Ident, TokenStream};
use syn::LitStr;

#[derive(Debug)]
pub(crate) enum ParseExpectation {
    FnKeyword,
    FunctionIdentifier {
        fn_keyword: Ident,
    },
    ArgumentsTuple {
        fn_keyword: Ident,
        fn_ident: Ident,
    },
    FunctionOrResult {
        fn_keyword: Ident,
        fn_ident: Ident,
        fn_args: Group,
    },
    Arrow {
        fn_keyword: Ident,
        fn_ident: Ident,
        fn_args: Group,
    },
    Result {
        fn_keyword: Ident,
        fn_ident: Ident,
        fn_args: Group,
    },
}

#[derive(Clone)]
pub(crate) enum FunctionArgument {
    Path {
        variable_name: Ident,
        variable_type: TokenStream,
    },
    Query {
        variable_name: Ident,
        #[allow(unused)]
        variable_type: TokenStream,
        variable_type_wopt: TokenStream,
        required: bool,
        desc: Option<LitStr>,
    },
    Header {
        variable_name: Ident,
        variable_type: TokenStream,
        variable_type_wopt: TokenStream,
        required: bool,
    },
    Payload {
        variable_name: Ident,
        variable_type: TokenStream,
    },
}
