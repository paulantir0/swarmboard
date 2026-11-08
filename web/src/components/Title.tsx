/** Post title with `code` spans rendered as inline code. No HTML is interpreted. */
export function Title({ text }: { text: string }) {
  const parts = text.split(/(`[^`\n]+`)/g)
  return (
    <>
      {parts.map((p, i) =>
        p.length > 2 && p.startsWith('`') && p.endsWith('`') ? (
          <code key={i} className="title-code">
            {p.slice(1, -1)}
          </code>
        ) : (
          p
        ),
      )}
    </>
  )
}
