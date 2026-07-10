use core::{net::Ipv4Addr, str::FromStr};

use embassy_executor::Spawner;
use embassy_net::{
    IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4, tcp::TcpSocket,
};
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use esp_hal::rng::Rng;
use esp_radio::wifi::{Config, Interface, WifiController, ap::AccessPointConfig};
use static_cell::StaticCell;

use super::{WifiCredentials, WifiError};

const AP_SSID: &str = "ESpeaker-Setup";
const AP_IP: &str = "192.168.4.1";
const AP_PORT: u16 = 80;

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: StaticCell<$t> = StaticCell::new();
        STATIC_CELL.init($val)
    }};
}

/// Start AP mode, serve the portal via `handler`, return submitted credentials.
///
/// # Example
/// ```rust
/// let creds = start_ap(&spawner, peripherals.WIFI, |_req| {
///     include_str!("../portal.html")
/// }).await?;
/// ```
pub async fn start_ap(
    spawner: &Spawner,
    controller: WifiController<'static>,
    device: Interface<'static>,
    handler: impl Fn(&str) -> &'static str,
) -> Result<WifiCredentials, WifiError> {
    defmt::info!("wifi: starting AP '{}'", AP_SSID);
    static CONTROLLER: StaticCell<WifiController<'static>> = StaticCell::new();
    let controller = CONTROLLER.init(controller);
    // Configure AP on existing controller
    controller
        .set_config(&Config::AccessPoint(
            AccessPointConfig::default().with_ssid(AP_SSID),
        ))
        .map_err(|_| WifiError::ApStartFailed)?;

    let gw_ip = Ipv4Addr::from_str(AP_IP).unwrap();

    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip, 24),
        gateway: Some(gw_ip),
        dns_servers: Default::default(),
    });

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        device,
        net_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(ap_net_task(runner).unwrap());
    spawner.spawn(ap_wifi_task(controller).unwrap());
    spawner.spawn(ap_dhcp_task(stack, AP_IP).unwrap());

    stack.wait_config_up().await;
    defmt::info!("wifi: AP up at {}", AP_IP);

    loop {
        if let Some(creds) = serve_once(stack, &handler).await? {
            defmt::info!("wifi: credentials received");
            return Ok(creds);
        }
    }
}
/// Handle one HTTP connection. Returns credentials if form was POSTed.
async fn serve_once(
    stack: Stack<'static>,
    handler: &impl Fn(&str) -> &'static str,
) -> Result<Option<WifiCredentials>, WifiError> {
    let mut rx_buf = [0u8; 1536];
    let mut tx_buf = [0u8; 1536];
    let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
    socket.set_timeout(Some(Duration::from_secs(10)));

    defmt::info!("wifi: waiting for connection on port {}", AP_PORT);
    socket
        .accept(IpListenEndpoint {
            addr: None,
            port: AP_PORT,
        })
        .await
        .map_err(|_| WifiError::SocketError)?;

    defmt::info!("wifi: client connected");

    // Read request
    let mut buf = [0u8; 1536];
    let mut pos = 0usize;
    loop {
        match socket.read(&mut buf[pos..]).await {
            Ok(0) => break,
            Ok(n) => {
                pos += n;
                let so_far = unsafe { core::str::from_utf8_unchecked(&buf[..pos]) };
                if so_far.contains("\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return Err(WifiError::SocketError),
        }
    }

    let request = core::str::from_utf8(&buf[..pos]).unwrap_or("");

    // POST → parse credentials
    if request.starts_with("POST") {
        if let Some(creds) = parse_credentials(request) {
            socket
                .write_all(
                    b"HTTP/1.0 200 OK\r\n\r\n\
                      <html><body><h1>Saved! Rebooting...</h1></body></html>",
                )
                .await
                .map_err(|_| WifiError::SocketError)?;
            socket.flush().await.map_err(|_| WifiError::SocketError)?;
            Timer::after(Duration::from_millis(500)).await;
            socket.close();
            return Ok(Some(creds));
        }
    }

    // GET → serve portal page
    let html = handler(request);
    let header = format_header(html.len());
    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| WifiError::SocketError)?;
    socket
        .write_all(html.as_bytes())
        .await
        .map_err(|_| WifiError::SocketError)?;
    socket.flush().await.map_err(|_| WifiError::SocketError)?;
    Timer::after(Duration::from_millis(500)).await;
    socket.close();

    Ok(None)
}

/// Parse URL-encoded POST body: ssid=...&password=...
fn parse_credentials(request: &str) -> Option<WifiCredentials> {
    let body = request.split("\r\n\r\n").nth(1)?;

    let mut ssid = "";
    let mut password = "";

    for pair in body.split('&') {
        let mut kv = pair.splitn(2, '=');
        match (kv.next(), kv.next()) {
            (Some("ssid"), Some(v)) => ssid = v,
            (Some("password"), Some(v)) => password = v,
            _ => {}
        }
    }

    if ssid.is_empty() {
        return None;
    }

    Some(WifiCredentials::new(ssid, password))
}

fn format_header(content_len: usize) -> heapless::String<128> {
    let mut s = heapless::String::new();
    let _ = core::fmt::write(
        &mut s,
        format_args!(
            "HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n",
            content_len
        ),
    );
    s
}

// ── Embassy tasks ─────────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn ap_net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn ap_wifi_task(controller: &'static mut WifiController<'static>) {
    loop {
        match controller
            .wait_for_access_point_connected_event_async()
            .await
        {
            Ok(esp_radio::wifi::AccessPointStationEventInfo::Connected(_info)) => {
                defmt::info!("wifi: station connected");
            }
            Ok(esp_radio::wifi::AccessPointStationEventInfo::Disconnected(_info)) => {
                defmt::info!("wifi: station disconnected");
            }
            Err(_) => {
                defmt::warn!("wifi: AP event error");
            }
        }
        Timer::after(Duration::from_millis(5000)).await;
    }
}

#[embassy_executor::task]
async fn ap_dhcp_task(stack: Stack<'static>, gw_ip_str: &'static str) {
    use core::net::{Ipv4Addr, SocketAddrV4};
    use edge_dhcp::{
        io::{self, DEFAULT_SERVER_PORT},
        server::{Server, ServerOptions},
    };
    use edge_nal::UdpBind;
    use edge_nal_embassy::{Udp, UdpBuffers};

    let ip = Ipv4Addr::from_str(gw_ip_str).unwrap();
    let mut buf = [0u8; 1500];
    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];
    let buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
    let unbound = Udp::new(stack, &buffers);
    let mut bound = unbound
        .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
        .unwrap();

    loop {
        let _ = io::server::run(
            &mut Server::<_, 64>::new_with_et(ip),
            &ServerOptions::new(ip, Some(&mut gw_buf)),
            &mut bound,
            &mut buf,
        )
        .await;
        Timer::after(Duration::from_millis(500)).await;
    }
}
