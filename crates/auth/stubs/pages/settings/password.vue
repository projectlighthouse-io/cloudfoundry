<script setup lang="ts">
import { useForm } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AppLayout from '../../layouts/AppLayout.vue'

defineProps<{ email: string; status: string | null }>()

const form = useForm({ current_password: '', password: '', password_confirmation: '' })
const submit = () => form.post('/settings/password', { onFinish: () => form.reset() })
</script>

<template>
  <AppLayout title="Settings" :user="email">
    <header class="phead">
      <div class="phead-id">
        <div>
          <h1>Settings</h1>
          <p class="sub">Signed in as {{ email }}.</p>
        </div>
      </div>
    </header>

    <section aria-labelledby="password-h">
      <div class="strip"><h2 id="password-h">Password</h2></div>
      <p v-if="status" class="flash flash-ok" role="status">{{ status }}</p>
      <div class="setting">
        <div>
          <h3>Change password</h3>
          <p>Use at least 8 characters. Other devices that chose "remember me" will need to log in again.</p>
        </div>
        <div>
          <form @submit.prevent="submit">
            <Field v-model="form.current_password" label="Current password" type="password" autocomplete="current-password" :error="form.errors.current_password" />
            <Field v-model="form.password" label="New password" type="password" autocomplete="new-password" :error="form.errors.password" />
            <Field v-model="form.password_confirmation" label="Confirm new password" type="password" autocomplete="new-password" :error="form.errors.password_confirmation" />
            <button class="btn btn-ink" type="submit" :disabled="form.processing">{{ form.processing ? 'Saving…' : 'Change password' }}</button>
          </form>
        </div>
      </div>
    </section>
    <div class="tail" />
  </AppLayout>
</template>
