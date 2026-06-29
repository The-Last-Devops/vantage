# Protecting a public Vantage hub (exposure check)

Vantage authenticates every human/API request (session cookie or PAT), but if the hub
is reachable directly on the public internet the **login page and `/api` surface are
exposed** — a brute-force/zero-day target. Best practice for a self-hosted control
plane is to put an **auth gate in front** so the outside world can't even reach the app,
**while still letting agents through**.

## The one rule: allow `/pub/*` through the gate

Agents and install assets are unauthenticated by design and live under `/pub/*`:

- `POST /pub/ingest` — agents push metrics (per-server `x-api-key`)
- `GET /pub/push/{token}` — push-monitor heartbeats
- `GET /pub/tunnel` — the agent reverse tunnel (shell/exec)
- `GET /pub/agent.yaml`, `GET /pub/install.sh` — install assets

Your gate **must bypass `/pub/*`** (and only that) — everything else (the SPA, `/api/*`)
should require passing the gate.

## The exposure self-check

Go to **Settings → Security → Public exposure → Check now**. The hub works out its public
URL automatically from the request (`X-Forwarded-Proto`/`-Host`, or `Host`) — set
`PUBLIC_URL` only to override. It then fetches `<public-url>/exposure-check` (a marker
endpoint that is **not** under `/pub`) with no credentials:

- **Blocked (302/401/403)** → a gate is protecting the app. ✅
- **200 + marker** → the app is open to the internet with no gate. ⚠️ Configure one below.

## Option A — Cloudflare Zero Trust (Access)

1. Put the hub behind Cloudflare (proxied DNS / Tunnel).
2. Cloudflare Zero Trust → **Access → Applications** → add a self-hosted app for your
   hostname with an allow policy (email/IdP).
3. Add a **Bypass** policy for the path `/pub/*` (Everyone) so agents aren't challenged.

## Option B — nginx basic auth

```nginx
server {
    server_name vantage.example.com;

    # agents + install assets: open
    location /pub/ {
        proxy_pass http://127.0.0.1:8080;
    }

    # everything else: behind basic auth
    location / {
        auth_basic "Vantage";
        auth_basic_user_file /etc/nginx/.htpasswd;   # htpasswd -c … admin
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $remote_addr;
    }
}
```

Either way, re-run the exposure check afterwards — it should now report **protected**.

> A VPN / Tailscale / WireGuard in front (so the hub isn't publicly routable at all) is
> equally good; the agents then reach `/pub` over the same private network or a separate
> public ingress.
