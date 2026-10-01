use anyhow::{Context, bail};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, RelayUrl,
    address_lookup::{PkarrPublisher, PkarrResolver},
    endpoint::{ConnectionError, presets::N0},
};
use std::{env, time::Duration};
use url::Url;

const ALPN: &[u8] = b"iron/vm-ping-pong/0";
const DEADLINE: Duration = Duration::from_secs(90);
const USAGE: &str = "usage:
  iron-vm-ping-pong server RELAY_URL PKARR_URL
  iron-vm-ping-pong client RELAY_URL PKARR_URL ENDPOINT_ID";

enum Role {
    Server,
    Client(EndpointId),
}

fn parse_args() -> anyhow::Result<(Role, RelayUrl, Url)> {
    let args: Vec<_> = env::args().skip(1).collect();
    let role = match args.as_slice() {
        [role, _, _] if role == "server" => Role::Server,
        [role, _, _, peer] if role == "client" => Role::Client(
            peer.parse()
                .with_context(|| format!("invalid endpoint ID; {USAGE}"))?,
        ),
        _ => bail!("{USAGE}"),
    };
    let relay = args[1]
        .parse()
        .with_context(|| format!("invalid relay URL; {USAGE}"))?;
    let pkarr = args[2]
        .parse()
        .with_context(|| format!("invalid pkarr URL; {USAGE}"))?;
    Ok((role, relay, pkarr))
}

async fn server(endpoint: &Endpoint) -> anyhow::Result<()> {
    let incoming = tokio::time::timeout(DEADLINE, endpoint.accept())
        .await
        .context("timed out accepting connection")?
        .context("endpoint closed")?;
    let connection = tokio::time::timeout(DEADLINE, incoming.accept()?)
        .await
        .context("timed out establishing connection")??;
    let (mut send, mut recv) = tokio::time::timeout(DEADLINE, connection.accept_bi())
        .await
        .context("timed out waiting for ping stream")??;
    let ping = tokio::time::timeout(DEADLINE, recv.read_to_end(64))
        .await
        .context("timed out reading ping")??;
    if ping != b"ping" {
        bail!("unexpected request: {ping:?}");
    }
    tokio::time::timeout(DEADLINE, send.write_all(b"pong"))
        .await
        .context("timed out writing pong")??;
    send.finish()?;
    // The receiver closes only after it has read and checked the full pong.
    // Waiting for that close keeps this endpoint alive while the reply is read.
    let close = tokio::time::timeout(DEADLINE, connection.closed())
        .await
        .context("timed out waiting for client to receive pong")?;
    match close {
        ConnectionError::ApplicationClosed(ref reason)
            if reason.error_code == 0u8.into() && reason.reason.as_ref() == b"pong received" => {}
        other => bail!("client did not complete ping-pong: {other}"),
    }
    println!("PONG_SENT");
    Ok(())
}

async fn client(endpoint: &Endpoint, peer: EndpointId) -> anyhow::Result<()> {
    let connection =
        tokio::time::timeout(DEADLINE, endpoint.connect(EndpointAddr::new(peer), ALPN))
            .await
            .context("timed out connecting to peer")??;
    let (mut send, mut recv) = tokio::time::timeout(DEADLINE, connection.open_bi())
        .await
        .context("timed out opening ping stream")??;
    tokio::time::timeout(DEADLINE, send.write_all(b"ping"))
        .await
        .context("timed out writing ping")??;
    send.finish()?;
    let reply = tokio::time::timeout(DEADLINE, recv.read_to_end(64))
        .await
        .context("timed out waiting for pong")??;
    if reply != b"pong" {
        bail!("unexpected response: {reply:?}");
    }
    // QUIC's close is terminal; only the side that received the last
    // application message can initiate it without truncating that message.
    connection.close(0u8.into(), b"pong received");
    println!("PONG_RECEIVED");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (role, relay, pkarr) = parse_args()?;
    let endpoint = Endpoint::builder(N0)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Custom(RelayMap::from(relay)))
        .clear_address_lookup()
        .address_lookup(PkarrPublisher::builder(pkarr.clone()))
        .address_lookup(PkarrResolver::builder(pkarr))
        .bind()
        .await?;
    let result = async {
        tokio::time::timeout(DEADLINE, endpoint.online())
            .await
            .context("timed out waiting for endpoint to come online")?;
        println!("ENDPOINT_ID={}", endpoint.id());
        match role {
            Role::Server => server(&endpoint).await,
            Role::Client(peer) => client(&endpoint, peer).await,
        }
    }
    .await;
    endpoint.close().await;
    result
}
