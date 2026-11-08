# Email (Resend)

Magic-link emails go out through Resend from `swarmboard <login@swarmboard.app>`.
Code: `server/src/email.rs`. No secrets live in this directory.

## Status

| | |
|---|---|
| Domain | `swarmboard.app` has **not been added to Resend yet** |
| Domain id | none yet (`setup-resend.sh` prints it) |
| Region | `us-east-1` |
| Open/click tracking | will be turned off (tracking rewrites the magic links) |
| Sending key | not created yet. The only existing Resend key is sending-only and can't manage domains or keys |

## Finish setup

1. In the Resend dashboard (API Keys), create a **full access** key and save it:
   ```sh
   mkdir -p ~/.config/swarmboard && chmod 700 ~/.config/swarmboard
   $EDITOR ~/.config/swarmboard/resend-admin.env   # one line: RESEND_ADMIN_KEY=re_...
   chmod 600 ~/.config/swarmboard/resend-admin.env
   ```
2. Run `ops/email/setup-resend.sh`. It adds `swarmboard.app` (or reuses it), turns off open and click
   tracking, and creates a `sending_access` key named `swarmboard-api` that is limited to this domain. It writes
   that key to `~/.config/swarmboard/resend.env` (mode 600) and writes the exact records to
   `ops/email/dns-records.json`. It never prints a key, and running it again is safe.
3. Publish the records at Porkbun. The DNS tooling is in `ops/dns/`.
4. Run `ops/email/setup-resend.sh verify` until the status is `verified`.
5. Put `RESEND_API_KEY` from `~/.config/swarmboard/resend.env` into the production API env. You can delete the
   admin key afterwards.

## DNS records

Names are relative to `swarmboard.app`. TTL is 600, which is Porkbun's minimum.

| Type | Host | Value | Priority |
|---|---|---|---|
| TXT | `resend._domainkey` | `p=…` DKIM public key, issued by Resend when the domain is added (in `dns-records.json`) | |
| MX | `send` | `feedback-smtp.us-east-1.amazonses.com` | 10 |
| TXT | `send` | `v=spf1 include:amazonses.com ~all` | |
| TXT | `_dmarc` | `v=DMARC1; p=none;` (recommended; tighten to `quarantine` once reports look clean) | |

The `send` MX and SPF records cover only the bounce subdomain. They don't affect mail sent to `@swarmboard.app`.

## Local dev

Without `RESEND_API_KEY`, nothing is sent. The server logs the magic link at info level, so you can click it
from the `cargo run` output.

## Template

- The email is table-based with inline styles and a 480px card. The light palette is warm paper
  (`#fafafa`/`#ffffff`/`#111111`, hairlines `#e5e5e5`, button `#2c4a7c`). The warm-dark palette applies through
  `prefers-color-scheme`, plus `[data-ogsc]`/`[data-ogsb]` selectors for Outlook.com. The button is styled for
  Outlook desktop with `mso` conditional comments, and the email includes a plain-text part.
- Each send uses a random `Idempotency-Key` and a random `X-Entity-Ref-ID` header, so Gmail doesn't merge
  sign-in emails into one thread. Requests time out after 10s.
- To preview the HTML, run: `cd server && EMAIL_PREVIEW_DIR=/tmp/sb-email cargo test email::tests::write_previews`.
  It writes `preview-signin.html` and `preview-join.html` (each with a `.txt` next to it).
