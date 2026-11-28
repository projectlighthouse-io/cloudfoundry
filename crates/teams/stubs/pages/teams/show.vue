<script setup lang="ts">
import type { TeamPage } from '../../types/TeamPage'
import { Link, router } from '@inertiajs/vue3'
import TeamMembers from '../../components/TeamMembers.vue'
import TeamSettings from '../../components/TeamSettings.vue'
import AppLayout from '../../layouts/AppLayout.vue'

const props = defineProps<TeamPage>()

const base = `/teams/${props.team.uuid}`
const article = (r: string) => (/^[aeiou]/.test(r) ? 'an' : 'a')
</script>

<template>
  <AppLayout :title="team.name" :user="email">
    <p class="label-row"><Link href="/teams">Teams</Link><template v-if="organization"> / <Link :href="`/organizations/${organization.uuid}`">{{ organization.name }}</Link></template></p>
    <header class="phead">
      <div class="phead-id">
        <span class="tile" aria-hidden="true">{{ team.name.charAt(0).toUpperCase() }}</span>
        <div>
          <h1>{{ team.name }}</h1>
          <p class="sub">You are {{ article(role) }} {{ role }}<template v-if="via_org"> through <Link :href="`/organizations/${organization?.uuid}`">{{ organization?.name }}</Link></template>.</p>
        </div>
      </div>
      <div class="phead-act">
        <span v-if="current" class="state state-on">current team</span>
        <button v-else class="btn btn-line" type="button" @click="router.post(`${base}/switch`, {}, { preserveScroll: true })">Make current</button>
      </div>
    </header>

    <TeamMembers :base="base" :user-id="user_uuid" :members="members" :invitations="invitations" :sent-link="invitation_link" :can-manage="can.manage_members" :can-invite="can.invite" remove-note="They lose access to this team." />

    <TeamSettings
      :base="base" kind="team" :name="team.name"
      :can-rename="can.rename" :can-leave="!via_org" :can-delete="can.delete"
      leave-note="You lose access right away. The last owner cannot leave; make someone else an owner first."
      delete-note="Removes the team, its members and its pending invitations. This cannot be undone."
    />
  </AppLayout>
</template>
