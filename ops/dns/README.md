# DNS (swarmboard.app at Porkbun)

The domain is registered and hosted at Porkbun (nameservers `*.ns.porkbun.com`). The desired records live in
`records.json`. Once `ops/email/setup-resend.sh` has written `ops/email/dns-records.json`, the Resend records come
from that file instead.

| Type | Name | Value | For |
|---|---|---|---|
| A | `@` | `76.76.21.21` | Vercel (web) |
| CNAME | `www` | `cname.vercel-dns.com` | Vercel (set the www to apex redirect in the Vercel project) |
| A | `api` | `46.225.177.152` | Hetzner `0phone-runtime`, where Caddy terminates TLS |
| MX 10 | `send` | `feedback-smtp.us-east-1.amazonses.com` | Resend bounces |
| TXT | `send` | `v=spf1 include:amazonses.com ~all` | Resend SPF |
| TXT | `resend._domainkey` | `p=…` | Resend DKIM. **Pending** until the domain is added to Resend |
| TXT | `_dmarc` | `v=DMARC1; p=none;` | DMARC, monitor only |

There is no CAA record on purpose: one could block certificates from Vercel or Let's Encrypt. TTL is 600, Porkbun's minimum.

## Scripts

```sh
ops/dns/porkbun-apply.sh           # dry run: show the plan
ops/dns/porkbun-apply.sh --apply   # make the zone match, then re-read it
ops/dns/check.sh                   # dig each record on a Porkbun NS and on 1.1.1.1: OK / MISSING / WRONG / PENDING
ops/dns/desired.sh                 # print the merged desired records as JSON
```

- **Credentials.** The scripts read `PORKBUN_API_KEY` and `PORKBUN_SECRET_API_KEY` from the environment, or from
  `~/.config/swarmboard/porkbun.env` (mode 600, never in the repo). "API Access" must be on for the domain in Porkbun.
- **What apply changes.** It creates or edits only the records listed above. It deletes only two kinds of records:
  ones at `@` or `www` that conflict with the targets (ALIAS, CNAME, AAAA, or another A), and Porkbun's parking
  defaults (`pixie.porkbun.com`, the `*` wildcard). It also deletes URL forwards at `@` or `www`.
- **What apply leaves alone.** NS, any MX at `@` (email forwarding) and anything else stay untouched. Apply lists
  them as `keep`, and anything suspicious as `WARN`.
- **Running it again** is safe. When the zone already matches, it changes nothing.
- **Adding DKIM.** After `ops/email/setup-resend.sh`, run `porkbun-apply.sh --apply` again to publish the DKIM key,
  then `ops/email/setup-resend.sh verify`.

The parking page that used to be at the apex was `pixie.porkbun.com` (207.207.210.107 and .229).
