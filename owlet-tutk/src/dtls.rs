//! OpenSSL DTLS over one IOTC channel. Each DATA packet carries one DTLS record.
use crate::{
    invalid,
    iotc::session::{Role, Session},
    sha256::sha256,
    timeout, Config,
};
use openssl::ssl::{
    ErrorCode, Ssl, SslContextBuilder, SslMethod, SslOptions, SslRef, SslStream, SslVerifyMode,
    SslVersion,
};
use std::{
    io::{self, Read, Write},
    time::Instant,
};

const MTU: usize = 0x56c;
const RECORD_HEADER: usize = 13;
// The captured vendor ClientHello offers DTLS 1.2 (fe fd). The PSK callbacks
// and the 13-byte record header below apply through DTLS 1.2.
const MAX_DTLS_VERSION: SslVersion = SslVersion::DTLS1_2;
// C_session_dtls 5.2 and SPEC.md: split a BIO flight at each record boundary.
const SPLIT_DTLS_RECORDS: bool = true;
const DTLS_METHOD: fn() -> SslMethod = SslMethod::dtls;
// C_session_dtls 5.4: the digest is assumed to be standard SHA-256.
// Binary strncpy stops at the first zero byte and pads the remaining bytes.
pub fn password_psk(password: &str) -> [u8; 32] {
    let mut key = sha256(password.as_bytes());
    if let Some(zero) = key.iter().position(|&b| b == 0) {
        key[zero..].fill(0);
    }
    key
}
struct Records<'a> {
    session: &'a Session,
    remaining: Vec<u8>,
    plain_peer: bool,
}
impl Read for Records<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.remaining.is_empty() {
            match self.session.recv()? {
                Some((1, record)) if !record.is_empty() => self.remaining = record,
                Some((0, b)) => {
                    self.plain_peer = b == b"recv dtls data!";
                    return Err(invalid("plain packet on the DTLS channel"));
                }
                _ => return Err(io::ErrorKind::WouldBlock.into()),
            }
        }
        let n = out.len().min(self.remaining.len());
        out[..n].copy_from_slice(&self.remaining[..n]);
        self.remaining.drain(..n);
        Ok(n)
    }
}
impl Write for Records<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // OpenSSL can pack several handshake records into one BIO write.
        if !SPLIT_DTLS_RECORDS && bytes.len() <= MTU {
            self.session.send(1, bytes)?;
            return Ok(bytes.len());
        }
        let mut rest = bytes;
        while !rest.is_empty() {
            let header = rest
                .get(..RECORD_HEADER)
                .ok_or_else(|| invalid("short DTLS record"))?;
            let len = RECORD_HEADER + usize::from(u16::from_be_bytes([header[11], header[12]]));
            let record = rest
                .get(..len)
                .ok_or_else(|| invalid("incomplete DTLS record"))?;
            self.session.send(1, record)?;
            rest = &rest[len..];
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
/// Builds the TLS settings for one side of the channel.
fn context(cfg: &Config, role: Role) -> io::Result<SslContextBuilder> {
    let mut ctx = SslContextBuilder::new(DTLS_METHOD()).map_err(io::Error::other)?;
    ctx.set_min_proto_version(Some(MAX_DTLS_VERSION))
        .map_err(io::Error::other)?;
    ctx.set_max_proto_version(Some(MAX_DTLS_VERSION))
        .map_err(io::Error::other)?;
    // Defence in depth. PSK suites carry no certificate, so this changes nothing for the
    // camera. If a cipher list ever lets a certificate suite in, it fails here: no CA is
    // trusted. OpenSSL clients verify nothing unless they are told to.
    ctx.set_verify(SslVerifyMode::PEER);
    ctx.set_options(SslOptions::NO_QUERY_MTU);
    if cfg.cipher_list.as_bytes().contains(&0) {
        return Err(invalid("invalid cipher list"));
    }
    ctx.set_cipher_list(&cfg.cipher_list)
        .map_err(io::Error::other)?;
    let identity = format!("AUTHPWD_{}", cfg.account).into_bytes();
    let key = password_psk(&cfg.password);
    if role == Role::Client {
        // C_session_dtls 5.4: the callback ignores the unknown identity hint.
        ctx.set_psk_client_callback(move |_, _, id, out| {
            if id.len() <= identity.len() || out.len() < key.len() {
                return Ok(0);
            }
            id[..identity.len()].copy_from_slice(&identity);
            id[identity.len()] = 0;
            out[..32].copy_from_slice(&key);
            Ok(32)
        });
    } else {
        ctx.set_psk_server_callback(move |_, id, out| {
            if id != Some(identity.as_slice()) || out.len() < 32 {
                return Ok(0);
            }
            out[..32].copy_from_slice(&key);
            Ok(32)
        });
    }
    Ok(ctx)
}
/// Refuses a finished handshake that the pre-shared key did not authenticate.
fn require_psk(ssl: &SslRef) -> io::Result<()> {
    match ssl.current_cipher() {
        Some(c) if is_psk(c.name()) => Ok(()),
        _ => Err(invalid("peer did not prove that it knows the password")),
    }
}
fn is_psk(cipher: &str) -> bool {
    ["PSK-", "DHE-PSK-", "ECDHE-PSK-"]
        .iter()
        .any(|prefix| cipher.starts_with(prefix))
}
pub(crate) struct Secure<'a>(SslStream<Records<'a>>);
impl<'a> Secure<'a> {
    pub fn open(session: &'a Session, cfg: &Config) -> io::Result<Self> {
        if cfg.account.is_empty()
            || cfg.account.len() + 8 >= 120
            || cfg.account.as_bytes().contains(&0)
            || cfg.password.as_bytes().contains(&0)
        {
            return Err(invalid("invalid DTLS credential length (-20024)"));
        }
        let ctx = context(cfg, session.role)?;
        let mut ssl = Ssl::new(&ctx.build()).map_err(io::Error::other)?;
        ssl.set_mtu(MTU as u32).map_err(io::Error::other)?;
        let mut stream = SslStream::new(
            ssl,
            Records {
                session,
                remaining: Vec::new(),
                plain_peer: false,
            },
        )
        .map_err(io::Error::other)?;
        let start = Instant::now();
        while start.elapsed() < cfg.timeout {
            let result = if session.role == Role::Client {
                stream.connect()
            } else {
                stream.accept()
            };
            match result {
                Ok(()) => {
                    require_psk(stream.ssl())?;
                    return Ok(Self(stream));
                }
                Err(_) if stream.get_ref().plain_peer => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "peer requires plain mode (-20039)",
                    ))
                }
                Err(e) if matches!(e.code(), ErrorCode::WANT_READ | ErrorCode::WANT_WRITE) => {
                    continue
                }
                // A PSK peer can silently discard a bad MAC and close its IOTC
                // session when its own handshake timer expires (C_session_dtls 5.5).
                Err(e)
                    if e.io_error()
                        .is_some_and(|e| e.kind() == io::ErrorKind::ConnectionAborted) =>
                {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "peer closed before DTLS handshake completed (-20011)",
                    ));
                }
                Err(e) => {
                    if e.ssl_error().is_some_and(|s| {
                        s.errors().iter().any(|e| {
                            e.reason().is_some_and(|r| {
                                r.contains("decrypt")
                                    || r.contains("bad record mac")
                                    || r.contains("psk identity")
                            })
                        })
                    }) {
                        return Err(invalid("DTLS password rejected (-20040)"));
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("DTLS handshake failed: {e} (-20041)"),
                    ));
                }
            }
        }
        Err(timeout("DTLS handshake timeout (-20011)"))
    }
    pub fn send(&mut self, b: &[u8]) -> io::Result<()> {
        if self.0.ssl_write(b).map_err(io::Error::other)? != b.len() {
            return Err(io::ErrorKind::WriteZero.into());
        }
        Ok(())
    }
    pub fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut b = [0; MTU];
        match self.0.ssl_read(&mut b) {
            Ok(0) => Err(io::ErrorKind::ConnectionAborted.into()),
            Ok(n) => Ok(Some(b[..n].to_vec())),
            Err(e) if matches!(e.code(), ErrorCode::WANT_READ | ErrorCode::WANT_WRITE) => Ok(None),
            Err(e) if e.code() == ErrorCode::ZERO_RETURN => {
                Err(io::ErrorKind::ConnectionAborted.into())
            }
            Err(e) => Err(e.into_io_error().unwrap_or_else(io::Error::other)),
        }
    }
}

