use proc_macro2::TokenStream;

pub(crate) fn ts(crate_prefix: TokenStream, attr: TokenStream, input: TokenStream) -> TokenStream {
    #[cfg(feature = "ts")]
    {
        use proc_macro2::Span;
        use quote::quote;
        use syn::LitStr;

        let extra = if attr.is_empty() {
            TokenStream::new()
        } else {
            quote! {
                #[ts( #attr )]
            }
        };

        let crate_literal = LitStr::new(&format!("{}ts_rs", crate_prefix), Span::call_site());
        quote! {
            #[derive(#crate_prefix ts_rs::TS)]
            #[ts(crate = #crate_literal)]
            #extra
            #input
        }
    }
    #[cfg(not(feature = "ts"))]
    {
        filter_stream(&crate_prefix, &attr, input)
    }
}

#[cfg(not(feature = "ts"))]
fn filter_stream(
    _crate_prefix: &TokenStream,
    _attr: &TokenStream,
    input: TokenStream,
) -> TokenStream {
    use proc_macro2::{Group, TokenTree};

    let mut iter = input.into_iter();
    let mut out = Vec::new();
    while let Some(token) = iter.next() {
        if let TokenTree::Group(group) = token {
            out.push(TokenTree::Group(Group::new(
                group.delimiter(),
                filter_stream(_crate_prefix, _attr, group.stream()),
            )));
            continue;
        }
        if let TokenTree::Punct(p) = &token {
            if p.as_char() == '#' {
                let next = iter.next();
                if let Some(next) = next {
                    if let TokenTree::Group(g) = &next {
                        if let Some(TokenTree::Ident(i)) = g.stream().into_iter().next() {
                            if i.to_string().eq("ts") {
                                continue;
                            }
                        }
                        out.push(token);
                        out.push(next);
                    } else {
                        out.push(token);
                        out.push(next);
                    }
                    continue;
                }
            }
        }
        out.push(token);
    }
    TokenStream::from_iter(out)
}
