use {
    hyper_rustls::{
        FixedServerNameResolver,
        HttpsConnectorBuilder,
    },
    hyper_util::{
        client::legacy::{
            Client,
            connect::HttpConnector,
        },
        rt::TokioExecutor,
    },
    http_body_util::Full,
    bytes::Bytes,
    rustls_pki_types::ServerName,
};

pub type Body = Full<Bytes>;
pub type HttpsConnector = hyper_rustls::HttpsConnector<HttpConnector>;
pub type HttpsClient = Client<HttpsConnector, Body>;

pub fn default_webpki_https_client() -> anyhow::Result<HttpsClient> {
    Ok(https_client(
        HttpsClientConfig {
            https_only: false,
            retry: true,
            ..webpki_root_cert_store().into()
        },
    ))
}

pub struct HttpsClientConfig {
    pub tls_config: rustls::ClientConfig,
    pub https_only: bool,
    pub http2_only: bool,
    pub retry: bool,
    pub server_name: Option<ServerName<'static>>,
}

impl std::fmt::Display for HttpsClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f, "HttpsClientConfig https_only:{} http2_only:{} server_name:{:?}",
            self.https_only, self.https_only, self.server_name,
        )
    }
}

impl From<rustls::ClientConfig> for HttpsClientConfig {
    fn from(tls_config: rustls::ClientConfig) -> Self {
        Self {
            tls_config,
            retry: true,
            https_only: true,
            http2_only: false,
            server_name: None,
        }
    }
}

impl From<rustls::RootCertStore> for HttpsClientConfig {
    fn from(value: rustls::RootCertStore) -> Self {
        rustls::ClientConfig::builder()
            .with_root_certificates(value)
            .with_no_client_auth()
            .into()
    }
}

pub fn webpki_root_cert_store() -> rustls::RootCertStore {
    rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.iter().cloned().collect(),
    }
}

pub fn https_client(
    config: impl Into<HttpsClientConfig>,
) -> HttpsClient {
    let config = config.into();
    tracing::info!(
        config = tracing::field::display(&config),
        "https client config",
    );

    let HttpsClientConfig {
        tls_config,
        https_only,
        http2_only,
        server_name,
        retry,
    } = config.into();

    let conn = HttpsConnectorBuilder::new()
        .with_tls_config(tls_config);

    let conn = if https_only {
        conn.https_only()
    } else {
        conn.https_or_http()
    };

    let conn = if let Some(server_name) = server_name {
        let resolver = FixedServerNameResolver::new(server_name);
        conn.with_server_name_resolver(resolver)
    } else {
        conn
    };

    let conn = if http2_only {
        conn.enable_http2()
    } else {
        conn.enable_all_versions()
    };

    let conn = conn.build();

    Client::builder(TokioExecutor::new())
        .http2_only(http2_only)
        .retry_canceled_requests(retry)
        .build(conn)
}
