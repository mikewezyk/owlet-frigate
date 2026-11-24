# Connect Frigate

Back up `config.yml` first. Nothing here has been applied to a running Frigate yet.

## 1. Join the network

In `/opt/frigate/docker-compose.yml`, keep the default network and add `owlet-net`:

```yaml
services:
  frigate:
    networks: [default, owlet-net]
networks:
  owlet-net:
    external: true
```

## 2. Share the RTSP password

Add `RTSP_PASSWORD` from this project's `.env` to `/opt/frigate/.env` as
`FRIGATE_OWLET_PASSWORD`. Do not print either file.

## 3. Add the camera

```yaml
go2rtc:
  streams:
    nursery:
      - rtsp://frigate:{FRIGATE_OWLET_PASSWORD}@owlet-frigate:8554/nursery
cameras:
  nursery:
    enabled: true
    birdseye:
      enabled: false
    ffmpeg:
      inputs:
        - path: rtsp://127.0.0.1:8554/nursery
          input_args: preset-rtsp-restream
          roles: [detect]
    detect:
      enabled: true
      width: 1280
      height: 720
      fps: 5
    record:
      enabled: false
    snapshots:
      enabled: false
```

Validate, then restart:

```sh
docker exec frigate python3 -m frigate --validate-config && docker restart frigate
```

Classification saves training crops on the server even with recording off.

## 4. Protect the feed

Frigate's port 5000 has no login and now shows the nursery. In Frigate's `ports:`, limit who can
reach it. Docker's published ports bypass most host firewalls, so use the `DOCKER-USER` chain to
allow only Home Assistant (<home-assistant-ip>).

Frigate also restreams every camera on port 8554, with no password unless you set one in its
go2rtc `rtsp:` section. Do not publish 8554 unless something needs it.

## 5. Awake or asleep

In Frigate: **Classification → States → Add**. Classes `empty`, `asleep`, `awake`. Crop tightly
around the crib. Label day and infrared images of each state.
