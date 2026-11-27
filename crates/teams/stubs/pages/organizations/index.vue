<script setup lang="ts">
import { Link, useForm } from '@inertiajs/vue3'

defineProps<{ email: string; organizations: { id: number; name: string; role: string }[] }>()

const form = useForm({ name: '' })
const submit = () => form.post('/organizations', { onSuccess: () => form.reset() })
</script>

<template>
  <main>
    <h1>Organizations</h1>
    <p v-if="organizations.length === 0">You are not in an organization yet.</p>
    <ul>
      <li v-for="org in organizations" :key="org.id">
        <Link :href="`/organizations/${org.id}`">{{ org.name }}</Link> · {{ org.role }}
      </li>
    </ul>

    <h2>New organization</h2>
    <form @submit.prevent="submit">
      <label>
        Name
        <input v-model="form.name" required maxlength="80" />
      </label>
      <p v-if="form.errors.name">{{ form.errors.name }}</p>
      <button type="submit" :disabled="form.processing">Create organization</button>
    </form>
    <p><Link href="/teams">Teams</Link></p>
  </main>
</template>