#[cfg(test)]
mod tests {
    //! An impostor on the LAN has the camera's address but not its password.
    use super::*;
    use openssl::{
        asn1::Asn1Time,
        hash::MessageDigest,
        pkey::PKey,
        rsa::Rsa,
        x509::{X509Name, X509},
    };
    use std::{net::UdpSocket, thread, time::Duration};

    /// A connected UDP socket is the datagram transport for these handshakes.
    struct Udp(UdpSocket);
    impl Read for Udp {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.0.recv(out)
        }
    }
    impl Write for Udp {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.send(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn pair() -> (Udp, Udp) {
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").unwrap();
        a.connect(b.local_addr().unwrap()).unwrap();
        b.connect(a.local_addr().unwrap()).unwrap();
        for s in [&a, &b] {
            s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        }
        (Udp(a), Udp(b))
    }
    /// The impostor: a self-signed certificate and no pre-shared key.
    fn impostor() -> SslContextBuilder {
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let mut name = X509Name::builder().unwrap();
        name.append_entry_by_text("CN", "impostor").unwrap();
        let name = name.build();
        let mut cert = X509::builder().unwrap();
        cert.set_version(2).unwrap();
        cert.set_subject_name(&name).unwrap();
        cert.set_issuer_name(&name).unwrap();
        cert.set_pubkey(&key).unwrap();
        cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        cert.sign(&key, MessageDigest::sha256()).unwrap();
        let mut ctx = SslContextBuilder::new(DTLS_METHOD()).unwrap();
        ctx.set_max_proto_version(Some(MAX_DTLS_VERSION)).unwrap();
        ctx.set_cipher_list("ALL:@SECLEVEL=0").unwrap();
        ctx.set_certificate(&cert.build()).unwrap();
        ctx.set_private_key(&key).unwrap();
        ctx
    }
    /// Returns the cipher the client accepted.
    fn handshake(client: SslContextBuilder, server: SslContextBuilder) -> Option<String> {
        let (a, b) = pair();
        let server = thread::spawn(move || {
            let mut ssl = Ssl::new(&server.build()).unwrap();
            ssl.set_mtu(MTU as u32).unwrap();
            let _ = SslStream::new(ssl, b).unwrap().accept();
        });
        let mut ssl = Ssl::new(&client.build()).unwrap();
        ssl.set_mtu(MTU as u32).unwrap();
        let mut stream = SslStream::new(ssl, a).unwrap();
        let done = stream.connect().is_ok();
        server.join().unwrap();
        done.then(|| stream.ssl().current_cipher().unwrap().name().to_owned())
    }
    /// The settings before this fork: any cipher and no verification.
    fn upstream_client() -> SslContextBuilder {
        let mut ctx = SslContextBuilder::new(DTLS_METHOD()).unwrap();
        ctx.set_max_proto_version(Some(MAX_DTLS_VERSION)).unwrap();
        ctx.set_cipher_list("DEFAULT:@SECLEVEL=0").unwrap();
        ctx
    }
    fn config() -> Config {
        Config {
            password: "test-camera-password".into(),
            ..Config::default()
        }
    }

    #[test]
    fn impostor_without_the_password_is_rejected() {
        let client = context(&config(), Role::Client).unwrap();
        assert_eq!(handshake(client, impostor()), None);
    }
    /// Proves that the impostor in these tests is a real threat and not a broken server.
    #[test]
    fn impostor_is_accepted_by_the_old_settings() {
        let cipher = handshake(upstream_client(), impostor()).unwrap();
        assert!(!is_psk(&cipher), "{cipher}");
    }
    /// The verify setting alone must hold if somebody widens the cipher list.
    #[test]
    fn impostor_is_rejected_even_with_a_wide_cipher_list() {
        let cfg = Config {
            cipher_list: "DEFAULT:@SECLEVEL=0".into(),
            ..config()
        };
        let client = context(&cfg, Role::Client).unwrap();
        assert_eq!(handshake(client, impostor()), None);
    }
    #[test]
    fn camera_with_the_password_is_accepted() {
        let client = context(&config(), Role::Client).unwrap();
        let server = context(&config(), Role::Device).unwrap();
        let cipher = handshake(client, server).unwrap();
        assert!(is_psk(&cipher), "{cipher}");
    }
    #[test]
    fn only_psk_authentication_counts() {
        assert!(is_psk("ECDHE-PSK-CHACHA20-POLY1305"));
        assert!(is_psk("PSK-AES128-CBC-SHA256"));
        assert!(!is_psk("ECDHE-RSA-AES256-GCM-SHA384"));
        // RSA-PSK mixes in a certificate. The camera is not known to use it.
        assert!(!is_psk("RSA-PSK-AES256-GCM-SHA384"));
    }
}
