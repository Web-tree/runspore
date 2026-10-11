// C17 PROTOTYPE, throwaway. Shared by the mod and `bun hooks/try-text.ts`:
// the scripted run, the DOT display transform, the text drawing and the file highlights.

// What `spore status --json` would say at each moment of one run of the C11 coding loop.
// Fields marked NEW are what C17 proposes adding to status 0.2.
export type Frame = {
  caption: string
  status: { status: string; needs: string; nodeId: string; visit: number; from: { nodeId: string; outcome: string } | null; attempt: number | null; maxAttempts: number | null; streak: { outcome: string; count: number; limit: number } | null }
}

const f = (caption: string, nodeId: string, visit: number, from: [string, string] | null, needs: string, attempt: number | null, streak: number | null, status = 'running'): Frame => ({
  caption,
  status: {
    status, needs, nodeId, visit,
    from: from ? { nodeId: from[0], outcome: from[1] } : null,
    attempt, maxAttempts: attempt === null ? null : 3,
    streak: streak === null ? null : { outcome: 'red', count: streak, limit: 3 },
  },
})

export const FRAMES: Frame[] = [
  f('The run starts. Claude implements the task in the background.', 'implement', 1, null, 'working', 1, null),
  f('The session was killed mid-step. Try 2 resumes the same Claude conversation.', 'implement', 1, null, 'working', 2, null),
  f('Claude handed in a summary. The tests run.', 'test', 1, ['implement', 'ok'], 'working', 1, null),
  f('Tests failed (red 1 of 3). Back to Claude, visit 2.', 'implement', 2, ['test', 'red'], 'working', 1, 1),
  f('Tests run again.', 'test', 2, ['implement', 'ok'], 'working', 1, 1),
  f('Red again (2 of 3). Claude, visit 3.', 'implement', 3, ['test', 'red'], 'working', 1, 2),
  f('Tests run again.', 'test', 3, ['implement', 'ok'], 'working', 1, 2),
  f('Red again (3 of 3). Claude, visit 4.', 'implement', 4, ['test', 'red'], 'working', 1, 3),
  f('Tests run again.', 'test', 4, ['implement', 'ok'], 'working', 1, 3),
  f('A fourth red in a row: the loop is used up. The run waits for you.', 'tests_stuck', 1, ['test', 'exhausted'], 'signal', null, null, 'waiting'),
  f('You said keep going. Claude, visit 5, with your hint.', 'implement', 5, ['tests_stuck', 'keep-going'], 'working', 1, null),
  f('Tests run again.', 'test', 5, ['implement', 'ok'], 'working', 1, null),
  f('Green. The run waits for your approval.', 'approve', 1, ['test', 'ok'], 'signal', null, null, 'waiting'),
  f('You approved. The run is done.', 'done', 1, ['approve', 'approved'], 'none', null, null, 'completed'),
]

// Display only (as C11's prototype): each edge shows its `on`, an explicit label wins; end nodes get a double outline.
// `here` and `taken` add the highlight for the picture variants.
export function forDisplay(dot: string, here?: string, taken?: { from: string; to: string; on: string }, forText = false) {
  let out = dot
    .replace(/^(\s*)(\w+)(\s*->\s*)(\w+)(\s*)(\[([^\]]*)\])?;/gm, (_m, ind, a, arrow, b, sp, _br, attrs = '') => {
      const on = (attrs.match(/on="([^"]+)"/) || [])[1] || 'ok'
      const label = (attrs.match(/label="([^"]+)"/) || [])[1]
      const isTaken = taken && taken.from === a && taken.to === b && taken.on === on
      const extra = isTaken ? ', color="#e8590c", fontcolor="#e8590c", penwidth=3' : ''
      const shown = label || (attrs.includes('on=') ? on : '')
      return `${ind}${a}${arrow}${b}${sp}[label="${shown}"${extra}];`
    })
    .replace(/(\w+)(\s+)\[kind="(complete|fail)"\]/g, '$1$2[shape=doublecircle, label="$1"]')
    .replace(/digraph "([^"]+)" \{/, 'digraph "$1" {\n  node [shape=box, style=rounded, fontname="Helvetica", fontsize=12];\n  edge [fontname="Helvetica", fontsize=10];\n  ' + (forText ? 'nodesep=0.9; ranksep=1.0;' : 'nodesep=0.4; ranksep=0.5; bgcolor="white"; pad=0.15;'))
  if (here) {
    out = out.replace(/\n\}\s*$/, `\n  ${here} [style="rounded,filled,bold", fillcolor="#ffd43b", penwidth=2];\n}\n`)
  }
  return out
}

