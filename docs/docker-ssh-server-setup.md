# Running the passalong server in Docker

This guide sets up a passalong server with Docker on any Linux machine that
is always on, such as a NAS, a home server, or a small VPS. The server is a
plain OpenSSH server in a container; your items are stored in an ordinary
directory on the host, so they are easy to back up.

The setup below is the same one `just test-deploy` starts and checks on
every CI run.

## What you need

- Docker with the Compose plugin on the server.
- On each client device, `passalong` and an SSH key pair. Create one with
  `ssh-keygen -t ed25519` if `~/.ssh/id_ed25519.pub` does not exist yet.

## 1. Prepare a directory

On the server, create a directory for passalong and copy in the example
files from this repository:

```sh
mkdir -p ~/passalong-server/keys ~/passalong-server/storage
cd ~/passalong-server
curl -fsSLO https://raw.githubusercontent.com/joelee/passalong/main/deploy/ssh-server/compose.yaml
curl -fsSL -o .env https://raw.githubusercontent.com/joelee/passalong/main/deploy/ssh-server/.env.sample
```

Edit `.env`:

| Variable | Set it to |
|---|---|
| `PUID`, `PGID` | The output of `id -u` and `id -g`, so stored files belong to you |
| `PASSALONG_STORAGE` | Where items are stored on the host, for example `/srv/passalong` |
| `PASSALONG_SSH_PORT` | The port clients connect to; default 2222 |

The storage directory must exist and belong to that user.

## 2. Add your client devices' keys

Copy each client's public key into `keys/`, one file per device:

```sh
scp laptop:.ssh/id_ed25519.pub keys/laptop.pub
```

Every file in `keys/` is added to the server's authorized keys when the
container starts.

## 3. Start the server

```sh
docker compose up -d
docker compose ps
```

Wait until the service is `healthy`. On first start the container creates
`config/`, which holds the server's host keys. Keep it: if it is lost, the
server gets new host keys and every client refuses to connect.

## 4. Read the server's host key fingerprint

```sh
docker compose exec passalong-sshd ssh-keygen -lf /config/ssh_host_keys/ssh_host_ed25519_key.pub
```

Note the `SHA256:...` part. Clients compare it before trusting the server.

## 5. Set up each client

On each client device, run:

```sh
passalong init --host <server> --port 2222 --user passalong --remote-path /data
```

Confirm the fingerprint only if it matches the one from step 4. `init`
writes the config and tests the connection. For a scripted setup, pass
`--fingerprint SHA256:... --yes` instead of confirming by hand.

## Exposing the server

- Keep the port on your local network or a VPN such as WireGuard or
  Tailscale where you can. Only key-based login is enabled, and passwords
  are refused, but an unexposed port is safer still.
- If clients connect over the internet, forward only the SSH port and
  consider changing it from 2222.

## Adding and removing devices

- **Add:** put the new device's public key in `keys/` and run
  `docker compose restart`, then run `passalong init` on the device.
- **Remove:** keys are only ever added at start-up, so delete the device's
  line from `config/.ssh/authorized_keys` and its file from `keys/`.

## Upgrading

Change the image tag in `compose.yaml`, then:

```sh
docker compose pull
docker compose up -d
```

The host keys and authorized keys live in `config/` and survive upgrades,
so clients keep working.

## Backups

Back up the storage directory for your items, and `config/` for the host
keys. Restoring both on a new machine keeps every client working without
changes.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `host key mismatch` on a client | `config/` was lost or replaced, so the server has new host keys. Check the new fingerprint as in step 4, then run `passalong init --force` on each client. |
| `rejected the key … for user passalong` | The client's public key is not in `keys/`, or the container was not restarted after adding it. |
| `permission denied` when storing items | The storage directory does not belong to `PUID`/`PGID`. Fix its owner, or the values in `.env`. |
| The port is already in use | Set another `PASSALONG_SSH_PORT` in `.env` and use it with `init --port`. |
