// C17 PROTOTYPE: print the text drawing for every frame in this terminal. `bun hooks/try-text.ts [cols]`
import { FRAMES, forDisplay, drawText, drawBraille, runs, onOfFactory, dotExcerpt, settingsExcerpt } from './view.ts'
const dot = await Bun.file(new URL('../workflow/workflow.dot', import.meta.url)).text()
const settings = await Bun.file(new URL('../workflow/settings.json', import.meta.url)).text()
const p = Bun.spawnSync(['dot', '-Tplain'], { stdin: new TextEncoder().encode(forDisplay(dot, undefined, undefined, true)) })
const plain = p.stdout.toString()
const cols = +(process.argv[2] ?? 56)
const ansi: Record<string, string> = { node: '\x1b[0m', here: '\x1b[1;30;43m', edge: '\x1b[2m', taken: '\x1b[1;38;5;208m', label: '\x1b[2;3m', takenLabel: '\x1b[1;3;38;5;208m', blank: '\x1b[0m' }
const only = process.argv[3] ? [FRAMES[+process.argv[3]]] : FRAMES.slice(0, 2)
for (const fr of only) {
  const s = fr.status
  const taken = s.from ? { from: s.from.nodeId, to: s.nodeId, on: s.from.outcome } : null
  console.log('\n== ' + fr.caption)
  for (const row of (process.env.B ? drawBraille : drawText)(plain, cols, s.nodeId, taken, onOfFactory(dot))) console.log(runs(row).map(r => ansi[r.st] + r.text).join('') + '\x1b[0m')
  for (const l of dotExcerpt(dot, s.nodeId, taken).lines) console.log((l.mark ? '\x1b[1;38;5;208m▶ ' : '  ') + String(l.n).padStart(2) + ' ' + l.text + '\x1b[0m')
  for (const l of settingsExcerpt(settings, s.nodeId).lines) console.log((l.mark ? '▶ ' : '  ') + String(l.n).padStart(2) + ' ' + l.text)
}
