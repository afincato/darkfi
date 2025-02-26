use std::{collections::HashMap, fs::File, net::SocketAddr};

use async_std::{net::TcpListener, sync::Arc};
use futures::{io::BufReader, AsyncRead, AsyncReadExt, AsyncWrite};
use futures_rustls::{rustls, TlsAcceptor};
use log::{error, info};

use darkfi::{system::SubscriberPtr, util::path::expand_path, Error, Result};

use crate::{
    privmsg::PrivMsgEvent,
    settings::{Args, ChannelInfo, ContactInfo},
};

mod client;

pub use client::IrcClient;

#[derive(Clone)]
pub struct IrcConfig {
    // init bool
    pub is_nick_init: bool,
    pub is_user_init: bool,
    pub is_registered: bool,
    pub is_cap_end: bool,
    pub is_pass_init: bool,

    // user config
    pub nickname: String,
    pub password: String,
    pub private_key: Option<String>,
    pub capabilities: HashMap<String, bool>,

    // channels and contacts
    pub channels: HashMap<String, ChannelInfo>,
    pub contacts: HashMap<String, ContactInfo>,
}

impl IrcConfig {
    pub fn new(settings: &Args) -> Result<Self> {
        let password = settings.password.as_ref().unwrap_or(&String::new()).clone();
        let private_key = settings.private_key.clone();

        let mut channels = settings.channels.clone();

        for chan in settings.autojoin.iter() {
            if !channels.contains_key(chan) {
                channels.insert(chan.clone(), ChannelInfo::new());
            }
        }

        let contacts = settings.contacts.clone();

        let mut capabilities = HashMap::new();
        capabilities.insert("no-history".to_string(), false);

        Ok(Self {
            is_nick_init: false,
            is_user_init: false,
            is_registered: false,
            is_cap_end: true,
            is_pass_init: false,
            nickname: "anon".to_string(),
            password,
            channels,
            contacts,
            private_key,
            capabilities,
        })
    }
}

#[derive(Clone)]
pub enum ClientSubMsg {
    Privmsg(PrivMsgEvent),
    Config(IrcConfig),
}
#[derive(Clone)]
pub enum NotifierMsg {
    Privmsg(PrivMsgEvent),
    UpdateConfig,
}

pub struct IrcServer {
    settings: Args,
    clients_subscriptions: SubscriberPtr<ClientSubMsg>,
}

impl IrcServer {
    pub async fn new(
        settings: Args,
        clients_subscriptions: SubscriberPtr<ClientSubMsg>,
    ) -> Result<Self> {
        Ok(Self { settings, clients_subscriptions })
    }
    pub async fn start(&self, executor: Arc<smol::Executor<'_>>) -> Result<()> {
        let (msg_notifier, msg_recv) = smol::channel::unbounded();

        // Listen to msgs from clients
        executor.spawn(Self::listen_to_msgs(msg_recv, self.clients_subscriptions.clone())).detach();

        // Start listening for new connections
        self.listen(msg_notifier, executor.clone()).await?;

        Ok(())
    }

    /// Start listening to msgs from irc clients
    pub async fn listen_to_msgs(
        recv: smol::channel::Receiver<(NotifierMsg, u64)>,
        clients_subscriptions: SubscriberPtr<ClientSubMsg>,
    ) -> Result<()> {
        loop {
            let (msg, subscription_id) = recv.recv().await?;

            match msg {
                NotifierMsg::Privmsg(msg) => {
                    // TODO Add to View to prevent duplicate msg, since a client may has already added the
                    // msg to its buffer

                    // Since this will be added to the View directly, other clients connected to irc
                    // server must get informed about this new msg
                    clients_subscriptions
                        .notify_with_exclude(ClientSubMsg::Privmsg(msg), &[subscription_id])
                        .await;

                    // TODO broadcast to the p2p network
                }

                NotifierMsg::UpdateConfig => {
                    //
                    // load and parse the new settings from configuration file and pass it to all
                    // irc clients
                    //
                    // let new_config = IrcConfig::new()?;
                    // clients_subscriptions.notify(ClientSubMsg::Config(new_config)).await;
                }
            }
        }
    }

