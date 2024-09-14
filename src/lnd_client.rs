//! LND Client

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Result};
use fedimint_tonic_lnd::{
    lnrpc::{
        ConnectPeerRequest, GetInfoRequest, GetInfoResponse, LightningAddress, ListChannelsRequest,
        NewAddressRequest, OpenChannelRequest, WalletBalanceRequest, WalletBalanceResponse,
    },
    Client,
};
use tokio::{sync::Mutex, time::sleep};

use crate::{hex, InvoiceStatus};

/// Lnd
pub struct LndClient {
    client: Arc<Mutex<Client>>,
}

impl LndClient {
    /// Create rpc client
    pub async fn new(addr: String, cert_file: PathBuf, macaroon_file: PathBuf) -> Result<Self> {
        let client = fedimint_tonic_lnd::connect(addr, cert_file, macaroon_file)
            .await
            .map_err(|_err| anyhow!("Could not connect to lnd rpc"))?;

        Ok(LndClient {
            client: Arc::new(Mutex::new(client)),
        })
    }

    /// Get node info
    pub async fn get_info(&self) -> Result<GetInfoResponse> {
        let client = &self.client;

        let get_info_request = GetInfoRequest {};

        let info = client
            .lock()
            .await
            .lightning()
            .get_info(get_info_request)
            .await?
            .into_inner();

        Ok(info)
    }

    /// Get new address
    pub async fn get_new_address(&self) -> Result<String> {
        let client = &self.client;

        let new_address_request = NewAddressRequest {
            r#type: 0,
            account: "".to_string(),
        };

        let new_address_response = client
            .lock()
            .await
            .lightning()
            .new_address(new_address_request)
            .await?
            .into_inner();

        Ok(new_address_response.address.to_string())
    }

    /// Connect to peer
    pub async fn connect(&self, pubkey: String, addr: String, port: u16) -> Result<()> {
        let client = &self.client;

        let host = format!("{}:{}", addr, port);

        let lightning_addr = LightningAddress { pubkey, host };

        let connect_peer_request = ConnectPeerRequest {
            addr: Some(lightning_addr),
            perm: false,
            timeout: 60,
        };

        let _connect_peer = client
            .lock()
            .await
            .lightning()
            .connect_peer(connect_peer_request)
            .await?
            .into_inner();

        Ok(())
    }

    /// Open channel to peer
    pub async fn open_channel(
        &self,
        amount_sat: u64,
        peer_id: &str,
        push_amount: Option<u64>,
    ) -> Result<()> {
        let client = &self.client;

        let mut open_channel_request = OpenChannelRequest::default();

        open_channel_request.node_pubkey = hex::decode(peer_id)?;
        open_channel_request.push_sat = push_amount.unwrap_or_default() as i64;
        open_channel_request.local_funding_amount = amount_sat as i64;

        let _connect_peer = client
            .lock()
            .await
            .lightning()
            .open_channel_sync(open_channel_request)
            .await?
            .into_inner();

        Ok(())
    }

    pub async fn get_balance(&self) -> Result<WalletBalanceResponse> {
        let client = &self.client;

        Ok(client
            .lock()
            .await
            .lightning()
            .wallet_balance(WalletBalanceRequest {})
            .await?
            .into_inner())
    }

    pub async fn pay_invoice(&self, bolt11: String) -> Result<String> {
        let pay_req = fedimint_tonic_lnd::lnrpc::SendRequest {
            payment_request: bolt11,
            ..Default::default()
        };

        let payment_response = self
            .client
            .lock()
            .await
            .lightning()
            .send_payment_sync(fedimint_tonic_lnd::tonic::Request::new(pay_req))
            .await?
            .into_inner();

        println!("{:?}", payment_response);

        Ok(hex::encode(payment_response.payment_preimage))
    }

