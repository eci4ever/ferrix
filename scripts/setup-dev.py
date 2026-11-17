#!/usr/bin/env python3
"""Create an ignored development environment for the current macOS user."""
import getpass
import secrets
from pathlib import Path

PROFILES = {"nmfairus": (8080, 5173, 5433), "mac2": (8081, 5174, 5434), "mac3": (8082, 5175, 5435)}

def configure(root, user):
    if user not in PROFILES:
        raise SystemExit(f"No Ferrix development profile for {user}")
    path = root / ".env"
    if path.exists():
        raise SystemExit(".env already exists; preserve it and reconcile with docs/development.md manually.")
    api, web, postgres = PROFILES[user]
    database = f"ferrix_{user}"
    text = f"""COMPOSE_PROJECT_NAME=ferrix-{user}
POSTGRES_DB={database}
POSTGRES_PORT={postgres}
DATABASE_URL=postgres://ferrix:ferrix@127.0.0.1:{postgres}/{database}
FERRIX_BIND=127.0.0.1:{api}
FERRIX_WEB_PORT={web}
FERRIX_AUTH_ORIGIN=http://127.0.0.1:{web}
FERRIX_AUTH_KEY={secrets.token_hex(32)}
FERRIX_SETUP_TOKEN={secrets.token_hex(32)}
RUST_LOG=ferrix_api=info,tower_http=info
"""
    socket = Path.home() / ".orbstack/run/docker.sock"
    if socket.exists():
        text += f"DOCKER_HOST=unix://{socket}\n"
    with path.open("x") as file:
        file.write(text)
    path.chmod(0o600)
    print(f"Created {path} for {user}: API {api}, web {web}, PostgreSQL {postgres}")

if __name__ == "__main__":
    configure(Path(__file__).resolve().parent.parent, getpass.getuser())
