//! CLN rpc client

use std::{path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use cln_rpc::{
    model::{
        requests::{
            ConnectRequest, FundchannelRequest, GetinfoRequest, InvoiceRequest,
            ListchannelsRequest, ListfundsRequest, ListinvoicesRequest, ListpaysRequest,
            ListtransactionsRequest, NewaddrRequest, PayRequest,
        },
        responses::{
            GetinfoResponse, ListchannelsResponse, ListfundsOutputsStatus,
            ListinvoicesInvoicesStatus, ListpaysPaysStatus,
        },
    },
    primitives::{Amount, AmountOrAll, AmountOrAny, PublicKey},
    ClnRpc,
};
use tokio::{sync::Mutex, time::sleep};

use crate::{hex, InvoiceStatus};

use super::{
    types::{Balance, ConnectInfo},
    LightningClient,
};

/// Cln
#[derive(Clone)]
pub struct ClnClient {
    client: Arc<Mutex<ClnRpc>>,
    pub rpc_path: PathBuf,
}

impl ClnClient {
    /// Create rpc client
    pub async fn new(data_dir: PathBuf, rpc_path: Option<PathBuf>) -> Result<Self> {
        let rpc_path = rpc_path.unwrap_or(data_dir.join("regtest/lightning-rpc"));

        println!("rpc_path: {}", rpc_path.display());

        let cln_client = cln_rpc::ClnRpc::new(&rpc_path).await?;

        Ok(Self {
            rpc_path,
            client: Arc::new(Mutex::new(cln_client)),
        })
    }

    /// Get node info
    pub async fn get_info(&self) -> Result<GetinfoResponse> {
        let client = &self.client;

        let get_info_request = GetinfoRequest {};

        let cln_response = client.lock().await.call(get_info_request.into()).await?;

        match cln_response {
            cln_rpc::Response::Getinfo(info_response) => Ok(info_response),
            _ => bail!("CLN returned wrong response kind"),
        }
    }

    pub async fn list_transactions(&self) -> Result<()> {
        let client = &self.client;

        let cln_response = client
            .lock()
            .await
            .call(cln_rpc::Request::ListTransactions(
                ListtransactionsRequest {},
            ))
            .await?;

        println!("{:#?}", cln_response);

        Ok(())
    }

    pub async fn list_channels(&self) -> Result<ListchannelsResponse> {
        let mut cln_client = self.client.lock().await;
        let cln_response = cln_client
            .call(cln_rpc::Request::ListChannels(ListchannelsRequest {
                destination: None,
                short_channel_id: None,
                source: None,
            }))
            .await?;

        match cln_response {
            cln_rpc::Response::ListChannels(channels) => Ok(channels),
            _ => {
                bail!("Wrong cln response");
            }
        }
    }
}

#[async_trait]
impl LightningClient for ClnClient {
    async fn get_connect_info(&self) -> Result<ConnectInfo> {
        let client = &self.client;

        let get_info_request = GetinfoRequest {};

        let cln_response = client.lock().await.call(get_info_request.into()).await?;

        let response = match cln_response {
            cln_rpc::Response::Getinfo(info_response) => info_response,
            _ => bail!("CLN returned wrong response kind"),
        };

        let address = response.binding.ok_or(anyhow!("Unknow cln address"))?;

        let address = address.first().ok_or(anyhow!("Unknown cln address"))?;

        let port = &address.port.ok_or(anyhow!("Unknown cln port"))?;

        let address = address
            .address
            .clone()
            .ok_or(anyhow!("Address not defined"))?;

        Ok(ConnectInfo {
            pubkey: response.id.to_string(),
            address,
            port: *port,
        })
    }

    async fn get_new_onchain_address(&self) -> Result<String> {
        let client = &self.client;

        let cln_response = client
            .lock()
            .await
            .call(cln_rpc::Request::NewAddr(NewaddrRequest {
                addresstype: None,
            }))
            .await?;

        let address = match cln_response {
            cln_rpc::Response::NewAddr(addr_res) => {
                &addr_res.bech32.ok_or(anyhow!("No bech32".to_string()))?
            }
            _ => bail!("CLN returned wrong response kind"),
        };

        Ok(address.to_string())
    }

    async fn connect_peer(&self, pubkey: String, addr: String, port: u16) -> Result<()> {
        let client = &self.client;

        let cln_response = client
            .lock()
            .await
            .call(cln_rpc::Request::Connect(ConnectRequest {
                id: pubkey,
                host: Some(addr),
                port: Some(port),
            }))
            .await?;

        let peer = match cln_response {
            cln_rpc::Response::Connect(connect_response) => connect_response.id,
            _ => bail!("CLN returned wrong response kind"),
        };

        tracing::debug!("CLN connected to peer: {}", peer);

        Ok(())
    }

    async fn open_channel(
        &self,
        amount_sat: u64,
        peer_id: &str,
        push_amount: Option<u64>,
    ) -> Result<()> {
        let client = &self.client;

        let cln_response = client
            .lock()
            .await
            .call(cln_rpc::Request::FundChannel(FundchannelRequest {
                amount: AmountOrAll::Amount(Amount::from_sat(amount_sat)),
                id: PublicKey::from_str(peer_id)?,
                push_msat: push_amount.map(Amount::from_sat),
                announce: None,
                close_to: None,
                compact_lease: None,
                feerate: None,
                minconf: None,
                mindepth: None,
                request_amt: None,
                reserve: None,
                channel_type: None,
                utxos: None,
            }))
            .await?;

        let channel_id = match cln_response {
            cln_rpc::Response::FundChannel(addr_res) => addr_res.channel_id,
            _ => bail!("CLN returned wrong response kind"),
        };

        tracing::info!("CLN opened channel: {}", channel_id);

        Ok(())
    }

    async fn balance(&self) -> Result<Balance> {
        let client = &self.client;

        let cln_response = client
            .lock()
            .await
            .call(cln_rpc::Request::ListFunds(ListfundsRequest {
                spent: None,
            }))
            .await?;

        let balance = match cln_response {
            cln_rpc::Response::ListFunds(funds_response) => {
                let mut on_chain_total = Amount::from_msat(0);
                let mut on_chain_spendable = Amount::from_msat(0);
                let mut ln = Amount::from_msat(0);

                for output in funds_response.outputs {
                    match output.status {
                        ListfundsOutputsStatus::UNCONFIRMED => {
                            on_chain_total = on_chain_total + output.amount_msat;
                        }
                        ListfundsOutputsStatus::IMMATURE => {
                            on_chain_total = on_chain_total + output.amount_msat;
                        }
                        ListfundsOutputsStatus::CONFIRMED => {
                            on_chain_total = on_chain_total + output.amount_msat;
                            on_chain_spendable = on_chain_spendable + output.amount_msat;
                        }
                        ListfundsOutputsStatus::SPENT => (),
                    }
                    println!("Fund: {:?}", output);
                }

                for channel in funds_response.channels {
                    ln = ln + channel.our_amount_msat;
                }

                Balance {
                    on_chain_spendable: on_chain_spendable.msat(),
                    on_chain_total: on_chain_total.msat(),
                    ln: ln.msat(),
                }
            }
            _ => {
                bail!("Wrong cln response")
            }
        };

        Ok(balance)
    }

    async fn create_invoice(&self, amount_sat: Option<u64>) -> Result<String> {
        let mut cln_client = self.client.lock().await;

        let label = uuid::Uuid::new_v4().to_string();

        //let amount_msat = AmountOrAny::Amount(Amount::from_sat(amount));

        let amount_msat = match amount_sat {
            Some(amount) => AmountOrAny::Amount(Amount::from_sat(amount)),
            None => AmountOrAny::Any,
        };

        let cln_response = cln_client
            .call(cln_rpc::Request::Invoice(InvoiceRequest {
                amount_msat,
                description: "".to_string(),
                label,
                expiry: None,
                fallbacks: None,
                preimage: None,
                cltv: None,
                deschashonly: None,
                exposeprivatechannels: None,
            }))
            .await?;

        match cln_response {
            cln_rpc::Response::Invoice(invoice_res) => Ok(invoice_res.bolt11),
            _ => {
                bail!("Wrong cln response");
            }
        }
    }

    async fn pay_invoice(&self, bolt11: String) -> Result<String> {
        let mut cln_client = self.client.lock().await;

        let cln_response = cln_client
            .call(cln_rpc::Request::Pay(PayRequest {
                bolt11,
                amount_msat: None,
                label: None,
                riskfactor: None,
                maxfeepercent: None,
                retry_for: None,
                maxdelay: None,
                exemptfee: None,
                localinvreqid: None,
                exclude: None,
                maxfee: None,
                description: None,
                partial_msat: None,
            }))
            .await?;

        let response = match cln_response {
            cln_rpc::Response::Pay(pay_response) => {
                Ok(hex::encode(pay_response.payment_preimage.to_vec()))
            }
            _ => {
                bail!("CLN returned wrong response kind");
            }
        };

        // match return_error {
        //     true => {
        //         bail!("Lighiting error");
        //     }
        //     false => response,
        // }

        response
    }

    async fn wait_chain_sync(&self) -> Result<()> {
        let mut count = 0;
        while count < 100 {
            let info = self.get_info().await?;

            if info.warning_lightningd_sync.is_none() || info.warning_bitcoind_sync.is_none() {
                tracing::info!("CLN completed chain sync");
                return Ok(());
            }
            count += 1;

            sleep(Duration::from_secs(2)).await;
        }

        bail!("Timeout waiting for pending")
    }

    async fn wait_channels_active(&self) -> Result<()> {
        let mut count = 0;
        while count < 100 {
            let mut cln_client = self.client.lock().await;
            let cln_response = cln_client
                .call(cln_rpc::Request::ListChannels(ListchannelsRequest {
                    destination: None,
                    short_channel_id: None,
                    source: None,
                }))
                .await?;

            match cln_response {
                cln_rpc::Response::ListChannels(channels) => {
                    let pending = channels
                        .channels
                        .iter()
                        .filter(|c| !c.active)
                        .collect::<Vec<_>>();

                    if pending.is_empty() {
                        tracing::info!("All CLN channels active");
                        return Ok(());
                    }

                    count += 1;

                    sleep(Duration::from_secs(2)).await;
                }

                _ => {
                    bail!("Wrong cln response");
                }
            };
        }

        bail!("Time out exceeded wait for cln channels")
    }

    async fn check_incoming_payment_status(&self, payment_hash: &str) -> Result<InvoiceStatus> {
        let mut cln_client = self.client.lock().await;

        let cln_response = cln_client
            .call(cln_rpc::Request::ListInvoices(ListinvoicesRequest {
                payment_hash: Some(payment_hash.to_string()),
                label: None,
                invstring: None,
                offer_id: None,
                index: None,
                limit: None,
                start: None,
            }))
            .await?;

        match cln_response {
            cln_rpc::Response::ListInvoices(invoice_response) => {
                match invoice_response.invoices.first() {
                    Some(invoice_response) => match invoice_response.status {
                        ListinvoicesInvoicesStatus::UNPAID => Ok(InvoiceStatus::Unpaid),
                        ListinvoicesInvoicesStatus::EXPIRED => Ok(InvoiceStatus::Expired),
                        ListinvoicesInvoicesStatus::PAID => Ok(InvoiceStatus::Paid),
                    },
                    None => {
                        bail!("Could not find invoice")
                    }
                }
            }
            _ => {
                bail!("Wrong cln response")
            }
        }
    }

    async fn check_outgoing_payment_status(&self, payment_hash: &str) -> Result<InvoiceStatus> {
        let mut cln_client = self.client.lock().await;
        let cln_response = cln_client
            .call(cln_rpc::Request::ListPays(ListpaysRequest {
                bolt11: None,
                payment_hash: Some(payment_hash.parse()?),
                status: None,
            }))
            .await?;

        let state = match cln_response {
            cln_rpc::Response::ListPays(pay_response) => {
                let pay = pay_response.pays.first();

                match pay {
                    Some(pay) => match pay.status {
                        ListpaysPaysStatus::COMPLETE => InvoiceStatus::Paid,
                        ListpaysPaysStatus::PENDING => InvoiceStatus::Pending,
                        ListpaysPaysStatus::FAILED => InvoiceStatus::Failed,
                    },
                    None => InvoiceStatus::Unpaid,
                }
            }
            _ => {
                bail!("Wrong cln response")
            }
        };

        Ok(state)
    }
}