    pub async fn create_invoice(&self, amount: u64) -> Result<String> {
        let invoice_request = fedimint_tonic_lnd::lnrpc::Invoice {
            value_msat: (amount * 1_000) as i64,
            ..Default::default()
        };

        let invoice = self
            .client
            .lock()
            .await
            .lightning()
            .add_invoice(fedimint_tonic_lnd::tonic::Request::new(invoice_request))
            .await
            .unwrap()
            .into_inner();

        Ok(invoice.payment_request)
    }

    pub async fn list_channels(&self) -> Result<()> {
        let channels = self
            .client
            .lock()
            .await
            .lightning()
            .list_channels(ListChannelsRequest {
                active_only: false,
                inactive_only: false,
                public_only: false,
                private_only: false,
                peer: vec![],
            })
            .await?
            .into_inner();

        for channel in channels.channels {
            println!("{:?}", channel);
        }

        Ok(())
    }

    pub async fn wait_channels_active(&self) -> Result<()> {
        let mut count = 0;
        while count < 100 {
            let pending = self
                .client
                .lock()
                .await
                .lightning()
                .list_channels(ListChannelsRequest {
                    inactive_only: true,
                    active_only: false,
                    public_only: false,
                    private_only: false,
                    peer: vec![],
                })
                .await?
                .into_inner();

            if pending.channels.is_empty() {
                println!("No pending");
                println!("{:?}", pending);
                return Ok(());
            }

            count += 1;

            sleep(Duration::from_secs(2)).await;
        }

        bail!("Timout waiting for pending")
    }

    pub async fn wait_chain_sync(&self) -> Result<()> {
        let mut count = 0;
        while count < 100 {
            let info = self.get_info().await?;

            if info.synced_to_chain {
                return Ok(());
            }
            count += 1;

            sleep(Duration::from_secs(2)).await;
        }

        bail!("Time out exceded")
    }

    pub async fn check_incoming_invoice(&self, payment_hash: String) -> Result<InvoiceStatus> {
        let invoice_request = fedimint_tonic_lnd::lnrpc::PaymentHash {
            r_hash: hex::decode(payment_hash)?,
            ..Default::default()
        };

        let invoice = self
            .client
            .lock()
            .await
            .lightning()
            .lookup_invoice(fedimint_tonic_lnd::tonic::Request::new(invoice_request))
            .await
            .unwrap()
            .into_inner();

        match invoice.state {
            // Open
            0 => Ok(InvoiceStatus::Unpaid),
            // Settled
            1 => Ok(InvoiceStatus::Paid),
            // Canceled
            2 => Ok(InvoiceStatus::Unpaid),
            // Accepted
            3 => Ok(InvoiceStatus::Unpaid),
            _ => bail!("Unkown state"),
        }
    }

    pub async fn check_outgoing_invoice(&self, payment_hash: String) -> Result<InvoiceStatus> {
        let invoice_request = fedimint_tonic_lnd::lnrpc::ListPaymentsRequest {
            include_incomplete: true,
            index_offset: 0,
            max_payments: 1000,
            reversed: false,
            count_total_payments: false,
        };

        let invoices = self
            .client
            .lock()
            .await
            .lightning()
            .list_payments(invoice_request)
            .await
            .unwrap()
            .into_inner();

        let invoice: Vec<&fedimint_tonic_lnd::lnrpc::Payment> = invoices
            .payments
            .iter()
            .filter(|p| p.payment_hash == payment_hash)
            .collect();

        if invoice.len() != 1 {
            bail!("Could not find invoice");
        }

        let invoice = invoice.first().expect("Checked len is one");

        match invoice.status {
            // Open
            0 => Ok(InvoiceStatus::Unpaid),
            // Settled
            1 => Ok(InvoiceStatus::Paid),
            // Canceled
            2 => Ok(InvoiceStatus::Unpaid),
            // Accepted
            3 => Ok(InvoiceStatus::Unpaid),
            _ => bail!("Unkown state"),
        }
    }
}
