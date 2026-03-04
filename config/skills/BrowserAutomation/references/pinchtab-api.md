# Pinchtab API Reference

Base URL for all examples: `http://localhost:9867`

> **Auth:** All requests require `-H "Authorization: Bearer $BRIDGE_TOKEN"`.
>
> **Multi-tab:** All endpoints use flat paths. Target a specific tab with `?tabId=ID` query parameter or `"tabId":"ID"` in POST body. Without `tabId`, the active (most recently used) tab is targeted.

## Navigate

```bash
# Navigate in a new tab (returns tabId)
curl -X POST /navigate \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"url": "https://example.com", "newTab": true}'
# Response: {"tabId":"ABC123","title":"Example","url":"https://example.com/"}

# Navigate in an existing tab
curl -X POST /navigate \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"url": "https://example.com", "tabId": "ABC123"}'
# Response: {"tabId":"ABC123","title":"Example","url":"https://example.com/"}

# With options: custom timeout, block images
curl -X POST /navigate \
  -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"url": "https://example.com", "newTab": true, "timeout": 60, "blockImages": true}'
```

**IMPORTANT:** Without `tabId`, navigates the active (most recently used) tab. With `"newTab": true`, a new tab is created and its ID is returned.

## Snapshot (accessibility tree)

```bash
# Token-efficient defaults (recommended)
curl "/snapshot?tabId=ABC123&format=compact&filter=interactive&maxTokens=2000" \
  -H "Authorization: Bearer $BRIDGE_TOKEN"

# Diff snapshot (only changes since last snapshot for this tab)
curl "/snapshot?tabId=ABC123&format=compact&filter=interactive&diff=true" \
  -H "Authorization: Bearer $BRIDGE_TOKEN"

# Scope to CSS selector
curl "/snapshot?selector=main" -H "Authorization: Bearer $BRIDGE_TOKEN"

# Disable animations before capture
curl "/snapshot?noAnimations=true" -H "Authorization: Bearer $BRIDGE_TOKEN"

# Limit tree depth
curl "/snapshot?depth=3" -H "Authorization: Bearer $BRIDGE_TOKEN"
```

**Query params** (all optional):
- `tabId` — target tab (omit for active tab)
- `format=compact` — most token-efficient (NOT `compact=true`)
- `filter=interactive` — only interactive elements (NOT `interactive=true`)
- `diff=true` — only changes since last snapshot
- `maxTokens=2000` — truncate output
- `selector=main` — CSS selector to scope the tree
- `depth=N` — max tree depth
- `noAnimations=true` — disable CSS animations before capture

Returns flat JSON array of nodes with `ref`, `role`, `name`, `depth`, `value`, `nodeId`.

## Act on elements

```bash
# Click by ref
curl -X POST /action -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"kind": "click", "ref": "e5", "tabId": "ABC123"}'

# Click and wait for navigation
curl -X POST /action -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"kind": "click", "ref": "e5", "tabId": "ABC123", "waitNav": true}'

# Type into element
curl -X POST /action -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"kind": "type", "ref": "e12", "text": "hello world", "tabId": "ABC123"}'

# Press a key
curl -X POST /action -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"kind": "press", "key": "Enter", "tabId": "ABC123"}'

# Fill (set value directly, no keystrokes)
curl -X POST /action -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"kind": "fill", "selector": "#email", "text": "user@example.com", "tabId": "ABC123"}'

# Hover (trigger dropdowns/tooltips)
curl -X POST /action -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"kind": "hover", "ref": "e8", "tabId": "ABC123"}'

# Select dropdown option
curl -X POST /action -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"kind": "select", "ref": "e10", "value": "option2", "tabId": "ABC123"}'

# Scroll by pixels (infinite scroll pages)
curl -X POST /action -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"kind": "scroll", "scrollY": 800, "tabId": "ABC123"}'

# Scroll to element
curl -X POST /action -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"kind": "scroll", "ref": "e20", "tabId": "ABC123"}'

# Focus
curl -X POST /action -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"kind": "focus", "ref": "e3", "tabId": "ABC123"}'
```

## Batch actions

```bash
curl -X POST /actions -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"actions":[{"kind":"click","ref":"e3"},{"kind":"type","ref":"e3","text":"hello"},{"kind":"press","key":"Enter"}],"stopOnError":true,"tabId":"ABC123"}'
```

## Extract text

```bash
# Readability mode (strips nav/footer/ads)
curl "/text?tabId=ABC123" -H "Authorization: Bearer $BRIDGE_TOKEN"

# Raw innerText
curl "/text?tabId=ABC123&mode=raw" -H "Authorization: Bearer $BRIDGE_TOKEN"
```

Returns `{url, title, text}`. Cheapest option (~1K tokens for most pages).

## Tab management

