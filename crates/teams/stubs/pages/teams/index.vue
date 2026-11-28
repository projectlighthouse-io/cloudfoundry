<script setup lang="ts">
import type { TeamsPage } from '../../types/TeamsPage'
import type { Uuid } from '../../types/Uuid'
import { Link, useForm } from '@inertiajs/vue3'
import AppLayout from '../../layouts/AppLayout.vue'

defineProps<TeamsPage>()

const form = useForm({ name: '', organization: null as Uuid<'Org'> | null })
const submit = () => form.post('/teams', { onSuccess: () => form.reset() })
</script>

<template>
  <AppLayout title="Teams" :user="email">
    <header class="phead">
      <div class="phead-id">
        <div>
          <h1>Teams</h1>
          <p class="sub">Work together on shared resources. <Link href="/organizations">Organizations</Link> group teams under one owner.</p>
        </div>
      </div>
    </header>

    <section aria-labelledby="teams-h">
      <div class="strip"><h2 id="teams-h">Your teams</h2><span class="meta">{{ teams.length }}</span></div>
      <p v-if="teams.length === 0" class="empty-row">You are not on a team yet. Create one below, or open an invitation link.</p>
      <ul class="ledger">
        <li v-for="t in teams" :key="t.uuid">
          <Link class="who" :href="`/teams/${t.uuid}`">
            <span class="face" aria-hidden="true">{{ t.name.charAt(0).toUpperCase() }}</span>
            <span class="who-text">
              <span class="name">{{ t.name }}</span>
              <span class="meta">{{ t.organization ?? 'Standalone team' }}</span>
            </span>
          </Link>
          <span class="trail">
            <span v-if="t.current" class="state state-on">current</span>
            <span class="role">{{ t.role }}</span>
            <svg class="ic chev-r" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
          </span>
        </li>
        <li id="new-team" class="form-row">
          <form class="add" @submit.prevent="submit">
            <input v-model="form.name" class="input" placeholder="Team name, e.g. Platform" aria-label="New team name" required maxlength="80" :aria-invalid="form.errors.name ? 'true' : undefined" />
            <select v-if="organizations.length" v-model="form.organization" class="select" aria-label="Organization">
              <option :value="null">Standalone</option>
              <option v-for="o in organizations" :key="o.uuid" :value="o.uuid">{{ o.name }}</option>
            </select>
            <button class="btn btn-ink" type="submit" :disabled="form.processing">{{ form.processing ? 'Creating…' : 'Create team' }}</button>
            <p v-if="form.errors.name || form.errors.organization" class="error">{{ form.errors.name ?? form.errors.organization }}</p>
          </form>
        </li>
      </ul>
    </section>
    <div class="tail" />
  </AppLayout>
</template>
