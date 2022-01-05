use clap::{ArgEnum, Parser};
use color_eyre::eyre::ContextCompat;
use core_extensions::ToTime;
use openssh::Session;
use std::{net::SocketAddr, path::PathBuf, process::Stdio, sync::atomic::AtomicBool};
use tokio::io::AsyncWriteExt;
use tracing::Instrument;

static CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(ArgEnum, Clone, Copy, PartialEq, Eq, Debug)]
#[clap(rename_all = "snake_case")]
enum L4Mode {
    Udp,
    Tcp,
}

impl L4Mode {
    fn name(&self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::Tcp => "tcp",
        }
    }
}

fn do_socketaddr(s: &str) -> Result<SocketAddr, std::io::Error> {
    use std::net::ToSocketAddrs;

    Ok(s.to_socket_addrs()?.next().unwrap())
}

/// Set up a packet redirect on some remote host that forwards packets to you
///
/// example: `easy_expose test_redir tcp root@vps 9912 100.82.95.116:9912`
#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Params {
    /// A unique name to identify this forwarding instance
    identifier: String,

    /// What type of packet to forward
    #[clap(arg_enum)]
    mode: L4Mode,

    /// The remote host to expose on [format: a ssh destination]
    ///
    /// NFTables needs to be installed on this host, the user to connect as also
    /// needs permission to run `nft` (aka: root)
    destination: String,

    /// The ssh identity file to use
    #[clap(short, long, parse(from_os_str), value_name = "FILE")]
    identity: Option<PathBuf>,

    /// The remote port to expose on
    //#[clap(short, long)]
    remote: u16,

    /// Where to forward packets to [format: <ip/hostname>:<port>]
    #[clap(parse(try_from_str = do_socketaddr))]
    local: SocketAddr,
}

async fn open_ssh(p: &Params) -> color_eyre::Result<Session> {
    use openssh::SessionBuilder;

    let mut s = SessionBuilder::default();
    s.known_hosts_check(openssh::KnownHosts::Accept);

    if let Some(f) = p.identity.as_deref() {
        s.keyfile(f);
    }

    let span = tracing::info_span!("Connecting to destination", destination = %p.destination);
    Ok(s.connect(&p.destination).instrument(span).await?)
}

fn nft_rule(p: &Params) -> String {
    format!(
        r#"
table ip {identifier}
delete table {identifier}
table ip {identifier} {{
        chain prerouting {{
                type nat hook prerouting priority dstnat; policy accept;
                {mode} dport {remote_port} dnat to {local}
        }}

        chain postrouting {{
                type nat hook postrouting priority srcnat; policy accept;
                masquerade
        }}
}}
"#,
        identifier = p.identifier,
        mode = p.mode.name(),
        remote_port = p.remote,
        local = p.local
    )
}

async fn setup_redirect(p: &Params, s: &Session) -> color_eyre::Result<()> {
    let rule = nft_rule(p);

    let span = tracing::info_span!("installing rule", %rule);

    let mut r = s
        .command("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let stdin = r
        .stdin()
        .as_mut()
        .wrap_err("Didn't get a stdin for some reason")?;
    stdin.write_all(rule.as_bytes()).await?;
    stdin.shutdown().await?;

    let out = r.wait_with_output().instrument(span).await?;

    if !out.status.success() {
        return Err(color_eyre::eyre::eyre!(
            "Installing redirect failed: {}",
            std::str::from_utf8(&out.stderr)?
        ));
    }

    Ok(())
}

async fn check_rule(p: &Params, s: &Session) -> color_eyre::Result<()> {
    let span = tracing::debug_span!("Checking rule", rule = %p.identifier);

    let exists = s
        .command("nft")
        .args(["list", "table"])
        .arg(&p.identifier)
        .status()
        .instrument(span)
        .await?
        .success();

    if !exists {
        return Err(color_eyre::eyre::eyre!("Rule got dropped for some reason"));
    }

    Ok(())
}

async fn delete_rule(p: &Params, s: &Session) -> color_eyre::Result<()> {
    let span = tracing::info_span!("Deleting rule", rule = %p.identifier);

    s.command("nft")
        .args(["delete", "table"])
        .arg(&p.identifier)
        .status()
        .instrument(span)
        .await?;

    Ok(())
}

async fn wait_for_quit() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigint = signal(SignalKind::terminate()).unwrap();
    let mut sigterm = signal(SignalKind::interrupt()).unwrap();

    tokio::select! {
        _ = sigint.recv() => {
            CANCELLED.store(true, std::sync::atomic::Ordering::SeqCst);
        }

        _ = sigterm.recv() => {
            CANCELLED.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

async fn inner(p: &Params) -> color_eyre::Result<()> {
    let inner = || async {
        let s = open_ssh(p).await?;

        setup_redirect(p, &s).await?;

        loop {
            tokio::time::sleep(1.minutes()).await;
            check_rule(p, &s).await?;
        }
    };

    tokio::select! {
        r = inner() => {
            return r;
        }

        _ = wait_for_quit() => {}
    };

    let s = open_ssh(p).await?;
    // if we get here we need to clean up
    delete_rule(p, &s).await?;

    Ok(())
}

async fn main_loop(p: &Params) {
    loop {
        if let Err(e) = inner(p).await {
            tracing::error!(reason = ?e, "Something broke, retrying in 10 seconds");

            if CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }

            let delay = tokio::time::sleep(10.seconds());
            tokio::pin!(delay);

            tokio::select! {
                _ = &mut delay => {}

                _ = wait_for_quit() => {
                    return;
                }
            }
        } else {
            return;
        }
    }
}

fn install_tracing() -> color_eyre::Result<()> {
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let fmt_layer = tracing_subscriber::fmt::layer().with_span_events(FmtSpan::CLOSE);
    // .pretty();
    let filter_layer = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("easy_expose=debug".parse()?);

    tracing_subscriber::registry()
        .with(tracing_error::ErrorLayer::default())
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    Ok(())
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let params = Params::parse();

    install_tracing()?;

    color_eyre::install()?;

    main_loop(&params).await;

    Ok(())
}
