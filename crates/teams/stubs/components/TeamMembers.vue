<script setup lang="ts">
import type { Uuid } from '../types/Uuid'
import { router, useForm, usePage } from '@inertiajs/vue3'
import { computed, ref } from 'vue'

const props = defineProps<{
  base: string
  userId: Uuid<'User'>
  members: { uuid: Uuid<'User'>; email: string; name: string | null; role: string }[]
  invitations: { uuid: Uuid<'Invitation'>; email: string; role: string; expires_at: number }[]
  /** The link of the invitation just sent; tokens are not stored, so only a fresh one can be shown. */
  sentLink: { uuid: Uuid<'Invitation'>; url: string } | null
  canManage: boolean
  canInvite: boolean
  removeNote: string
}>()

const roles = ['owner', 'admin', 'member']
const opts = { preserveScroll: true }
const page = usePage()
const roleError = computed(() => (page.props.errors as Record<string, string> | undefined)?.role)
const invite = useForm({ email: '', role: 'member' })

const initial = (s: string) => s.charAt(0).toUpperCase()
const changeRole = (user: Uuid<'User'>, role: string) => router.put(`${props.base}/members/${user}`, { role }, opts)
const remove = (m: { uuid: Uuid<'User'>; email: string }) => confirm(`Remove ${m.email}? ${props.removeNote}`) && router.delete(`${props.base}/members/${m.uuid}`, opts)
const revoke = (id: Uuid<'Invitation'>) => router.delete(`${props.base}/invitations/${id}`, opts)
const resend = (id: Uuid<'Invitation'>) => router.post(`${props.base}/invitations/${id}/resend`, {}, opts)
const copied = ref(false)
const copy = async (url: string) => {
  await navigator.clipboard.writeText(url)
  copied.value = true
  setTimeout(() => (copied.value = false), 1200)
}
const expires = (at: number) => {
  const days = Math.ceil((at * 1000 - Date.now()) / 86_400_000)
  return days <= 0 ? 'expired' : days === 1 ? 'link expires in 1 day' : `link expires in ${days} days`
}
const send = () => invite.post(`${props.base}/invitations`, { ...opts, onSuccess: () => invite.reset('email') })
const count = computed(() => {
  const n = props.members.length
  const p = props.invitations.length
  return `${n} ${n === 1 ? 'member' : 'members'}${p ? ` · ${p} pending` : ''}`
})
</script>

<template>
  <section aria-labelledby="members-h">
    <div class="strip"><h2 id="members-h">Members</h2><span class="meta">{{ count }}</span></div>
    <p v-if="roleError" class="flash" role="alert">{{ roleError }}</p>
    <ul class="ledger">
      <li v-for="m in members" :key="m.uuid">
        <span class="who">
          <span class="face" :class="{ 'face-me': m.uuid === userId }" aria-hidden="true">{{ initial(m.name ?? m.email) }}</span>
          <span class="who-text">
            <span class="name">{{ m.name ?? m.email }}<span v-if="m.uuid === userId" class="you">you</span></span>
            <span class="meta">{{ m.email }}</span>
          </span>
        </span>
        <span class="trail">
          <template v-if="canManage && m.uuid !== userId">
            <select class="select select-sm" :value="m.role" :aria-label="`Role of ${m.email}`" @change="changeRole(m.uuid, ($event.target as HTMLSelectElement).value)">
              <option v-for="r in roles" :key="r" :value="r">{{ r }}</option>
            </select>
            <button class="btn-icon" type="button" :aria-label="`Remove ${m.email}`" title="Remove" @click="remove(m)">
              <svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
            </button>
          </template>
          <span v-else class="role">{{ m.role }}</span>
        </span>
      </li>
      <li v-for="i in invitations" :key="`i${i.uuid}`">
        <span class="who">
          <span class="face face-pending" aria-hidden="true">{{ initial(i.email) }}</span>
          <span class="who-text">
            <span class="name">{{ i.email }}</span>
            <span class="meta">Invited as {{ i.role }} · {{ expires(i.expires_at) }}</span>
          </span>
        </span>
        <span class="trail">
          <span class="state state-wait">pending</span>
          <template v-if="canInvite">
            <button class="btn btn-line btn-sm" type="button" @click="resend(i.uuid)">Send again</button>
            <button class="btn btn-line btn-sm" type="button" @click="revoke(i.uuid)">Withdraw</button>
          </template>
        </span>
        <div v-if="sentLink?.uuid === i.uuid" class="copy-link">
          <input class="input mono" :value="sentLink.url" readonly aria-label="Invitation link" @focus="($event.target as HTMLInputElement).select()" />
          <button class="btn btn-ink btn-sm" type="button" @click="copy(sentLink.url)">{{ copied ? 'Copied' : 'Copy link' }}</button>
          <p class="hint">Anyone logged in as {{ i.email }} can join with this link. Sending again replaces it.</p>
        </div>
      </li>
      <li v-if="canInvite" id="invite" class="form-row">
        <form class="add" @submit.prevent="send">
          <input v-model="invite.email" class="input" type="email" placeholder="name@company.com" aria-label="Email to invite" required :aria-invalid="invite.errors.email ? 'true' : undefined" />
          <select v-model="invite.role" class="select" aria-label="Role">
            <option v-for="r in roles" :key="r" :value="r">{{ r }}</option>
          </select>
          <button class="btn btn-ink" type="submit" :disabled="invite.processing">{{ invite.processing ? 'Sending…' : 'Invite member' }}</button>
          <p v-if="invite.errors.email || invite.errors.role" class="error">{{ invite.errors.email ?? invite.errors.role }}</p>
        </form>
      </li>
    </ul>
  </section>
</template>
