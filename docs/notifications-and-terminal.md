# Notifications and service terminal

## Notifications

The header bell opens an organization inbox. Deployment, database backup and
restore transitions to succeeded/failed generate notifications atomically with
the status update. Restore bootstrap deployments are hidden so a restore produces
one outcome. Existing historical jobs are not replayed. Notifications contain
service/project/environment labels, never deployment logs, database credentials
or error details that may contain secrets.

Every organization member can view its notifications. Read state belongs to the
individual user; marking an event read does not dismiss it for colleagues.
The latest 100 events are shown, with an unread count across all unexpired events.
Events expire after 30 days and existing bounded housekeeping removes them and
associated read receipts. Deleting a service leaves its notification text but
removes the service link; deleting its project removes the related events.

A background worker samples only Ferrix-owned active service containers. CPU and
RAM alerts compare usage with explicit container limits, not the whole host.
At least 90% over a 60-second sampling window emits one alert; it re-arms below
80%. Missing readings, a sample gap over two minutes or a different active
container reset the window. No limit or unavailable data produces no alert.
Sampling checks up to 32 services per 30-second round, rotating through services,
with at most four concurrent Docker reads. This keeps daemon overhead bounded;
large installations may need longer sampling windows or a dedicated metrics
pipeline. The browser polls the inbox every 15 seconds while visible.

Browser-session endpoints:

- `GET /api/v1/notifications`
- `POST /api/v1/notifications/{id}/read`
- `POST /api/v1/notifications/read-all`

API keys cannot read or manage the browser inbox.

## Interactive terminal

Owners/admins can choose **Open terminal** from a running service's Overview.
The dialog loads xterm.js only when opened and attaches a Docker exec PTY through
`GET /api/v1/services/{id}/terminal` over same-origin WebSocket. It uses `/bin/sh`
and the container's configured user, without privileged exec or host terminal
access. The image needs `/bin/sh` and writable `/tmp`; shell-free images are not
supported. A reverse proxy in front of Ferrix must forward WebSocket upgrades.
The Vite development proxy forwards upgrades without rewriting Origin.

The handshake requires an authenticated browser cookie, owner/admin membership,
exact matching Origin, organization access to the service and verified Ferrix
ownership labels on its active container. API keys and members are denied.
The container is chosen server-side, never from a client-supplied container ID.
Session validity, organization, role and active container are checked every ten
seconds; logout, expiry, ban, membership changes, switching organizations,
service stop or redeploy close the terminal at the next check.

Limits: two connections per account, sixteen per API process, 16 KiB WebSocket
messages, five minutes without keyboard input and thirty minutes total. PTY
resize accepts 2–300 columns and 2–120 rows. Slow Docker/socket operations time
out. There is no automatic reconnect or command replay. Close the dialog to
disconnect; Reconnect explicitly creates a new shell. Terminal output is rendered
as terminal text, never HTML, and no web-link/clipboard addons are enabled.

Only connection metadata is retained in `terminal_connections` for 30 days.
Commands, output and cookies are not stored. A 60-second renewable lease and
background cleanup recover expired sessions after API interruption. Cleanup
checks container ownership and a unique shell environment marker before sending
HUP to that shell. If Docker is unavailable cleanup is retried. Changes to files
remain in the container; deliberately detached jobs can outlive a shell.
