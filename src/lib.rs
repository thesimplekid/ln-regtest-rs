pub mod bitcoin_client;
pub mod bitcoind;
pub mod cln;
pub mod cln_client;
pub mod hex;
pub mod lnd;
pub mod lnd_client;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum InvoiceStatus {
    Paid,
    Pending,
    Unpaid,
    Expired,
    Failed,
}
