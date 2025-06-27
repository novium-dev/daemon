use proc_macro2::{Group, Ident, TokenStream};
use syn::LitStr;

#[derive(Debug)]
#[expect(
    clippy::arbitrary_source_item_ordering,
    reason = "In the order of occurrence"
)]
pub enum ParseExpectation {
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
pub enum FunctionArgument {
    Header {
        variable_name: Ident,
        variable_type: TokenStream,
        variable_type_wopt: TokenStream,
        required: bool,
    },
    Path {
        variable_name: Ident,
        variable_type: TokenStream,
    },
    Payload {
        variable_name: Ident,
        variable_type: TokenStream,
    },
    Query {
        variable_name: Ident,
        #[cfg(feature = "ts")]
        variable_type: TokenStream,
        variable_type_wopt: TokenStream,
        required: bool,
        desc: Option<LitStr>,
    },
}
