import './lib/tokens.css'
import { mount } from 'svelte'
import App from './App.svelte'

// Svelte 5 runes components (App.svelte uses `$state`/`$props` as of W30) are not
// legacy-class-constructable — `mount()` is the runes-mode entry point.
const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
