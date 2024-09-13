//! Bitcoind

use anyhow::{bail, Result};

use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread::sleep,
    time::Duration,
};

/// Bitcoind
pub struct Bitcoind {
    rpc_user: String,
    rpc_password: String,
    addr: PathBuf,
    data_dir: PathBuf,
    child: Option<Child>,
    zmq_raw_block: String,
    zmq_raw_tx: String,
}

impl Bitcoind {
    /// Create new [`Bitcoind`]
    pub fn new(
        data_dir: PathBuf,
        addr: PathBuf,
        rpc_user: String,
        rpc_password: String,
        zmq_raw_block: String,
        zmq_raw_tx: String,
    ) -> Self {
        Bitcoind {
            rpc_user,
            rpc_password,
            addr,
            data_dir,
            child: None,
            zmq_raw_block,
            zmq_raw_tx,
        }
    }

    /// Start bitcoind
    pub fn start_bitcoind(&mut self) -> Result<()> {
        let mut cmd = Command::new("bitcoind");

        cmd.arg("-regtest");
        cmd.arg(format!("-datadir={}", self.data_dir.to_string_lossy()));
        cmd.arg("-fallbackfee=0.01");
        cmd.arg("-rpcallowip=0.0.0.0/0");
        cmd.arg(format!("-rpcuser={}", self.rpc_user));
        cmd.arg(format!("-rpcpassword={}", self.rpc_password));
        cmd.arg(format!("-zmqpubrawblock={}", self.zmq_raw_block));
        cmd.arg(format!("-zmqpubrawtx={}", self.zmq_raw_tx));

        //        cmd.arg(format!("-bind={}", self.addr.to_string_lossy()));

        // Send output to dev null
        cmd.stdout(Stdio::null());

        let child = cmd.spawn()?;

        self.child = Some(child);

        // Let bitcoind start up
        sleep(Duration::from_secs(5));

        Ok(())
    }

    /// Stop bitcoind
    pub fn stop_bitcoind(&mut self) -> Result<()> {
        let child = self.child.take();

        match child {
            Some(mut child) => {
                child.kill()?;
            }
            None => bail!("No child to kill"),
        }

        Ok(())
    }
}
