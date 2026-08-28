use std::collections::BTreeMap;

struct Invoice {
    subtotal_pence: u64,
    tax_pence: u64,
}

enum InvoiceState {
    Draft,
    Issued,
}

trait TotalPence {
    fn total_pence(&self) -> u64;
}

impl TotalPence for Invoice {
    fn total_pence(&self) -> u64 {
        self.subtotal_pence + self.tax_pence
    }
}

type InvoiceIndex = BTreeMap<u64, InvoiceState>;
