// C17 PROTOTYPE, throwaway: four ways a mod could show a run as it moves, flipped with /c17 1..4.
// The run is scripted (FRAMES in view.ts): what `spore status --json` would say at each moment of
// the C11 coding loop. Graphviz (`dot` on PATH) lays out workflow.dot. Not product code.
import { atom, read, update } from 'claude-code'
import type { Register } from 'claude-code'

import type { ViewState } from '../types'
import { FRAMES, forDisplay, drawText, drawBraille, runs, onOfFactory, dotExcerpt, settingsExcerpt, instructionsFile, type Style, type Excerpt } from './view.ts'

const PANE = 'c17-run'
const OUT = '/private/tmp/c17-live-run-view'
const PORT = 47917
const VARIANTS = [
  { key: 'text', name: 'Text drawing in the pane' },
  { key: 'picture', name: 'Graphviz picture in the pane' },
  { key: 'browser', name: 'Local browser page, pane keeps a summary' },
  { key: 'band', name: 'One line above the prompt, view on demand' },
  { key: 'fine', name: 'Finer text drawing: Braille-dot lines, real text boxes' },
] as const

const view = atom({ plugin: 'c17-live-run-view', key: 'view' } as const, { frame: 0, variant: 'text', playing: true } as ViewState)

let dot = '', settings = '', plain = ''
const instructions = new Map<string, string>()
let ready = false
let browserOpened = false

const COLOR: Record<Style, { color?: string; bold?: boolean; dimColor?: boolean; backgroundColor?: string; italic?: boolean }> = {
  node: {},
  here: { bold: true, color: 'black', backgroundColor: '#ffd43b' },
  edge: { dimColor: true },
  taken: { bold: true, color: '#e8590c' },
  label: { dimColor: true, italic: true },
  takenLabel: { bold: true, italic: true, color: '#e8590c' },
  blank: {},
}

function takenOf(i: number) {
  const s = FRAMES[i].status
  return s.from ? { from: s.from.nodeId, to: s.nodeId, on: s.from.outcome } : null
}

function factsLine(i: number) {
  const s = FRAMES[i].status
  const parts = [`coding-loop · run_7f3a · ${s.status}`, `at ${s.nodeId} (visit ${s.visit})`]
  if (s.from) parts.push(`came by ${s.from.nodeId} —${s.from.outcome}→`)
  if (s.attempt !== null) parts.push(`try ${s.attempt}/${s.maxAttempts}`)
  if (s.streak) parts.push(`${s.streak.outcome} ${s.streak.count}/${s.streak.limit} in a row`)
  if (s.needs === 'signal') parts.push('NEEDS YOU')
  return parts.join(' · ')
}

// The status JSON as the prototype assumes spore 0.2 reports it (from, maxAttempts, streak are new).
function statusJson(i: number) {
  const s = FRAMES[i].status
  return JSON.stringify({
    status: s.status, needs: s.needs,
    position: { nodeId: s.nodeId, visit: s.visit, from: s.from },
    invocation: s.attempt === null ? null : { attempt: s.attempt, maxAttempts: s.maxAttempts },
    streak: s.streak,
  })
}

async function prepare($: any) {
  if (ready) return
  const root = $.plugin.root + '/workflow/'
  dot = await $.fs.read(root + 'workflow.dot')
  settings = await $.fs.read(root + 'settings.json')
  for (const n of ['implement', 'approve', 'tests_stuck']) {
    try { instructions.set(n, await $.fs.read(root + `steps/${n}.md`)) } catch {}
  }
  const p = await $.process.run(['dot', '-Tplain'], { stdin: forDisplay(dot, undefined, undefined, true) })
  plain = p.stdout
  // Pictures and page fragments for every moment, drawn once: the frames are fixed.
  for (let i = 0; i < FRAMES.length; i++) {
    const d = forDisplay(dot, FRAMES[i].status.nodeId, takenOf(i) ?? undefined)
    await $.process.run(['dot', '-Tpng', '-Gdpi=110', '-o', `${OUT}/frame-${i}.png`], { stdin: d })
    const svg = await $.process.run(['dot', '-Tsvg'], { stdin: d })
    await $.fs.write(`${OUT}/frame-${i}.svg`, svg.stdout)
  }
  ready = true
}

const esc = (t: string) => t.replace(/&/g, '&amp;').replace(/</g, '&lt;')

