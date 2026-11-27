# Keeping secrets local

Nothing secret belongs in this repository. Where each secret lives:

| Secret | Lives in | Notes |
|---|---|---|
| Camera UID, password, auth key, RTSP password | `.env` on the Docker host (mode 600) | Made by `scripts/fetch_credentials.py`; gitignored |
| Owlet account password | Nowhere | Typed once into the script, never saved |
| Frigate's copy of the RTSP password | `/opt/frigate/.env` on the host | As `FRIGATE_OWLET_PASSWORD` |
| Hostnames, LAN IPs, handoff notes | `CLAUDE.md` (local only) | Gitignored |

Protections:

- `.gitignore` excludes `.env*` (except `.env.example`) and `CLAUDE.md`.
- `.githooks/pre-commit` refuses staged `.env` files, secret-looking assignments, private IPs and tokens.
  Enable it after cloning: `git config core.hooksPath .githooks`
- At runtime secrets reach `owlet-tutk` only through its environment, never command-line arguments.
- Docs use placeholders such as `<home-assistant-ip>`.

If a secret is ever committed: change it first (re-run the credential script, change the Owlet password),
then rewrite history. Deleting the file in a new commit does not remove it.
