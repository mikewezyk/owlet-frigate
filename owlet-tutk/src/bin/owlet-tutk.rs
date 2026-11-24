//! Command line for the camera client.
//!
//! Secrets come from the environment only. Command lines are visible to every user in `ps`.
use owlet_tutk::{
    client, device, hex_decode, iotc::session::POLL, Config, Datagram, Security, LAN_PORT,
};
use std::{
    env, fs,
    io::{self, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

const USAGE: &str = "owlet-tutk search|stream|fake-camera [options]

Environment:
  OWLET_TUTK_PASSWORD    camera password
  OWLET_TUTK_AUTHKEY     8 alphanumeric characters

Options:
  --uid UID              20 alphanumeric characters (or set OWLET_TUTK_UID)
  --ip ADDRESS           unicast destination or fake-camera bind address
  --port PORT|scan       UDP destination or camera listen port (default 32761);
                         scan finds the port of the host given by --ip
  --port-hint PORT       with --port scan: try this port before the sweep
  --port-cache FILE      with --port scan: remember the port in FILE
  --bind-port PORT       client source port (default 0)
  --name NAME            name returned by the camera
  --account NAME         AV account (default admin)
  --security MODE        dtls (default), simple, or auto; stream allows simple and
                         auto on loopback only, because they can send the password
                         without encryption
  --cipher-list LIST     OpenSSL cipher list
  --start-ioctrl T:HEX   IOCtrl type in hex and payload in hex
  --out DEST             -, a file path, or tcp://host:port (default -)
  --h264 FILE            camera input in Annex B format
  --fps N                camera frames per second (default 15)
  --start-on-login       stream camera input immediately after AV login
  --resend               request packet retransmission (stream)
  --loop                 repeat camera input
  --timeout SECONDS      search or handshake timeout (default 5)
  --stall SECONDS        stop when no frame arrives for this time (default 20)
  --duration SECONDS     limit stream duration
  --key-mask HEX         mask for the 16-byte transcode key
  --trace                print decoded datagrams to stderr with credentials masked";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Search,
    Stream,
    FakeCamera,
}

/// Everything the command line decides. `parse` fills it and does no I/O.
struct Options {
    command: Command,
    config: Config,
    ip: Option<Ipv4Addr>,
    port: u16,
    bind_port: u16,
    scan: bool,
    port_hint: Option<u16>,
    port_cache: Option<PathBuf>,
    output: String,
    source: Option<PathBuf>,
}

fn bad(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
fn number<T: std::str::FromStr>(value: &str, what: &str) -> io::Result<T> {
    value.parse().map_err(|_| bad(format!("invalid {what}")))
}
fn seconds(value: &str) -> io::Result<Duration> {
    let duration = value
        .parse()
        .ok()
        .and_then(|v| Duration::try_from_secs_f64(v).ok())
        .ok_or_else(|| bad("invalid time in seconds"))?;
    if duration.is_zero() {
        return Err(bad("time must be positive"));
    }
    Ok(duration)
}
fn auth_key(value: &str) -> io::Result<[u8; 8]> {
    if !value.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err(bad("invalid auth key"));
    }
    value
        .as_bytes()
        .try_into()
        .map_err(|_| bad("auth key must be 8 characters"))
}

/// Returns `None` when the user asks for help. `env` is a parameter so tests can supply values.
fn parse(
    mut args: impl Iterator<Item = String>,
    env: impl Fn(&str) -> Option<String>,
) -> io::Result<Option<Options>> {
    let command = match args.next().as_deref() {
        None | Some("help" | "-h" | "--help") => return Ok(None),
        Some("search") => Command::Search,
        Some("stream") => Command::Stream,
        Some("fake-camera") => Command::FakeCamera,
        Some(_) => return Err(bad("expected search, stream, or fake-camera")),
    };
    // An empty variable means "not set": Compose passes unset variables as empty strings.
    let env = |name| env(name).filter(|v| !v.is_empty());
    let mut o = Options {
        command,
        config: Config::default(),
        ip: None,
        port: LAN_PORT,
        bind_port: 0,
        scan: false,
        port_hint: None,
        port_cache: None,
        output: "-".into(),
        source: None,
    };
    let cfg = &mut o.config;
    if command == Command::FakeCamera {
        cfg.security = Security::Auto;
    }
    cfg.uid = env("OWLET_TUTK_UID").unwrap_or_default();
    cfg.password = env("OWLET_TUTK_PASSWORD").unwrap_or_default();
    cfg.auth_key = env("OWLET_TUTK_AUTHKEY")
        .as_deref()
        .map(auth_key)
        .transpose()?;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--help" | "-h" => return Ok(None),
            "--trace" => cfg.trace = true,
            "--loop" => cfg.repeat = true,
            "--resend" if command == Command::Stream => cfg.resend = true,
            "--start-on-login" if command == Command::FakeCamera => cfg.start_on_login = true,
            _ => {
                let v = args
                    .next()
                    .ok_or_else(|| bad(format!("{flag}: unknown option or missing value")))?;
                match flag.as_str() {
                    "--ip" => o.ip = Some(number(&v, "IPv4 address")?),
                    "--port" if v == "scan" && command != Command::FakeCamera => o.scan = true,
                    "--port" => o.port = number(&v, "UDP port")?,
                    "--port-hint" => o.port_hint = Some(number(&v, "port hint")?),
                    "--port-cache" => o.port_cache = Some(v.into()),
                    "--bind-port" => o.bind_port = number(&v, "bind port")?,
                    "--out" => o.output = v,
                    "--h264" => o.source = Some(v.into()),
                    "--uid" => cfg.uid = v,
                    "--name" => cfg.name = v,
                    "--account" => cfg.account = v,
                    "--cipher-list" => cfg.cipher_list = v,
                    "--timeout" => cfg.timeout = seconds(&v)?,
                    "--stall" => cfg.stall = seconds(&v)?,
                    "--duration" => cfg.duration = Some(seconds(&v)?),
                    "--fps" => cfg.fps = number(&v, "fps")?,
                    "--key-mask" => {
                        cfg.key_mask = hex_decode(&v)?
                            .try_into()
                            .map_err(|_| bad("key mask must be 16 bytes"))?;
                    }
                    "--security" => {
                        cfg.security = match v.as_str() {
                            "simple" => Security::Simple,
                            "dtls" => Security::Dtls,
                            "auto" => Security::Auto,
                            _ => return Err(bad("invalid security mode")),
                        };
                    }
                    "--start-ioctrl" => {
                        let (kind, body) = v
                            .split_once(':')
                            .ok_or_else(|| bad("expected type:payload"))?;
                        let kind = u32::from_str_radix(kind.trim_start_matches("0x"), 16)
                            .map_err(|_| bad("invalid IOCtrl type"))?;
                        cfg.start = (kind, hex_decode(body)?);
                    }
                    _ => return Err(bad(format!("{flag}: unknown option"))),
                }
            }
        }
    }
    validate(&o)?;
    // A_lan 7: unicast discovery and connect are inferred; --ip selects the destination.
    o.config.dest = SocketAddrV4::new(o.ip.unwrap_or(Ipv4Addr::BROADCAST), o.port).into();
    Ok(Some(o))
}

