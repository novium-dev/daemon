use proc_macro2::{Ident, Punct, TokenStream, TokenTree};
use quote::TokenStreamExt;
use syn::Token;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};

pub(crate) struct TokenArray<T: Parse>(pub(crate) Vec<T>);

impl<T: Parse> Parse for TokenArray<T> {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input
            .parse_terminated(parse_token_array, Token![,])
            .map(|f| TokenArray(f.into_iter().collect()))
    }
}

fn parse_token_array<T: Parse>(stream: ParseStream) -> Result<T, syn::Error> {
    stream.parse()
}

pub(crate) struct TokenStreamArray(pub(crate) Vec<TokenStream>);

impl Parse for TokenStreamArray {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input
            .parse_terminated(parse_tokenstream_array, Token![,])
            .map(|f| TokenStreamArray(f.into_iter().collect()))
    }
}

fn parse_tokenstream_array(stream: ParseStream) -> Result<TokenStream, syn::Error> {
    let mut result = TokenStream::new();
    while !stream.is_empty() {
        if stream.peek(Token![,]) {
            break;
        }
        result.append(stream.parse::<TokenTree>()?);
    }
    Ok(result)
}

struct OptionTypeParseResult(Option<TokenStream>);

impl Parse for OptionTypeParseResult {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if !input.peek(syn::Ident::peek_any) {
            while !input.is_empty() {
                let _: TokenTree = input.parse()?;
            }
            return Ok(Self(None));
        }
        let ident: Ident = input.parse()?;
        if !ident.to_string().eq("Option") {
            while !input.is_empty() {
                let _: TokenTree = input.parse()?;
            }
            return Ok(Self(None));
        }
        let punct: Punct = input.parse()?;
        if punct.as_char() != '<' {
            return Err(syn::Error::new(punct.span(), "Expected < after Option"));
        }
        let mut stream = TokenStream::new();

        let mut layer = 1;
        while !input.is_empty() {
            let token: TokenTree = input.parse()?;
            if let TokenTree::Punct(ref punct) = token {
                if punct.as_char() == '<' {
                    layer += 1;
                }
                if punct.as_char() == '>' {
                    layer -= 1;
                }
            }
            stream.extend([token]);
            if layer == 0 {
                break;
            }
        }

        if layer != 0 || !input.is_empty() {
            return Err(syn::Error::new(punct.span(), "Unable to parse Option"));
        }
        Ok(Self(Some(stream)))
    }
}

pub(crate) fn type_option(input: TokenStream) -> (TokenStream, bool) {
    let OptionTypeParseResult(result) = syn::parse2(input.clone()).unwrap();
    match result {
        None => (input, false),
        Some(result) => (result, true),
    }
}
