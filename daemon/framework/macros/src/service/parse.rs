use proc_macro2::{Ident, TokenStream};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub(crate) enum ParseExpectation {
    FnKeyword,
    FunctionIdentifier,
    ArgumentsTuple,
    FunctionOrResult,
    Arrow,
    Result,
}

#[derive(Clone)]
pub(crate) enum FunctionArgument {
    Path {
        variable_name: Ident,
        variable_type: TokenStream,
    },
    Query {
        variable_name: Ident,
        variable_type: TokenStream,
        required: bool,
    },
    Header {
        variable_name: Ident,
        variable_type: TokenStream,
        required: bool,
    },
    Payload {
        variable_name: Ident,
        variable_type: TokenStream,
    },
}
