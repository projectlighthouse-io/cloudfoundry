<script setup lang="ts">
import type { TwoFactorPage } from '../../types/TwoFactorPage'
import type { TrustedDevice } from '../../types/TrustedDevice'
import { router, useForm } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AppLayout from '../../layouts/AppLayout.vue'

defineProps<TwoFactorPage>()

const base = '/settings/two-factor'
const opts = { preserveScroll: true }
const codeForm = useForm({ code: '' })
const enable = () => router.post(base, {}, opts)
const submitCode = () => codeForm.post(`${base}/confirm`, { ...opts, onFinish: () => codeForm.reset() })
const regenerate = () => router.post(`${base}/backup-codes`, {}, opts)
const disable = () => confirm('Turn off two-factor authentication? Trusted browsers are forgotten too.') && router.delete(base, opts)
const revoke = (d: TrustedDevice) => router.delete(`${base}/devices/${d.uuid}`, opts)
const date = (at: number) => new Date(at * 1000).toLocaleDateString()
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

    <section aria-labelledby="two-factor-h">
      <div class="strip">
        <h2 id="two-factor-h">Two-factor authentication</h2>
        <span class="meta">{{ status.enabled ? 'On' : status.confirming ? 'Finishing setup' : 'Off' }}</span>
      </div>

      <div v-if="!status.enabled && !status.confirming" class="setting">
        <div>
          <h3>Add a second step to logging in</h3>
          <p>After your password, you enter a code from an authenticator app (1Password, Google Authenticator, Authy, ...).</p>
        </div>
        <div><button class="btn btn-ink" type="button" @click="enable">Turn on</button></div>
      </div>

      <div v-if="setup" class="setting">
        <div>
          <h3>Scan this QR code</h3>
          <p>Scan it with your authenticator app, or type the key in by hand. Then enter the 6-digit code it shows.</p>
          <p class="hint">Key: <code class="mono">{{ setup.secret }}</code></p>
        </div>
        <div>
          <!-- The SVG comes from the server, built from the otpauth:// URI; it holds no user input. -->
          <div class="qr" v-html="setup.qr_svg" />
          <form @submit.prevent="submitCode">
            <Field v-model="codeForm.code" label="Code" autocomplete="one-time-code" :error="codeForm.errors.code" />
            <button class="btn btn-ink" type="submit" :disabled="codeForm.processing">Confirm</button>
          </form>
        </div>
      </div>

      <div v-if="backup_codes" class="setting">
        <div>
          <h3>Save your backup codes</h3>
          <p>Each code logs you in once if you lose your phone. They are shown only now: store them in your password manager.</p>
        </div>
        <div>
          <ul class="codes mono">
            <li v-for="c in backup_codes" :key="c">{{ c }}</li>
          </ul>
        </div>
      </div>

      <div v-if="status.enabled" class="setting">
        <div>
          <h3>Backup codes</h3>
          <p>{{ status.backup_codes_left }} left. New codes replace all old ones. You confirm your password first.</p>
        </div>
        <div><button class="btn btn-line" type="button" @click="regenerate">New backup codes</button></div>
      </div>
    </section>

    <section v-if="status.enabled" aria-labelledby="devices-h">
      <div class="strip"><h2 id="devices-h">Trusted browsers</h2><span class="meta">{{ devices.length }}</span></div>
      <p v-if="!devices.length" class="hint">None. Tick "Trust this browser" when you enter a code to skip it for 30 days.</p>
      <ul class="ledger">
        <li v-for="d in devices" :key="d.uuid">
          <span class="who-text">
            <span class="name">{{ d.user_agent ?? 'Unknown browser' }}</span>
            <span class="meta">{{ d.ip ?? 'unknown IP' }} · trusted {{ date(d.created_at) }} · until {{ date(d.expires_at) }}</span>
          </span>
          <span class="trail"><button class="btn btn-line btn-sm" type="button" @click="revoke(d)">Forget</button></span>
        </li>
      </ul>
    </section>

    <section v-if="status.enabled" aria-labelledby="danger-h">
      <div class="strip strip-danger"><h2 id="danger-h">Danger zone</h2></div>
      <div class="setting">
        <div>
          <h3>Turn off two-factor authentication</h3>
          <p>Logging in will only need your password. You confirm your password first.</p>
        </div>
        <div><button class="btn btn-danger-line" type="button" @click="disable">Turn off</button></div>
      </div>
    </section>
    <div class="tail" />
  </AppLayout>
</template>

<style scoped>
.qr {
  width: 200px;
  margin-bottom: 16px;
  background: #fff; /* scanners need contrast in dark mode too */
  padding: 8px;
}
.qr :deep(svg) {
  display: block;
  width: 100%;
  height: auto;
}
.codes {
  display: grid;
  grid-template-columns: repeat(2, max-content);
  gap: 6px 24px;
  margin: 0;
  padding: 0;
  list-style: none;
}
</style>
