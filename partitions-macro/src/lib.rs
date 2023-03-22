use esp_idf_part::PartitionTable;
use litrs::Literal;
use proc_macro::TokenStream;
use proc_macro_error::{abort_call_site, proc_macro_error};
use quote::quote;

#[proc_macro_error]
#[proc_macro]
pub fn partition_offset(input: TokenStream) -> TokenStream {
    let first_token = input
        .into_iter()
        .next()
        .unwrap_or_else(|| abort_call_site!("Expected a partition name"));

    let partname = match Literal::try_from(first_token) {
        Ok(Literal::String(name)) => name.value().to_owned(),
        _ => abort_call_site!("Expected a partition name as string"),
    };

    let csv = std::fs::read_to_string("partitions.csv").unwrap();
    let table = PartitionTable::try_from_str(csv).unwrap();

    let part = table.find(&partname).expect("No partition found");
    let offset = part.offset();

    quote! {
        #offset
    }
    .into()
}
