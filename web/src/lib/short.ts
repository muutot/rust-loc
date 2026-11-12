// Cut any scan root off an absolute path so labels name the file, not half a
// screen of the same directory prefix. Roots arrive with Windows separators
// while report paths use forward slashes, so both sides are normalized.
export function shortPath(roots: string[], path: string): string {
  const out = path.replace(/\\/g, '/')
  for (const raw of roots) {
    const root = raw.replace(/\\/g, '/').replace(/\/+$/, '')
    if (root && out.toLowerCase().startsWith(root.toLowerCase() + '/')) {
      return out.slice(root.length + 1)
    }
  }
  return out
}