    /// Start listening to new connections from irc clients
    pub async fn listen(
        &self,
        notifier: smol::channel::Sender<(NotifierMsg, u64)>,
        executor: Arc<smol::Executor<'_>>,
    ) -> Result<()> {
        let (listener, acceptor) = self.setup_listener().await?;
        info!("[IRC SERVER] listening on {}", self.settings.irc_listen);

        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok((s, a)) => (s, a),
                Err(e) => {
                    error!("[IRC SERVER] Failed accepting new connections: {}", e);
                    continue
                }
            };

            let result = if let Some(acceptor) = acceptor.clone() {
                // TLS connection
                let stream = match acceptor.accept(stream).await {
                    Ok(s) => s,
                    Err(e) => {
                        error!("[IRC SERVER] Failed accepting TLS connection: {}", e);
                        continue
                    }
                };
                self.process_connection(stream, peer_addr, notifier.clone(), executor.clone()).await
            } else {
                // TCP connection
                self.process_connection(stream, peer_addr, notifier.clone(), executor.clone()).await
            };

            if let Err(e) = result {
                error!("[IRC SERVER] Failed processing connection {}: {}", peer_addr, e);
                continue
            };

            info!("[IRC SERVER] Accept new connection: {}", peer_addr);
        }
    }

    /// On every new connection create new IrcClient
    async fn process_connection<C: AsyncRead + AsyncWrite + Send + Unpin + 'static>(
        &self,
        stream: C,
        peer_addr: SocketAddr,
        notifier: smol::channel::Sender<(NotifierMsg, u64)>,
        executor: Arc<smol::Executor<'_>>,
    ) -> Result<()> {
        let (reader, writer) = stream.split();
        let reader = BufReader::new(reader);

        // Subscription for the new client
        let client_subscription = self.clients_subscriptions.clone().subscribe().await;

        // new irc configuration
        let irc_config = IrcConfig::new(&self.settings)?;

        // New irc client
        let mut client =
            IrcClient::new(writer, reader, peer_addr, irc_config, notifier, client_subscription);

        // Start listening and detach
        executor
            .spawn(async move {
                client.listen().await;
            })
            .detach();

        Ok(())
    }

    /// Setup a listener for irc server
    async fn setup_listener(&self) -> Result<(TcpListener, Option<TlsAcceptor>)> {
        let listenaddr = self.settings.irc_listen.socket_addrs(|| None)?[0];
        let listener = TcpListener::bind(listenaddr).await?;

        let acceptor = match self.settings.irc_listen.scheme() {
            "tls" => {
                // openssl genpkey -algorithm ED25519 > example.com.key
                // openssl req -new -out example.com.csr -key example.com.key
                // openssl x509 -req -days 700 -in example.com.csr -signkey example.com.key -out example.com.crt

                if self.settings.irc_tls_secret.is_none() || self.settings.irc_tls_cert.is_none() {
                    error!("[IRC SERVER] To listen using TLS, please set irc_tls_secret and irc_tls_cert in your config file.");
                    return Err(Error::KeypairPathNotFound)
                }

                let file =
                    File::open(expand_path(self.settings.irc_tls_secret.as_ref().unwrap())?)?;
                let mut reader = std::io::BufReader::new(file);
                let secret = &rustls_pemfile::pkcs8_private_keys(&mut reader)?[0];
                let secret = rustls::PrivateKey(secret.clone());

                let file = File::open(expand_path(self.settings.irc_tls_cert.as_ref().unwrap())?)?;
                let mut reader = std::io::BufReader::new(file);
                let certificate = &rustls_pemfile::certs(&mut reader)?[0];
                let certificate = rustls::Certificate(certificate.clone());

                let config = rustls::ServerConfig::builder()
                    .with_safe_defaults()
                    .with_no_client_auth()
                    .with_single_cert(vec![certificate], secret)?;

                let acceptor = TlsAcceptor::from(Arc::new(config));
                Some(acceptor)
            }
            _ => None,
        };
        Ok((listener, acceptor))
    }
}