// ---------- text drawing from `dot -Tplain` ----------

export type Style = 'node' | 'here' | 'edge' | 'taken' | 'label' | 'takenLabel' | 'blank'
export type Cell = { ch: string; st: Style }

function tokens(line: string) {
  const out: string[] = []
  const re = /"((?:[^"\\]|\\.)*)"|(\S+)/g
  let m
  while ((m = re.exec(line))) out.push(m[1] ?? m[2])
  return out
}

type PNode = { id: string; x: number; y: number; w: number; h: number; label: string }
type PEdge = { from: string; to: string; pts: [number, number][]; label?: string; lx?: number; ly?: number }

export function parsePlain(plain: string) {
  let W = 0, H = 0
  const nodes: PNode[] = [], edges: PEdge[] = []
  for (const line of plain.split('\n')) {
    const t = tokens(line)
    if (t[0] === 'graph') { W = +t[2]; H = +t[3] }
    if (t[0] === 'node') nodes.push({ id: t[1], x: +t[2], y: +t[3], w: +t[4], h: +t[5], label: t[6] })
    if (t[0] === 'edge') {
      const n = +t[3], pts: [number, number][] = []
      for (let i = 0; i < n; i++) pts.push([+t[4 + 2 * i], +t[5 + 2 * i]])
      const rest = t.slice(4 + 2 * n)
      const e: PEdge = { from: t[1], to: t[2], pts }
      if (rest.length >= 5) { e.label = rest[0]; e.lx = +rest[1]; e.ly = +rest[2] }
      edges.push(e)
    }
  }
  return { W, H, nodes, edges }
}

function bez(p: [number, number][], t: number): [number, number] {
  const u = 1 - t
  return [
    u * u * u * p[0][0] + 3 * u * u * t * p[1][0] + 3 * u * t * t * p[2][0] + t * t * t * p[3][0],
    u * u * u * p[0][1] + 3 * u * u * t * p[1][1] + 3 * u * t * t * p[2][1] + t * t * t * p[3][1],
  ]
}

