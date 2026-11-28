<script setup lang="ts">
import type { PasskeysPage } from '../../types/PasskeysPage'
import type { Passkey } from '../../types/Passkey'
import { ref } from 'vue'
import { router, usePage } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AppLayout from '../../layouts/AppLayout.vue'
import { register } from '../../passkeys'

defineProps<PasskeysPage>()

const base = '/settings/passkeys'
const opts = { preserveScroll: true }
const page = usePage()
const name = ref('')
const browserError = ref('')
const supported = typeof PublicKeyCredential !== 'undefined' && 'parseCreationOptionsFromJSON' in PublicKeyCredential
const add = async () => {
  browserError.value = ''
  await register(name.value.trim() || 'Passkey', (m) => (browserError.value = m))
  name.value = ''
}
const rename = (p: Passkey) => {
  const next = prompt('Name this passkey', p.name)?.trim()
  if (next && next !== p.name) router.patch(`${base}/${p.public_id}`, { name: next }, opts)
}
const remove = (p: Passkey) => confirm(`Remove "${p.name}"? It will no longer sign you in.`) && router.delete(`${base}/${p.public_id}`, opts)
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

    <section aria-labelledby="passkeys-h">
      <div class="strip">
        <h2 id="passkeys-h">Passkeys</h2>
        <span class="meta">{{ passkeys.length }}</span>
      </div>

      <div class="setting">
        <div>
          <h3>Sign in without a password</h3>
          <p>A passkey uses your device's screen lock (fingerprint, face or PIN), or a security key. It also works as the second step after your password.</p>
        </div>
        <div>
          <form v-if="supported" @submit.prevent="add">
            <Field v-model="name" label="Name" placeholder="MacBook, YubiKey, ..." :error="browserError || page.props.errors?.passkey || page.props.errors?.name" />
            <button class="btn btn-ink" type="submit">Add a passkey</button>
          </form>
          <p v-else class="hint">This browser does not support passkeys.</p>
        </div>
      </div>

      <p v-if="!passkeys.length" class="hint">No passkeys yet.</p>
      <ul class="ledger">
        <li v-for="p in passkeys" :key="p.public_id">
          <span class="who-text">
            <span class="name">{{ p.name }}</span>
            <span class="meta">added {{ date(p.created_at) }} · {{ p.last_used_at ? `last used ${date(p.last_used_at)}` : 'never used' }}</span>
          </span>
          <span class="trail">
            <button class="btn btn-line btn-sm" type="button" @click="rename(p)">Rename</button>
            <button class="btn btn-danger-line btn-sm" type="button" @click="remove(p)">Remove</button>
          </span>
        </li>
      </ul>
      <p v-if="passkeys.length" class="hint">Removing a passkey asks for your password first.</p>
    </section>
    <div class="tail" />
  </AppLayout>
</template>
