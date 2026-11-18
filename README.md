# Ferrix

Ferrix ialah asas platform deployment kendiri yang dibina menggunakan Rust. Sasaran MVP ialah mengurus aplikasi Docker pada satu server melalui dashboard web.

For three-user setup and assigned ports, see [development setup](docs/development.md).

Authentication setup and security behavior: [Auth guide](docs/auth.md).

## Stack

| Bahagian | Teknologi |
| --- | --- |
| API | Rust, Axum, Tokio |
| Docker | Bollard |
| Data | PostgreSQL, SQLx, migrasi SQL |
| Frontend | React, TypeScript, Vite |
| Routing | TanStack Router |
| UI | shadcn/ui, Tailwind CSS |

Environment, Application, PostgreSQL and Compose deployment flow: [Deployment guide](docs/deployments.md).

Production VPS installation and release workflow: [Production guide](docs/production.md).

## Jalankan secara tempatan

Keperluan: Rust stable, Node.js 24 LTS, npm, Docker dengan Compose.

```sh
python3 scripts/setup-dev.py
docker compose up -d postgres --wait
cargo run -p ferrix-api
```

Dalam terminal kedua:

```sh
cd apps/web
npm ci
npm run dev
```

Dashboard: http://127.0.0.1:5173. API: http://127.0.0.1:8080.
Vite memproksi `/api` kepada Axum. Migrasi PostgreSQL dijalankan semasa API bermula; API memerlukan database yang boleh dihubungi.

Jika Docker menggunakan socket selain laluan biasa, tetapkan `DOCKER_HOST` dalam `.env`. Contoh OrbStack pada macOS:

```sh
DOCKER_HOST=unix:///Users/NAMA_ANDA/.orbstack/run/docker.sock
```

Port PostgreSQL pembangunan ialah `5433`, dengan volume `ferrix_postgres`. Kelayakan dalam Compose adalah untuk pembangunan tempatan sahaja. API mengikat `127.0.0.1` secara lalai. Auth menggunakan sesi cookie dan organisasi; lihat panduan auth sebelum menjalankan API.

## Fungsi sekarang

- Cipta projek dengan environment production, tambah environment dan aplikasi Docker.
- Deploy public Docker image melalui queue persisten, lihat progress dan container logs, serta redeploy dengan container lama dikekalkan jika deployment baharu gagal.
- Semak sambungan PostgreSQL dan Docker secara berasingan.
- Lihat inventori container Docker melalui Bollard (baca sahaja).
- Dashboard dengan navigasi Projects dan Containers, keadaan kosong, ralat dan borang projek.
- Log permintaan dan graceful shutdown untuk Ctrl+C/SIGTERM.

Projek ialah kumpulan aplikasi. Buka projek untuk mengurus environment dan pilih Create Service untuk Application, PostgreSQL atau Compose; simpan konfigurasi dan deploy melalui worker queue. Data UI datang daripada API sebenar, tanpa data contoh.

## API

| Kaedah | Laluan | Tujuan |
| --- | --- | --- |
| GET | `/health` | Liveness proses, bebas daripada servis luar |
| GET | `/api/v1/status` | Status database, Docker dan versi; HTTP 200 ialah respons diagnostik, bukan readiness deployment |
| GET | `/api/v1/projects` | 100 projek terkini |
| POST | `/api/v1/projects` | Cipta projek (`name`, `description` pilihan) |
| GET | `/api/v1/containers` | Inventori Docker; HTTP 503 jika Docker tidak tersedia |

```sh
curl -X POST http://127.0.0.1:8080/api/v1/projects \
  -H 'Origin: http://127.0.0.1:5173' \
  -b cookies.txt \
  -H 'Content-Type: application/json' \
  -d '{"name":"My app","description":"Production services"}'
```

## Struktur

```text
apps/api/          Axum, SQLx, Bollard
  migrations/      Skema PostgreSQL
  src/projects.rs  Model, validasi dan endpoint projek
  tests/           Ujian HTTP dan integrasi PostgreSQL
apps/web/          React, TanStack Router, shadcn/ui
compose.yaml       PostgreSQL pembangunan
```

## Pengesahan

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Dengan PostgreSQL pembangunan berjalan, termasuk ujian persistence:
FERRIX_TEST_DATABASE_URL=postgres://ferrix:ferrix@127.0.0.1:5433/ferrix_nmfairus cargo test --workspace -- --include-ignored
cd apps/web && npm run build
```

## Urutan pembangunan seterusnya

1. Domain dan HTTPS aplikasi melalui reverse proxy, dengan alamat stabil selepas redeploy.
2. Persistent volumes dan private registry credentials.
3. Tindakan stop/restart, rollback serta pemantauan readiness aplikasi.
4. Git/Dockerfile builds, Compose dan server jauh.

API dan worker deployment kini berada dalam satu proses Rust. Kerja Docker berjalan dalam worker queue, bukan handler HTTP. Lihat [deployment guide](docs/deployments.md) untuk had MVP dan pengesahan.

Rujukan: [Dokploy](https://docs.dokploy.com/docs/core), [Axum](https://docs.rs/axum/0.8.9/axum/), [Bollard](https://docs.rs/bollard/0.20.2/bollard/), [TanStack Router](https://tanstack.com/router/latest/docs/framework/react/overview), [shadcn/ui](https://ui.shadcn.com/docs).

## UI

Susun atur dashboard merujuk Dokploy: navigasi sisi dan header ringkas. Tema menggunakan token neutral asal shadcn/ui. Gunakan komponen shadcn serta utiliti Tailwind dengan token semantik; jangan tambah palet atau stylesheet komponen tersuai.

Dashboard menggunakan pola shadcn `sidebar-07`: landing page di `/`, Home dashboard di `/dashboard`, projek di `/projects`, sidebar mod ikon/mobile, pilihan organisasi dan menu akaun. Tema gelap ialah lalai; tukar tema melalui menu akaun. Ringkasan Home menggunakan data API sebenar; statistik deployment belum tersedia.
