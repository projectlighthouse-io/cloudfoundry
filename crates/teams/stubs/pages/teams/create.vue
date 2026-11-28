<script setup lang="ts">
import type { CreateTeamPage } from '../../types/CreateTeamPage'
import type { Uuid } from '../../types/Uuid'
import { useForm } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AppLayout from '../../layouts/AppLayout.vue'

defineProps<CreateTeamPage>()

const form = useForm({ name: '', organization: null as Uuid<'Org'> | null })
const submit = () => form.post('/teams')
</script>

<template>
  <AppLayout title="Create a team" :user="email">
    <header class="phead">
      <div class="phead-id">
        <div>
          <h1>Create a team</h1>
          <p class="sub">Teams share their work. Invite people once it exists, or open an invitation link someone sent you.</p>
        </div>
      </div>
    </header>

    <section aria-labelledby="create-h">
      <div class="strip"><h2 id="create-h">New team</h2></div>
      <form class="setting" @submit.prevent="submit">
        <div>
          <Field v-model="form.name" label="Team name" placeholder="Platform" :error="form.errors.name" autofocus />
          <label v-if="organizations.length" class="field">
            <span class="label">Organization</span>
            <select v-model="form.organization" class="select">
              <option :value="null">Standalone</option>
              <option v-for="o in organizations" :key="o.uuid" :value="o.uuid">{{ o.name }}</option>
            </select>
          </label>
        </div>
        <div><button class="btn btn-ink" type="submit" :disabled="form.processing">{{ form.processing ? 'Creating…' : 'Create team' }}</button></div>
      </form>
    </section>
    <div class="tail" />
  </AppLayout>
</template>
