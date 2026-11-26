# owlet-frigate

Feeds an Owlet Cam into Frigate over the LAN. No portal, and no ports open to the LAN.

```
Owlet cam → owlet-tutk (Rust, DTLS) → go2rtc (RTSP) → Frigate → Home Assistant
```

**Status:** working on a real camera (firmware 4.14.2) and feeding Frigate. Upstream tested one
model: E96LM72, firmware 4.2.146. See [docs/OVERVIEW.md](docs/OVERVIEW.md) for what this is and how it fits with Frigate.
Video only. This is a convenience feed, not a medical or safety monitor.

## Set up

1. Give the camera a fixed IP in your router.
2. On the Docker host, fetch the camera credentials. You type your Owlet password; it is not saved.
   ```sh
   python3 scripts/fetch_credentials.py --ip <camera-ip>
   ```
3. Start the bridge.
   ```sh
   docker network create owlet-net
   docker compose up -d --build
   ```
4. Connect Frigate: see [docs/frigate.md](docs/frigate.md).

The first connection can take up to 3 minutes while the client finds the camera's UDP port.
The port is remembered after that.

## What differs from upstream

| Change | Why |
|---|---|
| Only PSK ciphers, certificate checks on, and a check after the handshake | Upstream accepted an impostor camera and would send it the password |
| DTLS by default; plaintext refused off loopback | Fail closed |
| Secrets only from the environment | Arguments are visible in `ps` |
| `--port-cache` | Replaces the portal's memory of the port |
| `--stall` | A frozen picture ends the stream, so it reconnects |
| `unsafe` forbidden; overflow checks in release builds | The parsers read untrusted packets |

`git diff 12551af -- owlet-tutk` shows every change to the imported code.

**Keep this repository private until the author has had a chance to fix the DTLS issue.**

## Develop

```sh
brew install rustup openssl@3 && rustup default stable   # macOS
cd owlet-tutk && cargo test && cargo clippy --all-targets -- -D warnings
cd ../scripts && python3 -m unittest
```

New to Rust? Start with [docs/learn-rust.md](docs/learn-rust.md).

## Security notes

- The camera password, its auth key, and the RTSP password are in `.env` (mode 600).
- go2rtc does not ask *local* clients for a password. Everything in this container is trusted.
- The camera's address and the LAN search are not secret. Only the DTLS channel is.
- `--cipher-list` can widen the list, but the other two checks still refuse an impostor.

## Credit and license

The camera client, the protocol notes, and the reverse engineering are the work of
**Simon Sickle**: [owlet-rtsp-bridge](https://github.com/simonsickle/owlet-rtsp-bridge),
imported at commit `d6e5983`. The credential script uses the endpoints found there.
Licensed under [AGPL-3.0](LICENSE), like the original. Modified 2026-11-24.
Not affiliated with Owlet or ThroughTek.
