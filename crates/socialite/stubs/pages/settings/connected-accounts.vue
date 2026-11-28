<script setup lang="ts">
import type { ConnectedAccountsPage } from '../../types/ConnectedAccountsPage'
import type { SocialAccount } from '../../types/SocialAccount'
import { computed } from 'vue'
import { router, usePage } from '@inertiajs/vue3'
import AppLayout from '../../layouts/AppLayout.vue'

const props = defineProps<ConnectedAccountsPage>()

const labels: Record<string, string> = { github: 'GitHub', google: 'Google', gitlab: 'GitLab', reddit: 'Reddit', twitter: 'X', auth0: 'Auth0' }
const page = usePage()
// `social` is the shared prop of registered providers; offer the ones not connected yet.
const unlinked = computed(() => ((page.props.social as string[] | undefined) ?? []).filter((p) => !props.accounts.some((a) => a.provider === p)))
const last = computed(() => !props.has_password && props.accounts.length === 1)
const unlink = (a: SocialAccount) =>
  confirm(`Disconnect ${labels[a.provider] ?? a.provider}?`) && router.delete(`/settings/connected-accounts/${a.provider}`, { preserveScroll: true })
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

    <section aria-labelledby="connected-h">
      <div class="strip">
        <h2 id="connected-h">Connected accounts</h2>
        <span class="meta">{{ accounts.length }}</span>
      </div>
      <p v-if="page.props.errors?.provider" class="flash flash-err" role="alert">{{ page.props.errors.provider }}</p>
      <p v-if="!accounts.length" class="hint">None yet.</p>
      <ul class="ledger">
        <li v-for="a in accounts" :key="a.provider">
          <img v-if="a.avatar_url" class="provider-avatar" :src="a.avatar_url" alt="" width="28" height="28" />
          <span class="who-text">
            <span class="name">{{ labels[a.provider] ?? a.provider }}<template v-if="a.username"> · {{ a.username }}</template></span>
            <span class="meta">{{ a.email ?? 'no email shared' }} · {{ a.last_used_at ? `last used ${date(a.last_used_at)}` : 'never used to sign in' }}</span>
          </span>
          <span class="trail">
            <button class="btn btn-line btn-sm" type="button" :disabled="last" :title="last ? 'Set a password first' : undefined" @click="unlink(a)">Disconnect</button>
          </span>
        </li>
      </ul>
      <p v-if="last" class="hint">This is your only way to sign in. Set a password before disconnecting it.</p>

      <div v-if="unlinked.length" class="setting">
        <div>
          <h3>Connect another account</h3>
          <p>Sign in with it next time instead of your password.</p>
        </div>
        <div>
          <a v-for="p in unlinked" :key="p" class="btn btn-line" :href="`/auth/${p}`">Connect {{ labels[p] ?? p }}</a>
        </div>
      </div>
    </section>
    <div class="tail" />
  </AppLayout>
</template>

<style scoped>
.provider-avatar {
  border-radius: 50%;
  margin-right: 12px;
}
</style>
