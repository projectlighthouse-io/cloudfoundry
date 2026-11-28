<script setup lang="ts">
import type { SelectTeamPage } from '../../types/SelectTeamPage'
import type { TeamItem } from '../../types/TeamItem'
import { Link, router } from '@inertiajs/vue3'
import AppLayout from '../../layouts/AppLayout.vue'

defineProps<SelectTeamPage>()

const pick = (t: TeamItem) => router.post(`/teams/${t.uuid}/switch`)
</script>

<template>
  <AppLayout title="Choose a team" :user="email">
    <header class="phead">
      <div class="phead-id">
        <div>
          <h1>Choose a team</h1>
          <p class="sub">Pick the team to work in. You can switch at any time from Teams.</p>
        </div>
      </div>
    </header>

    <section aria-labelledby="pick-h">
      <div class="strip"><h2 id="pick-h">Your teams</h2><span class="meta">{{ teams.length }}</span></div>
      <ul class="ledger">
        <li v-for="t in teams" :key="t.uuid">
          <span class="who">
            <span class="face" aria-hidden="true">{{ t.name.charAt(0).toUpperCase() }}</span>
            <span class="who-text">
              <span class="name">{{ t.name }}</span>
              <span class="meta">{{ t.organization ?? 'Standalone team' }} · {{ t.role }}</span>
            </span>
          </span>
          <span class="trail">
            <button class="btn btn-ink btn-sm" type="button" @click="pick(t)">{{ t.current ? 'Continue' : 'Use this team' }}</button>
          </span>
        </li>
      </ul>
      <p class="hint">Or <Link href="/teams/create">create a new team</Link>.</p>
    </section>
    <div class="tail" />
  </AppLayout>
</template>