fn validate(o: &Options) -> io::Result<()> {
    let loopback = o.ip.is_some_and(|ip| ip.is_loopback());
    let security = o.config.security;
    if security == Security::Dtls && !cfg!(feature = "dtls") {
        return Err(bad("DTLS requires the dtls Cargo feature"));
    }
    if o.command == Command::Stream && security != Security::Dtls && !loopback {
        return Err(bad("refusing to send the password without DTLS"));
    }
    if o.command == Command::Stream && o.config.password.is_empty() {
        return Err(bad("set OWLET_TUTK_PASSWORD"));
    }
    if o.scan && o.ip.is_none() {
        return Err(bad("--port scan requires --ip"));
    }
    Ok(())
}

/// A missing or damaged cache costs a sweep. It is never an error.
fn read_port(path: &Path) -> Option<u16> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn find_camera(socket: &Arc<UdpSocket>, o: &Options) -> io::Result<SocketAddr> {
    let ip = o.ip.ok_or_else(|| bad("--port scan requires --ip"))?;
    let mut quiet = o.config.clone();
    quiet.trace = false;
    let hint = o
        .port_hint
        .or_else(|| o.port_cache.as_deref().and_then(read_port));
    let socket: Arc<dyn Datagram> = socket.clone();
    let address = client::scan_ports(socket, &quiet, ip, hint)?
        .into_iter()
        .find(|d| o.config.uid.is_empty() || d.uid.eq_ignore_ascii_case(&o.config.uid))
        .ok_or_else(|| bad("port scan found no device"))?
        .address;
    eprintln!("device port {}", address.port());
    if let Some(path) = &o.port_cache {
        if let Err(e) = fs::write(path, address.port().to_string()) {
            eprintln!("cannot save the port in {}: {e}", path.display());
        }
    }
    Ok(address)
}

