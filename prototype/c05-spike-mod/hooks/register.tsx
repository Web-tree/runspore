// C05 throwaway spike: logs what the mods API does in this build. Not product code.
const LOG = '/tmp/c05-spike-log'
const SPORE = 'spore'
let seq = 0

async function log($, name, data) {
  seq += 1
  const at = new Date().toISOString()
  try {
    await $.fs.write(LOG + '/' + Date.now() + '-' + String(seq).padStart(3, '0') + '-' + name + '.json', JSON.stringify({ at, name, data }, null, 1))
  } catch (err) {
    $.ui.log('spike log failed: ' + String(err), { to: 'debug' })
  }
}

const MENU = ['run 1: waiting for approval', 'run 2: stuck at test', 'run 3: running', 'start: coding-loop', 'start: release', 'start: docs', 'write a new workflow', 'change a workflow']

export function register(on) {
  on('session.start', async ($, e, next) => {
    const t0 = Date.now()
    await $.command.register({ name: 'spike', description: 'C05 spike', argumentHint: '<test>' })
    await $.tool.register({
      name: 'report',
      description: 'C05 spike: report a step result. Only call it when the user asks you to.',
      inputSchema: { type: 'object', properties: { outcome: { type: 'string', enum: ['done', 'failed'] }, count: { type: 'integer' } }, required: ['outcome'], additionalProperties: false },
      isDeferred: false,
    })
    const t1 = Date.now()
    const r = await $.process.run([SPORE, '--json', '--db', LOG + '/../spike.db', 'list'], { timeoutMs: 9000 })
    const t2 = Date.now()
    $.ui.log('spike: 0 unfinished runs (session.start notice line)')
    await log($, 'session.start', { e, registerMs: t1 - t0, sporeListMs: t2 - t1, spore: { exit: r.exitCode, out: r.stdout.slice(0, 200), err: r.stderr.slice(0, 200) }, root: $.plugin.root, name: $.plugin.name })
    return next(e)
  })

  on('session.end', async ($, e, next) => {
    await log($, 'session.end', e)
    return next(e)
  })
  on('classic.SessionEnd', async ($, e, next) => {
    await log($, 'classic.SessionEnd', e)
    return next(e)
  })
  on('classic.SessionStart', async ($, e, next) => {
    await log($, 'classic.SessionStart', e)
    return next(e)
  })
  on('classic.Stop', async ($, e, next) => {
    await log($, 'classic.Stop', e)
    return next(e)
  })
  on('turn.start', async ($, e, next) => {
    await log($, 'turn.start', e)
    return next(e)
  })
  on('turn.complete', async ($, e, next) => {
    await log($, 'turn.complete', { reason: e.reason, isAborted: e.isAborted, agentId: e.agentId, turnId: e.turnId, answer: String(e.answer ?? '').slice(0, 300) })
    return next(e)
  })
  on('prompt.submit', async ($, e, next) => {
    await log($, 'prompt.submit', { text: e.text, origin: e.origin })
    return next(e)
  })
  on('skill.prompt', async ($, e, next) => {
    await log($, 'skill.prompt', e)
    return next(e)
  })

  on('tool.call', { tool: 'mcp__spike-mod__report' }, async ($, e) => {
    await log($, 'tool.report', e)
    return { result: 'spike got: ' + JSON.stringify(e) }
  })

  on('ui.render', { component: 'Pane', requestId: 'spike-menu' }, async ($, e) => {
    const ui = $.ui.resolve(e)
    return (
      <ui.Box flexDirection="column">
        <ui.Text>Runspore (spike menu, 8 entries)</ui.Text>
        <ui.Select key="menu" autoFocus options={MENU.map((m) => ({ label: m, value: m }))} onSelect={(v) => picked($, v)} />
      </ui.Box>
    )
  })

  on('command.run', { command: 'spike' }, async ($, e) => {
    const test = (e.args ?? '').trim()
    await log($, 'command.run', { args: e.args, origin: e.origin })
    if (test === 'ask') {
      $.clock.after(0, async () => {
        try {
          const a = await $.ui.ask('Which run do you want?', ['run 1: approve', 'run 2: stuck', 'start a workflow', 'write a new workflow'])
          await log($, 'ask.answer', { a })
        } catch (err) {
          await log($, 'ask.reject', { err: String(err) })
        }
      })
      return { text: 'ask queued' }
    }
    if (test === 'ask6') {
      $.clock.after(0, async () => {
        try {
          const a = await $.ui.ask('Six options?', ['a1', 'a2', 'a3', 'a4', 'a5', 'a6'])
          await log($, 'ask6.answer', { a })
        } catch (err) {
          await log($, 'ask6.reject', { err: String(err) })
        }
      })
      return { text: 'ask6 queued' }
    }
    if (test === 'askdirect') {
      try {
        const a = await $.ui.ask('Asked straight from command.run?', ['yes', 'no'])
        await log($, 'askdirect.answer', { a })
        return { text: 'askdirect: ' + a }
      } catch (err) {
        await log($, 'askdirect.reject', { err: String(err) })
        return { text: 'askdirect rejected: ' + String(err) }
      }
    }
    if (test === 'pane') {
      const r = await $.ui.open({ id: 'spike-menu', title: 'runspore', focus: true, closeOnEscape: true, rows: 14 })
      await log($, 'pane.open', r)
      return { text: 'pane opened' }
    }
    if (test === 'submit') {
      $.clock.after(0, async () => {
        try {
          const r = await $.prompt.submit({ text: '/spike-mod:hello from-submit' })
          await log($, 'submit.result', r)
        } catch (err) {
          await log($, 'submit.reject', { err: String(err) })
        }
      })
      return { text: 'submit queued via timer' }
    }
    if (test === 'submitplain') {
      $.clock.after(0, async () => {
        try {
          const r = await $.prompt.submit({ text: 'Reply with the single word SUBMITTED.' })
          await log($, 'submitplain.result', r)
        } catch (err) {
          await log($, 'submitplain.reject', { err: String(err) })
        }
      })
      return { text: 'submitplain queued via timer' }
    }
    if (test === 'submituser') {
      void $.prompt.submit({ text: '/spike-mod:hello from-submit-asuser', asUser: true })
      return { text: 'submit asUser queued' }
    }
    if (test === 'cmdrun') {
      $.clock.after(0, async () => {
        try {
          const r = await $.command.run({ command: 'spike-mod:hello', args: 'from-command-run' })
          await log($, 'cmdrun.result', r)
        } catch (err) {
          await log($, 'cmdrun.reject', { err: String(err) })
        }
      })
      return { text: 'command.run queued' }
    }
    if (test === 'cmdrunbare') {
      $.clock.after(0, async () => {
        try {
          const r = await $.command.run({ command: 'hello', args: 'from-command-run-bare' })
          await log($, 'cmdrunbare.result', r)
        } catch (err) {
          await log($, 'cmdrunbare.reject', { err: String(err) })
        }
      })
      return { text: 'command.run bare queued' }
    }
    if (test === 'agent') {
      try {
        const r = await $.agent.spawn({ prompt: 'Reply with the single word PONG and nothing else.', description: 'spike ping' })
        await log($, 'agent.spawn', r)
        return { text: 'agent.spawn: ' + JSON.stringify(r) }
      } catch (err) {
        await log($, 'agent.spawn.reject', { err: String(err) })
        return { text: 'agent.spawn rejected: ' + String(err) }
      }
    }
    if (test === 'spawn') {
      const s = $.process.spawn({ argv: ['sh', '-c', 'echo started; sleep 901; echo done'] })
      $.clock.after(0, async () => {
        try {
          for await (const c of s) await log($, 'spawn.chunk', c)
          await log($, 'spawn.end', await s.result)
        } catch (err) {
          await log($, 'spawn.reject', { err: String(err) })
        }
      })
      return { text: 'spawned sleep 901' }
    }
    if (test === 'detach') {
      const r = await $.process.run(['sh', '-c', 'nohup sleep 902 >/dev/null 2>&1 & echo $!'])
      await log($, 'detach', r)
      return { text: 'detached sleep 902 pid ' + r.stdout.trim() }
    }
    if (test === 'env') {
      const r = await $.process.run(['sh', '-c', 'echo "PATH=$PATH"; command -v spore || echo no-spore; command -v claude-spore || echo no-helper; echo "CLAUDE_PLUGIN_ROOT=$CLAUDE_PLUGIN_ROOT"'])
      await log($, 'env', { root: $.plugin.root, out: r.stdout })
      return { text: r.stdout }
    }
    if (test === 'slow') {
      const r = await $.process.run(['sh', '-c', 'sleep 15; echo slept'], { timeoutMs: 20000 })
      return { text: 'slow: ' + r.stdout }
    }
    return { text: 'tests: ask ask6 askdirect pane submit submituser cmdrun cmdrunbare agent spawn detach env slow' }
  })
}

async function picked($, v) {
  await log($, 'pane.select', { v })
  await $.ui.close({ id: 'spike-menu' })
}