async function writePage($: any, i: number) {
  const s = FRAMES[i].status, taken = takenOf(i)
  const svg = await $.fs.read(`${OUT}/frame-${i}.svg`)
  const ex = (e: Excerpt) => `<h3>${e.file}</h3><pre>` + e.lines.map(l => `<span class="${l.mark ?? ''}">${l.n ? String(l.n).padStart(3) : '   '}  ${esc(l.text)}</span>`).join('\n') + '</pre>'
  const instr = instructionsFile(settings, s.nodeId)
  // The page never reloads: it polls state.json and swaps the body only when the moment changes.
  const body = `<p class="facts">${esc(factsLine(i))}</p><p class="cap">${esc(FRAMES[i].caption)}</p>
<div class="grid"><div>${svg.replace(/^[\s\S]*?<svg/, '<svg')}</div><div>${ex(dotExcerpt(dot, s.nodeId, taken))}${ex(settingsExcerpt(settings, s.nodeId))}${instr && instructions.has(s.nodeId) ? `<h3>${instr}</h3><pre>${esc(instructions.get(s.nodeId)!)}</pre>` : ''}</div></div>
<pre>${esc(statusJson(i))}</pre>`
  await $.fs.write(`${OUT}/state.json`, JSON.stringify({ rev: i, title: `${s.nodeId} · coding-loop`, body }))
  await $.fs.write(`${OUT}/view.html`, PAGE)
}

const PAGE = `<!doctype html><meta charset="utf-8"><title>coding-loop</title>
<style>body{font:14px system-ui;margin:16px;background:#fafafa}pre{font:12px ui-monospace,Menlo;background:#fff;border:1px solid #ddd;padding:8px;overflow:auto}
.here{background:#ffd43b;font-weight:bold}.taken{color:#e8590c;font-weight:bold}.grid{display:grid;grid-template-columns:minmax(300px,1fr) 1.2fr;gap:16px}
.facts{font-weight:600}.cap{color:#555;font-style:italic}.proto{background:#222;color:#fff;padding:4px 8px;border-radius:4px;font-size:12px}</style>
<p><span class="proto">C17 PROTOTYPE · variant 3 · live, no reload</span> <span id="conn" style="color:#999;font-size:12px"></span></p>
<div id="view"></div>
<script>
let rev = null
async function poll() {
  try {
    const r = await fetch('state.json', { cache: 'no-store' })
    const s = await r.json()
    if (s.rev !== rev) { rev = s.rev; document.getElementById('view').innerHTML = s.body; document.title = s.title }
    document.getElementById('conn').textContent = ''
  } catch { document.getElementById('conn').textContent = '(waiting for the session)' }
  setTimeout(poll, 700)
}
poll()
</script>`

