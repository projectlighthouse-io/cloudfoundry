<script setup lang="ts">
import type { OrgPage } from '../../types/OrgPage'
import { Link, useForm } from '@inertiajs/vue3'
import TeamMembers from '../../components/TeamMembers.vue'
import TeamSettings from '../../components/TeamSettings.vue'
import AppLayout from '../../layouts/AppLayout.vue'

const props = defineProps<OrgPage>()

const base = `/organizations/${props.organization.uuid}`
const article = (r: string) => (/^[aeiou]/.test(r) ? 'an' : 'a')
const team = useForm({ name: '', organization: props.organization.uuid })
</script>

<template>
  <AppLayout :title="organization.name" :user="email">
    <p class="label-row"><Link href="/organizations">Organizations</Link></p>
    <header class="phead">
      <div class="phead-id">
        <span class="tile" aria-hidden="true">{{ organization.name.charAt(0).toUpperCase() }}</span>
        <div>
          <h1>{{ organization.name }}</h1>
          <p class="sub">You are {{ article(role) }} {{ role }}. Owners and admins manage every team in it.</p>
        </div>
      </div>
    </header>

    <section aria-labelledby="teams-h">
      <div class="strip"><h2 id="teams-h">Teams</h2><span class="meta">{{ teams.length }}</span></div>
      <ul class="ledger">
        <li v-for="t in teams" :key="t.uuid">
          <Link class="who" :href="`/teams/${t.uuid}`">
            <span class="face" aria-hidden="true">{{ t.name.charAt(0).toUpperCase() }}</span>
            <span class="who-text"><span class="name">{{ t.name }}</span></span>
          </Link>
          <svg class="ic chev-r" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
        </li>
        <li id="new-team" class="form-row">
          <form class="add" @submit.prevent="team.post('/teams')">
            <input v-model="team.name" class="input" placeholder="Team name, e.g. Platform" aria-label="New team name" required maxlength="80" :aria-invalid="team.errors.name ? 'true' : undefined" />
            <button class="btn btn-line" type="submit" :disabled="team.processing">{{ team.processing ? 'Creating…' : 'Create team' }}</button>
            <p v-if="team.errors.name" class="error">{{ team.errors.name }}</p>
          </form>
        </li>
      </ul>
    </section>

    <TeamMembers :base="base" :user-id="user_uuid" :members="members" :invitations="invitations" :sent-link="invitation_link" :can-manage="can.manage_members" :can-invite="can.manage_members" remove-note="They also leave every team in this organization." />

    <TeamSettings
      :base="base" kind="organization" :name="organization.name"
      :can-rename="can.rename" :can-leave="true" :can-delete="can.delete"
      leave-note="You also leave its teams. The last owner cannot leave; make someone else an owner first."
      delete-note="Deletes every team in it, their members and pending invitations. This cannot be undone."
    />
  </AppLayout>
</template>
