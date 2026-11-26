# owlet-frigate: overview

## What this is

A small bridge that lets an **Owlet Cam** (a baby camera that normally only talks to Owlet's app and cloud portal) feed video to **Frigate**, a self-hosted NVR, entirely over your home network.

It is a hardened fork of Simon Sickle's [owlet-rtsp-bridge](https://github.com/simonsickle/owlet-rtsp-bridge), which did the hard part: reverse engineering the camera's ThroughTek (TUTK) protocol. See [Credit](#credit).

## What it does

```
Owlet cam ──LAN, DTLS──> owlet-tutk ──H.264──> go2rtc ──RTSP──> Frigate ──> Home Assistant
              (UDP)       (Rust client)         (in the same container)
```

1. **`owlet-tutk`** (Rust, in `owlet-tutk/`) finds the camera on the LAN, opens an encrypted DTLS session using the camera's credentials, and writes the raw H.264 video to standard output. It never goes through Owlet's servers for video.
2. **go2rtc** (in the same container, configured by `deploy/go2rtc.yaml`) runs `owlet-tutk` as a child process, takes its H.264 output, and serves it as an RTSP stream named `nursery`, protected by a password.
3. **Frigate** pulls that RTSP stream like any other IP camera.
4. **Home Assistant** gets the camera and sensors from Frigate's integration.

`scripts/fetch_credentials.py` is a one-time helper. You type your Owlet account password once and it fetches the camera's UID, password and auth key into a local `.env` (mode 600). The account password is not saved.

### Changes from upstream

| Change | Why |
|---|---|
| PSK-only ciphers, certificate checks on, and a post-handshake check | Upstream accepted a certificate-only impostor camera and would then send it the password |
| DTLS by default; plaintext refused off loopback | Fail closed |
| Secrets read only from the environment | Command-line arguments are visible in `ps` |
| `--port-cache` | Remembers the camera's UDP port, replacing what the portal used to tell the client |
| `--stall` | A frozen picture ends the stream so it reconnects |
| `unsafe` forbidden; overflow checks in release builds | The packet parsers read untrusted input |

`git diff 12551af -- owlet-tutk` shows every change to the imported code.

## How it works with Frigate

Everything runs in Docker on the same host as Frigate.

- **Network.** The bridge container has no published ports. It joins a Docker network, `owlet-net`, and Frigate joins the same network. Only containers on `owlet-net` can reach the video.
- **Auth.** go2rtc requires the username `frigate` and a generated password (`RTSP_PASSWORD` in this project's `.env`). Frigate gets the same value as `FRIGATE_OWLET_PASSWORD`.
- **Camera entry.** Frigate's own go2rtc restreams `rtsp://frigate:<password>@owlet-frigate:8554/nursery`, and the `nursery` camera reads that restream locally on `rtsp://127.0.0.1:8554/nursery` for the `detect` role at 5 fps.
- **Privacy settings.** Recording, snapshots, review, birdseye and LPR are off, and `objects.track` is empty, so Frigate runs no object detection and keeps no clips or thumbnails.
- **Crib state.** Frigate's state classification (a custom model, `crib_state`, with classes `empty`, `asleep`, `awake`) is trained from labeled still frames. `scripts/labeler/` is a small local web tool that makes labeling faster and sends labels to Frigate.
- **Home Assistant.** Frigate's integration exposes `camera.nursery` and the classification state.
- **Exposure.** Frigate's ports 5000 and 8554 are open and passwordless by default. Limit them to Home Assistant with a `DOCKER-USER` firewall rule.

The exact config is in [frigate.md](frigate.md).

## Limits

- Video only: no audio and no two-way talk.
- Tested on firmware 4.14.2. Upstream tested one model (E96LM72) on firmware 4.2.146.
- The first connection can take up to 3 minutes while the client finds the camera's UDP port.
- This is a convenience feed, **not a medical or safety monitor**. Do not rely on it to tell you a baby is safe.

## Credit

The camera client, the protocol notes and the reverse engineering (`owlet-tutk/`, including `spec/`, `re/` and `evidence/`) come from **Simon Sickle**'s [owlet-rtsp-bridge](https://github.com/simonsickle/owlet-rtsp-bridge), imported at commit `d6e5983`. The credential script uses the endpoints found there.

The DTLS hardening, port cache, stall detection, container, Frigate integration and labeling tools are new in this repository.

Licensed under [AGPL-3.0](../LICENSE), like the original. Not affiliated with Owlet or ThroughTek.