fn sink(output: &str) -> io::Result<Box<dyn Write>> {
    Ok(match output.strip_prefix("tcp://") {
        Some(address) => Box::new(TcpStream::connect(address)?),
        None if output == "-" => Box::new(io::stdout().lock()),
        None => Box::new(fs::File::create(output)?),
    })
}

fn run(mut o: Options) -> io::Result<()> {
    let bind_ip = match o.ip {
        Some(ip) if o.command == Command::FakeCamera => ip,
        Some(ip) if ip.is_loopback() => Ipv4Addr::LOCALHOST,
        _ => Ipv4Addr::UNSPECIFIED,
    };
    let bind_port = match o.command {
        Command::FakeCamera => o.port,
        _ => o.bind_port,
    };
    let socket = Arc::new(UdpSocket::bind(SocketAddrV4::new(bind_ip, bind_port))?);
    socket.set_read_timeout(Some(POLL))?;
    if o.scan {
        o.config.dest = find_camera(&socket, &o)?;
    }
    match o.command {
        Command::FakeCamera => {
            let path = o.source.ok_or_else(|| bad("fake-camera requires --h264"))?;
            let data = fs::read(path)?;
            println!("listening {}", socket.local_addr()?);
            io::stdout().flush()?;
            device::serve(socket, &o.config, &data, &AtomicBool::new(false))
        }
        Command::Search => {
            socket.set_broadcast(true)?;
            for d in client::search(socket, &o.config)? {
                let version = d
                    .version
                    .map_or_else(|| "unknown".into(), |v| format!("{v:#010x}"));
                println!(
                    "{} {} {} {} {version}",
                    d.uid.escape_default(),
                    d.address.ip(),
                    d.address.port(),
                    d.name.escape_default(),
                );
            }
            Ok(())
        }
        Command::Stream => {
            socket.set_broadcast(true)?;
            client::stream(socket, &o.config, &mut *sink(&o.output)?)
        }
    }
}

fn main() {
    let result = parse(env::args().skip(1), |name| env::var(name).ok()).and_then(|o| match o {
        Some(options) => run(options),
        None => {
            println!("{USAGE}");
            Ok(())
        }
    });
    if let Err(e) = result {
        eprintln!("owlet-tutk: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(line: &str) -> io::Result<Options> {
        let env = |name: &str| match name {
            "OWLET_TUTK_PASSWORD" => Some("secret".to_owned()),
            "OWLET_TUTK_AUTHKEY" => Some(String::new()),
            _ => None,
        };
        parse(line.split_whitespace().map(str::to_owned), env).map(Option::unwrap)
    }
    fn error(line: &str) -> String {
        options(line).err().expect("must fail").to_string()
    }

    #[test]
    fn stream_uses_dtls_by_default() {
        let o = options("stream --ip 192.0.2.10").unwrap();
        assert!(o.config.security == Security::Dtls);
        assert_eq!(o.config.auth_key, None);
    }
    #[test]
    fn plaintext_is_refused_off_loopback() {
        for mode in ["simple", "auto"] {
            let line = format!("stream --ip 192.0.2.10 --security {mode}");
            assert!(error(&line).contains("without DTLS"));
        }
        assert!(options("stream --ip 127.0.0.1 --security simple").is_ok());
    }
    #[test]
    fn secrets_are_not_accepted_as_arguments() {
        for flag in ["--password", "--auth-key"] {
            assert!(error(&format!("stream {flag} x")).contains("unknown option"));
        }
    }
    #[test]
    fn scan_needs_an_address() {
        assert!(error("stream --port scan").contains("requires --ip"));
    }
    #[test]
    fn damaged_port_cache_is_ignored() {
        let path = env::temp_dir().join(format!("owlet-port-{}", std::process::id()));
        assert_eq!(read_port(&path), None);
        for (text, expected) in [("37567\n", Some(37567)), ("99999", None), ("x", None)] {
            fs::write(&path, text).unwrap();
            assert_eq!(read_port(&path), expected);
        }
        fs::remove_file(path).unwrap();
    }
}
