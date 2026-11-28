<script setup lang="ts">
import type { Uuid } from '../../types/Uuid'
import { Link, useForm } from '@inertiajs/vue3'
import AppLayout from '../../layouts/AppLayout.vue'

defineProps<{ email: string; organizations: { uuid: Uuid<'Org'>; name: string; role: string }[] }>()

const form = useForm({ name: '' })
const submit = () => form.post('/organizations', { onSuccess: () => form.reset() })
</script>

<template>
  <AppLayout title="Organizations" :user="email">
    <header class="phead">
      <div class="phead-id">
        <div>
          <h1>Organizations</h1>
          <p class="sub">An organization holds teams. Its owners and admins manage all of them.</p>
        </div>
      </div>
    </header>

    <section aria-labelledby="orgs-h">
      <div class="strip"><h2 id="orgs-h">Your organizations</h2><span class="meta">{{ organizations.length }}</span></div>
      <p v-if="organizations.length === 0" class="empty-row">You are not in an organization yet. Create one to group your teams.</p>
      <ul class="ledger">
        <li v-for="o in organizations" :key="o.uuid">
          <Link class="who" :href="`/organizations/${o.uuid}`">
            <span class="face" aria-hidden="true">{{ o.name.charAt(0).toUpperCase() }}</span>
            <span class="who-text"><span class="name">{{ o.name }}</span></span>
          </Link>
          <span class="trail">
            <span class="role">{{ o.role }}</span>
            <svg class="ic chev-r" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
          </span>
        </li>
        <li id="new-org" class="form-row">
          <form class="add" @submit.prevent="submit">
            <input v-model="form.name" class="input" placeholder="Organization name" aria-label="New organization name" required maxlength="80" :aria-invalid="form.errors.name ? 'true' : undefined" />
            <button class="btn btn-ink" type="submit" :disabled="form.processing">{{ form.processing ? 'Creating…' : 'Create organization' }}</button>
            <p v-if="form.errors.name" class="error">{{ form.errors.name }}</p>
          </form>
        </li>
      </ul>
    </section>
    <div class="tail" />
  </AppLayout>
</template>
