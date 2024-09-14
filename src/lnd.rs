//! LND

use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread::sleep,
    time::Duration,
};

use anyhow::{bail, Result};

/// Lnd
pub struct Lnd {
    addr: PathBuf,
    data_dir: PathBuf,
    bitcoin_data_dir: PathBuf,
    bitcoin_rpc_user: String,
    bitcoin_rpc_password: String,
    child: Option<Child>,
    zmq_raw_block: String,
    zmq_raw_tx: String,
}

impl Lnd {
    /// Create new [`Lnd`]
    pub fn new(
        bitcoin_data_dir: PathBuf,
        data_dir: PathBuf,
        addr: PathBuf,
        bitcoin_rpc_user: String,
        bitcoin_rpc_password: String,
        zmq_raw_block: String,
        zmq_raw_tx: String,
    ) -> Self {
        Self {
            data_dir,
            bitcoin_data_dir,
            addr,
            bitcoin_rpc_user,
            bitcoin_rpc_password,
            child: None,
            zmq_raw_block,
            zmq_raw_tx,
        }
    }

    /// Start lnd
    pub fn start_lnd(&mut self) -> Result<()> {
        let mut cmd = Command::new("lnd");
        cmd.arg("--bitcoin.active");
        cmd.arg("--bitcoin.regtest");
        cmd.arg("--bitcoin.node=bitcoind");
        cmd.arg(format!(
            "--bitcoind.config={}",
            self.bitcoin_data_dir
                .join("regtest/settings.json")
                .display(),
        ));
        cmd.arg(format!(
            "--bitcoind.dir={}",
            self.bitcoin_data_dir.to_string_lossy()
        ));
        cmd.arg(format!("--bitcoind.rpcuser={}", self.bitcoin_rpc_user));
        cmd.arg(format!("--bitcoind.rpcpass={}", self.bitcoin_rpc_password));
        cmd.arg(format!("--lnddir={}", self.data_dir.display()));
        cmd.arg(format!("--bitcoind.zmqpubrawblock={}", self.zmq_raw_block));
        cmd.arg(format!("--bitcoind.zmqpubrawtx={}", self.zmq_raw_tx));
        cmd.arg("--noseedbackup");

        cmd.arg(format!("--externalip={}", self.addr.to_string_lossy()));

        // Send output to dev null
        cmd.stdout(Stdio::null());

        let child = cmd.spawn()?;

        self.child = Some(child);

        // Let clnd start up
        sleep(Duration::from_secs(10));

        Ok(())
    }

    /// Stop lnd
    pub fn stop_lnd(&mut self) -> Result<()> {
        let child = self.child.take();

        match child {
            Some(mut child) => {
                child.kill()?;
            }
            None => bail!("No child to kill"),
        }

        Ok(())
    }

    pub fn create_wallet(&self, tls_cert_path: String) -> Result<()> {
        let mut cmd = Command::new("lncli create");
        cmd.arg("--lnddir");
        cmd.arg(self.data_dir.display().to_string());
        cmd.arg("--network");
        cmd.arg("regtest");
        cmd.arg("--chain");
        cmd.arg("bitcoin");
        cmd.arg("--tlscertpath");
        cmd.arg(tls_cert_path);

        let mut child = cmd.spawn()?;

        child.wait()?;

        Ok(())
    }
}

impl Drop for Lnd {
    fn drop(&mut self) {
        tracing::info!("Droping lnd");
        if let Err(err) = self.stop_lnd() {
            tracing::error!("Could not stop lnd: {}", err);
        }
    }
}
