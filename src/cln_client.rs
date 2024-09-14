//! CLN rpc client

use std::{path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Result};
use cln_rpc::{
    model::{
        requests::{
            ConnectRequest, FundchannelRequest, GetinfoRequest, InvoiceRequest,
            ListchannelsRequest, ListfundsRequest, ListtransactionsRequest, NewaddrRequest,
            PayRequest,
        },
        responses::{GetinfoResponse, ListchannelsResponse, ListfundsOutputsStatus},
    },
    primitives::{Amount, AmountOrAll, AmountOrAny, PublicKey},
    ClnRpc,
};
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, time::sleep};

use crate::hex;

/// Cln
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

    /// Get new address
    pub async fn get_new_address(&self) -> Result<String> {
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

    /// Connect to peer
    pub async fn connect(&self, pubkey: String, addr: String, port: u16) -> Result<()> {
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

        let _peers = match cln_response {
            cln_rpc::Response::Connect(connect_response) => connect_response.id,
            _ => bail!("CLN returned wrong response kind"),
        };

        Ok(())
    }

    /// Open channel to peer
    pub async fn open_channel(
        &self,
        amount_sat: u64,
        peer_id: &str,
        push_amount: Option<u64>,
    ) -> Result<String> {
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

        Ok(channel_id.to_string())
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

    pub async fn get_balance(&self) -> Result<BalanceResponse> {
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

                BalanceResponse {
                    on_chain_spendable: Amount::from_msat(on_chain_spendable.msat()),
                    on_chain_total: Amount::from_msat(on_chain_total.msat()),
                    ln: Amount::from_msat(ln.msat()),
                }
            }
            _ => {
                bail!("Wrong cln response")
            }
        };

        Ok(balance)
    }

    pub async fn create_invoice(&self, amount: u64) -> Result<String> {
        let mut cln_client = self.client.lock().await;

        let label = uuid::Uuid::new_v4().to_string();

        let amount_msat = AmountOrAny::Amount(Amount::from_sat(amount));

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

    pub async fn pay_invoice(&self, bolt11: String) -> Result<String> {
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

        match cln_response {
            cln_rpc::Response::Pay(pay_response) => {
                Ok(hex::encode(pay_response.payment_preimage.to_vec()))
            }
            _ => {
                bail!("CLN returned wrong response kind");
            }
        }
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

    pub async fn wait_chain_sync(&self) -> Result<()> {
        let mut count = 0;
        while count < 100 {
            let info = self.get_info().await?;

            if info.warning_lightningd_sync.is_none() || info.warning_bitcoind_sync.is_none() {
                return Ok(());
            }
            count += 1;

            sleep(Duration::from_secs(2)).await;
        }

        bail!("Timout waiting for pending")
    }

    pub async fn wait_channles_active(&self) -> Result<()> {
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

        bail!("Time out exceded")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceResponse {
    pub on_chain_spendable: Amount,
    pub on_chain_total: Amount,
    pub ln: Amount,
}
