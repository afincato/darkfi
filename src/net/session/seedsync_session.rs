use std::time::Duration;

use async_std::{
    future::timeout,
    sync::{Arc, Weak},
};
use async_trait::async_trait;
use futures::future::join_all;
use log::*;
use serde_json::json;
use smol::Executor;
use url::Url;

use crate::Result;

use super::{
    super::{Connector, P2p},
    Session, SessionBitflag, SESSION_SEED,
};

/// Defines seed connections session.
pub struct SeedSyncSession {
    p2p: Weak<P2p>,
}

impl SeedSyncSession {
    /// Create a new seed sync session instance.
    pub fn new(p2p: Weak<P2p>) -> Arc<Self> {
        Arc::new(Self { p2p })
    }

    /// Start the seed sync session. Creates a new task for every seed connection and
    /// starts the seed on each task.
    pub async fn start(self: Arc<Self>, executor: Arc<Executor<'_>>) -> Result<()> {
        debug!(target: "net", "SeedSyncSession::start() [START]");
        let settings = self.p2p().settings();

        if settings.seeds.is_empty() {
            warn!("Skipping seed sync process since no seeds are configured.");
            // Store external addresses in hosts explicitly
            if !settings.external_addr.is_empty() {
                self.p2p().hosts().store(settings.external_addr.clone()).await
            }

            return Ok(())
        }

        // if cached addresses then quit

        let mut tasks = Vec::new();

        // This loops through all the seeds and tries to start them.
        // If the seed_query_timeout_seconds times out before they are finished,
        // it will return an error.
        for (i, seed) in settings.seeds.iter().enumerate() {
            let ex2 = executor.clone();
            let self2 = self.clone();
            let sett2 = settings.clone();
            tasks.push(async move {
                let task = self2.clone().start_seed(i, seed.clone(), ex2.clone());

                let result =
                    timeout(Duration::from_secs(sett2.seed_query_timeout_seconds.into()), task)
                        .await;

                match result {
                    Ok(t) => match t {
                        Ok(()) => {
                            info!("Seed #{} connected successfully", i)
                        }
                        Err(err) => {
                            warn!("Seed #{} failed for reason {}", i, err)
                        }
                    },
                    Err(_err) => error!("Seed #{} timed out", i),
                }
            });
        }
        join_all(tasks).await;

        // Seed process complete
        if self.p2p().hosts().is_empty().await {
            warn!("Hosts pool still empty after seeding");
        }

        debug!(target: "net", "SeedSyncSession::start() [END]");
        Ok(())
    }

    /// Connects to a seed socket address.
    async fn start_seed(
        self: Arc<Self>,
        seed_index: usize,
        seed: Url,
        executor: Arc<Executor<'_>>,
    ) -> Result<()> {
        debug!(target: "net", "SeedSyncSession::start_seed(i={}) [START]", seed_index);
        let (_hosts, settings) = {
            let p2p = self.p2p.upgrade().unwrap();
            (p2p.hosts(), p2p.settings())
        };

        let parent = Arc::downgrade(&self);
        let connector = Connector::new(settings.clone(), Arc::new(parent));
        match connector.connect(seed.clone()).await {
            Ok(channel) => {
                // Blacklist goes here

                info!("Connected seed #{} [{}]", seed_index, seed);

                if let Err(err) =
                    self.clone().register_channel(channel.clone(), executor.clone()).await
                {
                    warn!("Failure during seed sync session #{} [{}]: {}", seed_index, seed, err);
                }

                info!("Disconnecting from seed #{} [{}]", seed_index, seed);
                channel.stop().await;

                debug!(target: "net", "SeedSyncSession::start_seed(i={}) [END]", seed_index);
                Ok(())
            }
            Err(err) => {
                warn!("Failure contacting seed #{} [{}]: {}", seed_index, seed, err);
                Err(err)
            }
        }
    }

    // Starts keep-alive messages and seed protocol.
    /*async fn attach_protocols(
      self: Arc<Self>,
      channel: ChannelPtr,
      hosts: HostsPtr,
      settings: SettingsPtr,
      executor: Arc<Executor<'_>>,
      ) -> Result<()> {
      let protocol_ping = ProtocolPing::new(channel.clone(), self.p2p());
      protocol_ping.start(executor.clone()).await;

      let protocol_seed = ProtocolSeed::new(channel.clone(), hosts, settings.clone());
    // This will block until seed process is complete
    protocol_seed.start(executor.clone()).await?;

    channel.stop().await;

    Ok(())
    }*/
}

#[async_trait]
impl Session for SeedSyncSession {
    async fn get_info(&self) -> serde_json::Value {
        json!({
            "key": 110
        })
    }

    fn p2p(&self) -> Arc<P2p> {
        self.p2p.upgrade().unwrap()
    }

    fn type_id(&self) -> SessionBitflag {
        SESSION_SEED
    }
}
