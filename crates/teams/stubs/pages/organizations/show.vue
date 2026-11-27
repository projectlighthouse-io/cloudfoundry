<script setup lang="ts">
import { Link, router, useForm } from '@inertiajs/vue3'

const props = defineProps<{
  email: string
  organization: { id: number; name: string }
  user_id: number
  role: string
  can: { rename: boolean; delete: boolean; manage_members: boolean }
  members: { id: number; email: string; name: string | null; role: string }[]
  teams: { id: number; name: string }[]
  invitations: { id: number; email: string; role: string }[]
}>()

const roles = ['owner', 'admin', 'member']
const base = `/organizations/${props.organization.id}`

const rename = useForm({ name: props.organization.name })
const invite = useForm({ email: '', role: 'member' })
const team = useForm({ name: '', organization_id: props.organization.id })

const changeRole = (user: number, role: string) => router.put(`${base}/members/${user}`, { role }, { preserveScroll: true })
const remove = (user: number) => confirm('Remove this member from the organization and its teams?') && router.delete(`${base}/members/${user}`, { preserveScroll: true })
const revoke = (id: number) => router.delete(`${base}/invitations/${id}`, { preserveScroll: true })
const leave = () => confirm('Leave this organization and its teams?') && router.delete(`${base}/membership`)
const destroy = () => confirm(`Delete ${props.organization.name} and all of its teams? This cannot be undone.`) && router.delete(base)
</script>

<template>
  <main>
    <p><Link href="/organizations">All organizations</Link></p>
    <h1>{{ organization.name }}</h1>
    <p>Your role: {{ role }}</p>

    <form v-if="can.rename" @submit.prevent="rename.put(base)">
      <label>
        Name
        <input v-model="rename.name" required maxlength="80" />
      </label>
      <p v-if="rename.errors.name">{{ rename.errors.name }}</p>
      <button type="submit" :disabled="rename.processing">Rename</button>
    </form>

    <h2>Teams</h2>
    <ul>
      <li v-for="t in teams" :key="t.id"><Link :href="`/teams/${t.id}`">{{ t.name }}</Link></li>
    </ul>
    <form @submit.prevent="team.post('/teams')">
      <label>
        New team
        <input v-model="team.name" required maxlength="80" />
      </label>
      <p v-if="team.errors.name">{{ team.errors.name }}</p>
      <button type="submit" :disabled="team.processing">Create team</button>
    </form>

    <h2>Members</h2>
    <p v-if="$page.props.errors?.role">{{ $page.props.errors.role }}</p>
    <ul>
      <li v-for="member in members" :key="member.id">
        {{ member.name ?? member.email }}
        <template v-if="can.manage_members && member.id !== user_id">
          <select :value="member.role" @change="changeRole(member.id, ($event.target as HTMLSelectElement).value)">
            <option v-for="r in roles" :key="r" :value="r">{{ r }}</option>
          </select>
          <button type="button" @click="remove(member.id)">Remove</button>
        </template>
        <span v-else> · {{ member.role }}</span>
      </li>
    </ul>

    <template v-if="can.manage_members">
      <h2>Invite</h2>
      <form @submit.prevent="invite.post(`${base}/invitations`, { onSuccess: () => invite.reset(), preserveScroll: true })">
        <label>
          Email
          <input v-model="invite.email" type="email" required />
        </label>
        <p v-if="invite.errors.email">{{ invite.errors.email }}</p>
        <label>
          Role
          <select v-model="invite.role">
            <option v-for="r in roles" :key="r" :value="r">{{ r }}</option>
          </select>
        </label>
        <button type="submit" :disabled="invite.processing">Send invitation</button>
      </form>
      <ul v-if="invitations.length">
        <li v-for="i in invitations" :key="i.id">
          {{ i.email }} · {{ i.role }}
          <button type="button" @click="revoke(i.id)">Withdraw</button>
        </li>
      </ul>
    </template>

    <h2>Leave or delete</h2>
    <button type="button" @click="leave">Leave organization</button>
    <button v-if="can.delete" type="button" @click="destroy">Delete organization</button>
  </main>
</template>
