/** Browser tab titles: most specific first, the product last ("Inbox · Acme Robotics · swarmboard"). */
export function setTitle(...parts: (string | null | undefined | false)[]) {
  document.title = [...parts.filter(Boolean), 'swarmboard'].join(' · ')
}