// Lay the plain layout onto a grid of `cols` columns; terminal cells are about twice as tall as wide.
export function drawText(plain: string, cols: number, here: string | null, taken: { from: string; to: string; on: string } | null, onOf: (from: string, to: string, label?: string) => string): Cell[][] {
  const { W, H, nodes, edges } = parsePlain(plain)
  // Wide enough that boxes on one rank never overlap, whatever `cols` asked for.
  let sx = (cols - 2) / W
  for (const a of nodes) for (const b of nodes) {
    if (a === b || Math.abs(a.y - b.y) > 0.05 || b.x <= a.x) continue
    sx = Math.max(sx, ((a.label.length + 6) / 2 + (b.label.length + 6) / 2 + 1) / (b.x - a.x))
  }
  cols = Math.ceil(W * sx) + 2
  const sy = sx / 3
  const rows = Math.round(H * sy) + 2
  const grid: Cell[][] = Array.from({ length: rows }, () => Array.from({ length: cols }, () => ({ ch: ' ', st: 'blank' as Style })))
  const cx = (x: number) => Math.round(x * sx) + 1
  const cy = (y: number) => Math.round((H - y) * sy) + 1
  const put = (r: number, c: number, ch: string, st: Style, force = false) => {
    if (r < 0 || r >= rows || c < 0 || c >= cols) return
    const cell = grid[r][c]
    if (!force && (cell.st === 'node' || cell.st === 'here')) return
    if (!force && cell.st === 'taken' && st === 'edge') return
    grid[r][c] = { ch, st }
  }
  // Node boxes first, so edges stop at them.
  const boxes = new Map<string, { r: number; c0: number; c1: number }>()
  for (const n of nodes) {
    const text = ` ${n.label} `
    const r = cy(n.y)
    const c0 = Math.min(cols - text.length - 2, Math.max(0, cx(n.x) - Math.floor((text.length + 2) / 2)))
    const st: Style = n.id === here ? 'here' : 'node'
    const s = `[${text}]`
    for (let i = 0; i < s.length; i++) put(r, c0 + i, s[i], st, true)
    boxes.set(n.id, { r, c0, c1: c0 + s.length - 1 })
  }
  // Edges: sample the splines, pick a glyph from the step direction, arrowhead at the end.
  const ordered = [...edges].sort((a, b) => Number(isTaken(a)) - Number(isTaken(b)))
  function isTaken(e: PEdge) { return !!taken && taken.from === e.from && taken.to === e.to && taken.on === onOf(e.from, e.to, e.label) }
  for (const e of ordered) {
    const st: Style = isTaken(e) ? 'taken' : 'edge'
    const cells: [number, number][] = []
    for (let s = 0; s + 3 < e.pts.length; s += 3) {
      const seg = e.pts.slice(s, s + 4)
      for (let i = 0; i <= 40; i++) {
        const [x, y] = bez(seg, i / 40)
        const c: [number, number] = [cy(y), cx(x)]
        const last = cells[cells.length - 1]
        if (!last || last[0] !== c[0] || last[1] !== c[1]) cells.push(c)
      }
    }
    // One column per row the edge crosses, so a mostly vertical edge reads as one clean line.
    const byRow = new Map<number, number[]>()
    for (const [r, c] of cells) {
      if (!grid[r]?.[c] || grid[r][c].st === 'node' || grid[r][c].st === 'here') continue
      if (!byRow.has(r)) byRow.set(r, [])
      byRow.get(r)!.push(c)
    }
    const goingDown = cells.length > 1 && cells[cells.length - 1][0] >= cells[0][0]
    const rowsHit = [...byRow.keys()].sort((a, b) => (goingDown ? a - b : b - a))
    const col = (r: number) => { const cs = byRow.get(r)!; return Math.round(cs.reduce((a, b) => a + b, 0) / cs.length) }
    for (let i = 0; i < rowsHit.length; i++) {
      const r = rowsHit[i], c = col(r)
      const isEnd = i === rowsHit.length - 1
      if (isEnd) { put(r, c, goingDown ? '▼' : '▲', st); continue }
      const c2 = col(rowsHit[i + 1])
      const d = goingDown ? c2 - c : c2 - c
      const down = goingDown
      if (d === 0) put(r, c, '│', st)
      else {
        // slant toward the next row, with a horizontal run when it jumps more than one column
        const slant = (d > 0) === down ? '╲' : '╱'
        put(r, c, slant, st)
        const step = d > 0 ? 1 : -1
        for (let k = c + step; k !== c2 && Math.abs(k - c) < Math.abs(d); k += step) put(r, k, '─', st)
      }
    }
    if (e.label && e.lx !== undefined && e.ly !== undefined) {
      const r = cy(e.ly), c0 = cx(e.lx) - Math.floor(e.label.length / 2)
      for (let i = 0; i < e.label.length; i++) put(r, c0 + i, e.label[i], st === 'taken' ? 'takenLabel' : 'label')
    }
  }
  return grid
}

// Runs of one style per row, for drawing as coloured Text spans or ANSI.
export function runs(row: Cell[]) {
  const out: { text: string; st: Style }[] = []
  for (const cell of row) {
    const last = out[out.length - 1]
    if (last && last.st === cell.st) last.text += cell.ch
    else out.push({ text: cell.ch, st: cell.st })
  }
  return out
}

// ---------- which lines of which file to light up ----------

export type Excerpt = { file: string; lines: { n: number; text: string; mark: 'here' | 'taken' | null }[] }

