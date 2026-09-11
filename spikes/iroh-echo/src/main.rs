//! Spike 0.3 — can two machines reach each other knowing only a public key?
//!
//! ```text
//! iroh-echo listen                      # prints this endpoint's id, echoes every stream
//! iroh-echo dial <ENDPOINT_ID> [COUNT]  # pings COUNT times (default 20), prints RTT + path type
//! ```
//!
//! Test matrix (docs/PLAN.md step 0.3):
//! 1. Same LAN, internet unplugged on both PCs  -> must connect via mDNS, path "direct".
//! 2. Different networks (home Wi-Fi <-> phone hotspot) -> connects "direct" (hole punched) or "relay".
//!
//! `RUST_LOG=iroh=debug` shows discovery and hole-punching details.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointId,
    endpoint::{Connection, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_mdns_address_lookup::MdnsAddressLookup;

const ALPN: &[u8] = b"cowatcher/spike-echo/0";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["listen"] => listen().await,
        ["dial", id] => dial(id, 20).await,
        ["dial", id, count] => dial(id, count.parse().context("COUNT must be a number")?).await,
        _ => bail!("usage: iroh-echo listen | iroh-echo dial <ENDPOINT_ID> [COUNT]"),
    }
}

/// N0 preset (public relays + DNS lookup) plus mDNS so LAN peers are found without internet.
async fn bind() -> Result<Endpoint> {
    Endpoint::builder(presets::N0)
        .address_lookup(MdnsAddressLookup::builder())
        .bind()
        .await
        .context("bind endpoint")
}

async fn listen() -> Result<()> {
    let endpoint = bind().await?;
    println!("endpoint id: {}", endpoint.id());
    println!("on the other PC run:  iroh-echo dial {}", endpoint.id());
    let router = Router::builder(endpoint).accept(ALPN, Echo).spawn();
    tokio::signal::ctrl_c().await?;
    router.shutdown().await.context("shutdown")?;
    Ok(())
}

async fn dial(id: &str, count: u32) -> Result<()> {
    let remote: EndpointId = id.parse().context("invalid endpoint id")?;
    let endpoint = bind().await?;

    let started = Instant::now();
    let conn = endpoint.connect(remote, ALPN).await.context("connect")?;
    println!("connected in {:?}", started.elapsed());
    print_paths(&conn);

    let (mut send, mut recv) = conn.open_bi().await.context("open stream")?;
    let mut rtts = Vec::with_capacity(count as usize);
    for seq in 0..u64::from(count) {
        let sent = Instant::now();
        send.write_all(&seq.to_le_bytes()).await.context("write")?;
        let mut echo = [0u8; 8];
        recv.read_exact(&mut echo).await.context("read")?;
        if u64::from_le_bytes(echo) != seq {
            bail!("echo mismatch at {seq}");
        }
        rtts.push(sent.elapsed());
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    rtts.sort();
    println!(
        "{count} pings: min {:?}  median {:?}  max {:?}",
        rtts[0],
        rtts[rtts.len() / 2],
        rtts[rtts.len() - 1]
    );
    // Hole punching can upgrade a relayed connection to direct while pinging.
    print_paths(&conn);

    send.finish().context("finish")?;
    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    Ok(())
}

fn print_paths(conn: &Connection) {
    for path in conn.paths().iter() {
        let kind = if path.is_relay() { "relay" } else { "direct" };
        let selected = if path.is_selected() { " (selected)" } else { "" };
        println!("  path {kind}{selected}: {:?} rtt {:?}", path.remote_addr(), path.rtt());
    }
}

#[derive(Debug, Clone)]
struct Echo;

impl ProtocolHandler for Echo {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        println!("accepted {}", connection.remote_id());
        let (mut send, mut recv) = connection.accept_bi().await?;
        let bytes = tokio::io::copy(&mut recv, &mut send).await?;
        send.finish()?;
        println!("echoed {bytes} bytes");
        connection.closed().await;
        Ok(())
    }
}
