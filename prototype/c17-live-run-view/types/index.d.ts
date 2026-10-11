export type ViewState = { frame: number; variant: 'text' | 'picture' | 'browser' | 'band'; playing: boolean }

declare module 'claude-code' {
  interface PluginState {
    'c17-live-run-view': { view: ViewState }
  }
}