export function dotExcerpt(dot: string, here: string, taken: { from: string; to: string; on: string } | null): Excerpt {
  const lines = dot.split('\n').map((text, i) => {
    const node = new RegExp(`^\\s*${here}\\s*\\[`).test(text)
    let edge = false
    if (taken) {
      const m = text.match(/^\s*(\w+)\s*->\s*(\w+)\s*(\[[^\]]*\])?/)
      if (m && m[1] === taken.from && m[2] === taken.to) edge = ((m[3] || '').match(/on="([^"]+)"/) || [])[1] === taken.on || (!m[3]?.includes('on=') && taken.on === 'ok')
    }
    return { n: i + 1, text, mark: node ? ('here' as const) : edge ? ('taken' as const) : null }
  })
  return { file: 'workflow.dot', lines: lines.filter(l => l.text.trim() && !l.text.trim().startsWith('//')) }
}

// The current node's block under "nodes" in settings.json, plus its action's block.
export function settingsExcerpt(settings: string, here: string): Excerpt {
  const all = settings.split('\n')
  const pick = (startRe: RegExp, after = 0) => {
    const i = all.findIndex((l, k) => k >= after && startRe.test(l))
    if (i < 0) return null
    const indent = all[i].match(/^\s*/)![0].length
    if (/\}\s*,?\s*$/.test(all[i])) return [i, i]
    let j = i + 1
    while (j < all.length && !(all[j].match(/^\s*/)![0].length === indent && /^\s*\}/.test(all[j]))) j++
    return [i, j]
  }
  const nodesAt = all.findIndex(l => /^\s*"nodes"\s*:/.test(l))
  const node = pick(new RegExp(`^\\s*"${here}"\\s*:`), nodesAt)
  const out: Excerpt = { file: 'settings.json', lines: [] }
  if (!node) return out
  const action = (all.slice(node[0], node[1] + 1).join('\n').match(/"action"\s*:\s*"([^"]+)"/) || [])[1]
  if (action) {
    const actionsAt = all.findIndex(l => /^\s*"actions"\s*:/.test(l))
    const a = pick(new RegExp(`^\\s*"${action}"\\s*:`), actionsAt)
    if (a) for (let k = a[0]; k <= a[1]; k++) out.lines.push({ n: k + 1, text: all[k], mark: null })
    out.lines.push({ n: 0, text: '  …', mark: null })
  }
  for (let k = node[0]; k <= node[1]; k++) out.lines.push({ n: k + 1, text: all[k], mark: k === node[0] ? 'here' : null })
  return out
}

export function instructionsFile(settings: string, here: string): string | null {
  try {
    const s = JSON.parse(settings)
    return s.nodes?.[here]?.instructions ?? null
  } catch {
    return null
  }
}

export function onOfFactory(dot: string) {
  // The `on` of an edge as the run reports it (no `on` means "ok").
  return (from: string, to: string, label?: string) => {
    const lines = dot.split('\n').filter(l => new RegExp(`^\\s*${from}\\s*->\\s*${to}\\b`).test(l))
    for (const l of lines) {
      const on = (l.match(/on="([^"]+)"/) || [])[1] || 'ok'
      const lab = (l.match(/label="([^"]+)"/) || [])[1] || (l.includes('on=') ? on : '')
      if (label === undefined || lab === label) return on
    }
    return 'ok'
  }
}

// ---------- finer text drawing: Braille dots for the lines, real text for boxes and labels ----------
// Each cell holds a 2×4 grid of dots, so Graphviz's curves come out smooth at character size.
const DOT_BIT = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]]

