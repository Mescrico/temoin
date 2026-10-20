# Self-hosting a Témoin server

The server receives encrypted segments from the phones and stores them. It **never** holds the private key: generate the key pair on the computer that will decrypt, and give the server only the public key (`age1…`).

Three ways to run it:

1. [Windows home PC](#1-windows-home-pc-desktop-app) — desktop app, no command line.
2. [Docker](#2-docker) — NAS, VPS, Linux box.
3. [Linux service](#3-linux-service-without-docker) — static binary + systemd.

Phones need **HTTPS** to reach the server. Section [Reaching the server](#reaching-the-server-from-the-internet) lists the options.

---

## 1. Windows home PC (desktop app)

1. Download `Témoin Serveur_x.y.z_x64-setup.exe` from the [releases](../../../releases) and install it (no administrator rights needed).
2. First launch: server name, storage folder, **passphrase**. The private key is created on the spot and stored encrypted with that passphrase (standard age format): a stolen or seized PC gives nothing without it. Save a copy of the key right away (button provided), e.g. on a USB stick kept elsewhere. **Lose the passphrase and the key = the recordings are unreadable forever.**
3. **Start**: the app opens a Cloudflare quick tunnel and shows a public `https://….trycloudflare.com` address. No account, no router setup.
4. **Invite phones**: shows a QR code to scan in the app.
5. **Recover** (passphrase) decrypts what arrived since last time into a folder. **Give files back** turns a person's request code into a one-time link. **Full administration** opens the `/admin` page, already logged in.

Closing the window keeps the server running in the notification area. The PC is kept awake while the server runs.

**Start automatically** (Settings, on by default) opens Témoin in the background when you log in to Windows and starts the server, so it comes back after a reboot. Windows must log in to your account for that: on a PC that reboots unattended, turn on automatic sign-in or use Docker / the Linux service instead.

The quick-tunnel address **changes every time the server restarts**. Phones then scan a new invitation; they keep their identity, quota and pending uploads. For a fixed address, see [built-in HTTPS](#b-built-in-https-with-your-own-domain).

## 2. Docker

### Generate the key pair — offline, on the computer that will decrypt

```bash
docker run --rm -it -v "$PWD:/w" --entrypoint sr ghcr.io/temoin-app/temoin-server keygen -o /w/identity.txt --passphrase
```

This prints the **public key** (`age1…`). `identity.txt` is the **private key**, encrypted with your passphrase. Keep it off the server, in several places. *Lost = everything unreadable forever. Stolen together with the passphrase = everything readable.*

### Run the server

```bash
mkdir temoin && cd temoin
curl -O https://raw.githubusercontent.com/temoin-app/temoin/main/server/docker-compose.yml
curl -o .env https://raw.githubusercontent.com/temoin-app/temoin/main/server/.env.example
# edit .env: SR_RECIPIENT=age1…, SR_ADMIN_TOKEN=<long random string>, SR_PUBLIC_URL=https://…
docker compose up -d
```

The compose file uses `restart: always`: the server comes back after a crash and every time Docker starts. Make sure Docker itself starts at boot (`sudo systemctl enable docker` on most Linux systems; NAS apps such as TrueNAS do it for you).

Open `https://your-server/admin`, enter `SR_ADMIN_TOKEN`: statistics, pause switch, **invitations** (QR codes), devices, journal, hand-back requests.

Put it behind HTTPS: a reverse proxy you already have (Traefik, Caddy, nginx — see [`server/deploy/`](../server/deploy)), or the built-in options below.

## 3. Linux service (without Docker)

```bash
cd server && cargo build --release            # or take the binaries from the Docker image
sudo install target/release/temoin-server target/release/sr /usr/local/bin/
sudo useradd --system --home /var/lib/temoin --create-home temoin
sudo install -m 600 -o temoin /dev/null /etc/temoin.env   # then fill it like .env.example
sudo cp deploy/temoin-server.service /etc/systemd/system/
sudo systemctl enable --now temoin-server
```

`enable` starts it at every boot; the unit restarts it after any exit (`Restart=always`). Check with `systemctl status temoin-server`.

## Reaching the server from the Internet

### A. Cloudflare quick tunnel (zero setup)

`SR_TUNNEL=cloudflare` (with [`cloudflared`](https://github.com/cloudflare/cloudflared) installed, or `SR_CLOUDFLARED=/path/to/cloudflared`). The server starts the tunnel and uses its `https://….trycloudflare.com` address in invitations. The address changes on restart. Cloudflare sees connection metadata (IPs, sizes, times), never content.

### B. Built-in HTTPS with your own domain

`SR_ACME_DOMAIN=myserver.duckdns.org` (and optionally `SR_ACME_EMAIL`). The server obtains and renews a Let's Encrypt certificate (TLS-ALPN-01) and listens on `SR_HTTPS_BIND` (default `0.0.0.0:443`). You need a domain pointing to your public IP ([DuckDNS](https://www.duckdns.org) is free) and port **443** forwarded to the machine. Not possible behind carrier-grade NAT (4G boxes, some ISPs sharing IPv4 addresses).

### C. Your own reverse proxy or relay

Any HTTPS reverse proxy works. Set `SR_TRUST_PROXY=true` only when the proxy is the sole way to reach the server, so the journal records the real client IP. Examples in [`server/deploy/`](../server/deploy).

## Recovering the recordings

Copy the data directory (or only what is new) to the computer holding the key, then:

```bash
sr verify  -d ./data                                        # ledger intact?
sr decrypt -i identity.txt -d ./data -o ./out --remux       # photos .jpg, videos .mp4 (+ metadata .json)
```

`--remux` needs [ffmpeg](https://ffmpeg.org) to turn Android MPEG-TS into MP4. Incremental mode: `--state last-index.txt` processes only sessions that received something since last time; `sr pending` lists the objects to copy.

## Giving files back to someone

1. In the app, the person taps **Get my files back** and passes you the short code (`K7F3-QX9M`) by any channel. The code is random and only shown on their phone.
2. `/admin` shows which device it belongs to.
3. On the computer holding the key (desktop app: **Give files back**; command line below):
   ```bash
   sr share -i identity.txt -d ./data --code K7F3-QX9M --server https://your-server --token-file token.txt --days 7 --clipboard
   ```
4. Send the link through **another channel** than the code (e.g. Signal, or show the QR in person). It opens once; unopened links expire after `--days` days (1–30).

Nothing is deleted on the server: retrieved recordings move from `objects/` to `returned/` and stop counting against the device quota.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `SR_RECIPIENT` | — (required) | public key `age1…` (a private key is refused at startup) |
| `SR_PUBLIC_URL` | — | public URL used in invitations (set automatically by the tunnel / built-in HTTPS) |
| `SR_NAME` | `Témoin` | server name shown in the app |
| `SR_ADMIN_TOKEN` | — | enables `/admin` and the admin API |
| `SR_ACCEPTING` | `true` | initial intake state (then controlled from `/admin`) |
| `SR_MAX_OBJECT_BYTES` | 16 MiB | maximum object size |
| `SR_DEVICE_QUOTA_BYTES` | 20 GiB | default per-phone quota (overridable per invitation) |
| `SR_MAX_TOTAL_BYTES` | — | global cap |
| `SR_MIN_FREE_BYTES` | 50 GiB | free disk space to keep |
| `SR_MAX_VIDEO_HEIGHT` | 1080 | highest quality allowed in the apps |
| `SR_DEFAULT_VIDEO_HEIGHT` | 720 | default quality |
| `SR_VIDEO_BITRATE_720` / `_1080` | 2.5 / 5 Mbit/s | bitrates imposed on the apps |
| `SR_PHOTO_MAX_PIXELS` | 4 000 000 | maximum photo resolution |
| `SR_STEALTH` | `false` | **discreet mode**: everything answers a bare 404 to anyone without an invitation code, a registered device key or the admin token; neutral `/admin` login page |
| `SR_ADMIN_PATH` | `/admin` | admin page path |
| `SR_LOG_IPS` | `true` | record client IPs in the journal |
| `SR_TRUST_PROXY` | `false` | read the client IP from `Cf-Connecting-IP` / `X-Real-IP` / `X-Forwarded-For` |
| `SR_TUNNEL` | `off` | `cloudflare`: Cloudflare quick tunnel |
| `SR_CLOUDFLARED` | `cloudflared` | path to the `cloudflared` executable |
| `SR_ACME_DOMAIN` | — | built-in HTTPS (Let's Encrypt) for this domain |
| `SR_ACME_EMAIL` | — | Let's Encrypt contact (optional) |
| `SR_HTTPS_BIND` | `0.0.0.0:443` | HTTPS listen address (certificates cached in `<data>/acme`) |

Rough sizing: 720p ≈ 1.1 GB per hour per phone.

## What the server sees (and doesn't)

| Visible to the server | Encrypted (invisible) |
|---|---|
| random capture id, segment number, size, arrival time, client IP (unless `SR_LOG_IPS=false`), random device id | picture, sound, capture time, phone model, camera, GPS position (optional), resolution |
