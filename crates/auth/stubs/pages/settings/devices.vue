<script setup lang="ts">
import type { DevicesPage } from '../../types/DevicesPage'
import type { Device } from '../../types/Device'
import { router } from '@inertiajs/vue3'
import AppLayout from '../../layouts/AppLayout.vue'

const props = defineProps<DevicesPage>()

const base = '/settings/devices'
const opts = { preserveScroll: true }
const others = () => props.devices.filter((d) => d.public_id !== props.current).length
const revoke = (d: Device) => confirm('Log this browser out?') && router.delete(`${base}/${d.public_id}`, opts)
const revokeOthers = () => confirm('Log out of every other browser?') && router.delete(base, opts)
const seen = (at: number) => {
  const mins = Math.round((Date.now() / 1000 - at) / 60)
  return mins < 2 ? 'active now' : mins < 60 ? `${mins} minutes ago` : new Date(at * 1000).toLocaleString()
}
</script>

<template>
  <AppLayout title="Settings" :user="email">
    <header class="phead">
      <div class="phead-id">
        <div>
          <h1>Settings</h1>
          <p class="sub">Signed in as {{ email }}.</p>
        </div>
      </div>
    </header>

    <section aria-labelledby="devices-h">
      <div class="strip">
        <h2 id="devices-h">Browsers &amp; devices</h2>
        <span class="meta">{{ devices.length }}</span>
      </div>
      <p class="hint">Where you are logged in. Logging a browser out asks for your password first.</p>
      <ul class="ledger">
        <li v-for="d in devices" :key="d.public_id">
          <span class="who-text">
            <span class="name">{{ d.user_agent ?? 'Unknown browser' }}<template v-if="d.public_id === current"> · this browser</template></span>
            <span class="meta">{{ d.ip ?? 'unknown IP' }} · {{ seen(d.last_seen_at) }}</span>
          </span>
          <span v-if="d.public_id !== current" class="trail">
            <button class="btn btn-line btn-sm" type="button" @click="revoke(d)">Log out</button>
          </span>
        </li>
      </ul>
    </section>

    <section v-if="others()" aria-labelledby="others-h">
      <div class="strip strip-danger"><h2 id="others-h">Other browsers</h2></div>
      <div class="setting">
        <div>
          <h3>Log out everywhere else</h3>
          <p>Every other browser is logged out, and "remember me" stops working on them.</p>
        </div>
        <div><button class="btn btn-danger-line" type="button" @click="revokeOthers">Log out others</button></div>
      </div>
    </section>
    <div class="tail" />
  </AppLayout>
</template>
