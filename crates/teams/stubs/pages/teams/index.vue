<script setup lang="ts">
import { Link, useForm } from '@inertiajs/vue3'

defineProps<{
  email: string
  teams: { id: number; name: string; organization: string | null; role: string; current: boolean }[]
  organizations: { id: number; name: string }[]
}>()

const form = useForm({ name: '', organization_id: null as number | null })
const submit = () => form.post('/teams', { onSuccess: () => form.reset() })
</script>

<template>
  <main>
    <h1>Teams</h1>
    <p v-if="teams.length === 0">You are not on a team yet.</p>
    <ul>
      <li v-for="team in teams" :key="team.id">
        <Link :href="`/teams/${team.id}`">{{ team.name }}</Link>
        <span v-if="team.organization"> · {{ team.organization }}</span>
        · {{ team.role }}
        <strong v-if="team.current"> (current)</strong>
      </li>
    </ul>

    <h2>New team</h2>
    <form @submit.prevent="submit">
      <label>
        Name
        <input v-model="form.name" required maxlength="80" />
      </label>
      <p v-if="form.errors.name">{{ form.errors.name }}</p>
      <label v-if="organizations.length">
        Organization
        <select v-model="form.organization_id">
          <option :value="null">None (a team of its own)</option>
          <option v-for="org in organizations" :key="org.id" :value="org.id">{{ org.name }}</option>
        </select>
      </label>
      <button type="submit" :disabled="form.processing">Create team</button>
    </form>
    <p><Link href="/organizations">Organizations</Link></p>
  </main>
</template>
