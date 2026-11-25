# Music Assistant

Music library/streaming server that drives the home stereo. The appliance
provisions it as an Incus container and controls playback through
`hs-system` (PipeWire/`wpctl`); this directory is the reference standalone
deployment.

## Files

| File | Purpose |
| --- | --- |
| `docker-compose.yml` | standalone container + the audio caveat that matters |

## Ports

| Port | Scope | Purpose |
| --- | --- | --- |
| `8095/tcp` | LAN (via Caddy app vhost) | web UI + API |
| player protocols | LAN | AirPlay/Chromecast/Snapcast discovery, as enabled per player |

## Audio: the important part

A container does **not** see the host PipeWire session, so Music Assistant
inside a container cannot play out of the appliance's own speakers:

| Scenario | Works? | Notes |
| --- | --- | --- |
| Network players (AirPlay, Chromecast, Snapcast, HA `media_player`) | yes | Music Assistant only streams over the network; no host audio needed |
| Appliance speakers/USB DAC inside a container | no (supported path) | The host PipeWire socket is not available in the container |
| Appliance speakers with a **native** Music Assistant install | yes | Recommended for a wired stereo; install with `pipx`, run under the `homeserver` user session |
| Bind-mounting `/run/user/<uid>/pipewire-0` into the container | lab only | UID/cgroup/socket-permission fragility; not part of the appliance design |

The appliance's supported native-audio flow:

1. `pipewire` + `wireplumber` run as the `homeserver` user's session on the
   host (installed by the ISO).
2. `hs-system` selects the output sink with `wpctl set-default <sink_id>`
   (`SystemConfig.audio.sink_id`).
3. The recommended physical path is an **external USB DAC** (or HDMI audio) so
   the laptop's internal amplifier is not driving a wall-mounted speaker.

## Setup

1. Open the app vhost `https://music.<base_domain>:8443` (rendered by the
   daemon) and add music providers (local files, Spotify, ...).
2. Add players: prefer network players; for the appliance itself use the
   native install described above.
3. Volume/bass presets are managed by the portal; the daemon calls
   `wpctl set-volume` on the configured sink, never inside the container.

## Backups

`data/` contains the library database and provider caches. It is included in
the appliance app-volume backup; provider credentials stay in Music
Assistant's own encrypted store and in the appliance secret vault for API
tokens the daemon needs.