export function drawBraille(plain: string, cols: number, here: string | null, taken: { from: string; to: string; on: string } | null, onOf: (from: string, to: string, label?: string) => string): Cell[][] {
  const { W, H, nodes, edges } = parsePlain(plain)
  let sx = (cols - 2) / W
  for (const a of nodes) for (const b of nodes) {
    if (a === b || Math.abs(a.y - b.y) > 0.05 || b.x <= a.x) continue
    sx = Math.max(sx, ((a.label.length + 4) / 2 + (b.label.length + 4) / 2 + 2) / (b.x - a.x))
  }
  cols = Math.ceil(W * sx) + 4
  const sy = sx / 2.5
  const rows = Math.ceil(H * sy) + 3
  const bits = Array.from({ length: rows }, () => new Array(cols).fill(0))
  const hot = Array.from({ length: rows }, () => new Array(cols).fill(false))
  const grid: Cell[][] = Array.from({ length: rows }, () => Array.from({ length: cols }, () => ({ ch: ' ', st: 'blank' as Style })))
  const px = (x: number) => (x * sx + 2) * 2
  const py = (y: number) => ((H - y) * sy + 1.5) * 4
  const isTaken = (e: PEdge) => !!taken && taken.from === e.from && taken.to === e.to && taken.on === onOf(e.from, e.to, e.label)
  const ends: { r: number; c: number; ch: string; hot: boolean }[] = []
  for (const e of edges) {
    const t = isTaken(e)
    let lastP: [number, number] | null = null, prevP: [number, number] | null = null
    for (let s = 0; s + 3 < e.pts.length; s += 3) {
      const seg = e.pts.slice(s, s + 4)
      for (let i = 0; i <= 200; i++) {
        const [x, y] = bez(seg, i / 200)
        const X = Math.floor(px(x)), Y = Math.floor(py(y))
        const r = Math.floor(Y / 4), c = Math.floor(X / 2)
        if (r < 0 || r >= rows || c < 0 || c >= cols) continue
        bits[r][c] |= DOT_BIT[X % 2][Y % 4]
        if (t) hot[r][c] = true
        prevP = lastP; lastP = [X, Y]
      }
    }
    if (lastP && prevP) {
      // Graphviz ends the spline where the arrowhead starts; point it the way the line was going.
      const p0 = e.pts[Math.max(0, e.pts.length - 4)], p1 = e.pts[e.pts.length - 1]
      const dx = px(p1[0]) - px(p0[0]), dy = py(p1[1]) - py(p0[1])
      const ch = Math.abs(dy) >= Math.abs(dx) / 2 ? (dy > 0 ? '▼' : '▲') : dx > 0 ? '▶' : '◀'
      ends.push({ r: Math.floor(lastP[1] / 4), c: Math.floor(lastP[0] / 2), ch, hot: t })
    }
  }
  for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) {
    if (bits[r][c]) grid[r][c] = { ch: String.fromCharCode(0x2800 + bits[r][c]), st: hot[r][c] ? 'taken' : 'edge' }
  }
  for (const a of ends) if (grid[a.r]?.[a.c]) grid[a.r][a.c] = { ch: a.ch, st: a.hot ? 'taken' : 'edge' }
  for (const e of edges) {
    if (!e.label || e.lx === undefined || e.ly === undefined) continue
    const r = Math.floor(py(e.ly) / 4), c0 = Math.floor(px(e.lx) / 2) - Math.floor(e.label.length / 2)
    for (let i = 0; i < e.label.length; i++) if (grid[r]?.[c0 + i]) grid[r][c0 + i] = { ch: e.label[i], st: isTaken(e) ? 'takenLabel' : 'label' }
  }
  for (const n of nodes) {
    const r = Math.floor(py(n.y) / 4), w = n.label.length + 4
    const c0 = Math.max(0, Math.min(cols - w, Math.round(px(n.x) / 2 - w / 2)))
    const st: Style = n.id === here ? 'here' : 'node'
    const lines = ['╭' + '─'.repeat(w - 2) + '╮', '│ ' + n.label + ' │', '╰' + '─'.repeat(w - 2) + '╯']
    lines.forEach((line, k) => {
      for (let i = 0; i < w; i++) if (grid[r - 1 + k]?.[c0 + i]) grid[r - 1 + k][c0 + i] = { ch: line[i], st: k === 1 || n.id === here ? st : 'node' }
    })
  }
  while (grid.length && grid[0].every(x => x.ch === ' ')) grid.shift()
  while (grid.length && grid[grid.length - 1].every(x => x.ch === ' ')) grid.pop()
  return grid
}
