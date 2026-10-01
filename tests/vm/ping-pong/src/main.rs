use anyhow::{Context, bail};
use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl,
    address_lookup::{PkarrPublisher, PkarrResolver},
    endpoint::{ConnectionError, presets::N0},
};
use std::{env, time::Duration};
use url::Url;

const ALPN: &[u8] = b"iron/vm-ping-pong/0";
const DEADLINE: Duration = Duration::from_secs(90);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    let role = args
        .next()
        .context("usage: iron-vm-ping-pong server|client [peer-id]")?;
    let relay: RelayUrl = args.next().context("missing relay URL")?.parse()?;
    let pkarr: Url = args.next().context("missing pkarr URL")?.parse()?;
    let peer = args.next().map(|id| id.parse()).transpose()?;
    if args.next().is_some() {
        bail!("unexpected arguments");
    }

    let endpoint = Endpoint::builder(N0)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Custom(RelayMap::from(relay)))
        .clear_address_lookup()
        .address_lookup(PkarrPublisher::builder(pkarr.clone()))
        .address_lookup(PkarrResolver::builder(pkarr))
        .bind()
        .await?;
    tokio::time::timeout(DEADLINE, endpoint.online())
        .await
        .context("timed out waiting for endpoint to come online")?;
    println!("ENDPOINT_ID={}", endpoint.id());

    match role.as_str() {
        "server" if peer.is_none() => {
            let incoming = tokio::time::timeout(DEADLINE, endpoint.accept())
                .await
                .context("timed out accepting connection")?
                .context("endpoint closed")?;
            let accepting = incoming.accept()?;
            let connection = tokio::time::timeout(DEADLINE, accepting)
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
                    if reason.error_code == 0u8.into()
                        && reason.reason.as_ref() == b"pong received" => {}
                other => bail!("client did not complete ping-pong: {other}"),
            }
            println!("PONG_SENT");
        }
        "client" if peer.is_some() => {
            let peer = peer.context("client requires an endpoint id")?;
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
        }
        _ => bail!("use server RELAY_URL PKARR_URL or client RELAY_URL PKARR_URL ENDPOINT_ID"),
    }
    endpoint.close().await;
    Ok(())
}
