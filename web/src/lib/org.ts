/** Top-level paths an org slug can never take (mirrors the server's RESERVED_SLUGS). */
export const RESERVED_SLUGS = new Set(
  'app api auth join new orgs org f u p mcp healthz assets brand fonts crew login logout signup settings admin www docs help about terms privacy static invite connect github slug'.split(
    ' ',
  ),
)

/** "swarmboard.app" in production, "localhost:5191" locally. */
export const HOST = typeof window !== 'undefined' ? window.location.host : 'swarmboard.app'