async function go($: any, fn: (v: ViewState) => ViewState) {
  await update($, view, fn)
  const v = await read($, view)
  if (v.variant === 'browser') {
    await writePage($, v.frame)
    if (!browserOpened) {
      browserOpened = true
      // A local server for the session's life, so the page can poll without reloading (file:// can't fetch).
      void (async () => { for await (const _ of $.process.spawn({ argv: ['python3', '-m', 'http.server', String(PORT), '--bind', '127.0.0.1', '-d', OUT] })) {} })()
      await $.clock.sleep(400)
      await $.process.run(['open', `http://127.0.0.1:${PORT}/view.html`])
    }
  }
  if (v.variant === 'band') await $.ui.close({ id: PANE })
  else await $.ui.open({ id: PANE, title: 'Run · coding-loop', rows: 40, columns: 96 })
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({ name: 'c17', description: 'C17 prototype: live view of a run', argumentHint: '[1-5 | next | prev | play | pause]' })
    $.clock.after(0, () => { void prepare($) })
    $.clock.every(3500, async () => {
      const v = await read($, view)
      if (!v.playing || !ready) return
      await go($, x => ({ ...x, frame: (x.frame + 1) % FRAMES.length }))
    })
    return next(e)
  })

  on('command.run', { command: 'c17' }, async ($, e) => {
    await prepare($)
    const a = e.args.trim()
    if (/^[1-5]$/.test(a)) await go($, v => ({ ...v, variant: VARIANTS[+a - 1].key }))
    else if (a === 'next') await go($, v => ({ ...v, playing: false, frame: Math.min(FRAMES.length - 1, v.frame + 1) }))
    else if (a === 'prev') await go($, v => ({ ...v, playing: false, frame: Math.max(0, v.frame - 1) }))
    else if (a === 'play' || a === 'pause') await go($, v => ({ ...v, playing: a === 'play' }))
    else await go($, v => v)
    const v = await read($, view)
    return { text: `C17 prototype: variant ${VARIANTS.findIndex(x => x.key === v.variant) + 1} (${VARIANTS.find(x => x.key === v.variant)!.name}), moment ${v.frame + 1}/${FRAMES.length}` }
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    const v = await read($, view)
    if (v.variant !== 'band' || e.props.hasSurvey) return next(e)
    const { Box, Text, Button } = $.ui.resolve(e)
    const s = FRAMES[v.frame].status
    return (
      <Box flexDirection="row" gap={1}>
        <Text color={s.needs === 'signal' ? '#e8590c' : undefined} bold={s.needs === 'signal'}>⏵ {factsLine(v.frame)}</Text>
        <Button key="open" hotkey="v" label="view" onPress={() => go($, x => ({ ...x, variant: 'text' }))} />
      </Box>
    )
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button, Image } = $.ui.resolve(e) as any
    const v = await read($, view)
    const i = v.frame, s = FRAMES[i].status, taken = takenOf(i)
    const cols = e.viewport?.columns ?? 100
    const wide = cols >= 150

    const controls = (
      <Box flexDirection="row" gap={1} flexWrap="wrap">
        <Button key="prev" hotkey="h" label="◀ prev" onPress={() => go($, x => ({ ...x, playing: false, frame: Math.max(0, x.frame - 1) }))} />
        <Button key="next" hotkey="l" label="next ▶" onPress={() => go($, x => ({ ...x, playing: false, frame: Math.min(FRAMES.length - 1, x.frame + 1) }))} />
        <Button key="play" hotkey="p" label={v.playing ? 'pause' : 'play'} onPress={() => go($, x => ({ ...x, playing: !x.playing }))} />
        {VARIANTS.map((x, k) => (
          <Button key={x.key} hotkey={String(k + 1)} label={`${k + 1} ${x.key}${x.key === v.variant ? ' ●' : ''}`} onPress={() => go($, y => ({ ...y, variant: x.key }))} />
        ))}
      </Box>
    )

    const header = (
      <Box flexDirection="column">
        <Text inverse> C17 PROTOTYPE · {VARIANTS.findIndex(x => x.key === v.variant) + 1}: {VARIANTS.find(x => x.key === v.variant)!.name} · moment {i + 1}/{FRAMES.length} </Text>
        <Text bold color={s.needs === 'signal' ? '#e8590c' : undefined}>{factsLine(i)}</Text>
        <Text italic dimColor>{FRAMES[i].caption}</Text>
      </Box>
    )

    if (!ready) return <Box flexDirection="column">{header}<Text dimColor>Laying out workflow.dot…</Text></Box>

    const excerpt = (ex: Excerpt) => (
      <Box flexDirection="column" marginTop={1}>
        <Text underline>{ex.file}</Text>
        {ex.lines.map(l => (
          <Text wrap="truncate-end" color={l.mark === 'taken' ? '#e8590c' : undefined} bold={l.mark !== null} backgroundColor={l.mark === 'here' ? '#ffd43b' : undefined}>
            {l.mark === 'here' ? <Text color="black">{(l.n ? String(l.n).padStart(3) : '   ') + ' ' + l.text}</Text> : (l.n ? String(l.n).padStart(3) : '   ') + ' ' + l.text}
          </Text>
        ))}
      </Box>
    )
    const instr = instructionsFile(settings, s.nodeId)
    const files = (
      <Box flexDirection="column" flexGrow={1}>
        {excerpt(dotExcerpt(dot, s.nodeId, taken))}
        {excerpt(settingsExcerpt(settings, s.nodeId))}
        {instr && instructions.has(s.nodeId) && (
          <Box flexDirection="column" marginTop={1}>
            <Text underline>{instr}</Text>
            {instructions.get(s.nodeId)!.split('\n').filter(Boolean).slice(0, 3).map(l => <Text wrap="truncate-end" dimColor>{l}</Text>)}
            <Text dimColor>…</Text>
          </Box>
        )}
      </Box>
    )
    const state = <Text dimColor wrap="wrap">status --json: {statusJson(i)}</Text>

    let graph
    if (v.variant === 'text' || v.variant === 'fine') {
      const gcols = Math.max(40, Math.min(64, wide ? Math.floor(cols / 2) - 4 : cols - 4))
      const grid = (v.variant === 'fine' ? drawBraille : drawText)(plain, gcols, s.nodeId, taken, onOfFactory(dot))
      while (grid.length && grid[0].every(c => c.ch === ' ')) grid.shift()
      while (grid.length && grid[grid.length - 1].every(c => c.ch === ' ')) grid.pop()
      graph = (
        <Box flexDirection="column" marginTop={1}>
          {grid.map(row => <Text>{runs(row).map(r => <Text {...COLOR[r.st]}>{r.text}</Text>)}</Text>)}
        </Box>
      )
    } else if (v.variant === 'picture') {
      graph = (
        <Box marginTop={1}>
          <Image key="graph" source={{ file: `${OUT}/frame-${i}.png`, format: 'png', generation: i }} columns={wide ? 60 : Math.min(70, cols - 4)} rows={22} alt={`[picture of the workflow: at ${s.nodeId}; this terminal cannot draw pictures]`} />
        </Box>
      )
    } else {
      graph = (
        <Box flexDirection="column" marginTop={1}>
          <Text>The drawing and the files are in your browser: <Text color="cyan" underline>http://127.0.0.1:{PORT}/view.html</Text></Text>
          <Text dimColor>The page updates in place when the run moves. This pane keeps only the line above.</Text>
        </Box>
      )
    }

    return (
      <Box flexDirection="column">
        {header}
        {controls}
        {v.variant === 'browser' ? graph : wide ? <Box flexDirection="row" gap={3}><Box flexDirection="column">{graph}</Box>{files}</Box> : <Box flexDirection="column">{graph}{files}</Box>}
        <Box marginTop={1}>{state}</Box>
      </Box>
    )
  })
}