```bash
# List tabs
curl /tabs -H "Authorization: Bearer $BRIDGE_TOKEN"
# Response: {"tabs":[{"id":"ABC","title":"...","url":"...","type":"page"}]}
# Locked tabs also include "owner" and "lockedUntil" fields

# Open new tab
curl -X POST /tab -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"action": "new", "url": "https://example.com"}'
# Response: {"tabId":"NEW_ID",...}

# Open blank tab
curl -X POST /tab -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"action": "new"}'

# Close tab
curl -X POST /tab -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"action": "close", "tabId": "TARGET_ID"}'
```

## Tab locking

```bash
# Lock a tab
curl -X POST /tab/lock -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"tabId": "ABC123", "owner": "my-session-id", "timeoutSec": 120}'
# Success: {"tabId":"ABC123","owner":"my-session-id","lockedUntil":"..."}
# Conflict (409): {"error":"tab ABC123 is locked by other-owner for another 9m30s"}

# Unlock a tab (owner must match)
curl -X POST /tab/unlock -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"tabId": "ABC123", "owner": "my-session-id"}'

# Renew lock (re-lock with same owner extends TTL)
curl -X POST /tab/lock -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"tabId": "ABC123", "owner": "my-session-id", "timeoutSec": 120}'
```

**Critical field names:**
- `timeoutSec` (NOT `ttl`, NOT `timeout`)
- `tabId` (NOT `tab_id`, NOT `id`)
- `owner` (free-form string for identity matching)

**Defaults:**
- `timeoutSec` omitted or 0 → server default of **10 minutes**

## Screenshot

```bash
# Save directly (raw bytes)
curl "/screenshot?tabId=ABC123&raw=true" -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -o /shared/screenshot.png

# Lower quality JPEG
curl "/screenshot?tabId=ABC123&raw=true&quality=50" -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -o /shared/screenshot.jpg
```

## PDF export

```bash
# Save to disk (path must be under /shared/.pinchtab/)
curl "/pdf?tabId=ABC123&output=file&path=/shared/.pinchtab/page.pdf" \
  -H "Authorization: Bearer $BRIDGE_TOKEN"
# Then copy: cp /shared/.pinchtab/page.pdf /shared/page.pdf

# Raw PDF bytes
curl "/pdf?tabId=ABC123&raw=true" -H "Authorization: Bearer $BRIDGE_TOKEN" -o page.pdf

# Landscape with custom scale
curl "/pdf?tabId=ABC123&landscape=true&scale=0.8&raw=true" \
  -H "Authorization: Bearer $BRIDGE_TOKEN" -o page.pdf
```

**Query params:** `tabId`, `paperWidth`, `paperHeight`, `landscape`, `marginTop/Bottom/Left/Right`, `scale` (0.1–2.0), `pageRanges`, `displayHeaderFooter`, `headerTemplate`, `footerTemplate`, `preferCSSPageSize`, `output` (file/JSON), `path`, `raw`.

**Note:** When using `output=file`, the `path` must be under `/shared/.pinchtab/`. Copy the file to `/shared/` afterwards.

## Download files

```bash
# Save directly to disk (must be under /shared/.pinchtab/)
curl "/download?url=https://site.com/export.csv&output=file&path=/shared/.pinchtab/export.csv" \
  -H "Authorization: Bearer $BRIDGE_TOKEN"
# Then copy: cp /shared/.pinchtab/export.csv /shared/export.csv

# Raw bytes
curl "/download?url=https://site.com/image.jpg&raw=true" \
  -H "Authorization: Bearer $BRIDGE_TOKEN" -o /shared/image.jpg
```

## Upload files

```bash
curl -X POST "/upload" -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"selector": "input[type=file]", "paths": ["/shared/photo.jpg"], "tabId": "ABC123"}'
```

## Evaluate JavaScript

```bash
curl -X POST /evaluate -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"expression": "document.title", "tabId": "ABC123"}'
```

## Cookies

```bash
# Get cookies for current page
curl "/cookies?tabId=ABC123" -H "Authorization: Bearer $BRIDGE_TOKEN"

# Set cookies
curl -X POST /cookies -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://example.com","cookies":[{"name":"session","value":"abc123"}]}'
```

## Stealth / Fingerprint

```bash
# Check stealth status
curl /stealth/status -H "Authorization: Bearer $BRIDGE_TOKEN"

# Rotate fingerprint
curl -X POST /fingerprint/rotate -H "Authorization: Bearer $BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"os":"windows"}'
```

## Health check

```bash
curl /health -H "Authorization: Bearer $BRIDGE_TOKEN"
# Response: {"status":"ok","tabs":1}
```

## Known Documentation Errors

| Source | Error | Correct Value |
|--------|-------|---------------|
| pinchtab.com | Lock field: `ttl` | `timeoutSec` |
| pinchtab.com | `GET /tabs` returns flat array | Returns `{"tabs": [...]}` |
| pinchtab.com | Snapshot params: `interactive=true`, `compact=true` | `filter=interactive`, `format=compact` |
