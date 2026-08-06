use proc_macro::TokenStream;

#[proc_macro]
pub fn acceptance_marker(_input: TokenStream) -> TokenStream {
    "pub const PROC_MACRO_RAN: bool = true;".parse().expect("fixed token stream")
}
